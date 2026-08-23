use super::grid::{GridSnapshot, GridState};
use super::msgpack::{self, Value, array, map};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RPC_TIMEOUT: Duration = Duration::from_secs(3);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ACTIONS: usize = 8;
const MIN_API_LEVEL: u64 = 11;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableIdentity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Default)]
pub struct ReaderShared {
    pub latest_grid: Option<Arc<GridSnapshot>>,
    pub document_dirty: bool,
    pub pending_actions: VecDeque<String>,
    pub error: Option<String>,
}

struct RpcResponse {
    id: u64,
    error: Value,
    result: Value,
}

pub struct RpcProcess {
    child: Option<Child>,
    input: Option<ChildStdin>,
    responses: Receiver<RpcResponse>,
    shared: Arc<Mutex<ReaderShared>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    temp_dir: Option<PathBuf>,
    next_id: u64,
    pub channel_id: u64,
}

impl RpcProcess {
    pub fn start(
        executable: &Path,
        source: &str,
        synthetic_name: &str,
        language: &str,
        width: u16,
        height: u16,
    ) -> Result<Self, String> {
        let executable = fs::canonicalize(executable)
            .map_err(|error| format!("cannot resolve Neovim executable: {error}"))?;
        let identity = trusted_identity(&executable)?;
        validate_version(&executable)?;
        if trusted_identity(&executable)? != identity {
            return Err("Neovim executable changed after version probe".into());
        }
        let temp_dir = create_temp_dir()?;
        let mut command = Command::new(&executable);
        command
            .arg("--clean")
            .arg("--embed")
            .current_dir(&temp_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group_and_limits(&mut command, None);
        if trusted_identity(&executable)? != identity {
            let _ = fs::remove_dir(&temp_dir);
            return Err("Neovim executable changed before spawn".into());
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start Neovim: {error}"))?;
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("INTERVIEW_TUTOR_TEST_NEOVIM_PID_FILE")
            && let Err(error) = fs::write(&path, format!("{}\n", child.id()))
        {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            return Err(format!("cannot write Neovim test PID file: {error}"));
        }
        let input = child.stdin.take().ok_or("Neovim stdin pipe unavailable")?;
        let output = child
            .stdout
            .take()
            .ok_or("Neovim stdout pipe unavailable")?;
        let error = child
            .stderr
            .take()
            .ok_or("Neovim stderr pipe unavailable")?;
        let (response_sender, responses) = mpsc::sync_channel(2);
        let shared = Arc::new(Mutex::new(ReaderShared::default()));
        let reader_shared = Arc::clone(&shared);
        let stdout_reader = thread::spawn(move || read_rpc(output, response_sender, reader_shared));
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_STDERR_BYTES)));
        let stderr_reader_buffer = Arc::clone(&stderr);
        let stderr_reader = thread::spawn(move || drain_stderr(error, stderr_reader_buffer));
        let mut process = Self {
            child: Some(child),
            input: Some(input),
            responses,
            shared,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            stderr,
            temp_dir: Some(temp_dir),
            next_id: 1,
            channel_id: 0,
        };
        if let Err(error) = process.initialize(source, synthetic_name, language, width, height) {
            let cleanup = process.shutdown();
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; Neovim cleanup failed: {cleanup}"),
            });
        }
        Ok(process)
    }

    pub fn shared(&self) -> Arc<Mutex<ReaderShared>> {
        Arc::clone(&self.shared)
    }

    fn initialize(
        &mut self,
        source: &str,
        synthetic_name: &str,
        language: &str,
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        let api = self.call_with_timeout("nvim_get_api_info", Vec::new(), STARTUP_TIMEOUT)?;
        let api = api
            .as_array()
            .ok_or("Neovim API metadata is not an array")?;
        self.channel_id = api
            .first()
            .and_then(Value::as_u64)
            .ok_or("Neovim API omitted channel id")?;
        validate_api(api.get(1).ok_or("Neovim API omitted metadata")?)?;
        self.call(
            "nvim_set_client_info",
            vec![
                Value::String("interview-tutor".into()),
                map([
                    ("major", Value::Unsigned(0)),
                    ("minor", Value::Unsigned(1)),
                    ("patch", Value::Unsigned(0)),
                ]),
                Value::String("ui".into()),
                Value::Map(Vec::new()),
                Value::Map(Vec::new()),
            ],
        )?;
        self.call_with_timeout(
            "nvim_ui_attach",
            vec![
                Value::Unsigned(u64::from(width.max(1))),
                Value::Unsigned(u64::from(height.max(1))),
                map([
                    ("rgb", Value::Bool(true)),
                    ("ext_linegrid", Value::Bool(true)),
                    ("ext_multigrid", Value::Bool(false)),
                    ("ext_cmdline", Value::Bool(false)),
                    ("ext_messages", Value::Bool(false)),
                    ("ext_popupmenu", Value::Bool(false)),
                    ("ext_tabline", Value::Bool(false)),
                    ("stdin_tty", Value::Bool(false)),
                    ("stdout_tty", Value::Bool(false)),
                    ("term_name", Value::String("interview-tutor".into())),
                    ("term_colors", Value::Unsigned(256)),
                ]),
            ],
            STARTUP_TIMEOUT,
        )?;
        let script = r#"
local source, name, filetype, channel = ...
local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_set_current_buf(buf)
vim.api.nvim_buf_set_name(buf, name)
local has_eol = #source > 0 and source:sub(-1) == "\n"
local body = has_eol and source:sub(1, -2) or source
local lines = vim.split(body, "\n", { plain = true })
if #lines == 0 then lines = { "" } end
vim.api.nvim_buf_set_lines(buf, 0, -1, true, lines)
local bo = vim.bo[buf]
bo.buftype = "acwrite"
bo.bufhidden = "hide"
bo.swapfile = false
bo.undofile = false
bo.modeline = false
bo.fixendofline = false
bo.endofline = has_eol
bo.filetype = filetype
vim.o.shadafile = "NONE"
vim.o.clipboard = ""
local function tutor(action)
  vim.rpcnotify(channel, "tutor_action", action)
end
vim.api.nvim_buf_create_user_command(buf, "TutorTest", function() tutor("test") end, {})
vim.api.nvim_buf_create_user_command(buf, "TutorSubmit", function() tutor("submit") end, {})
vim.api.nvim_buf_create_user_command(buf, "TutorBack", function() tutor("back") end, {})
vim.api.nvim_buf_create_user_command(buf, "TutorCollapse", function() tutor("collapse") end, {})
vim.api.nvim_buf_create_user_command(buf, "TutorQuit", function() tutor("quit") end, {})
vim.api.nvim_buf_create_user_command(buf, "TutorHint", function() tutor("hint") end, {})
for lhs, action in pairs({ ["<Space>t"] = "test", ["<Space>s"] = "submit", ["<Space>b"] = "back", ["<Space>c"] = "collapse", ["<Space>q"] = "quit", ["<Space>h"] = "hint" }) do
  vim.keymap.set("n", lhs, function() tutor(action) end, { buffer = buf, silent = true, nowait = true })
end
for _, mode in ipairs({ "n", "i", "x" }) do
  vim.keymap.set(mode, "<F5>", function() tutor("test") end, { buffer = buf, silent = true, nowait = true })
  vim.keymap.set(mode, "<F9>", function() tutor("submit") end, { buffer = buf, silent = true, nowait = true })
  vim.keymap.set(mode, "<C-s>", function() tutor("test") end, { buffer = buf, silent = true, nowait = true })
end
vim.api.nvim_create_autocmd("BufWriteCmd", { buffer = buf, callback = function() tutor("test") end })
vim.bo[buf].modified = false
return buf
"#;
        let buffer = self.call_with_timeout(
            "nvim_exec_lua",
            vec![
                Value::String(script.into()),
                array([
                    Value::String(source.into()),
                    Value::String(synthetic_name.into()),
                    Value::String(language.into()),
                    Value::Unsigned(self.channel_id),
                ]),
            ],
            STARTUP_TIMEOUT,
        )?;
        self.call(
            "nvim_buf_attach",
            vec![buffer, Value::Bool(false), Value::Map(Vec::new())],
        )?;
        self.call("nvim_command", vec![Value::String("redraw!".into())])?;
        Ok(())
    }

    pub fn feed_key(&mut self, key: &str) -> Result<(), String> {
        let accepted = self
            .call("nvim_input", vec![Value::String(key.into())])?
            .as_u64()
            .ok_or("Neovim nvim_input returned an invalid byte count")?;
        if accepted != key.len() as u64 {
            return Err("Neovim accepted only part of an input key sequence".into());
        }
        Ok(())
    }

    pub fn paste(&mut self, text: String) -> Result<(), String> {
        if text.len() > crate::editor::MAX_DOCUMENT_BYTES {
            return Err("paste exceeds editor document bound".into());
        }
        self.call(
            "nvim_paste",
            vec![Value::String(text), Value::Bool(false), Value::Integer(-1)],
        )?;
        Ok(())
    }

    pub fn mouse(
        &mut self,
        button: String,
        action: String,
        modifiers: String,
        row: u16,
        column: u16,
    ) -> Result<(), String> {
        self.call(
            "nvim_input_mouse",
            vec![
                Value::String(button),
                Value::String(action),
                Value::String(modifiers),
                Value::Unsigned(0),
                Value::Unsigned(u64::from(row)),
                Value::Unsigned(u64::from(column)),
            ],
        )?;
        Ok(())
    }

    pub fn resize(&mut self, width: u16, height: u16) -> Result<(), String> {
        self.call(
            "nvim_ui_try_resize",
            vec![
                Value::Unsigned(u64::from(width.max(1))),
                Value::Unsigned(u64::from(height.max(1))),
            ],
        )?;
        Ok(())
    }

    pub fn snapshot(&mut self) -> Result<(String, String, u64), String> {
        let script = r#"
local buf = vim.api.nvim_get_current_buf()
return {
  lines = vim.api.nvim_buf_get_lines(buf, 0, -1, true),
  eol = vim.bo[buf].endofline,
  mode = vim.api.nvim_get_mode().mode,
  tick = vim.api.nvim_buf_get_changedtick(buf),
}
"#;
        let result = self.call(
            "nvim_exec_lua",
            vec![Value::String(script.into()), Value::Array(Vec::new())],
        )?;
        let lines = result
            .map_get("lines")
            .and_then(Value::as_array)
            .ok_or("Neovim snapshot omitted lines")?;
        if lines.len() > crate::editor::MAX_DOCUMENT_LINES {
            return Err(format!(
                "document exceeds {} lines",
                crate::editor::MAX_DOCUMENT_LINES
            ));
        }
        let mut text = String::new();
        for (index, line) in lines.iter().enumerate() {
            let line = line
                .as_str()
                .ok_or("Neovim snapshot line is not UTF-8 text")?;
            if index > 0 {
                text.push('\n');
            }
            text.push_str(line);
            if text.len() > crate::editor::MAX_DOCUMENT_BYTES {
                return Err(format!(
                    "document exceeds {} bytes",
                    crate::editor::MAX_DOCUMENT_BYTES
                ));
            }
        }
        if result.map_get("eol").and_then(Value::as_bool) == Some(true) {
            text.push('\n');
        }
        crate::editor::validate_document(&text)?;
        let mode = result
            .map_get("mode")
            .and_then(Value::as_str)
            .ok_or("Neovim snapshot omitted mode")?
            .to_string();
        let tick = result
            .map_get("tick")
            .and_then(Value::as_u64)
            .ok_or("Neovim snapshot omitted changedtick")?;
        Ok((text, mode, tick))
    }

    fn call(&mut self, method: &str, parameters: Vec<Value>) -> Result<Value, String> {
        self.call_with_timeout(method, parameters, RPC_TIMEOUT)
    }

    fn call_with_timeout(
        &mut self,
        method: &str,
        parameters: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Neovim RPC request id overflow")?;
        let request = array([
            Value::Unsigned(0),
            Value::Unsigned(id),
            Value::String(method.into()),
            Value::Array(parameters),
        ]);
        let bytes = msgpack::encode(&request)?;
        let input = self.input.as_mut().ok_or("Neovim RPC input is closed")?;
        input
            .write_all(&bytes)
            .map_err(|error| format!("cannot write Neovim RPC request: {error}"))?;
        input
            .flush()
            .map_err(|error| format!("cannot flush Neovim RPC request: {error}"))?;
        match self.responses.recv_timeout(timeout) {
            Ok(response) if response.id == id => {
                if response.error == Value::Nil {
                    Ok(response.result)
                } else {
                    Err(format!(
                        "Neovim {method} failed: {}",
                        bounded_value(&response.error)
                    ))
                }
            }
            Ok(response) => Err(format!(
                "Neovim RPC response id mismatch: expected {id}, got {}",
                response.id
            )),
            Err(RecvTimeoutError::Timeout) => Err(format!("Neovim {method} timed out")),
            Err(RecvTimeoutError::Disconnected) => Err(self
                .reader_error()
                .unwrap_or_else(|| format!("Neovim disconnected during {method}"))),
        }
    }

    fn reader_error(&self) -> Option<String> {
        self.shared
            .lock()
            .expect("Neovim reader lock")
            .error
            .clone()
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        if self.child.is_none() {
            return Ok(());
        }
        self.input.take();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let mut error = None;
        if let Some(child) = self.child.as_mut() {
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Ok(None) => {
                        unsafe {
                            libc::kill(-(child.id() as i32), libc::SIGTERM);
                        }
                        thread::sleep(Duration::from_millis(100));
                        if child.try_wait().ok().flatten().is_none() {
                            unsafe {
                                libc::kill(-(child.id() as i32), libc::SIGKILL);
                            }
                        }
                        if let Err(wait_error) = child.wait() {
                            error = Some(format!("cannot reap Neovim: {wait_error}"));
                        }
                        break;
                    }
                    Err(wait_error) => {
                        error = Some(format!("cannot inspect Neovim exit: {wait_error}"));
                        break;
                    }
                }
            }
        }
        self.child = None;
        if let Some(reader) = self.stdout_reader.take() {
            if reader.join().is_err() && error.is_none() {
                error = Some("Neovim stdout reader panicked".into());
            }
        }
        if let Some(reader) = self.stderr_reader.take() {
            if reader.join().is_err() && error.is_none() {
                error = Some("Neovim stderr reader panicked".into());
            }
        }
        if let Some(temp_dir) = self.temp_dir.take()
            && let Err(remove_error) = fs::remove_dir(&temp_dir)
            && error.is_none()
        {
            error = Some(format!(
                "cannot remove Neovim temporary directory: {remove_error}"
            ));
        }
        error.map_or(Ok(()), Err)
    }

    pub fn stderr_tail(&self) -> String {
        String::from_utf8_lossy(
            &self
                .stderr
                .lock()
                .expect("Neovim stderr lock")
                .iter()
                .copied()
                .collect::<Vec<_>>(),
        )
        .into_owned()
    }
}

impl Drop for RpcProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn read_rpc(
    mut output: impl Read,
    response_sender: SyncSender<RpcResponse>,
    shared: Arc<Mutex<ReaderShared>>,
) {
    let mut grid = GridState::default();
    loop {
        let value = match msgpack::decode(&mut output) {
            Ok(value) => value,
            Err(error) => {
                shared.lock().expect("Neovim reader lock").error = Some(error);
                break;
            }
        };
        let Some(message) = value.as_array() else {
            shared.lock().expect("Neovim reader lock").error =
                Some("Neovim RPC message is not an array".into());
            break;
        };
        match message.first().and_then(Value::as_u64) {
            Some(1) if message.len() == 4 => {
                let Some(id) = message.get(1).and_then(Value::as_u64) else {
                    shared.lock().expect("Neovim reader lock").error =
                        Some("Neovim RPC response id is invalid".into());
                    break;
                };
                if response_sender
                    .send(RpcResponse {
                        id,
                        error: message[2].clone(),
                        result: message[3].clone(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Some(2) if message.len() == 3 => {
                let Some(method) = message.get(1).and_then(Value::as_str) else {
                    shared.lock().expect("Neovim reader lock").error =
                        Some("Neovim RPC notification method is invalid".into());
                    break;
                };
                let Some(parameters) = message.get(2).and_then(Value::as_array) else {
                    shared.lock().expect("Neovim reader lock").error =
                        Some("Neovim RPC notification parameters are invalid".into());
                    break;
                };
                let result = match method {
                    "redraw" => grid.apply_redraw(&message[2]).map(|snapshot| {
                        if let Some(snapshot) = snapshot {
                            shared.lock().expect("Neovim reader lock").latest_grid = Some(snapshot);
                        }
                    }),
                    "nvim_buf_lines_event" | "nvim_buf_changedtick_event" => {
                        shared.lock().expect("Neovim reader lock").document_dirty = true;
                        Ok(())
                    }
                    "nvim_buf_detach_event" => Err("Neovim detached the solution buffer".into()),
                    "tutor_action" if parameters.len() == 1 => {
                        let action = parameters[0]
                            .as_str()
                            .ok_or_else(|| "Neovim Tutor action is invalid".to_string())
                            .map(str::to_string);
                        action.and_then(|action| {
                            let mut shared = shared.lock().expect("Neovim reader lock");
                            if shared.pending_actions.len() == MAX_ACTIONS {
                                Err("Neovim Tutor action queue exceeds bound".into())
                            } else {
                                shared.pending_actions.push_back(action);
                                shared.document_dirty = true;
                                Ok(())
                            }
                        })
                    }
                    "nvim_error_event" => Err("Neovim reported an asynchronous API error".into()),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    shared.lock().expect("Neovim reader lock").error = Some(error);
                    break;
                }
            }
            Some(0) => {
                shared.lock().expect("Neovim reader lock").error =
                    Some("Neovim sent an unsupported RPC request".into());
                break;
            }
            _ => {
                shared.lock().expect("Neovim reader lock").error =
                    Some("Neovim RPC envelope is malformed".into());
                break;
            }
        }
    }
}

fn drain_stderr(mut error: impl Read, ring: Arc<Mutex<VecDeque<u8>>>) {
    let mut chunk = [0_u8; 4096];
    while let Ok(count) = error.read(&mut chunk) {
        if count == 0 {
            break;
        }
        let mut ring = ring.lock().expect("Neovim stderr lock");
        for byte in &chunk[..count] {
            if ring.len() == MAX_STDERR_BYTES {
                ring.pop_front();
            }
            ring.push_back(*byte);
        }
    }
}

fn validate_api(metadata: &Value) -> Result<(), String> {
    let version = metadata
        .map_get("version")
        .ok_or("Neovim metadata omitted version")?;
    let api_level = version
        .map_get("api_level")
        .and_then(Value::as_u64)
        .ok_or("Neovim metadata omitted API level")?;
    let api_compatible = version
        .map_get("api_compatible")
        .and_then(Value::as_u64)
        .ok_or("Neovim metadata omitted compatible API level")?;
    if version.map_get("api_prerelease").and_then(Value::as_bool) != Some(false) {
        return Err("prerelease Neovim APIs are unsupported".into());
    }
    if api_level < MIN_API_LEVEL || api_compatible > MIN_API_LEVEL {
        return Err(format!(
            "Neovim API {api_level} is incompatible with required level {MIN_API_LEVEL}"
        ));
    }
    let ui_options = metadata
        .map_get("ui_options")
        .and_then(Value::as_array)
        .ok_or("Neovim metadata omitted UI options")?;
    if !ui_options
        .iter()
        .any(|option| option.as_str() == Some("ext_linegrid"))
    {
        return Err("Neovim does not support ext_linegrid".into());
    }
    Ok(())
}

pub fn resolve_executable(requested: Option<&Path>) -> Result<PathBuf, String> {
    let candidate = match requested {
        Some(path) => find_executable(path)?,
        None => match std::env::var_os("INTERVIEW_TUTOR_NEOVIM_EXECUTABLE") {
            Some(value) if value.is_empty() => {
                return Err("INTERVIEW_TUTOR_NEOVIM_EXECUTABLE must not be empty".into());
            }
            Some(value) => find_executable(Path::new(&value))?,
            None => find_executable(Path::new("nvim"))?,
        },
    };
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("cannot resolve Neovim executable: {error}"))?;
    trusted_identity(&canonical)?;
    Ok(canonical)
}

fn find_executable(path: &Path) -> Result<PathBuf, String> {
    if path.components().count() > 1 {
        return Ok(path.to_path_buf());
    }
    let search = std::env::var_os("PATH").ok_or("PATH is unavailable while locating Neovim")?;
    for directory in std::env::split_paths(&search) {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("cannot find Neovim executable: {}", path.display()))
}

fn trusted_identity(path: &Path) -> Result<ExecutableIdentity, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot inspect Neovim executable: {error}"))?;
    if !metadata.is_file() {
        return Err("Neovim executable must be a regular file".into());
    }
    let effective_uid = unsafe { libc::geteuid() };
    if metadata.uid() != effective_uid && metadata.uid() != 0 {
        return Err("Neovim executable must be owned by the effective user or root".into());
    }
    if metadata.mode() & 0o022 != 0 {
        return Err("Neovim executable must not be group/world writable".into());
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err("Neovim executable is not executable".into());
    }
    Ok(ExecutableIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode(),
        size: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

fn validate_version(executable: &Path) -> Result<(), String> {
    const MAX_VERSION_BYTES: u64 = 64 * 1024;
    let temp_dir = create_temp_dir()?;
    let result = (|| {
        let stdout_path = temp_dir.join("stdout");
        let stderr_path = temp_dir.join("stderr");
        let stdout = create_probe_file(&stdout_path)?;
        let stderr = create_probe_file(&stderr_path)?;
        let mut command = Command::new(executable);
        command
            .arg("--version")
            .current_dir(&temp_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        configure_process_group_and_limits(&mut command, Some(MAX_VERSION_BYTES));
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot probe Neovim version: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGTERM);
                    }
                    thread::sleep(Duration::from_millis(100));
                    if child.try_wait().ok().flatten().is_none() {
                        unsafe {
                            libc::kill(-(child.id() as i32), libc::SIGKILL);
                        }
                    }
                    let _ = child.wait();
                    return Err("Neovim version probe timed out".into());
                }
                Err(error) => return Err(format!("cannot inspect Neovim version probe: {error}")),
            }
        };
        if !status.success() {
            return Err("Neovim version probe failed".into());
        }
        let stdout_size = fs::metadata(&stdout_path)
            .map_err(|error| format!("cannot inspect Neovim version stdout: {error}"))?
            .len();
        let stderr_size = fs::metadata(&stderr_path)
            .map_err(|error| format!("cannot inspect Neovim version stderr: {error}"))?
            .len();
        if stdout_size.saturating_add(stderr_size) > MAX_VERSION_BYTES {
            return Err("Neovim version output exceeds 64 KiB".into());
        }
        let mut stdout = String::new();
        File::open(&stdout_path)
            .and_then(|mut file| file.read_to_string(&mut stdout))
            .map_err(|error| format!("cannot read Neovim version output: {error}"))?;
        let first = stdout.lines().next().unwrap_or("");
        let version = first
            .strip_prefix("NVIM v")
            .ok_or("Neovim version output is incompatible")?;
        let mut components = version.split('.');
        let major = components
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or("Neovim major version is invalid")?;
        let minor = components
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or("Neovim minor version is invalid")?;
        if major >= 2 || (major == 0 && minor < 9) {
            return Err(format!(
                "Neovim {version} is unsupported; require >=0.9 and <2.0"
            ));
        }
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&temp_dir)
        .map_err(|error| format!("cannot remove Neovim version directory: {error}"));
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

fn create_probe_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("cannot create Neovim version output: {error}"))
}

fn create_temp_dir() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before epoch")?
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "interview-neovim-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&path)
        .map_err(|error| format!("cannot create Neovim temporary directory: {error}"))?;
    Ok(path)
}

fn configure_process_group_and_limits(command: &mut Command, file_size: Option<u64>) {
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, value) in [
                (libc::RLIMIT_AS, 512_u64 * 1024 * 1024),
                (libc::RLIMIT_NOFILE, 128),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(value) = file_size {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

fn bounded_value(value: &Value) -> String {
    let text = format!("{value:?}");
    text.chars().take(1024).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_neovim_round_trips_exact_source_and_reports_api() {
        let executable = resolve_executable(None).unwrap();
        for source in [
            "",
            "x",
            "x\n",
            "\n",
            "\n\n",
            "x\r\ny",
            "a\0b",
            "e\u{301}界👩‍💻",
        ] {
            let mut process =
                RpcProcess::start(&executable, source, "interview://p.py", "python", 40, 12)
                    .unwrap();
            let (round_trip, mode, _) = process.snapshot().unwrap();
            assert_eq!(round_trip, source);
            assert!(mode.starts_with('n'));
            process.shutdown().unwrap();
        }
    }

    #[test]
    fn executable_trust_and_version_fail_closed() {
        let root = create_temp_dir().unwrap();
        let executable = root.join("fake-nvim");
        fs::write(&executable, "#!/bin/sh\nprintf 'NVIM v2.0.0\\n'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o722)).unwrap();
        assert!(
            resolve_executable(Some(&executable))
                .unwrap_err()
                .contains("group/world writable")
        );
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            RpcProcess::start(&executable, "x\n", "interview://bad.py", "python", 20, 8,)
                .err()
                .expect("unsupported version fails")
                .contains("unsupported")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clean_embed_skips_user_initialization() {
        let executable = resolve_executable(None).unwrap();
        let root = create_temp_dir().unwrap();
        let config = root.join("nvim");
        fs::create_dir(&config).unwrap();
        let marker = root.join("loaded-marker");
        fs::write(
            config.join("init.lua"),
            format!("vim.fn.writefile({{ 'loaded' }}, {marker:?})\n"),
        )
        .unwrap();
        let wrapper = root.join("nvim-wrapper");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport XDG_CONFIG_HOME={}\nexec {} \"$@\"\n",
                root.display(),
                executable.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut process =
            RpcProcess::start(&wrapper, "x\n", "interview://clean.py", "python", 20, 8).unwrap();
        assert!(!marker.exists());
        process.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_neovim_executes_composed_vim_commands_and_tutor_mappings() {
        let executable = resolve_executable(None).unwrap();
        let mut process = RpcProcess::start(
            &executable,
            "one two\nthree\n",
            "interview://p.py",
            "python",
            50,
            14,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(20));
        let grid = process
            .shared()
            .lock()
            .unwrap()
            .latest_grid
            .clone()
            .expect("Neovim published a flushed grid");
        assert!(grid.cells.iter().any(|cell| cell.text == "o"));
        let feed = |process: &mut RpcProcess, keys: &str| {
            process.feed_key(keys).unwrap();
            thread::sleep(Duration::from_millis(10));
        };
        feed(&mut process, "gg0dw");
        assert_eq!(process.snapshot().unwrap().0, "two\nthree\n");
        feed(&mut process, "u");
        assert_eq!(process.snapshot().unwrap().0, "one two\nthree\n");
        feed(&mut process, "G$A!<Esc>");
        feed(&mut process, ".");
        assert_eq!(process.snapshot().unwrap().0, "one two\nthree!!\n");
        feed(&mut process, "gg0\"ayyGp");
        assert_eq!(process.snapshot().unwrap().0, "one two\nthree!!\none two\n");
        feed(&mut process, "/three<CR>");
        assert!(process.snapshot().unwrap().1.starts_with('n'));
        feed(&mut process, "<Space>t");
        let shared = process.shared();
        let action = shared.lock().unwrap().pending_actions.pop_front();
        assert_eq!(action.as_deref(), Some("test"));
        process.shutdown().unwrap();
    }
}
