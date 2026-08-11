use super::protocol::{self, Incoming, RequestId};
use crate::runner::CancellationToken;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const READER_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);
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
    reader_shutdown: Arc<AtomicBool>,
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
        let executable = fs::canonicalize(&executable)
            .map_err(|error| format!("cannot resolve Codex executable: {error}"))?;
        let probed_identity = trusted_executable_identity(&executable)?;
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
        let current_identity = trusted_executable_identity(&executable);
        if current_identity.as_ref() != Ok(&probed_identity) {
            let primary = match current_identity {
                Ok(_) => "Codex executable changed after version probe".to_string(),
                Err(error) => error,
            };
            return Err(combine_cleanup_error(primary, remove_temp_dir(&cwd)));
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let primary = format!("cannot start Codex app-server: {error}");
                return Err(combine_cleanup_error(primary, remove_temp_dir(&cwd)));
            }
        };
        let (input, output, error) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(input), Some(output), Some(error)) => (input, output, error),
                _ => {
                    let cleanup = cleanup_unmanaged_child(&mut child, &cwd);
                    return Err(combine_cleanup_error(
                        "Codex process pipes unavailable".into(),
                        cleanup,
                    ));
                }
            };
        if let Err(primary) = set_nonblocking(output.as_raw_fd(), "Codex stdout")
            .and_then(|()| set_nonblocking(error.as_raw_fd(), "Codex stderr"))
        {
            drop(input);
            drop(output);
            drop(error);
            let cleanup = cleanup_unmanaged_child(&mut child, &cwd);
            return Err(combine_cleanup_error(primary, cleanup));
        }
        let (sender, messages) = mpsc::sync_channel(PROTOCOL_QUEUE_CAPACITY);
        let reader_shutdown = Arc::new(AtomicBool::new(false));
        let reader_failure = Arc::new(Mutex::new(None));
        let stdout_reader = spawn_stdout_reader(
            output,
            sender,
            Arc::clone(&reader_failure),
            Arc::clone(&reader_shutdown),
        );
        let stderr_ring = Arc::new(Mutex::new(StderrRing::default()));
        let stderr_reader = spawn_stderr_reader(
            error,
            Arc::clone(&stderr_ring),
            Arc::clone(&reader_shutdown),
        );
        control_pid.store(child.id() as i32, Ordering::SeqCst);
        let mut process = Self {
            child: Some(child),
            input: Some(input),
            messages: Some(messages),
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            reader_shutdown,
            reader_failure,
            _stderr_ring: stderr_ring,
            cwd: Some(cwd),
            next_id: 1,
            pending_ids: HashSet::with_capacity(MAX_PENDING_IDS),
            control_pid,
        };
        if let Err(primary) = process.initialize(cancellation) {
            let cleanup = process.shutdown();
            return Err(combine_cleanup_error(primary, cleanup));
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
        let cleanup = self.shutdown();
        Err(combine_cleanup_error(message.to_string(), cleanup))
    }

    pub(crate) fn is_usable(&self) -> bool {
        self.child.is_some() && !self.reader_shutdown.load(Ordering::SeqCst)
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        self.input.take();
        self.reader_shutdown.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.as_mut() {
            match terminate_child_group(child, SHUTDOWN_TIMEOUT) {
                Ok(()) => {
                    self.child.take();
                    self.control_pid.store(0, Ordering::SeqCst);
                }
                Err(error) => errors.push(error),
            }
        } else {
            self.control_pid.store(0, Ordering::SeqCst);
        }
        self.messages.take();
        if let Some(reader) = self.stdout_reader.take()
            && reader.join().is_err()
        {
            errors.push("Codex stdout reader panicked".into());
        }
        if let Some(reader) = self.stderr_reader.take()
            && reader.join().is_err()
        {
            errors.push("Codex stderr reader panicked".into());
        }
        if let Some(cwd) = self.cwd.as_ref() {
            match remove_temp_dir(cwd) {
                Ok(()) => self.cwd = None,
                Err(error) => errors.push(error),
            }
        }
        self.pending_ids.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
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
        // Destructors cannot report cleanup failures. Callers that need reporting use shutdown;
        // this fallback is best effort for normal ownership teardown and unwinding.
        let _ = self.shutdown();
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
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut line = Vec::with_capacity(4096);
        let mut drain_deadline = None;
        'read: loop {
            if shutdown.load(Ordering::SeqCst) {
                let deadline =
                    drain_deadline.get_or_insert_with(|| Instant::now() + READER_DRAIN_TIMEOUT);
                if Instant::now() >= *deadline {
                    break;
                }
            }
            match output.read(&mut buffer) {
                Ok(0) => {
                    if !shutdown.load(Ordering::SeqCst) {
                        let message = if line.is_empty() {
                            "Codex app-server closed stdout"
                        } else {
                            "Codex closed stdout mid-message"
                        };
                        set_reader_failure(&failure, message);
                    }
                    break;
                }
                Ok(count) => {
                    for byte in &buffer[..count] {
                        if *byte == b'\n' {
                            let message = std::mem::replace(&mut line, Vec::with_capacity(4096));
                            match sender.try_send(message) {
                                Ok(()) => {}
                                Err(TrySendError::Full(_)) => {
                                    set_reader_failure(&failure, "Codex protocol queue overflow");
                                    break 'read;
                                }
                                Err(TrySendError::Disconnected(_)) => break 'read,
                            }
                        } else {
                            if line.len() == protocol::MAX_JSON_LINE_BYTES {
                                set_reader_failure(&failure, "Codex protocol line exceeds 2 MiB");
                                break 'read;
                            }
                            line.push(*byte);
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    set_reader_failure(&failure, &format!("cannot read Codex output: {error}"));
                    break;
                }
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
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut drain_deadline = None;
        loop {
            if shutdown.load(Ordering::SeqCst) {
                let deadline =
                    drain_deadline.get_or_insert_with(|| Instant::now() + READER_DRAIN_TIMEOUT);
                if Instant::now() >= *deadline {
                    break;
                }
            }
            match error.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => ring
                    .lock()
                    .expect("stderr ring lock")
                    .push(&buffer[..count]),
                Err(read_error) if read_error.kind() == ErrorKind::Interrupted => continue,
                Err(read_error) if read_error.kind() == ErrorKind::WouldBlock => {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
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
    let resolved = if path.components().count() > 1 {
        fs::canonicalize(path)
            .map_err(|error| format!("cannot resolve Codex executable: {error}"))?
    } else {
        let paths = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
        let selected = std::env::split_paths(&paths)
            .map(|base| base.join(path))
            .find(|candidate| candidate.is_file())
            .ok_or("Codex CLI not found; local solve remains available")?;
        fs::canonicalize(selected)
            .map_err(|error| format!("cannot resolve Codex executable: {error}"))?
    };
    trusted_executable_identity(&resolved)?;
    Ok(resolved)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn trusted_executable_identity(path: &Path) -> Result<ExecutableIdentity, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot inspect Codex executable: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Codex executable must be a regular file".into());
    }
    let current_user = unsafe { libc::geteuid() };
    if metadata.uid() != current_user && metadata.uid() != 0 {
        return Err("Codex executable must be owned by the current user or root".into());
    }
    if metadata.mode() & 0o022 != 0 {
        return Err("Codex executable must not be group- or world-writable".into());
    }
    Ok(ExecutableIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        size: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
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
    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(stdout), Some(stderr)) => (stdout, stderr),
        _ => {
            let cleanup = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
            return Err(combine_cleanup_error(
                "Codex version probe pipes unavailable".into(),
                cleanup,
            ));
        }
    };
    if let Err(primary) = set_nonblocking(stdout.as_raw_fd(), "Codex version stdout")
        .and_then(|()| set_nonblocking(stderr.as_raw_fd(), "Codex version stderr"))
    {
        drop(stdout);
        drop(stderr);
        let cleanup = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
        return Err(combine_cleanup_error(primary, cleanup));
    }
    let buffers = Arc::new(Mutex::new(CaptureBuffers::default()));
    let reader_shutdown = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_capture_reader(
        stdout,
        Arc::clone(&buffers),
        true,
        Arc::clone(&reader_shutdown),
    );
    let stderr_reader = spawn_capture_reader(
        stderr,
        Arc::clone(&buffers),
        false,
        Arc::clone(&reader_shutdown),
    );
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        if cancellation.is_cancelled() {
            break Err("Codex version probe cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(format!("cannot wait for Codex version: {error}")),
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "Codex version probe timed out after {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    };

    reader_shutdown.store(true, Ordering::SeqCst);
    let mut cleanup_errors = Vec::new();
    if let Err(error) = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT) {
        cleanup_errors.push(error);
    }
    if stdout_reader.join().is_err() {
        cleanup_errors.push("Codex version stdout reader panicked".into());
    }
    if stderr_reader.join().is_err() {
        cleanup_errors.push("Codex version stderr reader panicked".into());
    }
    let buffers = Arc::try_unwrap(buffers)
        .map_err(|_| "Codex version capture still shared".to_string())?
        .into_inner()
        .map_err(|_| "Codex version capture lock poisoned".to_string())?;
    if !cleanup_errors.is_empty() {
        let cleanup = Err(cleanup_errors.join("; "));
        return Err(match outcome {
            Ok(_) => combine_cleanup_error("cannot clean up Codex version probe".into(), cleanup),
            Err(primary) => combine_cleanup_error(primary, cleanup),
        });
    }
    let status = outcome?;
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
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        let mut drain_deadline = None;
        loop {
            if shutdown.load(Ordering::SeqCst) {
                let deadline =
                    drain_deadline.get_or_insert_with(|| Instant::now() + READER_DRAIN_TIMEOUT);
                if Instant::now() >= *deadline {
                    break;
                }
            }
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => buffers
                    .lock()
                    .expect("version capture lock")
                    .push(stdout, &chunk[..count]),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
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

fn set_nonblocking(file_descriptor: i32, name: &str) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(file_descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(format!(
            "cannot inspect {name} flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(file_descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "cannot make {name} nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn terminate_child_group(child: &mut Child, grace: Duration) -> Result<(), String> {
    let pid = i32::try_from(child.id()).map_err(|_| "Codex child PID exceeds platform bound")?;
    assert!(pid > 0);
    let mut errors = Vec::new();
    let mut reaped = match child.try_wait() {
        Ok(status) => status.is_some(),
        Err(error) => {
            errors.push(format!("cannot poll Codex child: {error}"));
            false
        }
    };
    if let Err(error) = signal_process(-pid, libc::SIGTERM) {
        errors.push(error);
    }
    if !reaped && let Err(error) = signal_process(pid, libc::SIGTERM) {
        errors.push(error);
    }

    let term_deadline = Instant::now() + grace;
    loop {
        if !reaped {
            match child.try_wait() {
                Ok(status) => reaped = status.is_some(),
                Err(error) => {
                    errors.push(format!("cannot poll Codex child: {error}"));
                    break;
                }
            }
        }
        if reaped && !process_group_exists(pid) {
            break;
        }
        if Instant::now() >= term_deadline {
            break;
        }
        thread::sleep(POLL_INTERVAL.min(term_deadline.saturating_duration_since(Instant::now())));
    }

    if process_group_exists(pid)
        && let Err(error) = signal_process(-pid, libc::SIGKILL)
    {
        errors.push(error);
    }
    if !reaped && let Err(error) = signal_process(pid, libc::SIGKILL) {
        errors.push(error);
    }
    let hard_deadline = Instant::now() + KILL_REAP_TIMEOUT;
    loop {
        if !reaped {
            match child.try_wait() {
                Ok(status) => reaped = status.is_some(),
                Err(error) => {
                    errors.push(format!("cannot poll Codex child: {error}"));
                    break;
                }
            }
        }
        if reaped && !process_group_exists(pid) {
            break;
        }
        if Instant::now() >= hard_deadline {
            break;
        }
        thread::sleep(POLL_INTERVAL.min(hard_deadline.saturating_duration_since(Instant::now())));
    }
    if !reaped {
        errors.push(format!(
            "Codex child {pid} was not reaped before the cleanup deadline"
        ));
    }
    if process_group_exists(pid) {
        errors.push(format!(
            "Codex process group {pid} survived the cleanup deadline"
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn signal_process(target: i32, signal: i32) -> Result<(), String> {
    let result = unsafe { libc::kill(target, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "cannot signal Codex process target {target} with signal {signal}: {error}"
        ))
    }
}

fn process_group_exists(pid: i32) -> bool {
    let result = unsafe { libc::kill(-pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn cleanup_unmanaged_child(child: &mut Child, cwd: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = terminate_child_group(child, SHUTDOWN_TIMEOUT) {
        errors.push(error);
    }
    if let Err(error) = remove_temp_dir(cwd) {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn combine_cleanup_error(primary: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => primary,
        Err(error) => format!("{primary}; cleanup failed: {error}"),
    }
}

fn remove_temp_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot remove Codex temporary directory {}: {error}",
            path.display()
        )),
    }
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
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&path)
        .map_err(|error| format!("cannot create Codex temporary directory: {error}"))?;
    let permissions = fs::Permissions::from_mode(0o700);
    if let Err(error) = fs::set_permissions(&path, permissions) {
        let primary = format!("cannot secure Codex temporary directory: {error}");
        return Err(combine_cleanup_error(primary, remove_temp_dir(&path)));
    }
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            let primary = format!("cannot verify Codex temporary directory: {error}");
            return Err(combine_cleanup_error(primary, remove_temp_dir(&path)));
        }
    };
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return Err(combine_cleanup_error(
            "Codex temporary directory is not a mode-0700 directory".into(),
            remove_temp_dir(&path),
        ));
    }
    Ok(path)
}

#[cfg(test)]
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

    struct EscapedProcess(i32);

    impl EscapedProcess {
        fn read(path: &Path) -> Self {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                if let Ok(value) = fs::read_to_string(path) {
                    return Self(value.parse().unwrap());
                }
                assert!(Instant::now() < deadline, "escaped PID was not recorded");
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn pid(&self) -> i32 {
            self.0
        }

        fn kill_and_verify(mut self) {
            assert_eq!(unsafe { libc::kill(self.0, libc::SIGKILL) }, 0);
            let deadline = Instant::now() + Duration::from_secs(2);
            while unsafe { libc::kill(self.0, 0) } == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(unsafe { libc::kill(self.0, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            self.0 = 0;
        }
    }

    impl Drop for EscapedProcess {
        fn drop(&mut self) {
            if self.0 > 0 {
                unsafe {
                    libc::kill(self.0, libc::SIGKILL);
                }
            }
        }
    }

    struct UmaskGuard(libc::mode_t);

    impl UmaskGuard {
        fn set(mask: libc::mode_t) -> Self {
            Self(unsafe { libc::umask(mask) })
        }
    }

    impl Drop for UmaskGuard {
        fn drop(&mut self) {
            unsafe {
                libc::umask(self.0);
            }
        }
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
    fn temp_directory_is_mode_0700_under_default_umask() {
        let _lock = ENVIRONMENT_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _umask = UmaskGuard::set(0o022);
        let directory = empty_temp_dir().unwrap();
        let metadata = fs::metadata(&directory).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.mode() & 0o777, 0o700);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn executable_trust_rejects_non_regular_and_writable_files() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("codex");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        for mode in [0o720, 0o702] {
            fs::set_permissions(&executable, fs::Permissions::from_mode(mode)).unwrap();
            let error = trusted_executable_identity(&executable).unwrap_err();
            assert!(error.contains("group- or world-writable"), "{error}");
        }
        let error = trusted_executable_identity(&directory).unwrap_err();
        assert!(error.contains("regular file"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn executable_swap_after_version_probe_is_rejected_before_app_server_spawn() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("codex-swap");
        let replacement = directory.join("codex-swap.replacement");
        let launched = directory.join("app-server-launched");
        fs::write(
            &executable,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  mv \"$0.replacement\" \"$0\"\n  echo 'codex-cli 0.146.0'\n  exit 0\nfi\nexit 91\n",
        )
        .unwrap();
        fs::write(
            &replacement,
            format!(
                "#!/bin/sh\necho launched > {}\nexit 92\n",
                launched.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
        let error = CodexProcess::start_executable(
            executable,
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .err()
        .expect("executable swap must fail");
        assert!(error.contains("changed after version probe"), "{error}");
        assert!(!launched.exists());
        fs::remove_dir_all(directory).unwrap();
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
    fn escaped_version_descendant_retaining_pipes_does_not_block_reader_join() {
        let environment = FakeEnvironment::new("escaped-version-pipes");
        let started = Instant::now();
        validate_version(&fake_executable(), &CancellationToken::new()).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        let escaped = EscapedProcess::read(&environment.directory.join("escaped-version-pid"));
        assert_eq!(unsafe { libc::kill(escaped.pid(), 0) }, 0);
        escaped.kill_and_verify();
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
    fn escaped_session_descendant_retaining_pipes_does_not_block_shutdown() {
        let (environment, mut process) = fake_process("escaped-session-pipes");
        process.account_ready().unwrap();
        let cwd = process.cwd_path();
        let escaped = EscapedProcess::read(&environment.directory.join("escaped-session-pid"));
        assert_eq!(unsafe { libc::kill(escaped.pid(), 0) }, 0);
        let started = Instant::now();
        process.shutdown().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!cwd.exists());
        assert_eq!(unsafe { libc::kill(escaped.pid(), 0) }, 0);
        escaped.kill_and_verify();
    }

    #[test]
    fn explicit_shutdown_reports_temp_cleanup_failure_without_leaking_fixture_artifacts() {
        let (environment, mut process) = fake_process("normal");
        let actual_cwd = process.cwd_path();
        let not_a_directory = environment.directory.join("cleanup-failure-file");
        fs::write(&not_a_directory, "fixture").unwrap();
        process.cwd = Some(not_a_directory.clone());
        let error = process.shutdown().unwrap_err();
        assert!(
            error.contains("cannot remove Codex temporary directory"),
            "{error}"
        );
        assert!(not_a_directory.is_file());
        fs::remove_file(not_a_directory).unwrap();
        process.cwd = Some(actual_cwd.clone());
        process.shutdown().unwrap();
        assert!(!actual_cwd.exists());
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
    fn fake_account_states_use_stable_schema_and_auth_required_is_not_ready() {
        {
            let (_environment, mut ready) = fake_process("normal");
            assert!(ready.account_ready().unwrap());
        }
        let (_environment, mut auth_required) = fake_process("auth-required");
        assert!(!auth_required.account_ready().unwrap());
    }

    #[test]
    fn session_roles_use_separate_threads_and_only_documented_payload_fields() {
        let environment = FakeEnvironment::new("normal");
        let mut session = crate::codex::CodexSession::connect_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            session
                .ask(crate::codex::InterviewRequest {
                    mode: crate::codex::prompt::Mode::Interviewer,
                    statement: "statement",
                    source: "question-source",
                    latest_output: "question-output",
                    question: "question",
                    source_revision: 7,
                    solved: false,
                })
                .unwrap(),
            "What invariant holds?"
        );
        assert_eq!(
            session
                .ask(crate::codex::InterviewRequest {
                    mode: crate::codex::prompt::Mode::Hint(1),
                    statement: "statement",
                    source: "hint-source",
                    latest_output: "hint-output",
                    question: "",
                    source_revision: 7,
                    solved: false,
                })
                .unwrap(),
            "Level 1 invariant"
        );
        assert_eq!(
            session
                .ask(crate::codex::InterviewRequest {
                    mode: crate::codex::prompt::Mode::SubmissionReview,
                    statement: "statement",
                    source: "recorded-source-bytes",
                    latest_output: "recorded-output",
                    question: "",
                    source_revision: 7,
                    solved: true,
                })
                .unwrap(),
            "Submission reviewed"
        );
        drop(session);

        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let turns = records
            .iter()
            .filter_map(|record| record.get("json"))
            .filter(|message| message["method"] == "turn/start")
            .collect::<Vec<_>>();
        assert_eq!(turns.len(), 3);
        assert_eq!(
            turns[0]["params"]["threadId"],
            turns[2]["params"]["threadId"]
        );
        assert_ne!(
            turns[0]["params"]["threadId"],
            turns[1]["params"]["threadId"]
        );
        for turn in &turns {
            let input = turn["params"]["input"][0]["text"].as_str().unwrap();
            let payload: Value =
                serde_json::from_str(input.split_once("INPUT_JSON:").unwrap().1).unwrap();
            let mut keys = payload
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
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
        let hint_input = turns[1]["params"]["input"][0]["text"].as_str().unwrap();
        let hint_payload: Value =
            serde_json::from_str(hint_input.split_once("INPUT_JSON:").unwrap().1).unwrap();
        assert_eq!(hint_payload["transcript"], "");
        let review_input = turns[2]["params"]["input"][0]["text"].as_str().unwrap();
        let review_payload: Value =
            serde_json::from_str(review_input.split_once("INPUT_JSON:").unwrap().1).unwrap();
        assert_eq!(review_payload["source"], "recorded-source-bytes");
        assert_eq!(review_payload["userQuestion"], "");
    }

    #[test]
    fn deferred_stale_response_is_absent_from_the_next_prompt_transcript() {
        let environment = FakeEnvironment::new("normal");
        let mut session = crate::codex::CodexSession::connect_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        session
            .ask_deferred_with_cancellation(
                crate::codex::InterviewRequest {
                    mode: crate::codex::prompt::Mode::Interviewer,
                    statement: "statement",
                    source: "old-source",
                    latest_output: "output",
                    question: "stale-question",
                    source_revision: 1,
                    solved: false,
                },
                &CancellationToken::new(),
            )
            .unwrap();
        session
            .ask(crate::codex::InterviewRequest {
                mode: crate::codex::prompt::Mode::SubmissionReview,
                statement: "statement",
                source: "new-source",
                latest_output: "output",
                question: "",
                source_revision: 2,
                solved: true,
            })
            .unwrap();
        drop(session);

        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let turns = records
            .iter()
            .filter_map(|record| record.get("json"))
            .filter(|message| message["method"] == "turn/start")
            .collect::<Vec<_>>();
        assert_eq!(turns.len(), 2);
        let next_input = turns[1]["params"]["input"][0]["text"].as_str().unwrap();
        let next_payload: Value =
            serde_json::from_str(next_input.split_once("INPUT_JSON:").unwrap().1).unwrap();
        assert_eq!(next_payload["transcript"], "");
        assert!(!next_input.contains("stale-question"));
        assert!(!next_input.contains("What invariant holds?"));
    }

    #[test]
    fn malformed_turn_restarts_once_for_next_distinct_operation_without_replay() {
        let environment = FakeEnvironment::new("malformed-envelope-restart");
        let mut session = crate::codex::CodexSession::connect_executable(
            fake_executable(),
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        let request = |question| crate::codex::InterviewRequest {
            mode: crate::codex::prompt::Mode::Interviewer,
            statement: "statement",
            source: "source",
            latest_output: "output",
            question,
            source_revision: 1,
            solved: false,
        };
        assert!(
            session
                .ask(request("failed-content"))
                .unwrap_err()
                .contains("malformed structured output twice")
        );
        assert!(session.requires_restart());
        assert_eq!(
            session.ask(request("next-content")).unwrap(),
            "What invariant holds?"
        );
        drop(session);
        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let payload_questions = records
            .iter()
            .filter_map(|record| record.get("json"))
            .filter(|message| message["method"] == "turn/start")
            .filter_map(|message| {
                let input = message["params"]["input"][0]["text"].as_str()?;
                let (_, json) = input.split_once("INPUT_JSON:")?;
                let payload: Value = serde_json::from_str(json).ok()?;
                payload["userQuestion"].as_str().map(str::to_string)
            })
            .collect::<Vec<_>>();
        assert_eq!(payload_questions, ["failed-content", "next-content"]);
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
        assert!(session.requires_restart());
        assert_eq!(session.ask(request()).unwrap(), "What invariant holds?");
        assert!(!session.requires_restart());
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
        let reader = spawn_stdout_reader(
            std::io::Cursor::new(input),
            sender,
            Arc::clone(&failure),
            Arc::new(AtomicBool::new(false)),
        );
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
