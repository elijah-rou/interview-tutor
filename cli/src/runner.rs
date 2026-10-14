use crate::attempt::{
    AttemptReport, CapturedText, CaseEntry, CaseName, CaseRun, MAX_CASES, MAX_STORED_STREAM_BYTES,
    NotRunReason, StepRun, Termination, TestOutput,
};
use crate::database;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
use nix::unistd::Pid;
use rusqlite::Connection;
use std::collections::HashSet;
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
const MIN_DISCOVERY_OUTPUT_BYTES: usize = 64;
const MAX_DISCOVERY_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MIN_READ_CHUNK_BYTES: usize = 256;
const MAX_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_EVENT_QUEUE_CAPACITY: usize = 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const FINAL_DRAIN_LIMIT: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct ExecutionLimits {
    pub wall_timeout: Duration,
    pub term_grace: Duration,
    /// Retained bytes per stream for `--list` discovery. Attempt steps always retain
    /// `attempt::MAX_STORED_STREAM_BYTES` per stream.
    pub discovery_output_bytes: usize,
    pub read_chunk_bytes: usize,
    pub event_queue_capacity: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            wall_timeout: Duration::from_secs(30),
            term_grace: Duration::from_millis(250),
            discovery_output_bytes: 64 * 1024,
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
        if !(MIN_DISCOVERY_OUTPUT_BYTES..=MAX_DISCOVERY_OUTPUT_BYTES)
            .contains(&self.discovery_output_bytes)
        {
            return Err("discovery output limit must be between 64 bytes and 16 MiB".to_string());
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

#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub root: PathBuf,
    pub language: String,
    pub problem_slug: String,
    pub set_slug: Option<String>,
    pub runner_path: PathBuf,
    pub solution_path: PathBuf,
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

/// Sanitized text from one stream: the first `limit` bytes plus a rolling tail of the last
/// `limit / 2` bytes, so a truncated stream keeps both its beginning and its end.
struct StreamRetention {
    limit: usize,
    total_bytes: u64,
    head: String,
    tail: String,
}

enum RetainedText {
    Complete(String),
    Truncated {
        head: String,
        omitted_bytes: u64,
        tail: String,
    },
}

impl StreamRetention {
    fn new(limit: usize) -> Self {
        assert!(limit >= MIN_DISCOVERY_OUTPUT_BYTES);
        Self {
            limit,
            total_bytes: 0,
            head: String::new(),
            tail: String::new(),
        }
    }

    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.total_bytes = self.total_bytes.saturating_add(text.len() as u64);
        if self.head.len() < self.limit {
            let end = floor_char_boundary(text, self.limit - self.head.len());
            self.head.push_str(&text[..end]);
        }
        let tail_limit = self.limit / 2;
        self.tail.push_str(text);
        if self.tail.len() > tail_limit {
            let excess = ceil_char_boundary(&self.tail, self.tail.len() - tail_limit);
            self.tail.drain(..excess);
        }
        assert!(self.head.len() <= self.limit);
        assert!(self.tail.len() <= tail_limit);
    }

    fn finish(mut self) -> RetainedText {
        if self.total_bytes <= self.head.len() as u64 {
            return RetainedText::Complete(self.head);
        }
        let head_end = floor_char_boundary(&self.head, self.limit / 2);
        self.head.truncate(head_end);
        let retained = (self.head.len() + self.tail.len()) as u64;
        assert!(self.total_bytes > retained);
        RetainedText::Truncated {
            head: self.head,
            omitted_bytes: self.total_bytes - retained,
            tail: self.tail,
        }
    }
}

impl RetainedText {
    fn into_captured(self) -> CapturedText {
        match self {
            Self::Complete(text) => CapturedText::complete(text),
            Self::Truncated {
                head,
                omitted_bytes,
                tail,
            } => CapturedText::truncated(head, omitted_bytes, tail),
        }
        .expect("attempt step retention uses the stored stream bound")
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

struct StreamCapture {
    retention: StreamRetention,
    parser: Parser,
    sanitized: SanitizedText,
}

impl StreamCapture {
    fn new(limit: usize) -> Self {
        Self {
            retention: StreamRetention::new(limit),
            parser: Parser::new(),
            sanitized: SanitizedText::default(),
        }
    }
}

fn retain_and_deliver(
    event: RawEvent,
    stdout: &mut StreamCapture,
    stderr: &mut StreamCapture,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
    dropped_events: &mut usize,
) {
    let capture = match event.stream {
        Stream::Stdout => stdout,
        Stream::Stderr => stderr,
    };
    let text = sanitize_chunk(&mut capture.parser, &mut capture.sanitized, &event.bytes);
    capture.retention.push(&text);
    if !text.is_empty() {
        deliver_event(event_sender, event.stream, text, dropped_events);
    }
}

struct CommandSpec<'a> {
    executable: &'a Path,
    arguments: &'a [&'a str],
    current_dir: &'a Path,
    database_path: Option<&'a Path>,
    stream_limit: usize,
    timeout: Duration,
}

struct CommandCapture {
    termination: Termination,
    duration_ms: i64,
    stdout: RetainedText,
    stderr: RetainedText,
    dropped_events: usize,
}

fn execute_command(
    spec: CommandSpec<'_>,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
) -> Result<CommandCapture, String> {
    limits.validate()?;
    assert!(spec.timeout >= MIN_WALL_TIMEOUT);
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

    let mut stdout = StreamCapture::new(spec.stream_limit);
    let mut stderr = StreamCapture::new(spec.stream_limit);
    let mut dropped_events = 0_usize;
    let mut observed_exit = false;
    let mut requested_termination = None;
    let mut finished_readers = 0_usize;

    while pending_error.is_none() && requested_termination.is_none() && !observed_exit {
        if let Some(termination) = priority_termination(cancellation, started, spec.timeout) {
            requested_termination = Some(termination);
            break;
        }
        let status_observation = child_has_exited(process_group);
        if let Some(termination) = priority_termination(cancellation, started, spec.timeout) {
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
                if let Some(termination) = priority_termination(cancellation, started, spec.timeout)
                {
                    requested_termination = Some(termination);
                }
                retain_and_deliver(
                    event,
                    &mut stdout,
                    &mut stderr,
                    event_sender,
                    &mut dropped_events,
                );
                if requested_termination.is_some() {
                    break;
                }
            }
            Ok(ReaderMessage::Finished(stream, result)) => {
                finished_readers += 1;
                if let Some(termination) = priority_termination(cancellation, started, spec.timeout)
                {
                    requested_termination = Some(termination);
                } else if let Err(error) = result {
                    pending_error = Some(format!("cannot drain runner {stream:?}: {error}"));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let status_observation = child_has_exited(process_group);
                if let Some(termination) = priority_termination(cancellation, started, spec.timeout)
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
                &mut stdout,
                &mut stderr,
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
                &mut stdout,
                &mut stderr,
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
    if cancellation.is_cancelled() {
        termination = Termination::Cancelled;
    }
    Ok(CommandCapture {
        termination,
        duration_ms,
        stdout: stdout.retention.finish(),
        stderr: stderr.retention.finish(),
        dropped_events,
    })
}

/// Runs one attempt: the setup step lists the problem's cases, then each case runs in its own
/// process, all against one wall deadline. A case failure does not stop later cases;
/// cancellation and the deadline mark the remaining cases as not run.
pub fn execute(
    plan: &ExecutionPlan,
    database_path: &Path,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    event_sender: Option<&SyncSender<ExecutionEvent>>,
) -> Result<AttemptReport, String> {
    limits.validate()?;
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
    let started = Instant::now();
    let deadline = started + limits.wall_timeout;
    let mut dropped_events = 0_usize;
    let mut run_step = |arguments: &[&str], timeout: Duration| {
        execute_command(
            CommandSpec {
                executable: &plan.runner_path,
                arguments,
                current_dir: parent,
                database_path: Some(database_path),
                stream_limit: MAX_STORED_STREAM_BYTES,
                timeout,
            },
            limits,
            cancellation,
            event_sender,
        )
        .map(|capture| {
            dropped_events = dropped_events.saturating_add(capture.dropped_events);
            StepRun {
                termination: capture.termination,
                duration_ms: capture.duration_ms,
                stdout: capture.stdout.into_captured(),
                stderr: capture.stderr.into_captured(),
            }
        })
    };

    let setup = run_step(&["--cases", &plan.problem_slug], limits.wall_timeout)?;
    let names = if setup.termination == Termination::Exited(0) {
        parse_case_names(&setup.stdout)
            .map_err(|error| format!("language runner case list is invalid: {error}"))?
    } else {
        Vec::new()
    };
    let mut cases = Vec::with_capacity(names.len());
    for name in names {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let run = if cancellation.is_cancelled() {
            CaseRun::NotRun(NotRunReason::Cancelled)
        } else if remaining < MIN_WALL_TIMEOUT {
            CaseRun::NotRun(NotRunReason::DeadlineExceeded)
        } else {
            CaseRun::Ran(run_step(
                &["--problem", &plan.problem_slug, "--case", name.as_str()],
                remaining,
            )?)
        };
        cases.push(CaseEntry { name, run });
    }
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let mut report =
        AttemptReport::new(TestOutput::new(setup, cases)?, duration_ms, dropped_events);
    if cancellation.is_cancelled() {
        report.mark_cancelled();
    }
    Ok(report)
}

fn parse_case_names(stdout: &CapturedText) -> Result<Vec<CaseName>, String> {
    let CapturedText::Complete(text) = stdout else {
        return Err("output was truncated".to_string());
    };
    let names = text
        .lines()
        .map(CaseName::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if !(1..=MAX_CASES).contains(&names.len()) {
        return Err(format!(
            "expected 1 to {MAX_CASES} case names, got {}",
            names.len()
        ));
    }
    let mut seen = HashSet::with_capacity(names.len());
    if let Some(duplicate) = names.iter().find(|name| !seen.insert(*name)) {
        return Err(format!("duplicate case name {:?}", duplicate.as_str()));
    }
    Ok(names)
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
            stream_limit: limits.discovery_output_bytes,
            timeout: limits.wall_timeout,
        },
        limits,
        cancellation,
        None,
    )?;
    if result.termination != Termination::Exited(0) {
        let stderr = match result.stderr {
            RetainedText::Complete(text) => text,
            RetainedText::Truncated { head, tail, .. } => format!("{head}\n{tail}"),
        };
        return Err(format!(
            "language runner discovery failed: {:?}: {}",
            result.termination,
            stderr.trim()
        ));
    }
    let RetainedText::Complete(stdout) = result.stdout else {
        return Err(format!(
            "language runner discovery exceeded {} output bytes",
            limits.discovery_output_bytes
        ));
    };
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
    report: &AttemptReport,
) -> Result<i64, String> {
    database::record_attempt(
        connection,
        &plan.problem_slug,
        &plan.language,
        plan.set_slug.as_deref(),
        report,
    )
}
