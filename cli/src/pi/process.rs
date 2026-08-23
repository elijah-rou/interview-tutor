use crate::interviewer::{Backend, InterviewerError};
use crate::runner::CancellationToken;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
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

pub const MAX_JSON_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROTOCOL_AGGREGATE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ASSISTANT_BYTES: usize = 64 * 1024;
const MAX_VERSION_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const PROTOCOL_QUEUE_CAPACITY: usize = 64;
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const ABORT_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const READER_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const _: () = assert!(PROTOCOL_QUEUE_CAPACITY >= 1);
const _: () = assert!(MAX_PROTOCOL_AGGREGATE_BYTES >= MAX_JSON_RECORD_BYTES);

const PI_RPC_ARGUMENTS: [&str; 11] = [
    "--mode",
    "rpc",
    "--no-session",
    "--no-tools",
    "--no-extensions",
    "--no-skills",
    "--no-prompt-templates",
    "--no-themes",
    "--no-context-files",
    "--no-approve",
    "--offline",
];

pub struct PiProcess {
    child: Option<Child>,
    input: Option<ChildStdin>,
    messages: Option<Receiver<Vec<u8>>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    reader_shutdown: Arc<AtomicBool>,
    reader_failure: Arc<Mutex<Option<String>>>,
    _stderr_ring: Arc<Mutex<StderrRing>>,
    cwd: Option<PathBuf>,
    control_pid: Arc<AtomicI32>,
    next_id: u64,
    provider: Option<String>,
    completed_turns: u8,
}

impl PiProcess {
    pub fn start(
        executable: PathBuf,
        expected_identity: &ExecutableIdentity,
        control_pid: Arc<AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<Self, InterviewerError> {
        if cancellation.is_cancelled() {
            return Err(InterviewerError::cancelled(
                Backend::Pi,
                "Pi startup cancelled",
            ));
        }
        let current_identity = trusted_executable_identity(&executable)
            .map_err(|error| InterviewerError::configuration(Backend::Pi, error))?;
        if &current_identity != expected_identity {
            return Err(InterviewerError::configuration(
                Backend::Pi,
                "Pi executable changed after version probe",
            ));
        }
        let cwd =
            empty_temp_dir().map_err(|error| InterviewerError::transport(Backend::Pi, error))?;
        let mut command = Command::new(&executable);
        command
            .args(PI_RPC_ARGUMENTS)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        copy_allowed_environment(&mut command, true);
        command
            .env("PI_OFFLINE", "1")
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0")
            .env("NO_COLOR", "1");
        configure_process_group(&mut command);
        let current_identity = trusted_executable_identity(&executable);
        if current_identity.as_ref() != Ok(expected_identity) {
            let primary = match current_identity {
                Ok(_) => "Pi executable changed before RPC spawn".to_string(),
                Err(error) => error,
            };
            return Err(InterviewerError::configuration(
                Backend::Pi,
                combine_cleanup_error(primary, remove_temp_dir(&cwd)),
            ));
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Err(InterviewerError::transport(
                    Backend::Pi,
                    combine_cleanup_error(
                        format!("cannot start Pi RPC process: {error}"),
                        remove_temp_dir(&cwd),
                    ),
                ));
            }
        };
        let (input, output, error) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(input), Some(output), Some(error)) => (input, output, error),
                _ => {
                    let cleanup = cleanup_unmanaged_child(&mut child, &cwd);
                    return Err(InterviewerError::transport(
                        Backend::Pi,
                        combine_cleanup_error("Pi process pipes unavailable".into(), cleanup),
                    ));
                }
            };
        if let Err(primary) = set_nonblocking(output.as_raw_fd(), "Pi stdout")
            .and_then(|()| set_nonblocking(error.as_raw_fd(), "Pi stderr"))
        {
            drop(input);
            drop(output);
            drop(error);
            let cleanup = cleanup_unmanaged_child(&mut child, &cwd);
            return Err(InterviewerError::transport(
                Backend::Pi,
                combine_cleanup_error(primary, cleanup),
            ));
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
        Ok(Self {
            child: Some(child),
            input: Some(input),
            messages: Some(messages),
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            reader_shutdown,
            reader_failure,
            _stderr_ring: stderr_ring,
            cwd: Some(cwd),
            control_pid,
            next_id: 1,
            provider: None,
            completed_turns: 0,
        })
    }

    pub fn turn(
        &mut self,
        input: String,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        self.turn_with_timeout(input, cancellation, TURN_TIMEOUT)
    }

    fn turn_with_timeout(
        &mut self,
        input: String,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<String, InterviewerError> {
        if input.len() > MAX_JSON_RECORD_BYTES / 2 {
            return Err(InterviewerError::configuration(
                Backend::Pi,
                "Pi prompt exceeds bound",
            ));
        }
        if self.provider.is_none() {
            self.load_state(cancellation)?;
        }
        let prompt_id = self.send_command(json!({"type":"prompt","message":input}))?;
        let deadline = Instant::now() + timeout;
        let mut prompt_accepted = false;
        let mut observation = TurnObservation::default();
        let mut interruption: Option<Interruption> = None;
        loop {
            let now = Instant::now();
            if interruption.is_none() && (cancellation.is_cancelled() || now >= deadline) {
                let reason = if cancellation.is_cancelled() {
                    InterruptReason::Cancelled
                } else {
                    InterruptReason::TimedOut(timeout)
                };
                if !prompt_accepted {
                    return self.fail(reason.error());
                }
                let abort_id = self.send_command(json!({"type":"abort"}))?;
                interruption = Some(Interruption {
                    reason,
                    abort_id,
                    acknowledged: false,
                    deadline: now + ABORT_TIMEOUT,
                });
            }
            if let Some(interruption) = interruption.as_ref()
                && now >= interruption.deadline
            {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    format!(
                        "{}; Pi did not acknowledge abort and settle within 2s",
                        interruption.reason.message()
                    ),
                ));
            }
            let receive_deadline = interruption
                .as_ref()
                .map_or(deadline, |value| value.deadline);
            let wait =
                POLL_INTERVAL.min(receive_deadline.saturating_duration_since(Instant::now()));
            let Some(value) = self.receive_poll(wait)? else {
                continue;
            };
            let object = value.as_object().ok_or_else(|| {
                InterviewerError::protocol(Backend::Pi, "Pi record is not an object")
            });
            let object = match object {
                Ok(object) => object,
                Err(error) => return self.fail(error),
            };
            let record_type = match required_string(object, "type", "Pi record omitted type") {
                Ok(record_type) => record_type,
                Err(error) => return self.fail(error),
            };

            if record_type == "response" {
                if let Some(state) = interruption.as_mut() {
                    if response_id(object) == Some(state.abort_id.as_str()) {
                        if let Err(error) =
                            validate_success_response(object, &state.abort_id, "abort")
                        {
                            return self.fail(error);
                        }
                        if state.acknowledged {
                            return self.fail(InterviewerError::protocol(
                                Backend::Pi,
                                "Pi duplicated abort acknowledgement",
                            ));
                        }
                        state.acknowledged = true;
                        continue;
                    }
                    return self.fail(InterviewerError::protocol(
                        Backend::Pi,
                        "Pi emitted an unexpected response id while aborting",
                    ));
                }
                if !prompt_accepted && response_id(object) == Some(prompt_id.as_str()) {
                    if let Err(error) = validate_success_response(object, &prompt_id, "prompt") {
                        return self.fail(error);
                    }
                    prompt_accepted = true;
                    continue;
                }
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi emitted an unexpected or duplicate response id",
                ));
            }

            if !prompt_accepted {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi emitted an event before prompt acceptance",
                ));
            }
            if let Some(state) = interruption.as_ref() {
                if is_forbidden_event(record_type, object) {
                    return self.fail(InterviewerError::protocol(
                        Backend::Pi,
                        format!("Pi emitted forbidden {record_type} during abort"),
                    ));
                }
                if record_type == "agent_settled" {
                    if let Err(error) = require_exact_keys(object, &["type"], "agent_settled") {
                        return self.fail(error);
                    }
                    if !state.acknowledged {
                        return self.fail(InterviewerError::protocol(
                            Backend::Pi,
                            "Pi settled before acknowledging abort",
                        ));
                    }
                    return self.fail(state.reason.error());
                }
                if !is_known_turn_event(record_type) {
                    return self.fail(InterviewerError::protocol(
                        Backend::Pi,
                        format!("Pi emitted unexpected event {record_type} during abort"),
                    ));
                }
                continue;
            }

            if let Err(error) = observation.accept(record_type, object, self.provider.as_deref()) {
                return self.fail(error);
            }
            if observation.settled {
                let authoritative = observation
                    .assistant_text
                    .clone()
                    .expect("settled observation has assistant text");
                let text = match self.get_last_assistant_text(cancellation) {
                    Ok(text) => text,
                    Err(error) => return self.fail(error),
                };
                if text != authoritative {
                    return self.fail(InterviewerError::protocol(
                        Backend::Pi,
                        "Pi get_last_assistant_text was stale or inconsistent",
                    ));
                }
                self.completed_turns = self
                    .completed_turns
                    .checked_add(1)
                    .expect("Pi correction turn count overflow");
                assert!(self.completed_turns <= 2);
                return Ok(text);
            }
        }
    }

    fn load_state(&mut self, cancellation: &CancellationToken) -> Result<(), InterviewerError> {
        let id = self.send_command(json!({"type":"get_state"}))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cancellation.is_cancelled() {
                return self.fail(InterviewerError::cancelled(
                    Backend::Pi,
                    "Pi startup cancelled",
                ));
            }
            if Instant::now() >= deadline {
                return self.fail(InterviewerError::timeout(
                    Backend::Pi,
                    "Pi get_state timed out",
                ));
            }
            let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
            let Some(value) = self.receive_poll(wait)? else {
                continue;
            };
            let Some(object) = value.as_object() else {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi state response is not an object",
                ));
            };
            if let Err(error) = validate_success_response(object, &id, "get_state") {
                return self.fail(error);
            }
            if let Err(error) = require_exact_keys(
                object,
                &["id", "type", "command", "success", "data"],
                "get_state response",
            ) {
                return self.fail(error);
            }
            let Some(data) = object.get("data").and_then(Value::as_object) else {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi get_state omitted data",
                ));
            };
            if let Err(error) = validate_state_data(data) {
                return self.fail(error);
            }
            let Some(model) = data.get("model") else {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi get_state omitted model",
                ));
            };
            if model.is_null() {
                return self.fail(InterviewerError::authentication(
                    Backend::Pi,
                    "Pi authentication required; configure credentials for the selected provider",
                ));
            }
            let Some(provider) = model.get("provider").and_then(Value::as_str) else {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi selected model omitted provider",
                ));
            };
            if provider.is_empty() || provider.len() > 256 {
                return self.fail(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi selected model provider is invalid",
                ));
            }
            self.provider = Some(provider.to_string());
            return Ok(());
        }
    }

    fn get_last_assistant_text(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<String, InterviewerError> {
        let id = self.send_command(json!({"type":"get_last_assistant_text"}))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cancellation.is_cancelled() {
                return Err(InterviewerError::cancelled(
                    Backend::Pi,
                    "Pi turn cancelled after settlement",
                ));
            }
            if Instant::now() >= deadline {
                return Err(InterviewerError::timeout(
                    Backend::Pi,
                    "Pi get_last_assistant_text timed out",
                ));
            }
            let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
            let Some(value) = self.receive_poll(wait)? else {
                continue;
            };
            let Some(object) = value.as_object() else {
                return Err(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi final text response is not an object",
                ));
            };
            validate_success_response(object, &id, "get_last_assistant_text")?;
            require_exact_keys(
                object,
                &["id", "type", "command", "success", "data"],
                "get_last_assistant_text response",
            )?;
            let data = object
                .get("data")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    InterviewerError::protocol(
                        Backend::Pi,
                        "Pi get_last_assistant_text omitted data",
                    )
                })?;
            require_exact_keys(data, &["text"], "get_last_assistant_text data")?;
            let text = data.get("text").and_then(Value::as_str).ok_or_else(|| {
                InterviewerError::protocol(
                    Backend::Pi,
                    "Pi get_last_assistant_text returned null or non-text",
                )
            })?;
            if text.len() > MAX_ASSISTANT_BYTES {
                return Err(InterviewerError::protocol(
                    Backend::Pi,
                    "Pi assistant response exceeds 64 KiB",
                ));
            }
            return Ok(text.to_string());
        }
    }

    fn send_command(&mut self, mut command: Value) -> Result<String, InterviewerError> {
        let id = format!("interview-tutor-{}", self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("Pi request id overflow");
        let object = command.as_object_mut().expect("Pi command object");
        assert!(
            object
                .insert("id".into(), Value::String(id.clone()))
                .is_none()
        );
        let mut bytes = serde_json::to_vec(&command).map_err(|_| {
            InterviewerError::configuration(Backend::Pi, "cannot encode Pi RPC command")
        })?;
        bytes.push(b'\n');
        if bytes.len() > MAX_JSON_RECORD_BYTES {
            return Err(InterviewerError::configuration(
                Backend::Pi,
                "Pi RPC command exceeds 2 MiB",
            ));
        }
        let result = self
            .input
            .as_mut()
            .ok_or_else(|| InterviewerError::transport(Backend::Pi, "Pi stdin unavailable"))?
            .write_all(&bytes)
            .and_then(|_| self.input.as_mut().expect("input checked").flush());
        if let Err(error) = result {
            return self.fail(InterviewerError::transport(
                Backend::Pi,
                format!("cannot write to Pi: {error}"),
            ));
        }
        Ok(id)
    }

    fn receive_poll(&mut self, timeout: Duration) -> Result<Option<Value>, InterviewerError> {
        let failure = self
            .reader_failure
            .lock()
            .expect("Pi reader failure lock")
            .clone();
        if let Some(error) = failure {
            return self.fail(InterviewerError::protocol(Backend::Pi, error));
        }
        let result = self
            .messages
            .as_ref()
            .ok_or_else(|| InterviewerError::transport(Backend::Pi, "Pi protocol reader stopped"))?
            .recv_timeout(timeout);
        match result {
            Ok(line) => serde_json::from_slice(&line)
                .map(Some)
                .map_err(|_| InterviewerError::protocol(Backend::Pi, "Pi emitted malformed JSON")),
            Err(RecvTimeoutError::Timeout) => {
                let failure = self
                    .reader_failure
                    .lock()
                    .expect("Pi reader failure lock")
                    .clone();
                match failure {
                    Some(error) => self.fail(InterviewerError::protocol(Backend::Pi, error)),
                    None => Ok(None),
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                let failure = self
                    .reader_failure
                    .lock()
                    .expect("Pi reader failure lock")
                    .clone();
                match failure {
                    Some(failure) => self.fail(InterviewerError::protocol(Backend::Pi, failure)),
                    None => self.fail(InterviewerError::transport(
                        Backend::Pi,
                        "Pi protocol reader stopped",
                    )),
                }
            }
        }
    }

    fn fail<T>(&mut self, primary: InterviewerError) -> Result<T, InterviewerError> {
        match self.shutdown() {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(InterviewerError::new(
                Backend::Pi,
                primary.kind(),
                format!("{primary}; cleanup failed: {cleanup}"),
            )),
        }
    }

    pub fn shutdown(&mut self) -> Result<(), InterviewerError> {
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
            errors.push("Pi stdout reader panicked".into());
        }
        if let Some(reader) = self.stderr_reader.take()
            && reader.join().is_err()
        {
            errors.push("Pi stderr reader panicked".into());
        }
        if let Some(cwd) = self.cwd.as_ref() {
            match remove_temp_dir(cwd) {
                Ok(()) => self.cwd = None,
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(InterviewerError::transport(Backend::Pi, errors.join("; ")))
        }
    }

    #[cfg(test)]
    fn cwd_path(&self) -> PathBuf {
        self.cwd.as_ref().expect("live Pi process cwd").clone()
    }

    #[cfg(test)]
    fn stderr_len(&self) -> usize {
        self._stderr_ring
            .lock()
            .expect("Pi stderr ring lock")
            .bytes
            .len()
    }
}

impl Drop for PiProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Default)]
struct TurnObservation {
    agent_started: bool,
    turn_started: bool,
    message_started: bool,
    message_ended: bool,
    turn_ended: bool,
    agent_ended: bool,
    settled: bool,
    assistant_text: Option<String>,
}

impl TurnObservation {
    fn accept(
        &mut self,
        record_type: &str,
        object: &Map<String, Value>,
        provider: Option<&str>,
    ) -> Result<(), InterviewerError> {
        match record_type {
            "agent_start" => {
                require_exact_keys(object, &["type"], "agent_start")?;
                if self.agent_started {
                    return Err(protocol("Pi duplicated agent_start"));
                }
                self.agent_started = true;
            }
            "turn_start" => {
                require_exact_keys(object, &["type"], "turn_start")?;
                if !self.agent_started || self.turn_started {
                    return Err(protocol("Pi turn_start ordering is invalid"));
                }
                self.turn_started = true;
            }
            "message_start" => {
                require_exact_keys(object, &["type", "message"], "message_start")?;
                if !self.turn_started || self.message_started {
                    return Err(protocol("Pi message_start ordering is invalid"));
                }
                validate_message_has_no_tools(
                    object.get("message").expect("required key"),
                    "message_start",
                )?;
                self.message_started = true;
            }
            "message_update" => {
                require_exact_keys(
                    object,
                    &["type", "usage", "assistantMessageEvent"],
                    "message_update",
                )?;
                if !self.message_started || self.message_ended {
                    return Err(protocol("Pi message_update ordering is invalid"));
                }
                let event = object
                    .get("assistantMessageEvent")
                    .and_then(Value::as_object)
                    .ok_or_else(|| protocol("Pi message_update omitted assistant event"))?;
                let event_type =
                    required_string(event, "type", "Pi assistant update omitted type")?;
                let expected_keys = match event_type {
                    "text_start" | "thinking_start" => &["type", "contentIndex"][..],
                    "text_delta" | "thinking_delta" => &["type", "contentIndex", "delta"][..],
                    "text_end" => &["type", "contentIndex", "content"][..],
                    "thinking_end" => &["type", "contentIndex", "content"][..],
                    _ => {
                        return Err(protocol(&format!(
                            "Pi requested forbidden assistant event {event_type}"
                        )));
                    }
                };
                require_exact_keys(event, expected_keys, "assistant update")?;
            }
            "message_end" => {
                require_exact_keys(object, &["type", "message"], "message_end")?;
                if !self.message_started || self.message_ended {
                    return Err(protocol("Pi message_end ordering is invalid"));
                }
                let text = validate_completed_assistant(
                    object.get("message").expect("required key"),
                    provider,
                )?;
                self.assistant_text = Some(text);
                self.message_ended = true;
            }
            "turn_end" => {
                require_exact_keys(object, &["type", "message", "toolResults"], "turn_end")?;
                if !self.message_ended || self.turn_ended {
                    return Err(protocol("Pi turn_end ordering is invalid"));
                }
                let tools = object
                    .get("toolResults")
                    .and_then(Value::as_array)
                    .ok_or_else(|| protocol("Pi turn_end toolResults is malformed"))?;
                if !tools.is_empty() {
                    return Err(protocol("Pi emitted forbidden tool results"));
                }
                let text = validate_completed_assistant(
                    object.get("message").expect("required key"),
                    provider,
                )?;
                if self.assistant_text.as_deref() != Some(&text) {
                    return Err(protocol("Pi turn_end assistant text changed"));
                }
                self.turn_ended = true;
            }
            "agent_end" => {
                require_exact_keys(object, &["type", "messages", "willRetry"], "agent_end")?;
                if !self.turn_ended || self.agent_ended {
                    return Err(protocol("Pi agent_end ordering is invalid"));
                }
                if object.get("willRetry").and_then(Value::as_bool) != Some(false) {
                    return Err(protocol("Pi completion was retrying or non-terminal"));
                }
                let messages = object
                    .get("messages")
                    .and_then(Value::as_array)
                    .ok_or_else(|| protocol("Pi agent_end messages is malformed"))?;
                if messages.len() != 1 {
                    return Err(protocol(
                        "Pi agent_end contained an unexpected message count",
                    ));
                }
                let text = validate_completed_assistant(&messages[0], provider)?;
                if self.assistant_text.as_deref() != Some(&text) {
                    return Err(protocol("Pi agent_end assistant text changed"));
                }
                self.agent_ended = true;
            }
            "agent_settled" => {
                require_exact_keys(object, &["type"], "agent_settled")?;
                if !self.agent_ended || self.settled {
                    return Err(protocol("Pi agent_settled ordering is invalid"));
                }
                self.settled = true;
            }
            name if is_forbidden_event(name, object) => {
                return Err(protocol(&format!("Pi emitted forbidden {name}")));
            }
            name => return Err(protocol(&format!("Pi emitted unexpected event {name}"))),
        }
        Ok(())
    }
}

fn validate_message_has_no_tools(value: &Value, context: &str) -> Result<(), InterviewerError> {
    let object = value
        .as_object()
        .ok_or_else(|| protocol(&format!("Pi {context} message is malformed")))?;
    if object.get("role").and_then(Value::as_str) != Some("assistant") {
        return Err(protocol(&format!(
            "Pi {context} contained a non-assistant message"
        )));
    }
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol(&format!("Pi {context} content is malformed")))?;
    for item in content {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "text" | "thinking") {
            return Err(protocol(&format!(
                "Pi {context} requested forbidden content {kind}"
            )));
        }
    }
    Ok(())
}

fn validate_completed_assistant(
    value: &Value,
    provider: Option<&str>,
) -> Result<String, InterviewerError> {
    validate_message_has_no_tools(value, "completed")?;
    let object = value.as_object().expect("message validated");
    if object.get("stopReason").and_then(Value::as_str) != Some("stop") {
        return Err(protocol("Pi assistant completion stop reason was not stop"));
    }
    let expected_provider = provider.ok_or_else(|| protocol("Pi provider was not established"))?;
    if object.get("provider").and_then(Value::as_str) != Some(expected_provider) {
        return Err(protocol("Pi assistant provider changed during the turn"));
    }
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol("Pi assistant content is malformed"))?;
    let mut text = String::new();
    for item in content {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                let part = item
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| protocol("Pi assistant text block is malformed"))?;
                if text.len().saturating_add(part.len()) > MAX_ASSISTANT_BYTES {
                    return Err(protocol("Pi assistant response exceeds 64 KiB"));
                }
                text.push_str(part);
            }
            Some("thinking") => {}
            Some(kind) => {
                return Err(protocol(&format!("Pi requested forbidden content {kind}")));
            }
            None => return Err(protocol("Pi assistant content omitted type")),
        }
    }
    if text.is_empty() {
        return Err(protocol("Pi assistant completion contained no text"));
    }
    Ok(text)
}

fn validate_state_data(data: &Map<String, Value>) -> Result<(), InterviewerError> {
    const REQUIRED: [&str; 11] = [
        "model",
        "thinkingLevel",
        "isStreaming",
        "isCompacting",
        "steeringMode",
        "followUpMode",
        "sessionFile",
        "sessionId",
        "autoCompactionEnabled",
        "messageCount",
        "pendingMessageCount",
    ];
    let expected_length = REQUIRED.len() + usize::from(data.contains_key("sessionName"));
    if data.len() != expected_length || REQUIRED.iter().any(|key| !data.contains_key(*key)) {
        return Err(protocol("Pi get_state data envelope is malformed"));
    }
    if data.get("thinkingLevel").and_then(Value::as_str).is_none()
        || data.get("isStreaming").and_then(Value::as_bool).is_none()
        || data.get("isCompacting").and_then(Value::as_bool).is_none()
        || data.get("steeringMode").and_then(Value::as_str).is_none()
        || data.get("followUpMode").and_then(Value::as_str).is_none()
        || !data
            .get("sessionFile")
            .is_some_and(|value| value.is_null() || value.is_string())
        || data.get("sessionId").and_then(Value::as_str).is_none()
        || data
            .get("autoCompactionEnabled")
            .and_then(Value::as_bool)
            .is_none()
        || data.get("messageCount").and_then(Value::as_u64).is_none()
        || data
            .get("pendingMessageCount")
            .and_then(Value::as_u64)
            .is_none()
        || data
            .get("sessionName")
            .is_some_and(|value| !value.is_string())
    {
        return Err(protocol("Pi get_state data types are malformed"));
    }
    Ok(())
}

fn validate_success_response(
    object: &Map<String, Value>,
    expected_id: &str,
    expected_command: &str,
) -> Result<(), InterviewerError> {
    if object.get("type").and_then(Value::as_str) != Some("response") {
        return Err(protocol("Pi response type was not response"));
    }
    if response_id(object) != Some(expected_id) {
        return Err(protocol("Pi response id did not match the pending command"));
    }
    if object.get("command").and_then(Value::as_str) != Some(expected_command) {
        return Err(protocol(
            "Pi response command did not match the pending command",
        ));
    }
    if object.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(protocol(&format!(
            "Pi rejected the {expected_command} command"
        )));
    }
    let allowed = if object.contains_key("data") {
        &["id", "type", "command", "success", "data"][..]
    } else {
        &["id", "type", "command", "success"][..]
    };
    require_exact_keys(object, allowed, "response")
}

fn response_id(object: &Map<String, Value>) -> Option<&str> {
    object.get("id").and_then(Value::as_str)
}

fn require_exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
    context: &str,
) -> Result<(), InterviewerError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(protocol(&format!("Pi {context} envelope is malformed")));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a str, InterviewerError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| protocol(message))
}

fn is_forbidden_event(record_type: &str, object: &Map<String, Value>) -> bool {
    record_type.starts_with("tool_")
        || record_type == "bash_execution_update"
        || record_type == "extension_ui_request"
        || record_type == "extension_error"
        || record_type == "queue_update"
        || record_type.starts_with("compaction_")
        || record_type.starts_with("auto_retry_")
        || record_type.starts_with("summarization_retry_")
        || object
            .get("assistantMessageEvent")
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("toolcall_"))
}

fn is_known_turn_event(record_type: &str) -> bool {
    matches!(
        record_type,
        "agent_start"
            | "turn_start"
            | "message_start"
            | "message_update"
            | "message_end"
            | "turn_end"
            | "agent_end"
    )
}

fn protocol(message: &str) -> InterviewerError {
    InterviewerError::protocol(Backend::Pi, message)
}

#[derive(Clone, Copy)]
enum InterruptReason {
    Cancelled,
    TimedOut(Duration),
}

impl InterruptReason {
    fn message(self) -> String {
        match self {
            Self::Cancelled => "Pi turn cancelled".into(),
            Self::TimedOut(timeout) => {
                format!("Pi turn timed out after {}ms", timeout.as_millis())
            }
        }
    }

    fn error(self) -> InterviewerError {
        match self {
            Self::Cancelled => InterviewerError::cancelled(Backend::Pi, self.message()),
            Self::TimedOut(_) => InterviewerError::timeout(Backend::Pi, self.message()),
        }
    }
}

struct Interruption {
    reason: InterruptReason,
    abort_id: String,
    acknowledged: bool,
    deadline: Instant,
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
        let mut aggregate = 0_usize;
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
                        set_reader_failure(
                            &failure,
                            if line.is_empty() {
                                "Pi RPC process closed stdout"
                            } else {
                                "Pi closed stdout mid-record"
                            },
                        );
                    }
                    break;
                }
                Ok(count) => {
                    for byte in &buffer[..count] {
                        aggregate = match aggregate.checked_add(1) {
                            Some(value) => value,
                            None => {
                                set_reader_failure(&failure, "Pi protocol aggregate overflowed");
                                break 'read;
                            }
                        };
                        if aggregate > MAX_PROTOCOL_AGGREGATE_BYTES {
                            set_reader_failure(&failure, "Pi protocol aggregate exceeds 2 MiB");
                            break 'read;
                        }
                        if *byte == b'\n' {
                            if line.last() == Some(&b'\r') {
                                set_reader_failure(
                                    &failure,
                                    "Pi protocol used CRLF instead of LF framing",
                                );
                                break 'read;
                            }
                            let record = std::mem::replace(&mut line, Vec::with_capacity(4096));
                            match sender.try_send(record) {
                                Ok(()) => {}
                                Err(TrySendError::Full(_)) => {
                                    set_reader_failure(&failure, "Pi protocol queue overflow");
                                    break 'read;
                                }
                                Err(TrySendError::Disconnected(_)) => break 'read,
                            }
                        } else {
                            if line.len() == MAX_JSON_RECORD_BYTES {
                                set_reader_failure(&failure, "Pi protocol record exceeds 2 MiB");
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
                    set_reader_failure(&failure, &format!("cannot read Pi output: {error}"));
                    break;
                }
            }
        }
    })
}

fn set_reader_failure(failure: &Mutex<Option<String>>, message: &str) {
    let mut failure = failure.lock().expect("Pi reader failure lock");
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
                    .expect("Pi stderr ring lock")
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

pub fn configured_executable() -> Result<PathBuf, String> {
    let configured = std::env::var_os("INTERVIEW_TUTOR_PI_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pi"));
    resolve_executable(&configured)
}

fn resolve_executable(path: &Path) -> Result<PathBuf, String> {
    let resolved = if path.components().count() > 1 {
        fs::canonicalize(path).map_err(|error| format!("cannot resolve Pi executable: {error}"))?
    } else {
        let paths = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
        let selected = std::env::split_paths(&paths)
            .map(|base| base.join(path))
            .find(|candidate| candidate.is_file())
            .ok_or("Pi CLI not found; local solve remains available")?;
        fs::canonicalize(selected)
            .map_err(|error| format!("cannot resolve Pi executable: {error}"))?
    };
    trusted_executable_identity(&resolved)?;
    Ok(resolved)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutableIdentity {
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

pub fn trusted_executable_identity(path: &Path) -> Result<ExecutableIdentity, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot inspect Pi executable: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Pi executable must be a regular file".into());
    }
    let current_user = unsafe { libc::geteuid() };
    if metadata.uid() != current_user && metadata.uid() != 0 {
        return Err("Pi executable must be owned by the current user or root".into());
    }
    if metadata.mode() & 0o022 != 0 {
        return Err("Pi executable must not be group- or world-writable".into());
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

pub fn validate_version(executable: &Path, cancellation: &CancellationToken) -> Result<(), String> {
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
            "cannot query Pi version: process exited with {}",
            capture.status
        ));
    }
    if capture.truncated {
        return Err("cannot query Pi version: output exceeded 64 KiB".into());
    }
    let version = std::str::from_utf8(&capture.stdout)
        .map_err(|_| "cannot query Pi version: stdout was not UTF-8")?
        .trim();
    if version != "0.84.2" {
        return Err(format!(
            "unsupported Pi CLI {version}; install verified version 0.84.2"
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
    total: usize,
    truncated: bool,
}

impl CaptureBuffers {
    fn push(&mut self, stdout: bool, bytes: &[u8]) {
        let remaining = MAX_VERSION_OUTPUT_BYTES.saturating_sub(self.total);
        let retained = bytes.len().min(remaining);
        if stdout {
            self.stdout.extend_from_slice(&bytes[..retained]);
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
    copy_allowed_environment(&mut command, false);
    command
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .env("NO_COLOR", "1");
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot query Pi version: {error}"))?;
    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(stdout), Some(stderr)) => (stdout, stderr),
        _ => {
            let cleanup = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
            return Err(combine_cleanup_error(
                "Pi version probe pipes unavailable".into(),
                cleanup,
            ));
        }
    };
    if let Err(primary) = set_nonblocking(stdout.as_raw_fd(), "Pi version stdout")
        .and_then(|()| set_nonblocking(stderr.as_raw_fd(), "Pi version stderr"))
    {
        drop(stdout);
        drop(stderr);
        let cleanup = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT);
        return Err(combine_cleanup_error(primary, cleanup));
    }
    let buffers = Arc::new(Mutex::new(CaptureBuffers::default()));
    let shutdown = Arc::new(AtomicBool::new(false));
    let stdout_reader =
        spawn_capture_reader(stdout, Arc::clone(&buffers), true, Arc::clone(&shutdown));
    let stderr_reader =
        spawn_capture_reader(stderr, Arc::clone(&buffers), false, Arc::clone(&shutdown));
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        if cancellation.is_cancelled() {
            break Err("Pi version probe cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(format!("cannot wait for Pi version: {error}")),
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "Pi version probe timed out after {}s",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    };
    shutdown.store(true, Ordering::SeqCst);
    let mut cleanup_errors = Vec::new();
    if let Err(error) = terminate_child_group(&mut child, SHUTDOWN_TIMEOUT) {
        cleanup_errors.push(error);
    }
    if stdout_reader.join().is_err() {
        cleanup_errors.push("Pi version stdout reader panicked".into());
    }
    if stderr_reader.join().is_err() {
        cleanup_errors.push("Pi version stderr reader panicked".into());
    }
    let buffers = Arc::try_unwrap(buffers)
        .map_err(|_| "Pi version capture still shared".to_string())?
        .into_inner()
        .map_err(|_| "Pi version capture lock poisoned".to_string())?;
    if !cleanup_errors.is_empty() {
        return Err(combine_cleanup_error(
            outcome
                .err()
                .unwrap_or_else(|| "cannot clean up Pi version probe".into()),
            Err(cleanup_errors.join("; ")),
        ));
    }
    Ok(VersionCapture {
        status: outcome?,
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
                    .expect("Pi version capture lock")
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

fn base_environment_names() -> &'static [&'static str] {
    &[
        "HOME",
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
        "PI_CODING_AGENT_DIR",
        "PI_PACKAGE_DIR",
    ]
}

fn provider_auth_environment_names() -> &'static [&'static str] {
    &[
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_OAUTH_TOKEN",
        "ANT_LING_API_KEY",
        "OPENAI_API_KEY",
        "AZURE_OPENAI_API_KEY",
        "AZURE_OPENAI_BASE_URL",
        "AZURE_OPENAI_RESOURCE_NAME",
        "AZURE_OPENAI_API_VERSION",
        "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
        "DEEPSEEK_API_KEY",
        "NVIDIA_API_KEY",
        "GEMINI_API_KEY",
        "GROQ_API_KEY",
        "CEREBRAS_API_KEY",
        "XAI_API_KEY",
        "FIREWORKS_API_KEY",
        "TOGETHER_API_KEY",
        "BASETEN_API_KEY",
        "OPENROUTER_API_KEY",
        "AI_GATEWAY_API_KEY",
        "ZAI_API_KEY",
        "ZAI_CODING_CN_API_KEY",
        "MISTRAL_API_KEY",
        "MINIMAX_API_KEY",
        "MOONSHOT_API_KEY",
        "OPENCODE_API_KEY",
        "KIMI_API_KEY",
        "CLOUDFLARE_API_KEY",
        "CLOUDFLARE_ACCOUNT_ID",
        "CLOUDFLARE_GATEWAY_ID",
        "QWEN_TOKEN_PLAN_API_KEY",
        "QWEN_TOKEN_PLAN_CN_API_KEY",
        "XIAOMI_API_KEY",
        "XIAOMI_TOKEN_PLAN_CN_API_KEY",
        "XIAOMI_TOKEN_PLAN_AMS_API_KEY",
        "XIAOMI_TOKEN_PLAN_SGP_API_KEY",
        "AWS_PROFILE",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_BEARER_TOKEN_BEDROCK",
        "AWS_REGION",
        "GOOGLE_APPLICATION_CREDENTIALS",
    ]
}

fn copy_allowed_environment(command: &mut Command, include_auth: bool) {
    for name in base_environment_names().iter().copied().chain(
        include_auth
            .then_some(provider_auth_environment_names())
            .into_iter()
            .flatten()
            .copied(),
    ) {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
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
    let pid = i32::try_from(child.id()).map_err(|_| "Pi child PID exceeds platform bound")?;
    assert!(pid > 0);
    let mut errors = Vec::new();
    let mut reaped = match child.try_wait() {
        Ok(status) => status.is_some(),
        Err(error) => {
            errors.push(format!("cannot poll Pi child: {error}"));
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
                    errors.push(format!("cannot poll Pi child: {error}"));
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
                    errors.push(format!("cannot poll Pi child: {error}"));
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
            "Pi child {pid} was not reaped before the cleanup deadline"
        ));
    }
    if process_group_exists(pid) {
        errors.push(format!(
            "Pi process group {pid} survived the cleanup deadline"
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
            "cannot signal Pi process target {target} with signal {signal}: {error}"
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
            "cannot remove Pi temporary directory {}: {error}",
            path.display()
        )),
    }
}

fn empty_temp_dir() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock invalid")?
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("interview-tutor-pi-{}-{nonce}", std::process::id()));
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&path)
        .map_err(|error| format!("cannot create Pi temporary directory: {error}"))?;
    if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o700)) {
        return Err(combine_cleanup_error(
            format!("cannot secure Pi temporary directory: {error}"),
            remove_temp_dir(&path),
        ));
    }
    let metadata = fs::metadata(&path).map_err(|error| {
        combine_cleanup_error(
            format!("cannot verify Pi temporary directory: {error}"),
            remove_temp_dir(&path),
        )
    })?;
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return Err(combine_cleanup_error(
            "Pi temporary directory is not a mode-0700 directory".into(),
            remove_temp_dir(&path),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    static ENVIRONMENT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct FakeEnvironment {
        directory: PathBuf,
        old_home: Option<std::ffi::OsString>,
        old_executable: Option<std::ffi::OsString>,
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
            let old_home = std::env::var_os("PI_CODING_AGENT_DIR");
            let old_executable = std::env::var_os("INTERVIEW_TUTOR_PI_EXECUTABLE");
            unsafe {
                std::env::set_var("PI_CODING_AGENT_DIR", &directory);
                std::env::set_var("INTERVIEW_TUTOR_PI_EXECUTABLE", fake_executable());
            }
            Self {
                directory,
                old_home,
                old_executable,
                _lock: lock,
            }
        }
    }

    impl Drop for FakeEnvironment {
        fn drop(&mut self) {
            unsafe {
                match self.old_home.as_ref() {
                    Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
                    None => std::env::remove_var("PI_CODING_AGENT_DIR"),
                }
                match self.old_executable.as_ref() {
                    Some(value) => std::env::set_var("INTERVIEW_TUTOR_PI_EXECUTABLE", value),
                    None => std::env::remove_var("INTERVIEW_TUTOR_PI_EXECUTABLE"),
                }
            }
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn fake_executable() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_pi_rpc.py")
    }

    fn fake_process(mode: &str) -> (FakeEnvironment, PiProcess) {
        let environment = FakeEnvironment::new(mode);
        let executable = configured_executable().unwrap();
        let identity = trusted_executable_identity(&executable).unwrap();
        validate_version(&executable, &CancellationToken::new()).unwrap();
        let process = PiProcess::start(
            executable,
            &identity,
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        (environment, process)
    }

    #[test]
    fn exact_version_and_trusted_executable_are_required() {
        let directory = empty_temp_dir().unwrap();
        let executable = directory.join("pi");
        fs::write(&executable, "#!/bin/sh\necho 0.84.1\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            validate_version(&executable, &CancellationToken::new())
                .unwrap_err()
                .contains("0.84.2")
        );
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o720)).unwrap();
        assert!(
            trusted_executable_identity(&executable)
                .unwrap_err()
                .contains("group- or world-writable")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rpc_argv_environment_cwd_and_fresh_process_per_turn_are_exact() {
        let environment = FakeEnvironment::new("normal");
        let mut session = crate::interviewer::InterviewerSession::connect(
            Backend::Pi,
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        for question in ["first", "second"] {
            let response = session
                .ask(crate::interviewer::InterviewRequest {
                    mode: crate::interviewer::Mode::Interviewer,
                    statement: "statement",
                    source: "source",
                    latest_output: "output",
                    question,
                    source_revision: 1,
                    solved: false,
                })
                .unwrap();
            assert_eq!(response, "What invariant holds?");
        }
        drop(session);
        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let processes = records
            .iter()
            .filter(|record| record["kind"] == "process")
            .collect::<Vec<_>>();
        assert_eq!(processes.len(), 2);
        for process in processes {
            assert_eq!(process["argv"], json!(PI_RPC_ARGUMENTS));
            assert!(!Path::new(process["cwd"].as_str().unwrap()).exists());
            let names = process["environment_names"].as_array().unwrap();
            assert!(names.iter().any(|name| name == "PI_CODING_AGENT_DIR"));
            assert!(!names.iter().any(|name| name == "INTERVIEW_TUTOR_SENTINEL"));
            assert_eq!(process["forced_offline"], true);
            assert_eq!(process["forced_telemetry_off"], true);
        }
    }

    #[test]
    fn auth_protocol_tools_stale_null_non_stop_eof_and_flood_fail_closed() {
        for (mode, kind, expected) in [
            (
                "auth",
                crate::interviewer::ErrorKind::Authentication,
                "authentication",
            ),
            ("tool", crate::interviewer::ErrorKind::Protocol, "forbidden"),
            ("stale", crate::interviewer::ErrorKind::Protocol, "stale"),
            ("null", crate::interviewer::ErrorKind::Protocol, "null"),
            (
                "non-stop",
                crate::interviewer::ErrorKind::Protocol,
                "stop reason",
            ),
            ("eof", crate::interviewer::ErrorKind::Protocol, "stdout"),
            ("crlf", crate::interviewer::ErrorKind::Protocol, "CRLF"),
            (
                "wrong-id",
                crate::interviewer::ErrorKind::Protocol,
                "response id",
            ),
            (
                "malformed",
                crate::interviewer::ErrorKind::Protocol,
                "malformed JSON",
            ),
            (
                "flood",
                crate::interviewer::ErrorKind::Protocol,
                "unexpected",
            ),
        ] {
            let (_environment, mut process) = fake_process(mode);
            let error = process
                .turn("prompt".into(), &CancellationToken::new())
                .unwrap_err();
            assert_eq!(error.kind(), kind, "{mode}: {error}");
            assert!(error.to_string().contains(expected), "{mode}: {error}");
        }
    }

    #[test]
    fn malformed_envelope_gets_one_correction_in_the_same_process() {
        let environment = FakeEnvironment::new("correction");
        let mut session = crate::interviewer::InterviewerSession::connect(
            Backend::Pi,
            Arc::new(AtomicI32::new(0)),
            &CancellationToken::new(),
        )
        .unwrap();
        let response = session
            .ask(crate::interviewer::InterviewRequest {
                mode: crate::interviewer::Mode::Interviewer,
                statement: "statement",
                source: "source",
                latest_output: "output",
                question: "question",
                source_revision: 1,
                solved: false,
            })
            .unwrap();
        assert_eq!(response, "Corrected question");
        drop(session);
        let records = fs::read_to_string(environment.directory.join("fake-capture.jsonl")).unwrap();
        assert_eq!(records.matches("\"kind\": \"process\"").count(), 1);
        assert_eq!(records.matches("\"command_type\": \"prompt\"").count(), 2);
    }

    #[test]
    fn cancellation_requires_abort_ack_before_settlement_and_cleans_process() {
        let (environment, mut process) = fake_process("hold");
        let cwd = process.cwd_path();
        let cancellation = CancellationToken::new();
        let other = cancellation.clone();
        let capture_path = environment.directory.join("fake-capture.jsonl");
        let cancel = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if fs::read_to_string(&capture_path)
                    .is_ok_and(|capture| capture.contains("\"command_type\": \"prompt\""))
                {
                    other.cancel();
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("Pi prompt was not accepted before cancellation deadline");
        });
        let error = process.turn("prompt".into(), &cancellation).unwrap_err();
        cancel.join().unwrap();
        assert_eq!(error.kind(), crate::interviewer::ErrorKind::Cancelled);
        assert!(!cwd.exists());
        let capture = fs::read_to_string(environment.directory.join("fake-capture.jsonl")).unwrap();
        assert!(capture.contains("\"command_type\": \"abort\""));
    }

    #[test]
    fn stderr_and_protocol_readers_are_bounded() {
        let (_environment, process) = fake_process("stderr");
        let deadline = Instant::now() + Duration::from_secs(2);
        while process.stderr_len() < MAX_STDERR_BYTES && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(process.stderr_len(), MAX_STDERR_BYTES);

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
            failure.lock().unwrap().as_deref(),
            Some("Pi protocol queue overflow")
        );
        drop(receiver);
    }
}
