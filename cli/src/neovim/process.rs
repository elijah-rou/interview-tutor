use super::grid::{GridSnapshot, GridState};
use super::msgpack::{self, Value, array, map};
use std::collections::VecDeque;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RPC_TIMEOUT: Duration = Duration::from_secs(3);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PROCESS_TERM_GRACE: Duration = Duration::from_millis(100);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ORIGIN_EVENTS: usize = 32;
const MIN_API_LEVEL: u64 = 11;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct ValidatedExecutable {
    file: File,
    display_path: PathBuf,
}

#[derive(Clone)]
struct Cancellation {
    local: Arc<AtomicBool>,
    external: Arc<AtomicBool>,
}

impl Cancellation {
    fn new(external: Arc<AtomicBool>) -> Self {
        Self {
            local: Arc::new(AtomicBool::new(false)),
            external,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.local.load(Ordering::Acquire) || self.external.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.local.store(true, Ordering::Release);
    }
}

struct BoundedThread {
    name: &'static str,
    join: Option<JoinHandle<()>>,
    done: Receiver<()>,
}

impl BoundedThread {
    fn spawn(
        name: &'static str,
        operation: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        let (done_sender, done) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name(format!("neovim-{name}"))
            .spawn(move || {
                operation();
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("cannot start Neovim {name} reader: {error}"))?;
        Ok(Self {
            name,
            join: Some(join),
            done,
        })
    }

    fn join_before(&mut self, deadline: Instant) -> Result<(), String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match self.done.recv_timeout(remaining) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                if self.join.take().is_some_and(|join| join.join().is_err()) {
                    Err(format!("Neovim {} reader panicked", self.name))
                } else {
                    Ok(())
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                self.join.take();
                Err(format!(
                    "Neovim {} reader did not stop before deadline",
                    self.name
                ))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub buffer: Value,
    pub text: Option<String>,
    pub mode: String,
    pub changedtick: u64,
}

#[derive(Clone, Debug)]
pub enum OriginEvent {
    Dirty { buffer: Value, changedtick: u64 },
    Action { action: String, snapshot: Snapshot },
    Barrier { id: u64, snapshot: Snapshot },
}

#[derive(Default)]
pub struct ReaderShared {
    pub latest_grid: Option<Arc<GridSnapshot>>,
    pub solution_buffer: Option<u64>,
    pub origin_events: VecDeque<OriginEvent>,
    pub multipart_changedtick: Option<u64>,
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
    stdout_reader: Option<BoundedThread>,
    stderr_reader: Option<BoundedThread>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    temp_dir: Option<PathBuf>,
    process_group: i32,
    cancellation: Cancellation,
    next_id: u64,
    pub channel_id: u64,
    buffer: Value,
}

impl RpcProcess {
    #[cfg(test)]
    pub fn start(
        executable: &Path,
        source: &str,
        synthetic_name: &str,
        language: &str,
        width: u16,
        height: u16,
    ) -> Result<Self, String> {
        Self::start_with_cancellation(
            executable,
            source,
            synthetic_name,
            language,
            width,
            height,
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub(super) fn start_with_cancellation(
        executable: &Path,
        source: &str,
        synthetic_name: &str,
        language: &str,
        width: u16,
        height: u16,
        external_cancellation: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if external_cancellation.load(Ordering::Acquire) {
            return Err("Neovim startup was cancelled".into());
        }
        let canonical = fs::canonicalize(executable)
            .map_err(|error| format!("cannot resolve Neovim executable: {error}"))?;
        let executable = ValidatedExecutable::open(&canonical)?;
        validate_version(&executable, &external_cancellation)?;
        let temp_dir = create_temp_dir()?;
        let mut command = executable.command();
        command
            .arg("--clean")
            .arg("--cmd")
            .arg("let g:loaded_clipboard_provider = 0")
            .arg("--embed")
            .current_dir(&temp_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group_and_limits(&mut command, None, Some(executable.raw_fd()));
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let cleanup = remove_temp_tree(&temp_dir);
                return Err(with_cleanup_error(
                    format!("cannot start Neovim: {error}"),
                    cleanup,
                ));
            }
        };
        let process_group = i32::try_from(child.id()).expect("Linux process IDs fit i32");
        let setup = (|| {
            #[cfg(debug_assertions)]
            if let Some(path) = std::env::var_os("INTERVIEW_TUTOR_TEST_NEOVIM_PID_FILE") {
                fs::write(&path, format!("{}\n", child.id()))
                    .map_err(|error| format!("cannot write Neovim test PID file: {error}"))?;
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
            set_nonblocking(input.as_raw_fd())?;
            set_nonblocking(output.as_raw_fd())?;
            set_nonblocking(error.as_raw_fd())?;
            Ok::<_, String>((input, output, error))
        })();
        let (input, output, error) = match setup {
            Ok(pipes) => pipes,
            Err(error) => {
                let process_cleanup = terminate_process_group(
                    &mut child,
                    process_group,
                    Instant::now() + SHUTDOWN_TIMEOUT,
                );
                let cleanup = process_cleanup.and_then(|_| remove_temp_tree(&temp_dir));
                return Err(with_cleanup_error(error, cleanup));
            }
        };

        let cancellation = Cancellation::new(external_cancellation);
        let (response_sender, responses) = mpsc::sync_channel(2);
        let shared = Arc::new(Mutex::new(ReaderShared::default()));
        let reader_shared = Arc::clone(&shared);
        let stdout_cancellation = cancellation.clone();
        let mut stdout_reader = match BoundedThread::spawn("stdout", move || {
            read_rpc(
                InterruptibleReader::new(output, stdout_cancellation.clone()),
                response_sender,
                reader_shared,
                stdout_cancellation,
            )
        }) {
            Ok(reader) => reader,
            Err(error) => {
                cancellation.cancel();
                let process_cleanup = terminate_process_group(
                    &mut child,
                    process_group,
                    Instant::now() + SHUTDOWN_TIMEOUT,
                );
                let cleanup = process_cleanup.and_then(|_| remove_temp_tree(&temp_dir));
                return Err(with_cleanup_error(error, cleanup));
            }
        };
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_STDERR_BYTES)));
        let stderr_reader_buffer = Arc::clone(&stderr);
        let stderr_cancellation = cancellation.clone();
        let stderr_reader = match BoundedThread::spawn("stderr", move || {
            drain_stderr(
                InterruptibleReader::new(error, stderr_cancellation),
                stderr_reader_buffer,
            )
        }) {
            Ok(reader) => reader,
            Err(error) => {
                cancellation.cancel();
                let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
                let mut cleanup_errors = Vec::new();
                if let Err(cleanup) = terminate_process_group(&mut child, process_group, deadline) {
                    cleanup_errors.push(cleanup);
                }
                if let Err(cleanup) = stdout_reader.join_before(deadline) {
                    cleanup_errors.push(cleanup);
                }
                if let Err(cleanup) = remove_temp_tree(&temp_dir) {
                    cleanup_errors.push(cleanup);
                }
                let cleanup = if cleanup_errors.is_empty() {
                    Ok(())
                } else {
                    Err(cleanup_errors.join("; "))
                };
                return Err(with_cleanup_error(error, cleanup));
            }
        };
        let mut process = Self {
            child: Some(child),
            input: Some(input),
            responses,
            shared,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            stderr,
            temp_dir: Some(temp_dir),
            process_group,
            cancellation,
            next_id: 1,
            channel_id: 0,
            buffer: Value::Nil,
        };
        if let Err(error) = process.initialize(source, synthetic_name, language, width, height) {
            let cleanup = process.shutdown();
            return Err(with_cleanup_error(error, cleanup));
        }
        Ok(process)
    }

    pub fn shared(&self) -> Arc<Mutex<ReaderShared>> {
        Arc::clone(&self.shared)
    }

    pub fn is_solution_buffer(&self, buffer: &Value) -> bool {
        buffer_id(&self.buffer)
            .zip(buffer_id(buffer))
            .is_some_and(|(expected, actual)| expected == actual)
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
local source, name, filetype, channel, max_bytes, max_lines = ...
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
local function snapshot_args()
  local tick = vim.api.nvim_buf_get_changedtick(buf)
  local mode = vim.api.nvim_get_mode().mode
  local line_count = vim.api.nvim_buf_line_count(buf)
  if line_count > max_lines then return tick, mode, false, {}, false end
  local current_lines = vim.api.nvim_buf_get_lines(buf, 0, -1, true)
  local eol = vim.bo[buf].endofline
  local size = eol and 1 or 0
  for index, line in ipairs(current_lines) do
    size = size + #line
    if index > 1 then size = size + 1 end
    if size > max_bytes then return tick, mode, false, {}, false end
  end
  return tick, mode, true, current_lines, eol
end
local function tutor(action)
  local tick, mode, valid, current_lines, eol = snapshot_args()
  vim.rpcnotify(channel, "tutor_action", action, buf, tick, mode, valid, current_lines, eol)
end
local function barrier(id)
  local tick, mode, valid, current_lines, eol = snapshot_args()
  vim.rpcnotify(channel, "tutor_barrier", id, buf, tick, mode, valid, current_lines, eol)
end
_G.__interview_tutor_barriers = _G.__interview_tutor_barriers or {}
_G.__interview_tutor_barrier_ids = _G.__interview_tutor_barrier_ids or {}
_G.__interview_tutor_barriers[channel] = barrier
_G.__interview_tutor_barrier_ids[channel] = {}
local function acknowledge_barrier()
  local ids = _G.__interview_tutor_barrier_ids[channel]
  local id = table.remove(ids, 1)
  if id == nil then error("source barrier id queue is empty") end
  barrier(id)
end
for _, mode in ipairs({ "n", "i", "x", "s", "o", "c", "t" }) do
  vim.keymap.set(mode, "<F35>", acknowledge_barrier, { silent = true, nowait = true })
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
                    Value::Unsigned(crate::editor::MAX_DOCUMENT_BYTES as u64),
                    Value::Unsigned(crate::editor::MAX_DOCUMENT_LINES as u64),
                ]),
            ],
            STARTUP_TIMEOUT,
        )?;
        self.buffer = buffer.clone();
        self.shared
            .lock()
            .expect("Neovim reader lock")
            .solution_buffer =
            Some(buffer_id(&buffer).ok_or("Neovim returned an invalid solution buffer handle")?);
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

    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        let script = r#"
local buf = ...
if not vim.api.nvim_buf_is_valid(buf) or not vim.api.nvim_buf_is_loaded(buf) then
  error("synthetic solution buffer is unavailable")
end
return {
  lines = vim.api.nvim_buf_get_lines(buf, 0, -1, true),
  eol = vim.bo[buf].endofline,
  mode = vim.api.nvim_get_mode().mode,
  tick = vim.api.nvim_buf_get_changedtick(buf),
}
"#;
        let result = self.call(
            "nvim_exec_lua",
            vec![Value::String(script.into()), array([self.buffer.clone()])],
        )?;
        let text = snapshot_text(
            result
                .map_get("lines")
                .ok_or("Neovim snapshot omitted lines")?,
            result.map_get("eol").and_then(Value::as_bool) == Some(true),
        )?;
        let mode = result
            .map_get("mode")
            .and_then(Value::as_str)
            .ok_or("Neovim snapshot omitted mode")?
            .to_string();
        let changedtick = result
            .map_get("tick")
            .and_then(Value::as_u64)
            .ok_or("Neovim snapshot omitted changedtick")?;
        Ok(Snapshot {
            buffer: self.buffer.clone(),
            text: Some(text),
            mode,
            changedtick,
        })
    }

    pub fn request_barrier(&mut self, id: u64) -> Result<(), String> {
        let script = r#"
local channel, id = ...
local barriers = _G.__interview_tutor_barriers
local ids = _G.__interview_tutor_barrier_ids
if barriers == nil or barriers[channel] == nil or ids == nil or ids[channel] == nil then
  error("synthetic solution barrier is unavailable")
end
if #ids[channel] >= 32 then error("source barrier id queue exceeds bound") end
table.insert(ids[channel], id)
return true
"#;
        self.call(
            "nvim_exec_lua",
            vec![
                Value::String(script.into()),
                array([Value::Unsigned(self.channel_id), Value::Unsigned(id)]),
            ],
        )?;
        // nvim_input() appends bytes to Neovim's FIFO input queue. The private mapped key is a
        // processed-input acknowledgement: its snapshot runs only after every earlier key.
        self.feed_key("<F35>")
    }

    pub fn acknowledge_saved(&mut self, changedtick: u64, source: String) -> Result<bool, String> {
        let script = r#"
local buf, tick, source = ...
if not vim.api.nvim_buf_is_valid(buf) or not vim.api.nvim_buf_is_loaded(buf) then return false end
if vim.api.nvim_buf_get_changedtick(buf) ~= tick then return false end
local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, true)
local current = table.concat(lines, "\n")
if vim.bo[buf].endofline then current = current .. "\n" end
if current ~= source then return false end
vim.bo[buf].modified = false
return true
"#;
        self.call(
            "nvim_exec_lua",
            vec![
                Value::String(script.into()),
                array([
                    self.buffer.clone(),
                    Value::Unsigned(changedtick),
                    Value::String(source),
                ]),
            ],
        )?
        .as_bool()
        .ok_or_else(|| "Neovim save acknowledgement returned an invalid result".into())
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
        let deadline = Instant::now() + timeout;
        let input = self.input.as_mut().ok_or("Neovim RPC input is closed")?;
        write_request(input, &bytes, deadline, &self.cancellation, method)?;
        flush_request(input, deadline, &self.cancellation, method)?;
        loop {
            if self.cancellation.is_cancelled() {
                return Err(format!("Neovim {method} was cancelled"));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Neovim {method} timed out"));
            }
            match self
                .responses
                .recv_timeout(remaining.min(CANCEL_POLL_INTERVAL))
            {
                Ok(response) if response.id == id => {
                    return if response.error == Value::Nil {
                        Ok(response.result)
                    } else {
                        Err(format!(
                            "Neovim {method} failed: {}",
                            bounded_value(&response.error)
                        ))
                    };
                }
                Ok(response) => {
                    return Err(format!(
                        "Neovim RPC response id mismatch: expected {id}, got {}",
                        response.id
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self
                        .reader_error()
                        .unwrap_or_else(|| format!("Neovim disconnected during {method}")));
                }
            }
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
        self.cancellation.cancel();
        self.input.take();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let mut errors = Vec::new();
        if let Some(mut child) = self.child.take()
            && let Err(error) = terminate_process_group(&mut child, self.process_group, deadline)
        {
            errors.push(error);
        }
        if let Some(mut reader) = self.stdout_reader.take()
            && let Err(error) = reader.join_before(deadline)
        {
            errors.push(error);
        }
        if let Some(mut reader) = self.stderr_reader.take()
            && let Err(error) = reader.join_before(deadline)
        {
            errors.push(error);
        }
        if let Some(temp_dir) = self.temp_dir.take()
            && let Err(error) = remove_temp_tree(&temp_dir)
        {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub fn stderr_tail(&self) -> String {
        let bytes = self
            .stderr
            .lock()
            .expect("Neovim stderr lock")
            .iter()
            .copied()
            .collect::<Vec<_>>();
        sanitize_diagnostic(&String::from_utf8_lossy(&bytes))
    }
}

impl Drop for RpcProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct InterruptibleReader<R> {
    inner: R,
    cancellation: Cancellation,
}

impl<R> InterruptibleReader<R> {
    fn new(inner: R, cancellation: Cancellation) -> Self {
        Self {
            inner,
            cancellation,
        }
    }
}

impl<R: Read + AsRawFd> Read for InterruptibleReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.cancellation.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Neovim pipe read cancelled",
                ));
            }
            let mut descriptor = libc::pollfd {
                fd: self.inner.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe {
                libc::poll(
                    &mut descriptor,
                    1,
                    i32::try_from(CANCEL_POLL_INTERVAL.as_millis()).unwrap(),
                )
            };
            if result == 0 {
                continue;
            }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if descriptor.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Neovim pipe descriptor became invalid",
                ));
            }
            match self.inner.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

fn set_nonblocking(descriptor: RawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(format!(
            "cannot inspect Neovim pipe flags: {}",
            io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "cannot make Neovim pipe nonblocking: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn wait_writable(
    descriptor: RawFd,
    deadline: Instant,
    cancellation: &Cancellation,
    method: &str,
) -> Result<(), String> {
    loop {
        if cancellation.is_cancelled() {
            return Err(format!("Neovim {method} was cancelled while writing"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("Neovim {method} timed out while writing"));
        }
        let timeout = remaining.min(CANCEL_POLL_INTERVAL);
        let mut poll = libc::pollfd {
            fd: descriptor,
            events: libc::POLLOUT,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut poll,
                1,
                i32::try_from(timeout.as_millis().max(1)).unwrap(),
            )
        };
        if result == 0 {
            continue;
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("cannot wait to write Neovim RPC request: {error}"));
        }
        if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err("Neovim RPC input closed while writing".into());
        }
        if poll.revents & libc::POLLOUT != 0 {
            return Ok(());
        }
    }
}

fn write_request(
    input: &mut ChildStdin,
    bytes: &[u8],
    deadline: Instant,
    cancellation: &Cancellation,
    method: &str,
) -> Result<(), String> {
    let mut written = 0;
    while written < bytes.len() {
        wait_writable(input.as_raw_fd(), deadline, cancellation, method)?;
        match input.write(&bytes[written..]) {
            Ok(0) => return Err("Neovim RPC input closed while writing".into()),
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("cannot write Neovim RPC request: {error}")),
        }
    }
    Ok(())
}

fn flush_request(
    input: &mut ChildStdin,
    deadline: Instant,
    cancellation: &Cancellation,
    method: &str,
) -> Result<(), String> {
    loop {
        if cancellation.is_cancelled() {
            return Err(format!("Neovim {method} was cancelled while flushing"));
        }
        if Instant::now() >= deadline {
            return Err(format!("Neovim {method} timed out while flushing"));
        }
        match input.flush() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_writable(input.as_raw_fd(), deadline, cancellation, method)?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("cannot flush Neovim RPC request: {error}")),
        }
    }
}

fn signal_process_group(process_group: i32, signal: i32) -> Result<(), String> {
    assert!(process_group > 0);
    if unsafe { libc::kill(-process_group, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!("cannot signal Neovim process group: {error}"))
    }
}

fn terminate_process_group(
    child: &mut Child,
    process_group: i32,
    deadline: Instant,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = signal_process_group(process_group, libc::SIGTERM) {
        errors.push(error);
    }
    let grace_deadline = deadline.min(Instant::now() + PROCESS_TERM_GRACE);
    let mut reaped = false;
    while Instant::now() < grace_deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                reaped = true;
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                errors.push(format!("cannot inspect Neovim exit: {error}"));
                break;
            }
        }
    }
    if let Err(error) = signal_process_group(process_group, libc::SIGKILL) {
        errors.push(error);
    }
    while !reaped && Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => reaped = true,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                errors.push(format!("cannot inspect Neovim exit: {error}"));
                break;
            }
        }
    }
    if !reaped {
        errors.push("Neovim direct child was not reaped before deadline".into());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn sanitize_diagnostic(text: &str) -> String {
    text.chars()
        .map(|character| match character as u32 {
            0x00..=0x1f | 0x7f..=0x9f => '�',
            _ => character,
        })
        .collect()
}

fn remove_temp_tree(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove Neovim temporary directory: {error}")),
    }
}

fn with_cleanup_error(error: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; Neovim cleanup failed: {cleanup}"),
    }
}

fn buffer_id(value: &Value) -> Option<u64> {
    match value {
        Value::Unsigned(value) => Some(*value),
        Value::Integer(value) => u64::try_from(*value).ok(),
        Value::Ext(0, bytes) if !bytes.is_empty() && bytes.len() <= 8 => Some(
            bytes
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
        ),
        _ => None,
    }
}

fn snapshot_text(lines: &Value, endofline: bool) -> Result<String, String> {
    let lines = lines
        .as_array()
        .ok_or("Neovim snapshot lines are not an array")?;
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
    if endofline {
        text.push('\n');
    }
    crate::editor::validate_document(&text)?;
    Ok(text)
}

fn notified_snapshot(parameters: &[Value], start: usize) -> Result<Snapshot, String> {
    let buffer = parameters
        .get(start)
        .ok_or("Neovim snapshot omitted solution buffer")?
        .clone();
    let changedtick = parameters
        .get(start + 1)
        .and_then(Value::as_u64)
        .ok_or("Neovim snapshot omitted changedtick")?;
    let mode = parameters
        .get(start + 2)
        .and_then(Value::as_str)
        .ok_or("Neovim snapshot omitted mode")?
        .to_string();
    let valid = parameters
        .get(start + 3)
        .and_then(Value::as_bool)
        .ok_or("Neovim snapshot omitted validity")?;
    let text = if valid {
        Some(snapshot_text(
            parameters
                .get(start + 4)
                .ok_or("Neovim snapshot omitted lines")?,
            parameters.get(start + 5).and_then(Value::as_bool) == Some(true),
        )?)
    } else {
        None
    };
    Ok(Snapshot {
        buffer,
        text,
        mode,
        changedtick,
    })
}

fn push_origin_event(shared: &Arc<Mutex<ReaderShared>>, event: OriginEvent) -> Result<(), String> {
    let mut shared = shared.lock().expect("Neovim reader lock");
    if shared.origin_events.len() == MAX_ORIGIN_EVENTS {
        return Err("Neovim origin event queue exceeds bound".into());
    }
    shared.origin_events.push_back(event);
    Ok(())
}

fn read_rpc(
    mut output: impl Read,
    response_sender: SyncSender<RpcResponse>,
    shared: Arc<Mutex<ReaderShared>>,
    cancellation: Cancellation,
) {
    let mut grid = GridState::default();
    loop {
        let value = match msgpack::decode(&mut output) {
            Ok(value) => value,
            Err(error) => {
                if !cancellation.is_cancelled() {
                    shared.lock().expect("Neovim reader lock").error = Some(error);
                }
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
                match response_sender.try_send(RpcResponse {
                    id,
                    error: message[2].clone(),
                    result: message[3].clone(),
                }) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        shared.lock().expect("Neovim reader lock").error =
                            Some("Neovim RPC response queue exceeds bound".into());
                        break;
                    }
                    Err(TrySendError::Disconnected(_)) => break,
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
                let result: Result<(), String> = (|| match method {
                    "redraw" => grid.apply_redraw(&message[2]).map(|snapshot| {
                        if let Some(snapshot) = snapshot {
                            shared.lock().expect("Neovim reader lock").latest_grid = Some(snapshot);
                        }
                    }),
                    "nvim_buf_lines_event" if parameters.len() == 6 => {
                        let buffer = parameters[0].clone();
                        let changedtick = parameters[1]
                            .as_u64()
                            .ok_or("Neovim line event changedtick is invalid")?;
                        let more = parameters[5]
                            .as_bool()
                            .ok_or("Neovim line event multipart flag is invalid")?;
                        let mut reader = shared.lock().expect("Neovim reader lock");
                        if buffer_id(&buffer) != reader.solution_buffer {
                            Err("Neovim line event targeted a non-solution buffer".into())
                        } else if reader
                            .multipart_changedtick
                            .is_some_and(|pending| pending != changedtick)
                        {
                            Err("Neovim multipart line events changed tick".into())
                        } else if more {
                            reader.multipart_changedtick = Some(changedtick);
                            Ok(())
                        } else {
                            reader.multipart_changedtick = None;
                            if !matches!(
                                reader.origin_events.back(),
                                Some(OriginEvent::Dirty { changedtick: pending, .. }) if *pending == changedtick
                            ) {
                                if reader.origin_events.len() == MAX_ORIGIN_EVENTS {
                                    Err("Neovim origin event queue exceeds bound".into())
                                } else {
                                    reader.origin_events.push_back(OriginEvent::Dirty {
                                        buffer,
                                        changedtick,
                                    });
                                    Ok(())
                                }
                            } else {
                                Ok(())
                            }
                        }
                    }
                    "nvim_buf_changedtick_event" if parameters.len() == 2 => {
                        let buffer = parameters[0].clone();
                        let changedtick = parameters[1]
                            .as_u64()
                            .ok_or("Neovim changedtick event is invalid")?;
                        let reader = shared.lock().expect("Neovim reader lock");
                        if buffer_id(&buffer) != reader.solution_buffer {
                            Err("Neovim changedtick event targeted a non-solution buffer".into())
                        } else if reader.multipart_changedtick.is_some() {
                            Err("Neovim changedtick event interrupted multipart lines".into())
                        } else {
                            drop(reader);
                            push_origin_event(
                                &shared,
                                OriginEvent::Dirty {
                                    buffer,
                                    changedtick,
                                },
                            )
                        }
                    }
                    "nvim_buf_detach_event" if parameters.len() == 1 => {
                        if shared.lock().expect("Neovim reader lock").solution_buffer
                            == parameters.first().and_then(buffer_id)
                        {
                            Err("Neovim detached the solution buffer".into())
                        } else {
                            Err("Neovim detached an unexpected buffer".into())
                        }
                    }
                    "tutor_action" if parameters.len() == 7 => {
                        let action = parameters[0]
                            .as_str()
                            .ok_or("Neovim Tutor action is invalid")?
                            .to_string();
                        let snapshot = notified_snapshot(parameters, 1)?;
                        let expected = shared.lock().expect("Neovim reader lock").solution_buffer;
                        if expected != buffer_id(&snapshot.buffer) {
                            Err("Neovim Tutor action targeted a non-solution buffer".into())
                        } else {
                            push_origin_event(&shared, OriginEvent::Action { action, snapshot })
                        }
                    }
                    "tutor_barrier" if parameters.len() == 7 => {
                        let id = parameters[0]
                            .as_u64()
                            .ok_or("Neovim barrier id is invalid")?;
                        let snapshot = notified_snapshot(parameters, 1)?;
                        let expected = shared.lock().expect("Neovim reader lock").solution_buffer;
                        if expected != buffer_id(&snapshot.buffer) {
                            Err("Neovim barrier targeted a non-solution buffer".into())
                        } else {
                            push_origin_event(&shared, OriginEvent::Barrier { id, snapshot })
                        }
                    }
                    "nvim_error_event" => Err("Neovim reported an asynchronous API error".into()),
                    _ => Ok(()),
                })();
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
    ValidatedExecutable::open(&canonical)?;
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

impl ValidatedExecutable {
    fn open(path: &Path) -> Result<Self, String> {
        let encoded = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "Neovim executable path contains NUL")?;
        let descriptor = unsafe {
            libc::open(
                encoded.as_ptr(),
                libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor == -1 {
            return Err(format!(
                "cannot open Neovim executable: {}",
                io::Error::last_os_error()
            ));
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = file
            .metadata()
            .map_err(|error| format!("cannot inspect Neovim executable: {error}"))?;
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
        if metadata.mode() & 0o111 == 0 {
            return Err("Neovim executable is not executable".into());
        }
        Ok(Self {
            file,
            display_path: path.to_path_buf(),
        })
    }

    fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    fn command(&self) -> Command {
        Command::new(format!("/proc/self/fd/{}", self.raw_fd()))
    }
}

fn validate_version(
    executable: &ValidatedExecutable,
    cancellation: &Arc<AtomicBool>,
) -> Result<(), String> {
    const MAX_VERSION_BYTES: u64 = 64 * 1024;
    let temp_dir = create_temp_dir()?;
    let result = (|| {
        let stdout_path = temp_dir.join("stdout");
        let stderr_path = temp_dir.join("stderr");
        let stdout = create_probe_file(&stdout_path)?;
        let stderr = create_probe_file(&stderr_path)?;
        let mut command = executable.command();
        command
            .arg("--version")
            .current_dir(&temp_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        configure_process_group_and_limits(
            &mut command,
            Some(MAX_VERSION_BYTES),
            Some(executable.raw_fd()),
        );
        let mut child = command.spawn().map_err(|error| {
            format!(
                "cannot probe Neovim version through validated descriptor {}: {error}",
                executable.display_path.display()
            )
        })?;
        let process_group = i32::try_from(child.id()).expect("Linux process IDs fit i32");
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if cancellation.load(Ordering::Acquire) {
                let cleanup = terminate_process_group(
                    &mut child,
                    process_group,
                    Instant::now() + SHUTDOWN_TIMEOUT,
                );
                return Err(with_cleanup_error(
                    "Neovim version probe was cancelled".into(),
                    cleanup,
                ));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut cleanup_error =
                        signal_process_group(process_group, libc::SIGTERM).err();
                    if let Err(error) = signal_process_group(process_group, libc::SIGKILL)
                        && cleanup_error.is_none()
                    {
                        cleanup_error = Some(error);
                    }
                    if let Some(error) = cleanup_error {
                        return Err(error);
                    }
                    break status;
                }
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let cleanup = terminate_process_group(
                        &mut child,
                        process_group,
                        Instant::now() + SHUTDOWN_TIMEOUT,
                    );
                    return Err(with_cleanup_error(
                        "Neovim version probe timed out".into(),
                        cleanup,
                    ));
                }
                Err(error) => {
                    let cleanup = terminate_process_group(
                        &mut child,
                        process_group,
                        Instant::now() + SHUTDOWN_TIMEOUT,
                    );
                    return Err(with_cleanup_error(
                        format!("cannot inspect Neovim version probe: {error}"),
                        cleanup,
                    ));
                }
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
    let cleanup = remove_temp_tree(&temp_dir);
    match result {
        Ok(()) => cleanup,
        Err(error) => Err(with_cleanup_error(error, cleanup)),
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

fn configure_process_group_and_limits(
    command: &mut Command,
    file_size: Option<u64>,
    executable_descriptor: Option<RawFd>,
) {
    unsafe {
        command.pre_exec(move || {
            if let Some(descriptor) = executable_descriptor
                && libc::fcntl(descriptor, libc::F_SETFD, 0) == -1
            {
                return Err(io::Error::last_os_error());
            }
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
    use std::os::unix::fs::PermissionsExt;

    fn rpc_response(id: u64, result: Value) -> Value {
        array([Value::Unsigned(1), Value::Unsigned(id), Value::Nil, result])
    }

    fn compatible_api() -> Value {
        array([
            Value::Unsigned(7),
            map([
                (
                    "version",
                    map([
                        ("api_level", Value::Unsigned(MIN_API_LEVEL)),
                        ("api_compatible", Value::Unsigned(MIN_API_LEVEL)),
                        ("api_prerelease", Value::Bool(false)),
                    ]),
                ),
                ("ui_options", array([Value::String("ext_linegrid".into())])),
            ]),
        ])
    }

    fn shell_encoded(value: &Value) -> String {
        msgpack::encode(value)
            .unwrap()
            .into_iter()
            .map(|byte| format!("\\{byte:03o}"))
            .collect()
    }

    fn fake_neovim(
        version_tail: &str,
        embedded_prefix: &str,
        initialize: bool,
        embedded_tail: &str,
    ) -> (PathBuf, PathBuf) {
        let root = create_temp_dir().unwrap();
        let executable = root.join("fake-nvim");
        let responses = initialize.then(|| {
            [
                rpc_response(1, compatible_api()),
                rpc_response(2, Value::Nil),
                rpc_response(3, Value::Nil),
                rpc_response(4, Value::Unsigned(1)),
                rpc_response(5, Value::Bool(true)),
                rpc_response(6, Value::Nil),
            ]
            .iter()
            .map(|response| format!("sleep 0.02\nprintf '{}'\n", shell_encoded(response)))
            .collect::<String>()
        });
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nif [ \"${{1-}}\" = --version ]; then\n  printf 'NVIM v0.11.5\\n'\n  {version_tail}\n  exit 0\nfi\n{embedded_prefix}\n{}{embedded_tail}\n",
                responses.unwrap_or_default(),
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        (root, executable)
    }

    fn wait_for_file(path: &Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture file timed out: {path:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_process_exit(process_id: u32, timeout: Duration) {
        let process = PathBuf::from(format!("/proc/{process_id}"));
        let deadline = Instant::now() + timeout;
        while process.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture process did not exit: {process_id}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_barrier(process: &RpcProcess, id: u64) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = process
                .shared()
                .lock()
                .unwrap()
                .origin_events
                .iter()
                .find_map(|event| match event {
                    OriginEvent::Barrier {
                        id: actual,
                        snapshot,
                    } if *actual == id => Some(snapshot.clone()),
                    OriginEvent::Dirty { .. }
                    | OriginEvent::Action { .. }
                    | OriginEvent::Barrier { .. } => None,
                });
            if let Some(snapshot) = snapshot {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "source barrier {id} timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

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
            let snapshot = process.snapshot().unwrap();
            assert_eq!(snapshot.text.as_deref(), Some(source));
            assert!(snapshot.mode.starts_with('n'));
            process.shutdown().unwrap();
        }
    }

    #[test]
    fn clipboard_provider_is_disabled_before_clean_neovim_initializes() {
        let executable = resolve_executable(None).unwrap();
        let mut process = RpcProcess::start(
            &executable,
            "x\n",
            "interview://clipboard.py",
            "python",
            20,
            8,
        )
        .unwrap();
        let loaded = process
            .call(
                "nvim_get_var",
                vec![Value::String("loaded_clipboard_provider".into())],
            )
            .unwrap();
        assert_eq!(loaded.as_u64(), Some(0));
        process.shutdown().unwrap();
    }

    #[test]
    fn validated_descriptor_survives_path_replacement_between_check_and_exec() {
        let (root, executable_path) = fake_neovim("", "", false, "exit 0");
        let executable = ValidatedExecutable::open(&executable_path).unwrap();
        let original = root.join("validated-original");
        fs::rename(&executable_path, &original).unwrap();
        fs::write(&executable_path, "#!/bin/sh\nprintf 'NVIM v2.0.0\\n'\n").unwrap();
        fs::set_permissions(&executable_path, fs::Permissions::from_mode(0o700)).unwrap();
        validate_version(&executable, &Arc::new(AtomicBool::new(false))).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_rpc_ids_envelopes_and_eof_fail_without_hanging() {
        for (name, payload, expected) in [
            (
                "id",
                "printf '\\224\\001\\243bad\\300\\300'\n",
                "response id is invalid",
            ),
            ("envelope", "printf '\\221\\300'\n", "envelope is malformed"),
            ("eof", "exit 0\n", "MessagePack stream"),
        ] {
            let (root, executable) = fake_neovim("", payload, false, "sleep 30");
            let started = Instant::now();
            let error = RpcProcess::start(
                &executable,
                "x\n",
                &format!("interview://{name}.py"),
                "python",
                20,
                8,
            )
            .err()
            .expect("malicious RPC must fail");
            assert!(error.contains(expected), "unexpected error: {error}");
            assert!(started.elapsed() < Duration::from_secs(2));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn response_flood_is_bounded_and_shutdown_stays_bounded() {
        let flood = (0..16)
            .map(|_| {
                format!(
                    "printf '{}'\n",
                    shell_encoded(&rpc_response(999, Value::Nil))
                )
            })
            .collect::<String>();
        let (root, executable) = fake_neovim("", "", true, &format!("{flood}sleep 30"));
        let mut process =
            RpcProcess::start(&executable, "x\n", "interview://flood.py", "python", 20, 8).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if process
                .reader_error()
                .is_some_and(|error| error.contains("response queue exceeds bound"))
            {
                break;
            }
            assert!(Instant::now() < deadline, "response flood was not rejected");
            thread::sleep(Duration::from_millis(10));
        }
        let started = Instant::now();
        process.shutdown().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_read_and_no_response_paths_obey_rpc_deadlines() {
        let (root, executable) = fake_neovim("", "", true, "sleep 30");
        let mut process = RpcProcess::start(
            &executable,
            "x\n",
            "interview://no-read.py",
            "python",
            20,
            8,
        )
        .unwrap();
        let started = Instant::now();
        let error = process
            .call_with_timeout(
                "nvim_paste",
                vec![Value::String("x".repeat(1024 * 1024))],
                Duration::from_millis(200),
            )
            .unwrap_err();
        assert!(error.contains("timed out while writing"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        process.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();

        let (root, executable) = fake_neovim("", "", true, "sleep 30");
        let mut process =
            RpcProcess::start(&executable, "x\n", "interview://hang.py", "python", 20, 8).unwrap();
        let started = Instant::now();
        let error = process
            .call_with_timeout("nvim_get_mode", Vec::new(), Duration::from_millis(200))
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        process.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stderr_flood_is_retained_bounded_and_control_sanitized() {
        let tail = "head -c 131072 /dev/zero | tr '\\000' '\\033' >&2\nsleep 30";
        let (root, executable) = fake_neovim("", "", true, tail);
        let mut process =
            RpcProcess::start(&executable, "x\n", "interview://stderr.py", "python", 20, 8)
                .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while process.stderr.lock().unwrap().len() < MAX_STDERR_BYTES {
            assert!(
                Instant::now() < deadline,
                "stderr flood did not reach bound"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let tail = process.stderr_tail();
        assert_eq!(tail.chars().count(), MAX_STDERR_BYTES);
        assert!(!tail.chars().any(char::is_control));
        process.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_and_embedded_direct_exit_kill_descendants_and_release_readers() {
        let root = create_temp_dir().unwrap();
        let version_pid_file = root.join("version-descendant.pid");
        let (fixture_root, executable) = fake_neovim(
            &format!(
                "(trap '' TERM; sleep 30) & printf '%s\\n' $! > '{}'",
                version_pid_file.display()
            ),
            "",
            false,
            "exit 0",
        );
        let validated = ValidatedExecutable::open(&executable).unwrap();
        validate_version(&validated, &Arc::new(AtomicBool::new(false))).unwrap();
        wait_for_file(&version_pid_file, Duration::from_secs(1));
        let version_pid = fs::read_to_string(&version_pid_file)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        wait_for_process_exit(version_pid, Duration::from_secs(2));
        fs::remove_dir_all(fixture_root).unwrap();

        let embedded_pid_file = root.join("embedded-descendant.pid");
        let (fixture_root, executable) = fake_neovim(
            "",
            "",
            true,
            &format!(
                "(trap '' TERM; sleep 30) & printf '%s\\n' $! > '{}'\nexit 0",
                embedded_pid_file.display()
            ),
        );
        let mut process = RpcProcess::start(
            &executable,
            "x\n",
            "interview://descendant.py",
            "python",
            20,
            8,
        )
        .unwrap();
        wait_for_file(&embedded_pid_file, Duration::from_secs(1));
        let embedded_pid = fs::read_to_string(&embedded_pid_file)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        let started = Instant::now();
        process.shutdown().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        wait_for_process_exit(embedded_pid, Duration::from_secs(2));
        fs::remove_dir_all(fixture_root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_embedded_start_removes_nonempty_owned_temp_tree() {
        let root = create_temp_dir().unwrap();
        let cwd_file = root.join("embedded-cwd");
        let prefix = format!(
            "mkdir residue\nprintf '%s' \"$PWD\" > '{}'\nprintf '\\221\\300'",
            cwd_file.display()
        );
        let (fixture_root, executable) = fake_neovim("", &prefix, false, "sleep 30");
        assert!(
            RpcProcess::start(
                &executable,
                "x\n",
                "interview://residue.py",
                "python",
                20,
                8,
            )
            .is_err()
        );
        wait_for_file(&cwd_file, Duration::from_secs(1));
        let embedded_cwd = PathBuf::from(fs::read_to_string(&cwd_file).unwrap());
        assert!(
            !embedded_cwd.exists(),
            "temporary tree leaked: {embedded_cwd:?}"
        );
        fs::remove_dir_all(fixture_root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_barrier_waits_until_a_delayed_mapping_has_processed_its_bytes() {
        let executable = resolve_executable(None).unwrap();
        let mut process = RpcProcess::start(
            &executable,
            "x\n",
            "interview://processed-input.py",
            "python",
            30,
            10,
        )
        .unwrap();
        process
            .call(
                "nvim_exec_lua",
                vec![
                    Value::String(
                        r#"
local buf = ...
vim.keymap.set("i", "Z", function()
  vim.wait(150)
  return "Z"
end, { buffer = buf, expr = true })
"#
                        .into(),
                    ),
                    array([process.buffer.clone()]),
                ],
            )
            .unwrap();

        let started = Instant::now();
        process.feed_key("iZ").unwrap();
        process.request_barrier(77).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let barrier = loop {
            let event = process
                .shared()
                .lock()
                .unwrap()
                .origin_events
                .iter()
                .find_map(|event| match event {
                    OriginEvent::Barrier { id: 77, snapshot } => Some(snapshot.clone()),
                    OriginEvent::Dirty { .. }
                    | OriginEvent::Action { .. }
                    | OriginEvent::Barrier { .. } => None,
                });
            if let Some(snapshot) = event {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "processed-input barrier timed out"
            );
            thread::sleep(Duration::from_millis(10));
        };

        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "barrier overtook delayed input"
        );
        assert_eq!(barrier.text.as_deref(), Some("Zx\n"));
        assert!(barrier.mode.starts_with('i'));
        process.shutdown().unwrap();
    }

    #[test]
    fn tutor_commands_and_barriers_work_across_global_neovim_contexts() {
        let executable = resolve_executable(None).unwrap();
        let mut process = RpcProcess::start(
            &executable,
            "solution\n",
            "interview://global-barrier.py",
            "python",
            40,
            12,
        )
        .unwrap();

        for command in [
            "TutorTest",
            "TutorSubmit",
            "TutorBack",
            "TutorCollapse",
            "TutorQuit",
            "TutorHint",
        ] {
            process.feed_key(&format!(":{command}<CR>")).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let actions = process
                .shared()
                .lock()
                .unwrap()
                .origin_events
                .iter()
                .filter_map(|event| match event {
                    OriginEvent::Action { action, snapshot } => {
                        assert_eq!(snapshot.text.as_deref(), Some("solution\n"));
                        Some(action.clone())
                    }
                    OriginEvent::Dirty { .. } | OriginEvent::Barrier { .. } => None,
                })
                .collect::<Vec<_>>();
            if actions.len() == 6 {
                assert_eq!(
                    actions,
                    ["test", "submit", "back", "collapse", "quit", "hint"]
                );
                break;
            }
            assert!(Instant::now() < deadline, "Tutor commands timed out");
            thread::sleep(Duration::from_millis(10));
        }

        process.feed_key(":enew<CR>iALTERNATE<Esc>").unwrap();
        process.request_barrier(101).unwrap();
        assert_eq!(
            wait_for_barrier(&process, 101).text.as_deref(),
            Some("solution\n")
        );

        process.feed_key(":new<CR>iSPLIT<Esc>").unwrap();
        process.request_barrier(102).unwrap();
        assert_eq!(
            wait_for_barrier(&process, 102).text.as_deref(),
            Some("solution\n")
        );

        process.feed_key(":tabnew<CR>iTAB<Esc>").unwrap();
        process.request_barrier(103).unwrap();
        assert_eq!(
            wait_for_barrier(&process, 103).text.as_deref(),
            Some("solution\n")
        );

        for (keys, id, expected_mode) in [
            ("i", 201, "i"),
            ("v", 202, "v"),
            ("gh", 203, "s"),
            ("d", 204, "no"),
            (":", 205, "c"),
        ] {
            process.feed_key(keys).unwrap();
            process.request_barrier(id).unwrap();
            let barrier = wait_for_barrier(&process, id);
            assert_eq!(barrier.text.as_deref(), Some("solution\n"));
            assert!(
                barrier.mode.starts_with(expected_mode),
                "expected {expected_mode} mode, got {}",
                barrier.mode
            );
            process.feed_key("<Esc>").unwrap();
        }

        process.feed_key(":tabnew<CR>:terminal<CR>i").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mode = process
                .call("nvim_get_mode", Vec::new())
                .unwrap()
                .map_get("mode")
                .and_then(Value::as_str)
                .unwrap()
                .to_string();
            if mode.starts_with('t') {
                break;
            }
            assert!(Instant::now() < deadline, "terminal mode timed out");
            thread::sleep(Duration::from_millis(10));
        }
        process.request_barrier(104).unwrap();
        let terminal_barrier = wait_for_barrier(&process, 104);
        assert_eq!(terminal_barrier.text.as_deref(), Some("solution\n"));
        assert!(terminal_barrier.mode.starts_with('t'));

        process.shutdown().unwrap();
    }

    #[test]
    fn snapshots_remain_bound_to_the_synthetic_solution_buffer() {
        let executable = resolve_executable(None).unwrap();
        let mut process = RpcProcess::start(
            &executable,
            "solution\n",
            "interview://bound.py",
            "python",
            30,
            10,
        )
        .unwrap();
        process.feed_key(":enew<CR>iother<Esc>").unwrap();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            process.snapshot().unwrap().text.as_deref(),
            Some("solution\n")
        );
        process.shutdown().unwrap();
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
    fn save_acknowledgement_requires_matching_tick_and_bytes() {
        let executable = resolve_executable(None).unwrap();
        let mut process =
            RpcProcess::start(&executable, "x\n", "interview://saved.py", "python", 20, 8).unwrap();
        process.feed_key("iZ<Esc>").unwrap();
        thread::sleep(Duration::from_millis(20));
        let saved = process.snapshot().unwrap();
        assert!(
            process
                .acknowledge_saved(saved.changedtick, saved.text.clone().unwrap())
                .unwrap()
        );
        process.feed_key("A!<Esc>").unwrap();
        thread::sleep(Duration::from_millis(20));
        assert!(
            !process
                .acknowledge_saved(saved.changedtick, saved.text.unwrap())
                .unwrap()
        );
        let modified = process
            .call(
                "nvim_buf_get_option",
                vec![process.buffer.clone(), Value::String("modified".into())],
            )
            .unwrap();
        assert_eq!(modified.as_bool(), Some(true));
        process.shutdown().unwrap();
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
        assert_eq!(
            process.snapshot().unwrap().text.as_deref(),
            Some("two\nthree\n")
        );
        feed(&mut process, "u");
        assert_eq!(
            process.snapshot().unwrap().text.as_deref(),
            Some("one two\nthree\n")
        );
        feed(&mut process, "G$A!<Esc>");
        feed(&mut process, ".");
        assert_eq!(
            process.snapshot().unwrap().text.as_deref(),
            Some("one two\nthree!!\n")
        );
        feed(&mut process, "gg0\"ayyGp");
        assert_eq!(
            process.snapshot().unwrap().text.as_deref(),
            Some("one two\nthree!!\none two\n")
        );
        feed(&mut process, "/three<CR>");
        assert!(process.snapshot().unwrap().mode.starts_with('n'));
        feed(&mut process, "<Space>t");
        let shared = process.shared();
        let action_observed = shared.lock().unwrap().origin_events.iter().any(|event| {
            matches!(
                event,
                OriginEvent::Action { action, snapshot }
                    if action == "test"
                        && snapshot.text.as_deref() == Some("one two\nthree!!\none two\n")
            )
        });
        assert!(action_observed);
        process.shutdown().unwrap();
    }
}
