use super::protocol::{self, Incoming, RequestId};
use crate::runner::CancellationToken;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_VERSION_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const PROTOCOL_QUEUE_CAPACITY: usize = 64;
const MAX_PENDING_IDS: usize = 16;
const _: () = assert!(MAX_PENDING_IDS >= 1);
const _: () = assert!(PROTOCOL_QUEUE_CAPACITY >= MAX_PENDING_IDS);

pub struct CodexProcess {
    child: Option<Child>,
    input: Option<ChildStdin>,
    messages: Option<Receiver<Vec<u8>>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    reader_failure: Arc<Mutex<Option<String>>>,
    _stderr_ring: Arc<Mutex<StderrRing>>,
    cwd: Option<PathBuf>,
    next_id: u64,
    pending_ids: HashSet<u64>,
    control_pid: Arc<AtomicI32>,
}

impl CodexProcess {
    pub fn start() -> Result<Self, String> {
        Self::start_with_control(Arc::new(AtomicI32::new(0)))
    }

    pub fn start_with_control(control_pid: Arc<AtomicI32>) -> Result<Self, String> {
        Self::start_with_control_and_cancellation(control_pid, &CancellationToken::new())
    }

    pub fn start_with_control_and_cancellation(
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        let executable = configured_executable()?;
        Self::start_executable(executable, control_pid, cancellation)
    }

    pub(crate) fn start_executable(
        executable: PathBuf,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        validate_version(&executable, cancellation)?;
        if cancellation.is_cancelled() {
            return Err("Codex startup cancelled".into());
        }
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
        copy_allowed_environment(&mut command);
        configure_process_group(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_dir_all(&cwd);
                return Err(format!("cannot start Codex app-server: {error}"));
            }
        };
        let input = child.stdin.take().ok_or("Codex stdin unavailable")?;
        let output = child.stdout.take().ok_or("Codex stdout unavailable")?;
        let error = child.stderr.take().ok_or("Codex stderr unavailable")?;
        let (sender, messages) = mpsc::sync_channel(PROTOCOL_QUEUE_CAPACITY);
        let reader_failure = Arc::new(Mutex::new(None));
        let stdout_reader = spawn_stdout_reader(output, sender, Arc::clone(&reader_failure));
        let stderr_ring = Arc::new(Mutex::new(StderrRing::default()));
        let stderr_reader = spawn_stderr_reader(error, Arc::clone(&stderr_ring));
        control_pid.store(child.id() as i32, Ordering::SeqCst);
        let mut process = Self {
            child: Some(child),
            input: Some(input),
            messages: Some(messages),
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            reader_failure,
            _stderr_ring: stderr_ring,
            cwd: Some(cwd),
            next_id: 1,
            pending_ids: HashSet::with_capacity(MAX_PENDING_IDS),
            control_pid,
        };
        if let Err(error) = process.initialize(cancellation) {
            process.shutdown();
            return Err(error);
        }
        Ok(process)
    }

    fn initialize(&mut self, cancellation: &CancellationToken) -> Result<(), String> {
        self.call(
            "initialize",
            json!({"clientInfo":{"name":"interview-tutor","title":"Interview Tutor","version":env!("CARGO_PKG_VERSION")},"capabilities":null}),
            STARTUP_TIMEOUT,
            cancellation,
        )?;
        let initialized = protocol::notification("initialized", json!({}))?;
        self.send(&initialized)?;
        Ok(())
    }

    pub fn account_ready(&mut self) -> Result<bool, String> {
        self.account_ready_with_cancellation(&CancellationToken::new())
    }

    pub fn account_ready_with_cancellation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<bool, String> {
        let result = self.call(
            "account/read",
            json!({"refreshToken":false}),
            STARTUP_TIMEOUT,
            cancellation,
        )?;
        let account_type = result.pointer("/account/type").and_then(Value::as_str);
        Ok(account_type == Some("chatgpt")
            && result.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(true))
    }

    pub fn start_thread(&mut self) -> Result<String, String> {
        self.start_thread_with_cancellation(&CancellationToken::new())
    }

    pub fn start_thread_with_cancellation(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        let cwd = self
            .cwd
            .as_ref()
            .and_then(|path| path.to_str())
            .ok_or("temporary path is not UTF-8")?
            .to_string();
        let result = self.call(
            "thread/start",
            json!({
                "ephemeral":true,"cwd":cwd,"sandbox":"read-only","approvalPolicy":"never",
                "config":{"web_search":"disabled"}
            }),
            STARTUP_TIMEOUT,
            cancellation,
        )?;
        let settings_match = result.get("cwd").and_then(Value::as_str) == Some(cwd.as_str())
            && result.pointer("/thread/cwd").and_then(Value::as_str) == Some(cwd.as_str())
            && result.pointer("/thread/ephemeral").and_then(Value::as_bool) == Some(true)
            && result.pointer("/thread/path") == Some(&Value::Null)
            && result.get("approvalPolicy").and_then(Value::as_str) == Some("never")
            && result.pointer("/sandbox/type").and_then(Value::as_str) == Some("readOnly")
            && result
                .pointer("/sandbox/networkAccess")
                .and_then(Value::as_bool)
                == Some(false);
        if !settings_match {
            return self.protocol_failure(
                "Codex did not enforce the requested cwd/ephemeral/read-only/no-network/never-approve settings",
            );
        }
        match result.pointer("/thread/id").and_then(Value::as_str) {
            Some(thread_id) => Ok(thread_id.to_string()),
            None => self.protocol_failure("Codex thread/start omitted thread id"),
        }
    }

    pub fn turn(
        &mut self,
        thread_id: &str,
        input: String,
        output_schema: Value,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        self.turn_with_timeout(thread_id, input, output_schema, cancellation, TURN_TIMEOUT)
    }

    fn turn_with_timeout(
        &mut self,
        thread_id: &str,
        input: String,
        output_schema: Value,
        cancellation: &CancellationToken,
        turn_timeout: Duration,
    ) -> Result<String, String> {
        if input.len() > protocol::MAX_JSON_LINE_BYTES / 2 {
            return Err("Codex prompt exceeds bound".into());
        }
        let result = self.call(
            "turn/start",
            json!({"threadId":thread_id,"input":[{"type":"text","text":input}],"outputSchema":output_schema,"approvalPolicy":"never","sandboxPolicy":{"type":"readOnly","networkAccess":false}}),
            STARTUP_TIMEOUT,
            cancellation,
        )?;
        let turn_id = match result.pointer("/turn/id").and_then(Value::as_str) {
            Some(turn_id) => turn_id.to_string(),
            None => return self.protocol_failure("Codex turn/start omitted turn id"),
        };
        let deadline = Instant::now() + turn_timeout;
        let mut text = String::new();
        let mut active_item_id: Option<String> = None;
        let mut interruption: Option<Interruption> = None;
        loop {
            let now = Instant::now();
            if interruption.is_none() && (cancellation.is_cancelled() || now >= deadline) {
                let reason = if cancellation.is_cancelled() {
                    InterruptReason::Cancelled
                } else {
                    InterruptReason::TimedOut
                };
                let request_id = self.send_request(
                    "turn/interrupt",
                    json!({"threadId":thread_id,"turnId":turn_id}),
                )?;
                interruption = Some(Interruption {
                    reason,
                    request_id,
                    deadline: now + INTERRUPT_TIMEOUT,
                    response_received: false,
                    turn_completed: false,
                });
            }
            if let Some(state) = interruption.as_ref() {
                if state.response_received && state.turn_completed {
                    return Err(state.reason.message(turn_timeout));
                }
                if now >= state.deadline {
                    let message = format!(
                        "{}; Codex did not acknowledge turn/interrupt within 2s",
                        state.reason.message(turn_timeout)
                    );
                    return self.protocol_failure(&message);
                }
            }
            let wait_deadline = interruption
                .as_ref()
                .map_or(deadline, |state| state.deadline);
            let wait = POLL_INTERVAL.min(wait_deadline.saturating_duration_since(Instant::now()));
            let Some(incoming) = self.receive_poll(wait)? else {
                continue;
            };
            match incoming {
                Incoming::Response { id, result } => {
                    if !self.pending_ids.remove(&id) {
                        return self
                            .protocol_failure(&format!("unexpected Codex response id {id}"));
                    }
                    let Some(state) = interruption.as_mut() else {
                        return self
                            .protocol_failure(&format!("unexpected Codex response id {id}"));
                    };
                    if id != state.request_id {
                        return self
                            .protocol_failure(&format!("unexpected Codex response id {id}"));
                    }
                    if let Err(error) = result {
                        return self
                            .protocol_failure(&format!("Codex turn/interrupt failed: {error}"));
                    }
                    state.response_received = true;
                }
                Incoming::ServerRequest { id, method, params } => {
                    self.handle_server_request(&id, &method, &params)?;
                    return self.protocol_failure(&format!(
                        "Codex requested forbidden operation {method}"
                    ));
                }
                Incoming::Notification { method, params } => match method.as_str() {
                    "item/started" => {
                        if !self.require_correlation(&params, thread_id, &turn_id)? {
                            continue;
                        }
                        let item_type = self.required_str(
                            &params,
                            "/item/type",
                            "malformed Codex item/started type",
                        )?;
                        if item_type != "agentMessage" {
                            continue;
                        }
                        let item_id =
                            self.required_str(&params, "/item/id", "malformed Codex item/started")?;
                        if active_item_id
                            .as_deref()
                            .is_some_and(|active| active != item_id)
                        {
                            return self.protocol_failure(
                                "Codex started multiple agent-message items for one turn",
                            );
                        }
                        active_item_id = Some(item_id.to_string());
                    }
                    "item/agentMessage/delta" => {
                        if !self.require_correlation(&params, thread_id, &turn_id)? {
                            continue;
                        }
                        let item_id = self.required_str(
                            &params,
                            "/itemId",
                            "malformed Codex agent delta item id",
                        )?;
                        if active_item_id.as_deref() != Some(item_id) {
                            return self.protocol_failure(
                                "Codex agent delta did not match the active item",
                            );
                        }
                        let delta =
                            self.required_str(&params, "/delta", "malformed Codex agent delta")?;
                        if text.len().saturating_add(delta.len()) > protocol::MAX_ASSISTANT_BYTES {
                            return self.protocol_failure("Codex response exceeds 64 KiB");
                        }
                        text.push_str(delta);
                    }
                    "item/completed" => {
                        if !self.require_correlation(&params, thread_id, &turn_id)? {
                            continue;
                        }
                        let item_type = self.required_str(
                            &params,
                            "/item/type",
                            "malformed Codex completed item type",
                        )?;
                        if item_type != "agentMessage" {
                            continue;
                        }
                        let item_id = self.required_str(
                            &params,
                            "/item/id",
                            "malformed Codex completed item id",
                        )?;
                        if active_item_id.as_deref() != Some(item_id) {
                            return self.protocol_failure(
                                "Codex completed item did not match the active item",
                            );
                        }
                        let completed = self.required_str(
                            &params,
                            "/item/text",
                            "malformed Codex completed agent message",
                        )?;
                        if completed.len() > protocol::MAX_ASSISTANT_BYTES {
                            return self.protocol_failure("Codex response exceeds 64 KiB");
                        }
                        text = completed.to_string();
                    }
                    "turn/completed" => {
                        let completed_thread_id = self.required_str(
                            &params,
                            "/threadId",
                            "malformed Codex completed thread id",
                        )?;
                        if completed_thread_id != thread_id {
                            continue;
                        }
                        let completed_turn_id = self.required_str(
                            &params,
                            "/turn/id",
                            "malformed Codex completed turn id",
                        )?;
                        if completed_turn_id != turn_id {
                            continue;
                        }
                        let status = self.required_str(
                            &params,
                            "/turn/status",
                            "malformed Codex completed turn status",
                        )?;
                        if let Some(state) = interruption.as_mut() {
                            if !matches!(status, "interrupted" | "completed") {
                                return Err("Codex turn failed while being interrupted".into());
                            }
                            state.turn_completed = true;
                        } else if status == "completed" {
                            return Ok(text);
                        } else {
                            return Err("Codex turn did not complete successfully".into());
                        }
                    }
                    "error" | "turn/error" => {
                        if !self.require_correlation(&params, thread_id, &turn_id)? {
                            continue;
                        }
                        let will_retry = self.required_bool(
                            &params,
                            "/willRetry",
                            "malformed Codex turn error retry flag",
                        )?;
                        if will_retry {
                            continue;
                        }
                        return Err("Codex reported a terminal turn error".into());
                    }
                    name if is_forbidden_notification(name) => {
                        return self.protocol_failure(
                            "Codex requested a forbidden approval without a request id",
                        );
                    }
                    _ => {}
                },
            }
        }
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<Value, String> {
        let id = self.send_request(method, params)?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancellation.is_cancelled() {
                return self.protocol_failure(&format!("Codex {method} cancelled"));
            }
            if Instant::now() >= deadline {
                return self.protocol_failure(&format!("Codex {method} timed out"));
            }
            let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
            let Some(incoming) = self.receive_poll(wait)? else {
                continue;
            };
            match incoming {
                Incoming::Response {
                    id: response_id,
                    result,
                } => {
                    if !self.pending_ids.remove(&response_id) || response_id != id {
                        return self.protocol_failure(&format!(
                            "unexpected Codex response id {response_id}"
                        ));
                    }
                    return result;
                }
                Incoming::ServerRequest { id, method, params } => {
                    self.handle_server_request(&id, &method, &params)?;
                    return self.protocol_failure(&format!(
                        "Codex requested forbidden operation {method}"
                    ));
                }
                Incoming::Notification { method, .. } if is_forbidden_notification(&method) => {
                    return self.protocol_failure(
                        "Codex requested a forbidden approval without a request id",
                    );
                }
                Incoming::Notification { .. } => {}
            }
        }
    }

    fn require_correlation(
        &mut self,
        params: &Value,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<bool, String> {
        match correlates(params, thread_id, turn_id) {
            Ok(matches) => Ok(matches),
            Err(error) => self.protocol_failure(&error),
        }
    }

    fn required_str<'a>(
        &mut self,
        value: &'a Value,
        pointer: &str,
        error: &str,
    ) -> Result<&'a str, String> {
        match value.pointer(pointer).and_then(Value::as_str) {
            Some(value) => Ok(value),
            None => self.protocol_failure(error),
        }
    }

    fn required_bool(&mut self, value: &Value, pointer: &str, error: &str) -> Result<bool, String> {
        match value.pointer(pointer).and_then(Value::as_bool) {
            Some(value) => Ok(value),
            None => self.protocol_failure(error),
        }
    }

    fn handle_server_request(
        &mut self,
        id: &RequestId,
        method: &str,
        params: &Value,
    ) -> Result<(), String> {
        if !params.is_object() {
            return self.protocol_failure("malformed Codex server request params");
        }
        let Some(result) = decline_result(method) else {
            return self.protocol_failure(&format!("unknown Codex server request {method}"));
        };
        let response = protocol::server_response(id, result)?;
        self.send(&response)
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<u64, String> {
        if self.pending_ids.len() >= MAX_PENDING_IDS {
            return self.protocol_failure("Codex pending request limit exceeded");
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("request id overflow");
        let bytes = protocol::request(id, method, params)?;
        self.send(&bytes)?;
        assert!(self.pending_ids.insert(id));
        assert!(self.pending_ids.len() <= MAX_PENDING_IDS);
        Ok(id)
    }

    fn receive_poll(&mut self, timeout: Duration) -> Result<Option<Incoming>, String> {
        let failure = self
            .reader_failure
            .lock()
            .expect("reader failure lock")
            .clone();
        if let Some(error) = failure {
            return self.protocol_failure(&error);
        }
        let result = match self.messages.as_ref() {
            Some(messages) => messages.recv_timeout(timeout),
            None => return Err("Codex protocol reader stopped".into()),
        };
        match result {
            Ok(line) => match protocol::decode(&line) {
                Ok(incoming) => Ok(Some(incoming)),
                Err(error) => self.protocol_failure(&error),
            },
            Err(RecvTimeoutError::Timeout) => {
                let failure = self
                    .reader_failure
                    .lock()
                    .expect("reader failure lock")
                    .clone();
                if let Some(error) = failure {
                    self.protocol_failure(&error)
                } else {
                    Ok(None)
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                let failure = self
                    .reader_failure
                    .lock()
                    .expect("reader failure lock")
                    .clone();
                self.protocol_failure(
                    failure
                        .as_deref()
                        .unwrap_or("Codex protocol reader stopped"),
                )
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let result = self
            .input
            .as_mut()
            .ok_or_else(|| "Codex stdin unavailable".to_string())?
            .write_all(bytes)
            .and_then(|_| self.input.as_mut().expect("input checked").flush());
        if let Err(error) = result {
            return self.protocol_failure(&format!("cannot write to Codex: {error}"));
        }
        Ok(())
    }

    fn protocol_failure<T>(&mut self, message: &str) -> Result<T, String> {
        self.shutdown();
        Err(message.to_string())
    }

    pub(crate) fn is_usable(&self) -> bool {
        self.child.is_some()
    }

    fn shutdown(&mut self) {
        self.input.take();
        if let Some(child) = self.child.as_mut() {
            terminate_child_group(child, SHUTDOWN_TIMEOUT);
        }
        self.child.take();
        self.control_pid.store(0, Ordering::SeqCst);
        self.messages.take();
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        if let Some(cwd) = self.cwd.take() {
            let _ = fs::remove_dir_all(cwd);
        }
        self.pending_ids.clear();
    }

    #[cfg(test)]
    fn stderr_len(&self) -> usize {
        self._stderr_ring
            .lock()
            .expect("stderr ring lock")
            .bytes
            .len()
    }

    #[cfg(test)]
    fn cwd_path(&self) -> PathBuf {
        self.cwd.as_ref().expect("live process cwd").clone()
    }
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Copy)]
enum InterruptReason {
    Cancelled,
    TimedOut,
}

impl InterruptReason {
    fn message(self, timeout: Duration) -> String {
        match self {
            Self::Cancelled => "Codex turn cancelled".into(),
            Self::TimedOut => format!("Codex turn timed out after {}ms", timeout.as_millis()),
        }
    }
}

struct Interruption {
    reason: InterruptReason,
    request_id: u64,
    deadline: Instant,
    response_received: bool,
    turn_completed: bool,
}

#[derive(Default)]
struct StderrRing {
    bytes: VecDeque<u8>,
    truncated: bool,
}

impl StderrRing {
    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= MAX_STDERR_BYTES {
            self.bytes.clear();
            self.bytes
                .extend(chunk[chunk.len() - MAX_STDERR_BYTES..].iter().copied());
            self.truncated = true;
            return;
        }
        let excess = self
            .bytes
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(MAX_STDERR_BYTES);
        if excess > 0 {
            self.bytes.drain(..excess);
            self.truncated = true;
        }
        self.bytes.extend(chunk.iter().copied());
        assert!(self.bytes.len() <= MAX_STDERR_BYTES);
    }
}

fn spawn_stdout_reader(
    mut output: impl Read + Send + 'static,
    sender: SyncSender<Vec<u8>>,
    failure: Arc<Mutex<Option<String>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let line = match read_bounded_line(&mut output) {
                Ok(Some(line)) => line,
                Ok(None) => {
                    set_reader_failure(&failure, "Codex app-server closed stdout");
                    break;
                }
                Err(error) => {
                    set_reader_failure(&failure, &error);
                    break;
                }
            };
            match sender.try_send(line) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    set_reader_failure(&failure, "Codex protocol queue overflow");
                    break;
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    })
}

fn set_reader_failure(failure: &Mutex<Option<String>>, message: &str) {
    let mut failure = failure.lock().expect("reader failure lock");
    if failure.is_none() {
        *failure = Some(message.to_string());
    }
}

fn spawn_stderr_reader(
    mut error: impl Read + Send + 'static,
    ring: Arc<Mutex<StderrRing>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match error.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => ring
                    .lock()
                    .expect("stderr ring lock")
                    .push(&buffer[..count]),
                Err(_) => break,
            }
        }
    })
}

fn correlates(params: &Value, thread_id: &str, turn_id: &str) -> Result<bool, String> {
    let event_thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or("malformed Codex event thread id")?;
    let event_turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .ok_or("malformed Codex event turn id")?;
    Ok(event_thread_id == thread_id && event_turn_id == turn_id)
}

fn decline_result(method: &str) -> Option<Value> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({"decision":"cancel"}))
        }
        "item/permissions/requestApproval" => {
            Some(json!({"permissions":{"fileSystem":null,"network":null},"scope":"turn"}))
        }
        "item/tool/requestUserInput" => Some(json!({"answers":{}})),
        "mcpServer/elicitation/request" => Some(json!({"action":"cancel"})),
        "applyPatchApproval" | "execCommandApproval" => Some(json!({"decision":"abort"})),
        _ => None,
    }
}

fn is_forbidden_notification(method: &str) -> bool {
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

fn copy_allowed_environment(command: &mut Command) {
    for name in allowed_environment_names() {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

pub(crate) fn configured_executable() -> Result<PathBuf, String> {
    let configured = std::env::var_os("INTERVIEW_TUTOR_CODEX_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"));
    resolve_executable(&configured)
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

fn validate_version(executable: &Path, cancellation: &CancellationToken) -> Result<(), String> {
    validate_version_with_timeout(executable, cancellation, VERSION_TIMEOUT)
}

fn validate_version_with_timeout(
    executable: &Path,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<(), String> {
    let capture = bounded_version_capture(executable, cancellation, timeout)?;
    if !capture.status.success() {
        return Err(format!(
            "cannot query Codex version: process exited with {}",
            capture.status
        ));
    }
    if capture.truncated {
        return Err("cannot query Codex version: output exceeded 64 KiB".into());
    }
    let version = std::str::from_utf8(&capture.stdout)
        .map_err(|_| "cannot query Codex version: stdout was not UTF-8")?
        .trim();
    if !matches!(version, "codex-cli 0.146.0" | "codex-cli 0.147.0") {
        return Err(format!(
            "unsupported Codex CLI {version}; install verified version 0.146.0 or 0.147.0"
        ));
    }
    Ok(())
}

struct VersionCapture {
    status: ExitStatus,
    stdout: Vec<u8>,
    truncated: bool,
}

#[derive(Default)]
struct CaptureBuffers {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    total: usize,
    truncated: bool,
}

impl CaptureBuffers {
    fn push(&mut self, stdout: bool, bytes: &[u8]) {
        let remaining = MAX_VERSION_OUTPUT_BYTES.saturating_sub(self.total);
        let retained = bytes.len().min(remaining);
        if stdout {
            self.stdout.extend_from_slice(&bytes[..retained]);
        } else {
            self.stderr.extend_from_slice(&bytes[..retained]);
        }
        self.total += retained;
        if retained != bytes.len() {
            self.truncated = true;
        }
        assert!(self.total <= MAX_VERSION_OUTPUT_BYTES);
    }
}

fn bounded_version_capture(
    executable: &Path,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<VersionCapture, String> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    copy_allowed_environment(&mut command);
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot query Codex version: {error}"))?;
    let pid = child.id() as i32;
    let stdout = child
        .stdout
        .take()
        .ok_or("Codex version stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Codex version stderr unavailable")?;
    let buffers = Arc::new(Mutex::new(CaptureBuffers::default()));
    let stdout_reader = spawn_capture_reader(stdout, Arc::clone(&buffers), true);
    let stderr_reader = spawn_capture_reader(stderr, Arc::clone(&buffers), false);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("Codex version probe cancelled".into());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot wait for Codex version: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(format!(
                "Codex version probe timed out after {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    };
    if process_group_exists(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    let mut buffers = Arc::try_unwrap(buffers)
        .map_err(|_| "Codex version capture still shared")?
        .into_inner()
        .map_err(|_| "Codex version capture lock poisoned")?;
    let _ = std::mem::take(&mut buffers.stderr);
    Ok(VersionCapture {
        status,
        stdout: buffers.stdout,
        truncated: buffers.truncated,
    })
}

fn spawn_capture_reader(
    mut reader: impl Read + Send + 'static,
    buffers: Arc<Mutex<CaptureBuffers>>,
    stdout: bool,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => buffers
                    .lock()
                    .expect("version capture lock")
                    .push(stdout, &chunk[..count]),
                Err(_) => break,
            }
        }
    })
}

fn configure_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn terminate_child_group(child: &mut Child, grace: Duration) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    let mut reaped = false;
    loop {
        if !reaped {
            reaped = child.try_wait().ok().flatten().is_some();
        }
        if !process_group_exists(pid) || Instant::now() >= deadline {
            break;
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    if process_group_exists(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    if !reaped {
        let _ = child.wait();
    }
}

fn process_group_exists(pid: i32) -> bool {
    let result = unsafe { libc::kill(-pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
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
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    static ENVIRONMENT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct FakeEnvironment {
        directory: PathBuf,
        old_codex_home: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl FakeEnvironment {
        fn new(mode: &str) -> Self {
            let lock = ENVIRONMENT_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let directory = empty_temp_dir().unwrap();
            fs::write(directory.join("fake-mode"), mode).unwrap();
            let old_codex_home = std::env::var_os("CODEX_HOME");
            unsafe {
                std::env::set_var("CODEX_HOME", &directory);
            }
            Self {
                directory,
                old_codex_home,
                _lock: lock,
            }
        }
    }

    impl Drop for FakeEnvironment {
        fn drop(&mut self) {
            unsafe {
                match self.old_codex_home.as_ref() {
                    Some(value) => std::env::set_var("CODEX_HOME", value),
                    None => std::env::remove_var("CODEX_HOME"),
                }
            }
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    struct EnvironmentVariable {
        name: &'static str,
        old_value: Option<std::ffi::OsString>,
    }

    impl EnvironmentVariable {
        fn set(name: &'static str, value: &str) -> Self {
            let old_value = std::env::var_os(name);
            unsafe {
                std::env::set_var(name, value);
            }
            Self { name, old_value }
        }
    }

    impl Drop for EnvironmentVariable {
        fn drop(&mut self) {
            unsafe {
                match self.old_value.as_ref() {
                    Some(value) => std::env::set_var(self.name, value),
                    None => std::env::remove_var(self.name),
                }
            }
        }
    }

    fn fake_executable() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_codex_app_server.py")
    }

    fn fake_process(mode: &str) -> (FakeEnvironment, CodexProcess) {
        let environment = FakeEnvironment::new(mode);
        let process = CodexProcess::start_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        (environment, process)
    }

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
    fn version_allowlist_rejects_unverified_patch_releases() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("codex-version");
        fs::write(&executable, "#!/bin/sh\necho 'codex-cli 0.146.9'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_version(&executable, &CancellationToken::new()).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn version_capture_rejects_more_than_combined_64_kib() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("codex-version-flood.py");
        fs::write(
            &executable,
            "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('x' * 65537)\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let error = validate_version(&executable, &CancellationToken::new()).unwrap_err();
        assert!(error.contains("output exceeded 64 KiB"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn version_allowlist_accepts_only_both_verified_exact_versions() {
        let directory = empty_temp_dir().unwrap();
        for version in ["0.146.0", "0.147.0"] {
            let executable = directory.join(format!("codex-{version}"));
            fs::write(
                &executable,
                format!("#!/bin/sh\necho 'codex-cli {version}'\n"),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            validate_version(&executable, &CancellationToken::new()).unwrap();
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn hostile_version_probe_is_bounded_cancellable_and_reaps_group() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("hostile-version.py");
        let pid_path = directory.join("version-pids");
        fs::write(
            &executable,
            format!(
                "#!/usr/bin/env python3\nimport os,signal,sys,time\nchild=os.fork()\nif child==0:\n signal.signal(signal.SIGTERM, signal.SIG_IGN)\n while True: time.sleep(1)\nopen({:?},'w').write(f'{{os.getpid()}} {{child}}')\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True:\n sys.stdout.write('x'*8192); sys.stdout.flush()\n",
                pid_path.to_string_lossy()
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let error = validate_version_with_timeout(
            &executable,
            &CancellationToken::new(),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
        let pids = fs::read_to_string(&pid_path)
            .unwrap()
            .split_whitespace()
            .map(|pid| pid.parse::<i32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pids.len(), 2);
        let descendant_deadline = Instant::now() + Duration::from_secs(1);
        while unsafe { libc::kill(pids[1], 0) } == 0 && Instant::now() < descendant_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(unsafe { libc::kill(pids[0], 0) }, -1);
        assert_eq!(unsafe { libc::kill(pids[1], 0) }, -1);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();
        let error =
            validate_version_with_timeout(&executable, &cancellation, Duration::from_secs(10))
                .unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fake_app_server_handshake_effective_settings_deltas_and_unknown_notifications() {
        let (_environment, mut process) = fake_process("normal");
        assert!(process.account_ready().unwrap());
        let cwd = process.cwd_path();
        let thread = process.start_thread().unwrap();
        let response = process
            .turn(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(response.contains("What invariant holds?"));
        drop(process);
        assert!(!cwd.exists());
    }

    #[test]
    fn privacy_fixture_records_only_allowed_environment_and_prompt_payload() {
        let environment = FakeEnvironment::new("normal");
        let _sentinel = EnvironmentVariable::set("INTERVIEW_TUTOR_SENTINEL", "do-not-copy");
        let _api_key = EnvironmentVariable::set("OPENAI_API_KEY", "test-key-must-not-copy");
        let mut process = CodexProcess::start_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(process.account_ready().unwrap());
        let cwd = process.cwd_path();
        let thread = process.start_thread().unwrap();
        process
            .turn(
                &thread,
                "contract\nINPUT_JSON:{\"statement\":\"statement\",\"source\":\"source\",\"latestTestOutput\":\"output\",\"transcript\":\"transcript\",\"userQuestion\":\"question\"}".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
            )
            .unwrap();
        drop(process);
        assert!(!cwd.exists());

        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let process_record = records
            .iter()
            .find(|record| record["kind"] == "process")
            .unwrap();
        assert_eq!(process_record["argv"], json!(["app-server", "--stdio"]));
        assert_eq!(process_record["cwd"], cwd.to_str().unwrap());
        let environment_names = process_record["environment_names"].as_array().unwrap();
        for allowed in ["HOME", "CODEX_HOME", "PATH"] {
            assert!(environment_names.iter().any(|name| name == allowed));
        }
        assert!(
            !environment_names
                .iter()
                .any(|name| name == "OPENAI_API_KEY" || name == "INTERVIEW_TUTOR_SENTINEL")
        );
        let messages = records
            .iter()
            .filter_map(|record| record.get("json"))
            .collect::<Vec<_>>();
        assert!(messages.iter().any(|message| {
            message["method"] == "account/read"
                && message["params"] == json!({"refreshToken":false})
        }));
        let thread_start = messages
            .iter()
            .find(|message| message["method"] == "thread/start")
            .unwrap();
        assert_eq!(thread_start["params"]["cwd"], cwd.to_str().unwrap());
        assert_eq!(thread_start["params"]["ephemeral"], true);
        assert_eq!(thread_start["params"]["approvalPolicy"], "never");
        assert_eq!(thread_start["params"]["sandbox"], "read-only");
        assert_eq!(thread_start["params"]["config"]["web_search"], "disabled");
        let turn_start = messages
            .iter()
            .find(|message| message["method"] == "turn/start")
            .unwrap();
        assert_eq!(turn_start["params"]["approvalPolicy"], "never");
        assert_eq!(
            turn_start["params"]["sandboxPolicy"],
            json!({"type":"readOnly","networkAccess":false})
        );
        let input = turn_start["params"]["input"][0]["text"].as_str().unwrap();
        assert!(!input.to_ascii_lowercase().contains("tool"));
        let payload: Value =
            serde_json::from_str(input.split_once("INPUT_JSON:").unwrap().1).unwrap();
        let mut keys = payload
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "latestTestOutput",
                "source",
                "statement",
                "transcript",
                "userQuestion"
            ]
        );
    }

    #[test]
    fn session_restarts_once_after_death_without_replaying_failed_turn() {
        let environment = FakeEnvironment::new("restart");
        let mut session = crate::codex::CodexSession::connect_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        let request = || crate::codex::InterviewRequest {
            mode: crate::codex::prompt::Mode::Interviewer,
            statement: "statement",
            source: "source",
            latest_output: "output",
            question: "question",
            source_revision: 1,
            solved: false,
        };
        assert!(
            session
                .ask(request())
                .unwrap_err()
                .contains("closed stdout")
        );
        assert_eq!(session.ask(request()).unwrap(), "What invariant holds?");
        drop(session);

        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let process_records = records
            .iter()
            .filter(|record| record["kind"] == "process")
            .collect::<Vec<_>>();
        assert_eq!(process_records.len(), 2);
        for record in process_records {
            assert!(!Path::new(record["cwd"].as_str().unwrap()).exists());
        }
        let turn_starts = records
            .iter()
            .filter_map(|record| record.get("json"))
            .filter(|message| message["method"] == "turn/start")
            .collect::<Vec<_>>();
        assert_eq!(turn_starts.len(), 2);
        for turn in turn_starts {
            let input = turn["params"]["input"][0]["text"].as_str().unwrap();
            let payload: Value =
                serde_json::from_str(input.split_once("INPUT_JSON:").unwrap().1).unwrap();
            assert_eq!(payload["transcript"], "");
        }
    }

    #[test]
    fn session_restart_is_limited_to_one_new_process() {
        let environment = FakeEnvironment::new("child-death");
        let mut session = crate::codex::CodexSession::connect_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        let request = || crate::codex::InterviewRequest {
            mode: crate::codex::prompt::Mode::Interviewer,
            statement: "statement",
            source: "source",
            latest_output: "output",
            question: "question",
            source_revision: 1,
            solved: false,
        };
        assert!(
            session
                .ask(request())
                .unwrap_err()
                .contains("closed stdout")
        );
        assert!(
            session
                .ask(request())
                .unwrap_err()
                .contains("closed stdout")
        );
        assert!(
            session
                .ask(request())
                .unwrap_err()
                .contains("restart limit")
        );
        drop(session);
        assert_eq!(
            fs::read_to_string(environment.directory.join("fake-start-count")).unwrap(),
            "2"
        );
    }

    #[test]
    fn bad_thread_cwd_and_terminal_current_error_fail_closed() {
        let (environment, mut process) = fake_process("bad-cwd");
        process.account_ready().unwrap();
        assert!(
            process
                .start_thread()
                .unwrap_err()
                .contains("requested cwd")
        );
        assert!(!process.is_usable());
        drop(process);
        drop(environment);

        let (_environment, mut process) = fake_process("terminal-error");
        process.account_ready().unwrap();
        let thread = process.start_thread().unwrap();
        let error = process
            .turn(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert!(error.contains("terminal turn error"));
    }

    #[test]
    fn decline_payloads_match_generated_response_shapes() {
        assert_eq!(
            decline_result("item/commandExecution/requestApproval").unwrap(),
            json!({"decision":"cancel"})
        );
        assert_eq!(
            decline_result("item/fileChange/requestApproval").unwrap(),
            json!({"decision":"cancel"})
        );
        assert_eq!(
            decline_result("item/permissions/requestApproval").unwrap(),
            json!({"permissions":{"fileSystem":null,"network":null},"scope":"turn"})
        );
        assert_eq!(
            decline_result("item/tool/requestUserInput").unwrap(),
            json!({"answers":{}})
        );
        assert_eq!(
            decline_result("mcpServer/elicitation/request").unwrap(),
            json!({"action":"cancel"})
        );
        assert!(decline_result("item/tool/call").is_none());
    }

    #[test]
    fn child_death_eof_malformed_oversize_and_queue_flood_fail_boundedly() {
        for (mode, expected) in [
            ("child-death", "closed stdout"),
            ("eof", "mid-message"),
            ("malformed", "malformed JSON"),
            ("oversize", "exceeds 2 MiB"),
        ] {
            let (_environment, mut process) = fake_process(mode);
            process.account_ready().unwrap();
            let thread = process.start_thread().unwrap();
            let error = process
                .turn(
                    &thread,
                    "bounded prompt".into(),
                    json!({"type":"object"}),
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert!(error.contains(expected), "{mode}: {error}");
            assert!(!process.is_usable());
        }
        let input = b"{}\n".repeat(PROTOCOL_QUEUE_CAPACITY + 1);
        let (sender, receiver) = mpsc::sync_channel(PROTOCOL_QUEUE_CAPACITY);
        let failure = Arc::new(Mutex::new(None));
        let reader = spawn_stdout_reader(std::io::Cursor::new(input), sender, Arc::clone(&failure));
        reader.join().unwrap();
        assert_eq!(
            failure.lock().expect("failure lock").as_deref(),
            Some("Codex protocol queue overflow")
        );
        drop(receiver);
    }

    #[test]
    fn cancellation_and_timeout_interrupt_once_then_require_ack() {
        let (environment, mut process) = fake_process("timeout-ack");
        process.account_ready().unwrap();
        let thread = process.start_thread().unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_thread = cancellation.clone();
        let cancel = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancellation_thread.cancel();
        });
        let error = process
            .turn(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &cancellation,
            )
            .unwrap_err();
        cancel.join().unwrap();
        assert!(error.contains("cancelled"));
        assert!(process.is_usable());
        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let interrupts = records
            .iter()
            .filter_map(|record| record.get("json"))
            .filter(|message| message["method"] == "turn/interrupt")
            .collect::<Vec<_>>();
        assert_eq!(interrupts.len(), 1);
        assert_eq!(
            interrupts[0]["params"],
            json!({"threadId":thread,"turnId":"turn-1"})
        );
        drop(process);
        drop(environment);

        let (_environment, mut process) = fake_process("timeout-ack");
        process.account_ready().unwrap();
        let thread = process.start_thread().unwrap();
        let error = process
            .turn_with_timeout(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
                Duration::from_millis(50),
            )
            .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(process.is_usable());
    }

    #[test]
    fn missing_interrupt_ack_kills_process_within_bound() {
        let (_environment, mut process) = fake_process("interrupt-no-ack");
        process.account_ready().unwrap();
        let thread = process.start_thread().unwrap();
        let started = Instant::now();
        let error = process
            .turn_with_timeout(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
                Duration::from_millis(25),
            )
            .unwrap_err();
        assert!(error.contains("did not acknowledge"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!process.is_usable());
    }

    #[test]
    fn typed_server_requests_are_declined_and_unknown_requests_fail_closed() {
        for mode in [
            "approval-command",
            "approval-file",
            "approval-permissions",
            "approval-user-input",
            "approval-mcp",
        ] {
            let (_environment, mut process) = fake_process(mode);
            process.account_ready().unwrap();
            let thread = process.start_thread().unwrap();
            let error = process
                .turn(
                    &thread,
                    "bounded prompt".into(),
                    json!({"type":"object"}),
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert!(error.contains("forbidden operation"), "{mode}: {error}");
            assert!(!process.is_usable());
        }
        let (_environment, mut process) = fake_process("approval-unknown");
        process.account_ready().unwrap();
        let thread = process.start_thread().unwrap();
        let error = process
            .turn(
                &thread,
                "bounded prompt".into(),
                json!({"type":"object"}),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert!(error.contains("unknown Codex server request"));
        assert!(!process.is_usable());
    }

    #[test]
    fn stderr_ring_is_bounded_and_reader_keeps_draining() {
        let (_environment, process) = fake_process("stderr-flood");
        let deadline = Instant::now() + Duration::from_secs(2);
        while process.stderr_len() < MAX_STDERR_BYTES && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(process.stderr_len(), MAX_STDERR_BYTES);
        assert!(
            process
                ._stderr_ring
                .lock()
                .expect("stderr ring lock")
                .truncated
        );
    }
}
