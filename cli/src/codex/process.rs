use super::protocol::{self, Incoming};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const MAX_PENDING_IDS: usize = 16;
const _: () = assert!(MAX_PENDING_IDS >= 1);

pub struct CodexProcess {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Result<Vec<u8>, String>>,
    readers: Vec<JoinHandle<()>>,
    cwd: PathBuf,
    next_id: u64,
    control_pid: Arc<AtomicI32>,
}

impl CodexProcess {
    pub fn start() -> Result<Self, String> {
        Self::start_with_control(Arc::new(AtomicI32::new(0)))
    }

    pub fn start_with_control(control_pid: Arc<AtomicI32>) -> Result<Self, String> {
        let configured = std::env::var_os("INTERVIEW_TUTOR_CODEX_EXECUTABLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("codex"));
        let executable = resolve_executable(&configured)?;
        Self::start_executable(executable, control_pid)
    }

    fn start_executable(executable: PathBuf, control_pid: Arc<AtomicI32>) -> Result<Self, String> {
        validate_version(&executable)?;
        let cwd = empty_temp_dir()?;
        let mut command = Command::new(&executable);
        command
            .arg("app-server")
            .arg("--stdio")
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for name in allowed_environment_names() {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start Codex app-server: {e}"))?;
        let input = child.stdin.take().ok_or("Codex stdin unavailable")?;
        let output = child.stdout.take().ok_or("Codex stdout unavailable")?;
        let error = child.stderr.take().ok_or("Codex stderr unavailable")?;
        let (sender, messages) = mpsc::sync_channel(64);
        let reader = thread::spawn(move || {
            let mut output = output;
            loop {
                match read_bounded_line(&mut output) {
                    Ok(Some(line)) => {
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = sender.send(Err("Codex app-server closed stdout".into()));
                        break;
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let stderr_reader = thread::spawn(move || {
            let mut error = error.take(MAX_STDERR_BYTES as u64 + 1);
            let mut sink = [0_u8; 8192];
            let mut total = 0;
            while let Ok(read) = error.read(&mut sink) {
                if read == 0 {
                    break;
                }
                total += read;
                if total > MAX_STDERR_BYTES {
                    break;
                }
            }
        });
        control_pid.store(child.id() as i32, Ordering::SeqCst);
        let mut process = Self {
            child,
            input,
            messages,
            readers: vec![reader, stderr_reader],
            cwd,
            next_id: 1,
            control_pid,
        };
        process.initialize()?;
        Ok(process)
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.call("initialize", json!({"clientInfo":{"name":"interview-tutor","title":"Interview Tutor","version":env!("CARGO_PKG_VERSION")},"capabilities":null}), STARTUP_TIMEOUT)?;
        self.send(&protocol::notification("initialized", json!({}))?)?;
        Ok(())
    }

    pub fn account_ready(&mut self) -> Result<bool, String> {
        let result = self.call(
            "account/read",
            json!({"refreshToken":false}),
            STARTUP_TIMEOUT,
        )?;
        let account_type = result.pointer("/account/type").and_then(Value::as_str);
        Ok(account_type == Some("chatgpt")
            && result.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(true))
    }

    pub fn start_thread(&mut self) -> Result<String, String> {
        let cwd = self.cwd.to_str().ok_or("temporary path is not UTF-8")?;
        let result = self.call(
            "thread/start",
            json!({
                "ephemeral":true,"cwd":cwd,"sandbox":"read-only","approvalPolicy":"never",
                "config":{"web_search":"disabled"}
            }),
            STARTUP_TIMEOUT,
        )?;
        if result.pointer("/thread/ephemeral").and_then(Value::as_bool) != Some(true)
            || result.pointer("/thread/path") != Some(&Value::Null)
            || result.get("approvalPolicy").and_then(Value::as_str) != Some("never")
            || result.pointer("/sandbox/type").and_then(Value::as_str) != Some("readOnly")
            || result
                .pointer("/sandbox/networkAccess")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            return Err(
                "Codex did not enforce ephemeral/read-only/no-network/never-approve settings"
                    .into(),
            );
        }
        result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "Codex thread/start omitted thread id".into())
    }

    pub fn turn(
        &mut self,
        thread_id: &str,
        input: String,
        output_schema: Value,
    ) -> Result<String, String> {
        if input.len() > protocol::MAX_JSON_LINE_BYTES / 2 {
            return Err("Codex prompt exceeds bound".into());
        }
        let result = self.call("turn/start", json!({"threadId":thread_id,"input":[{"type":"text","text":input}],"outputSchema":output_schema,"approvalPolicy":"never","sandboxPolicy":{"type":"readOnly","networkAccess":false}}), STARTUP_TIMEOUT)?;
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or("Codex turn/start omitted turn id")?
            .to_string();
        let deadline = std::time::Instant::now() + TURN_TIMEOUT;
        let mut text = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                let _ = self.interrupt(thread_id, &turn_id);
                return Err("Codex turn timed out after 120s".into());
            }
            match self.receive(remaining)? {
                Incoming::Notification { method, params } => match method.as_str() {
                    "item/agentMessage/delta"
                        if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                            && params.get("turnId").and_then(Value::as_str) == Some(&turn_id) =>
                    {
                        let delta = params
                            .get("delta")
                            .and_then(Value::as_str)
                            .ok_or("malformed Codex agent delta")?;
                        if text.len().saturating_add(delta.len()) > protocol::MAX_ASSISTANT_BYTES {
                            return Err("Codex response exceeds 64 KiB".into());
                        }
                        text.push_str(delta);
                    }
                    "item/completed"
                        if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                            && params.pointer("/item/type").and_then(Value::as_str)
                                == Some("agentMessage") =>
                    {
                        if let Some(completed) =
                            params.pointer("/item/text").and_then(Value::as_str)
                        {
                            if completed.len() > protocol::MAX_ASSISTANT_BYTES {
                                return Err("Codex response exceeds 64 KiB".into());
                            }
                            text = completed.to_string();
                        }
                    }
                    "turn/completed"
                        if params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                            && params.pointer("/turn/id").and_then(Value::as_str)
                                == Some(&turn_id) =>
                    {
                        if params.pointer("/turn/status").and_then(Value::as_str)
                            != Some("completed")
                        {
                            return Err("Codex turn did not complete successfully".into());
                        }
                        return Ok(text);
                    }
                    "turn/error" | "error" => return Err("Codex reported a turn error".into()),
                    name if is_approval(name) => {
                        return Err("Codex requested a forbidden approval".into());
                    }
                    _ => {}
                },
                Incoming::ServerRequest { id, method } => {
                    self.decline(id)?;
                    return Err(format!("Codex requested forbidden operation {method}"));
                }
                Incoming::Response { id, .. } => {
                    return Err(format!("unexpected Codex response id {id}"));
                }
            }
        }
    }

    pub fn interrupt(&mut self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        self.call(
            "turn/interrupt",
            json!({"threadId":thread_id,"turnId":turn_id}),
            Duration::from_secs(2),
        )
        .map(|_| ())
    }

    fn decline(&mut self, id: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&json!({"id":id,"result":{"decision":"decline"}}))
            .map_err(|_| "cannot encode decline")?;
        bytes.push(b'\n');
        self.send(&bytes)
    }

    fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("request id overflow");
        self.send(&protocol::request(id, method, params)?)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("Codex {method} timed out"));
            }
            match self.receive(remaining)? {
                Incoming::Response {
                    id: response_id,
                    result,
                } if response_id == id => return result,
                Incoming::Response {
                    id: response_id, ..
                } => return Err(format!("unexpected Codex response id {response_id}")),
                Incoming::ServerRequest { id, method } => {
                    self.decline(id)?;
                    return Err(format!("Codex requested forbidden operation {method}"));
                }
                Incoming::Notification { method, .. } if is_approval(&method) => {
                    return Err("Codex requested a forbidden approval".into());
                }
                Incoming::Notification { .. } => {}
            }
        }
    }

    fn receive(&self, timeout: Duration) -> Result<Incoming, String> {
        match self.messages.recv_timeout(timeout) {
            Ok(Ok(line)) => protocol::decode(&line),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Err("Codex operation timed out".into()),
            Err(RecvTimeoutError::Disconnected) => Err("Codex protocol reader stopped".into()),
        }
    }
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.input
            .write_all(bytes)
            .and_then(|_| self.input.flush())
            .map_err(|e| format!("cannot write to Codex: {e}"))
    }
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        let pid = self.child.id() as i32;
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + SHUTDOWN_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.wait();
        self.control_pid.store(0, Ordering::SeqCst);
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let _ = fs::remove_dir(&self.cwd);
    }
}

fn is_approval(method: &str) -> bool {
    method.contains("approval")
        || method.contains("requestApproval")
        || method.contains("permissions")
        || method.contains("mcpServer")
}
fn allowed_environment_names() -> &'static [&'static str] {
    &[
        "HOME",
        "CODEX_HOME",
        "PATH",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ]
}
fn resolve_executable(path: &Path) -> Result<PathBuf, String> {
    if path.components().count() > 1 {
        return fs::canonicalize(path).map_err(|e| format!("cannot resolve Codex executable: {e}"));
    }
    let paths = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
    std::env::split_paths(&paths)
        .map(|base| base.join(path))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
        .ok_or_else(|| "Codex CLI not found; local solve remains available".into())
}
fn validate_version(executable: &Path) -> Result<(), String> {
    let output = Command::new(executable)
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .map_err(|e| format!("cannot query Codex version: {e}"))?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !(version.contains(" 0.146.") || version.contains(" 0.147.")) {
        return Err(format!(
            "unsupported Codex CLI {}; install 0.146.x or 0.147.x",
            version.trim()
        ));
    }
    Ok(())
}
fn empty_temp_dir() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock invalid")?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "interview-tutor-codex-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).map_err(|e| format!("cannot create Codex temporary directory: {e}"))?;
    Ok(path)
}
fn read_bounded_line(reader: &mut impl Read) -> Result<Option<Vec<u8>>, String> {
    let mut line = Vec::with_capacity(4096);
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) if line.is_empty() => return Ok(None),
            Ok(0) => return Err("Codex closed stdout mid-message".into()),
            Ok(_) if byte[0] == b'\n' => return Ok(Some(line)),
            Ok(_) => {
                if line.len() == protocol::MAX_JSON_LINE_BYTES {
                    return Err("Codex protocol line exceeds 2 MiB".into());
                }
                line.push(byte[0]);
            }
            Err(error) => return Err(format!("cannot read Codex output: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_reader_rejects_oversize_and_eof() {
        assert_eq!(
            read_bounded_line(&mut &b"{}\n"[..]).unwrap(),
            Some(b"{}".to_vec())
        );
        assert!(read_bounded_line(&mut &b"{}"[..]).is_err());
        let input = vec![b'x'; protocol::MAX_JSON_LINE_BYTES + 1];
        assert!(read_bounded_line(&mut &input[..]).is_err());
    }
    #[test]
    fn environment_allowlist_excludes_keys_and_sentinels() {
        let names = allowed_environment_names();
        assert!(names.contains(&"HOME") && names.contains(&"CODEX_HOME"));
        assert!(!names.contains(&"OPENAI_API_KEY") && !names.contains(&"INTERVIEW_TUTOR_SENTINEL"));
    }

    #[test]
    fn fake_app_server_handshake_effective_settings_deltas_and_unknown_notifications() {
        let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fake_codex_app_server.py");
        let mut process =
            CodexProcess::start_executable(executable, Arc::new(AtomicI32::new(0))).unwrap();
        assert!(process.account_ready().unwrap());
        let thread = process.start_thread().unwrap();
        let response = process
            .turn(&thread, "bounded prompt".into(), json!({"type":"object"}))
            .unwrap();
        assert!(response.contains("What invariant holds?"));
    }
}
