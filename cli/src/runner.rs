use crate::database;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
use nix::unistd::Pid;
use rusqlite::Connection;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vte::{Parser, Perform};

const MIN_WALL_TIMEOUT: Duration = Duration::from_millis(10);
const MAX_WALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const MAX_TERM_GRACE: Duration = Duration::from_secs(10);
const MIN_DISPLAY_OUTPUT_BYTES: usize = 64;
const MAX_DISPLAY_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MIN_READ_CHUNK_BYTES: usize = 256;
const MAX_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_EVENT_QUEUE_CAPACITY: usize = 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const FINAL_DRAIN_LIMIT: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct ExecutionLimits {
    pub wall_timeout: Duration,
    pub term_grace: Duration,
    pub display_output_bytes: usize,
    pub read_chunk_bytes: usize,
    pub event_queue_capacity: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            wall_timeout: Duration::from_secs(30),
            term_grace: Duration::from_millis(250),
            display_output_bytes: 256 * 1024,
            read_chunk_bytes: 8 * 1024,
            event_queue_capacity: 64,
        }
    }
}

impl ExecutionLimits {
    fn validate(&self) -> Result<(), String> {
        if !(MIN_WALL_TIMEOUT..=MAX_WALL_TIMEOUT).contains(&self.wall_timeout) {
            return Err("wall timeout must be between 10ms and 1h".to_string());
        }
        if self.term_grace > MAX_TERM_GRACE {
            return Err("TERM grace must not exceed 10s".to_string());
        }
        if !(MIN_DISPLAY_OUTPUT_BYTES..=MAX_DISPLAY_OUTPUT_BYTES)
            .contains(&self.display_output_bytes)
        {
            return Err("display output limit must be between 64 bytes and 16 MiB".to_string());
        }
        if !(MIN_READ_CHUNK_BYTES..=MAX_READ_CHUNK_BYTES).contains(&self.read_chunk_bytes) {
            return Err("read chunk size must be between 256 bytes and 64 KiB".to_string());
        }
        if !(1..=MAX_EVENT_QUEUE_CAPACITY).contains(&self.event_queue_capacity) {
            return Err("event queue capacity must be between 1 and 1024".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    signal_state: Option<crate::signals::SignalState>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_signal_state(signal_state: crate::signals::SignalState) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            signal_state: Some(signal_state),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .signal_state
                .as_ref()
                .is_some_and(crate::signals::SignalState::is_cancelled)
    }

    pub fn signal_exit_code(&self) -> Option<i32> {
        self.signal_state
            .as_ref()
            .and_then(crate::signals::SignalState::exit_code)
    }

    pub fn signal_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionEvent {
    Stdout(String),
    Stderr(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Termination {
    Exited(i32),
    Cancelled,
    TimedOut,
    Signalled(i32),
}

#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub root: PathBuf,
    pub language: String,
    pub problem_slug: String,
    pub set_slug: Option<String>,
    pub runner_path: PathBuf,
    pub solution_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub termination: Termination,
    pub display_output: String,
    pub omitted_bytes: usize,
    pub dropped_events: usize,
    pub duration_ms: i64,
    discovery_stdout: Option<String>,
    discovery_stderr: Option<String>,
    discovery_stdout_truncated: bool,
}

impl ExecutionResult {
    #[cfg(test)]
    pub(crate) fn test_result(termination: Termination, output: &str) -> Self {
        Self {
            termination,
            display_output: output.to_string(),
            omitted_bytes: 0,
            dropped_events: 0,
            duration_ms: 1,
            discovery_stdout: None,
            discovery_stderr: None,
            discovery_stdout_truncated: false,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.termination {
            Termination::Exited(code) => Some(code),
            Termination::Cancelled | Termination::TimedOut | Termination::Signalled(_) => None,
        }
    }

    pub fn status_code(&self) -> i32 {
        match self.termination {
            Termination::Exited(code) => code,
            Termination::Cancelled => 130,
            Termination::TimedOut | Termination::Signalled(_) => 2,
        }
    }

    pub fn outcome(&self) -> database::AttemptOutcome {
        match self.termination {
            Termination::Exited(0) => database::AttemptOutcome::Pass,
            Termination::Exited(2) => database::AttemptOutcome::Error,
            Termination::Exited(_) => database::AttemptOutcome::Fail,
            Termination::Cancelled => database::AttemptOutcome::Cancelled,
            Termination::TimedOut | Termination::Signalled(_) => database::AttemptOutcome::Error,
        }
    }
}

pub fn plan_execution(
    connection: &Connection,
    root: &Path,
    language: &str,
    problem_reference: &str,
    set_slug: Option<&str>,
) -> Result<ExecutionPlan, String> {
    let problem = database::resolve_problem(connection, problem_reference, set_slug)?.problem;
    let implementation = database::get_implementation(connection, problem.id, language)?;
    let runner_path = root.join(implementation.language.runner_path);
    let solution_path = root.join(implementation.solution_path);
    validate_runner(&runner_path)?;
    if !solution_path.is_file() {
        return Err(format!(
            "solution file is not installed: {}",
            solution_path.display()
        ));
    }
    Ok(ExecutionPlan {
        root: root.to_path_buf(),
        language: language.to_string(),
        problem_slug: problem.slug,
        set_slug: set_slug.map(str::to_string),
        runner_path,
        solution_path,
    })
}

fn validate_runner(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "language runner is not installed: {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!("language runner is not a file: {}", path.display()));
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "language runner is not executable: {}",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug)]
struct RawEvent {
    stream: Stream,
    bytes: Vec<u8>,
}

enum ReaderMessage {
    Output(RawEvent),
    Finished(Stream, io::Result<()>),
}

struct OutputRetention {
    limit: usize,
    raw_bytes: usize,
    sanitized_bytes: usize,
    initial: Vec<SanitizedEvent>,
    initial_bytes: usize,
    tail: VecDeque<SanitizedEvent>,
    tail_bytes: usize,
}

#[derive(Clone, Debug)]
struct SanitizedEvent {
    stream: Stream,
    text: String,
}

impl OutputRetention {
    fn new(limit: usize) -> Self {
        assert!(limit >= MIN_DISPLAY_OUTPUT_BYTES);
        Self {
            limit,
            raw_bytes: 0,
            sanitized_bytes: 0,
            initial: Vec::new(),
            initial_bytes: 0,
            tail: VecDeque::new(),
            tail_bytes: 0,
        }
    }

    fn push_raw(&mut self, count: usize) {
        self.raw_bytes = self.raw_bytes.saturating_add(count);
    }

    fn push_sanitized(&mut self, stream: Stream, text: &str) {
        if text.is_empty() {
            return;
        }
        self.sanitized_bytes = self.sanitized_bytes.saturating_add(text.len());
        if self.initial_bytes < self.limit {
            let end = floor_char_boundary(text, (self.limit - self.initial_bytes).min(text.len()));
            push_sanitized_event(&mut self.initial, stream, &text[..end]);
            self.initial_bytes += end;
        }

        let tail_limit = self.limit / 2;
        push_sanitized_event_deque(&mut self.tail, stream, text);
        self.tail_bytes += text.len();
        while self.tail_bytes > tail_limit {
            let excess = self.tail_bytes - tail_limit;
            let front = self
                .tail
                .front_mut()
                .expect("tail byte count implies a chunk");
            if excess < front.text.len() {
                let removed = ceil_char_boundary(&front.text, excess);
                front.text.drain(..removed);
                self.tail_bytes -= removed;
                break;
            }
            let removed = self.tail.pop_front().expect("tail contains front chunk");
            self.tail_bytes -= removed.text.len();
        }
        assert!(self.initial_bytes <= self.limit);
        assert!(self.tail_bytes <= tail_limit);
    }

    fn finish(self, tagged: bool) -> (String, usize, bool) {
        let all = render_events(&self.initial, tagged);
        let omitted_bytes = self.raw_bytes.saturating_sub(self.limit);
        let truncated = omitted_bytes != 0
            || self.sanitized_bytes > self.initial_bytes
            || all.len() > self.limit;
        if !truncated {
            return (all, 0, false);
        }

        let tail: Vec<SanitizedEvent> = self.tail.into_iter().collect();
        let available = self.initial_bytes.min(self.limit);
        let mut low = 0_usize;
        let mut high = available;
        let mut best = render_truncated(&self.initial, &tail, 0, omitted_bytes, tagged);
        while low <= high {
            let budget = low + (high - low) / 2;
            let rendered = render_truncated(&self.initial, &tail, budget, omitted_bytes, tagged);
            if rendered.len() <= self.limit {
                best = rendered;
                low = budget.saturating_add(1);
            } else if budget == 0 {
                break;
            } else {
                high = budget - 1;
            }
        }
        assert!(best.len() <= self.limit);
        (best, omitted_bytes, true)
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn push_sanitized_event(events: &mut Vec<SanitizedEvent>, stream: Stream, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = events.last_mut().filter(|last| last.stream == stream) {
        last.text.push_str(text);
    } else {
        events.push(SanitizedEvent {
            stream,
            text: text.to_string(),
        });
    }
}

fn push_sanitized_event_deque(events: &mut VecDeque<SanitizedEvent>, stream: Stream, text: &str) {
    if let Some(last) = events.back_mut().filter(|last| last.stream == stream) {
        last.text.push_str(text);
    } else {
        events.push_back(SanitizedEvent {
            stream,
            text: text.to_string(),
        });
    }
}

fn take_prefix(events: &[SanitizedEvent], count: usize) -> Vec<SanitizedEvent> {
    let mut result = Vec::new();
    let mut remaining = count;
    for event in events {
        let taken = floor_char_boundary(&event.text, event.text.len().min(remaining));
        push_sanitized_event(&mut result, event.stream, &event.text[..taken]);
        remaining = remaining.saturating_sub(taken);
        if remaining == 0 {
            break;
        }
    }
    result
}

fn take_tail(events: &[SanitizedEvent], count: usize) -> Vec<SanitizedEvent> {
    let mut reversed = Vec::new();
    let mut remaining = count;
    for event in events.iter().rev() {
        let start = ceil_char_boundary(&event.text, event.text.len().saturating_sub(remaining));
        reversed.push(SanitizedEvent {
            stream: event.stream,
            text: event.text[start..].to_string(),
        });
        remaining = remaining.saturating_sub(event.text.len() - start);
        if remaining == 0 {
            break;
        }
    }
    reversed.reverse();
    reversed
}

fn render_truncated(
    prefix_source: &[SanitizedEvent],
    tail_source: &[SanitizedEvent],
    budget: usize,
    omitted_bytes: usize,
    tagged: bool,
) -> String {
    let prefix = take_prefix(prefix_source, budget.div_ceil(2));
    let tail = take_tail(tail_source, budget / 2);
    format!(
        "{}\n[... {omitted_bytes} bytes omitted ...]\n{}",
        render_events(&prefix, tagged),
        render_events(&tail, tagged)
    )
}

#[derive(Default)]
struct SanitizedText {
    text: String,
}

impl Perform for SanitizedText {
    fn print(&mut self, character: char) {
        if !character.is_control() {
            self.text.push(character);
        }
    }

    fn execute(&mut self, byte: u8) {
        if matches!(byte, b'\n' | b'\t') {
            self.text.push(char::from(byte));
        }
    }
}

fn sanitize_chunk(parser: &mut Parser, output: &mut SanitizedText, bytes: &[u8]) -> String {
    assert!(output.text.is_empty());
    parser.advance(output, bytes);
    std::mem::take(&mut output.text)
}

fn render_events(events: &[SanitizedEvent], tagged: bool) -> String {
    let mut result = String::new();
    for event in events {
        if tagged {
            result.push_str(match event.stream {
                Stream::Stdout => "[stdout] ",
                Stream::Stderr => "[stderr] ",
            });
        }
        result.push_str(&event.text);
    }
    result
}

fn set_nonblocking<R: AsRawFd>(reader: &R) -> io::Result<()> {
    let descriptor = reader.as_raw_fd();
    // SAFETY: fcntl only reads or updates flags for this owned, valid descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor remains owned by this reader for the thread lifetime.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn reader_thread<R: Read + AsRawFd + Send + 'static>(
    mut reader: R,
    stream: Stream,
    chunk_bytes: usize,
    shutdown: Arc<AtomicBool>,
    sender: SyncSender<ReaderMessage>,
) -> io::Result<JoinHandle<()>> {
    let name = match stream {
        Stream::Stdout => "runner-stdout-reader",
        Stream::Stderr => "runner-stderr-reader",
    };
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let result = set_nonblocking(&reader).and_then(|()| {
                let mut buffer = vec![0_u8; chunk_bytes];
                loop {
                    if shutdown.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    match reader.read(&mut buffer) {
                        Ok(0) => return Ok(()),
                        Ok(count) => {
                            if sender
                                .send(ReaderMessage::Output(RawEvent {
                                    stream,
                                    bytes: buffer[..count].to_vec(),
                                }))
                                .is_err()
                            {
                                return Ok(());
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(POLL_INTERVAL);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) => return Err(error),
                    }
                }
            });
            let _ = sender.send(ReaderMessage::Finished(stream, result));
        })
}

fn send_group_signal(process_group: Pid, signal: Signal) -> Result<bool, String> {
    match kill(Pid::from_raw(-process_group.as_raw()), signal) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(format!("cannot signal runner process group: {error}")),
    }
}

fn group_exists(process_group: Pid) -> Result<bool, String> {
    match kill(Pid::from_raw(-process_group.as_raw()), None) {
        Ok(()) | Err(Errno::EPERM) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(format!("cannot inspect runner process group: {error}")),
    }
}

fn terminate_group(process_group: Pid, grace: Duration) -> Result<(), String> {
    let mut first_error = None;
    match group_exists(process_group) {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(error) => first_error = Some(error),
    }
    if let Err(error) = send_group_signal(process_group, Signal::SIGTERM) {
        first_error.get_or_insert(error);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        match group_exists(process_group) {
            Ok(false) => return first_error.map_or(Ok(()), Err),
            Ok(true) => {
                thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())))
            }
            Err(error) => {
                first_error.get_or_insert(error);
                break;
            }
        }
    }
    if let Err(error) = send_group_signal(process_group, Signal::SIGKILL) {
        first_error.get_or_insert(error);
    }
    first_error.map_or(Ok(()), Err)
}

fn child_has_exited(child_pid: Pid) -> Result<bool, String> {
    assert!(child_pid.as_raw() > 0);
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT;
    match waitid(Id::Pid(child_pid), flags) {
        Ok(WaitStatus::StillAlive) => Ok(false),
        Ok(WaitStatus::Exited(observed_pid, _)) | Ok(WaitStatus::Signaled(observed_pid, _, _)) => {
            if observed_pid != child_pid {
                return Err(format!(
                    "observed unexpected runner pid {observed_pid}; expected {child_pid}"
                ));
            }
            Ok(true)
        }
        Ok(status) => Err(format!("observed unexpected runner status: {status:?}")),
        Err(error) => Err(format!("cannot inspect language runner: {error}")),
    }
}

fn termination_from_status(status: ExitStatus) -> Result<Termination, String> {
    if let Some(code) = status.code() {
        return Ok(Termination::Exited(code));
    }
    status
        .signal()
        .map(Termination::Signalled)
        .ok_or_else(|| "runner ended without an exit code or signal".to_string())
}

fn priority_termination(
    cancellation: &CancellationToken,
    started: Instant,
    timeout: Duration,
) -> Option<Termination> {
    if cancellation.is_cancelled() {
        Some(Termination::Cancelled)
    } else if started.elapsed() >= timeout {
        Some(Termination::TimedOut)
    } else {
        None
    }
}

fn deliver_event(
    sender: Option<&SyncSender<ExecutionEvent>>,
    stream: Stream,
    text: String,
    dropped: &mut usize,
) {
    let Some(sender) = sender else {
        return;
    };
    let event = match stream {
        Stream::Stdout => ExecutionEvent::Stdout(text),
        Stream::Stderr => ExecutionEvent::Stderr(text),
    };
    if matches!(
        sender.try_send(event),
        Err(TrySendError::Full(_) | TrySendError::Disconnected(_))
    ) {
        *dropped = dropped.saturating_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn retain_and_deliver(
    event: RawEvent,
    retention: &mut OutputRetention,
    discovery_stdout: &mut Option<OutputRetention>,
    discovery_stderr: &mut Option<OutputRetention>,
    stdout_parser: &mut Parser,
    stderr_parser: &mut Parser,
    stdout: &mut SanitizedText,
    stderr: &mut SanitizedText,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
    dropped_events: &mut usize,
) {
    retention.push_raw(event.bytes.len());
    let stream_retention = match event.stream {
        Stream::Stdout => discovery_stdout,
        Stream::Stderr => discovery_stderr,
    };
    if let Some(stream_retention) = stream_retention {
        stream_retention.push_raw(event.bytes.len());
    }
    let (parser, output) = match event.stream {
        Stream::Stdout => (stdout_parser, stdout),
        Stream::Stderr => (stderr_parser, stderr),
    };
    let text = sanitize_chunk(parser, output, &event.bytes);
    retention.push_sanitized(event.stream, &text);
    if let Some(stream_retention) = stream_retention {
        stream_retention.push_sanitized(event.stream, &text);
    }
    if !text.is_empty() {
        deliver_event(event_sender, event.stream, text, dropped_events);
    }
}

struct CommandSpec<'a> {
    executable: &'a Path,
    arguments: &'a [&'a str],
    current_dir: &'a Path,
    database_path: Option<&'a Path>,
    tagged_output: bool,
    capture_discovery_streams: bool,
}

fn execute_command(
    spec: CommandSpec<'_>,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
) -> Result<ExecutionResult, String> {
    limits.validate()?;
    validate_runner(spec.executable)?;
    let mut command = Command::new(spec.executable);
    command
        .args(spec.arguments)
        .current_dir(spec.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(database_path) = spec.database_path {
        command
            .env("PRACTICE_NO_RECORD", "1")
            .env("PRACTICE_DB_PATH", database_path);
    }

    let started = Instant::now();
    let mut spawn_attempts = 0_u8;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && spawn_attempts < 4 => {
                spawn_attempts += 1;
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                return Err(format!(
                    "cannot execute {}: {error}",
                    spec.executable.display()
                ));
            }
        }
    };
    let process_group = Pid::from_raw(
        i32::try_from(child.id()).expect("Linux process identifiers fit in signed 32 bits"),
    );
    assert!(process_group.as_raw() > 0);
    let shutdown = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel(limits.event_queue_capacity);
    let mut readers = Vec::with_capacity(2);
    let mut pending_error = None;

    match child.stdout.take() {
        Some(stdout) => match reader_thread(
            stdout,
            Stream::Stdout,
            limits.read_chunk_bytes,
            Arc::clone(&shutdown),
            sender.clone(),
        ) {
            Ok(reader) => readers.push(reader),
            Err(error) => {
                pending_error = Some(format!("cannot start runner stdout reader: {error}"));
            }
        },
        None => {
            pending_error =
                Some("language runner stdout was not captured after successful spawn".to_string());
        }
    }
    if pending_error.is_none() {
        match child.stderr.take() {
            Some(stderr) => match reader_thread(
                stderr,
                Stream::Stderr,
                limits.read_chunk_bytes,
                Arc::clone(&shutdown),
                sender.clone(),
            ) {
                Ok(reader) => readers.push(reader),
                Err(error) => {
                    pending_error = Some(format!("cannot start runner stderr reader: {error}"));
                }
            },
            None => {
                pending_error = Some(
                    "language runner stderr was not captured after successful spawn".to_string(),
                );
            }
        }
    }
    drop(sender);

    let mut retention = OutputRetention::new(limits.display_output_bytes);
    let mut discovery_stdout = spec
        .capture_discovery_streams
        .then(|| OutputRetention::new(limits.display_output_bytes));
    let mut discovery_stderr = spec
        .capture_discovery_streams
        .then(|| OutputRetention::new(limits.display_output_bytes));
    let mut event_stdout_parser = Parser::new();
    let mut event_stderr_parser = Parser::new();
    let mut event_stdout = SanitizedText::default();
    let mut event_stderr = SanitizedText::default();
    let mut dropped_events = 0_usize;
    let mut observed_exit = false;
    let mut requested_termination = None;
    let mut finished_readers = 0_usize;

    while pending_error.is_none() && requested_termination.is_none() && !observed_exit {
        if let Some(termination) = priority_termination(cancellation, started, limits.wall_timeout)
        {
            requested_termination = Some(termination);
            break;
        }
        let status_observation = child_has_exited(process_group);
        if let Some(termination) = priority_termination(cancellation, started, limits.wall_timeout)
        {
            requested_termination = Some(termination);
        } else {
            match status_observation {
                Ok(true) => observed_exit = true,
                Ok(false) => {}
                Err(error) => pending_error = Some(error),
            }
        }
        if requested_termination.is_some() || pending_error.is_some() || observed_exit {
            break;
        }
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(ReaderMessage::Output(event)) => {
                if let Some(termination) =
                    priority_termination(cancellation, started, limits.wall_timeout)
                {
                    requested_termination = Some(termination);
                }
                retain_and_deliver(
                    event,
                    &mut retention,
                    &mut discovery_stdout,
                    &mut discovery_stderr,
                    &mut event_stdout_parser,
                    &mut event_stderr_parser,
                    &mut event_stdout,
                    &mut event_stderr,
                    event_sender,
                    &mut dropped_events,
                );
                if requested_termination.is_some() {
                    break;
                }
            }
            Ok(ReaderMessage::Finished(stream, result)) => {
                finished_readers += 1;
                if let Some(termination) =
                    priority_termination(cancellation, started, limits.wall_timeout)
                {
                    requested_termination = Some(termination);
                } else if let Err(error) = result {
                    pending_error = Some(format!("cannot drain runner {stream:?}: {error}"));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let status_observation = child_has_exited(process_group);
                if let Some(termination) =
                    priority_termination(cancellation, started, limits.wall_timeout)
                {
                    requested_termination = Some(termination);
                } else {
                    match status_observation {
                        Ok(true) => observed_exit = true,
                        Ok(false) => {
                            pending_error = Some(
                                "runner pipes closed before child status was available".to_string(),
                            )
                        }
                        Err(error) => pending_error = Some(error),
                    }
                }
            }
        }
    }

    if let Err(error) = terminate_group(process_group, limits.term_grace) {
        pending_error.get_or_insert(error);
    }
    let final_status = match child.wait() {
        Ok(status) => Some(status),
        Err(error) => {
            pending_error.get_or_insert_with(|| format!("cannot reap language runner: {error}"));
            None
        }
    };

    let drain_deadline = Instant::now() + FINAL_DRAIN_LIMIT;
    while finished_readers < readers.len() && Instant::now() < drain_deadline {
        match receiver.recv_timeout(
            POLL_INTERVAL.min(drain_deadline.saturating_duration_since(Instant::now())),
        ) {
            Ok(ReaderMessage::Output(event)) => retain_and_deliver(
                event,
                &mut retention,
                &mut discovery_stdout,
                &mut discovery_stderr,
                &mut event_stdout_parser,
                &mut event_stderr_parser,
                &mut event_stdout,
                &mut event_stderr,
                event_sender,
                &mut dropped_events,
            ),
            Ok(ReaderMessage::Finished(stream, result)) => {
                finished_readers += 1;
                if let Err(error) = result {
                    pending_error
                        .get_or_insert_with(|| format!("cannot drain runner {stream:?}: {error}"));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    shutdown.store(true, Ordering::Release);
    while let Ok(message) = receiver.recv_timeout(POLL_INTERVAL) {
        match message {
            ReaderMessage::Output(event) => retain_and_deliver(
                event,
                &mut retention,
                &mut discovery_stdout,
                &mut discovery_stderr,
                &mut event_stdout_parser,
                &mut event_stderr_parser,
                &mut event_stdout,
                &mut event_stderr,
                event_sender,
                &mut dropped_events,
            ),
            ReaderMessage::Finished(stream, result) => {
                finished_readers += 1;
                if let Err(error) = result {
                    pending_error
                        .get_or_insert_with(|| format!("cannot drain runner {stream:?}: {error}"));
                }
            }
        }
        if finished_readers == readers.len() {
            break;
        }
    }
    for reader in readers {
        if reader.join().is_err() {
            pending_error.get_or_insert_with(|| "runner pipe reader thread panicked".to_string());
        }
    }

    if cancellation.is_cancelled() {
        requested_termination = Some(Termination::Cancelled);
    }
    if requested_termination.is_none()
        && let Some(error) = pending_error.as_ref()
    {
        return Err(error.clone());
    }
    let mut termination = if let Some(termination) = requested_termination {
        termination
    } else {
        termination_from_status(final_status.ok_or_else(|| {
            pending_error.unwrap_or_else(|| "language runner status is unavailable".to_string())
        })?)?
    };
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let (display_output, omitted_bytes, _display_truncated) = retention.finish(spec.tagged_output);
    let (discovery_stdout, discovery_stdout_truncated) = discovery_stdout
        .map(|retention| retention.finish(false))
        .map_or((None, false), |(output, _, truncated)| {
            (Some(output), truncated)
        });
    let discovery_stderr = discovery_stderr.map(|retention| retention.finish(false).0);
    assert!(display_output.len() <= limits.display_output_bytes);
    if cancellation.is_cancelled() {
        termination = Termination::Cancelled;
    }
    Ok(ExecutionResult {
        termination,
        display_output,
        omitted_bytes,
        dropped_events,
        duration_ms,
        discovery_stdout,
        discovery_stderr,
        discovery_stdout_truncated,
    })
}

pub fn execute(
    plan: &ExecutionPlan,
    database_path: &Path,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
) -> Result<ExecutionResult, String> {
    if !plan.root.is_dir() {
        return Err(format!(
            "project root does not exist: {}",
            plan.root.display()
        ));
    }
    let parent = plan
        .runner_path
        .parent()
        .ok_or_else(|| "language runner has no parent directory".to_string())?;
    execute_command(
        CommandSpec {
            executable: &plan.runner_path,
            arguments: &["--problem", &plan.problem_slug],
            current_dir: parent,
            database_path: Some(database_path),
            tagged_output: true,
            capture_discovery_streams: false,
        },
        limits,
        cancellation,
        event_sender,
    )
}

pub fn discover_adapters(
    runner_path: &Path,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, String> {
    let parent = runner_path
        .parent()
        .ok_or_else(|| "language runner has no parent directory".to_string())?;
    let result = execute_command(
        CommandSpec {
            executable: runner_path,
            arguments: &["--list"],
            current_dir: parent,
            database_path: None,
            tagged_output: false,
            capture_discovery_streams: true,
        },
        limits,
        cancellation,
        None,
    )?;
    let stdout = result
        .discovery_stdout
        .as_deref()
        .expect("discovery execution captures stdout");
    let stderr = result
        .discovery_stderr
        .as_deref()
        .expect("discovery execution captures stderr");
    if result.termination != Termination::Exited(0) {
        return Err(format!(
            "language runner discovery failed: {:?}: {}",
            result.termination,
            stderr.trim()
        ));
    }
    if result.discovery_stdout_truncated {
        return Err(format!(
            "language runner discovery exceeded {} output bytes",
            limits.display_output_bytes
        ));
    }
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn record_execution(
    connection: &Connection,
    plan: &ExecutionPlan,
    result: &ExecutionResult,
) -> Result<i64, String> {
    database::record_attempt(
        connection,
        &plan.problem_slug,
        &plan.language,
        result.outcome(),
        result.duration_ms,
        result.exit_code(),
        plan.set_slug.as_deref(),
    )
}
