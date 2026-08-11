use crate::database;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::{Pid, setpgid};
use rusqlite::Connection;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MIN_WALL_TIMEOUT: Duration = Duration::from_millis(10);
const MAX_WALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const MAX_TERM_GRACE: Duration = Duration::from_secs(10);
const MIN_DISPLAY_OUTPUT_BYTES: usize = 64;
const MAX_DISPLAY_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MIN_READ_CHUNK_BYTES: usize = 256;
const MAX_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_EVENT_QUEUE_CAPACITY: usize = 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

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
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
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
    EventDeliveryFailed(String),
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
    pub duration_ms: i64,
}

impl ExecutionResult {
    pub fn exit_code(&self) -> Option<i32> {
        match self.termination {
            Termination::Exited(code) => Some(code),
            Termination::Cancelled
            | Termination::TimedOut
            | Termination::Signalled(_)
            | Termination::EventDeliveryFailed(_) => None,
        }
    }

    pub fn status_code(&self) -> i32 {
        match self.termination {
            Termination::Exited(code) => code,
            Termination::Cancelled => 130,
            Termination::TimedOut
            | Termination::Signalled(_)
            | Termination::EventDeliveryFailed(_) => 2,
        }
    }

    pub fn outcome(&self) -> database::AttemptOutcome {
        match self.termination {
            Termination::Exited(0) => database::AttemptOutcome::Pass,
            Termination::Exited(2) => database::AttemptOutcome::Error,
            Termination::Exited(_) => database::AttemptOutcome::Fail,
            Termination::Cancelled => database::AttemptOutcome::Cancelled,
            Termination::TimedOut
            | Termination::Signalled(_)
            | Termination::EventDeliveryFailed(_) => database::AttemptOutcome::Error,
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum Stream {
    Stdout,
    Stderr,
}

struct RawEvent {
    stream: Stream,
    bytes: Vec<u8>,
}

struct OutputRetention {
    limit: usize,
    total_bytes: usize,
    initial: Vec<RawEvent>,
    initial_bytes: usize,
    tail: VecDeque<RawEvent>,
    tail_bytes: usize,
}

impl OutputRetention {
    fn new(limit: usize) -> Self {
        assert!(limit >= MIN_DISPLAY_OUTPUT_BYTES);
        Self {
            limit,
            total_bytes: 0,
            initial: Vec::new(),
            initial_bytes: 0,
            tail: VecDeque::new(),
            tail_bytes: 0,
        }
    }

    fn push(&mut self, stream: Stream, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        if self.initial_bytes < self.limit {
            let retained = bytes.len().min(self.limit - self.initial_bytes);
            if let Some(last) = self.initial.last_mut().filter(|last| last.stream == stream) {
                last.bytes.extend_from_slice(&bytes[..retained]);
            } else {
                self.initial.push(RawEvent {
                    stream,
                    bytes: bytes[..retained].to_vec(),
                });
            }
            self.initial_bytes += retained;
        }

        let tail_limit = self.limit / 2;
        if let Some(last) = self.tail.back_mut().filter(|last| last.stream == stream) {
            last.bytes.extend_from_slice(bytes);
        } else {
            self.tail.push_back(RawEvent {
                stream,
                bytes: bytes.to_vec(),
            });
        }
        self.tail_bytes += bytes.len();
        while self.tail_bytes > tail_limit {
            let excess = self.tail_bytes - tail_limit;
            let front = self
                .tail
                .front_mut()
                .expect("tail byte count implies a chunk");
            if excess < front.bytes.len() {
                front.bytes.drain(..excess);
                self.tail_bytes -= excess;
                break;
            }
            let removed = self.tail.pop_front().expect("tail contains front chunk");
            self.tail_bytes -= removed.bytes.len();
        }
        assert!(self.initial_bytes <= self.limit);
        assert!(self.tail_bytes <= tail_limit);
    }

    fn finish(self) -> (String, usize) {
        if self.total_bytes <= self.limit {
            return (render_events(self.initial.iter()), 0);
        }
        let prefix_limit = self.limit - self.limit / 2;
        let mut remaining = prefix_limit;
        let mut prefix = String::new();
        for event in &self.initial {
            if remaining == 0 {
                break;
            }
            let count = event.bytes.len().min(remaining);
            prefix.push_str(&render_event(event.stream, &event.bytes[..count]));
            remaining -= count;
        }
        let omitted_bytes = self.total_bytes - self.limit;
        let marker = format!("\n[... {omitted_bytes} bytes omitted ...]\n");
        let tail = render_events(self.tail.iter());
        (format!("{prefix}{marker}{tail}"), omitted_bytes)
    }
}

fn sanitize(bytes: &[u8]) -> String {
    let stripped = strip_ansi_escapes::strip(bytes);
    String::from_utf8_lossy(&stripped)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect()
}

fn render_event(stream: Stream, bytes: &[u8]) -> String {
    let tag = match stream {
        Stream::Stdout => "[stdout] ",
        Stream::Stderr => "[stderr] ",
    };
    format!("{tag}{}", sanitize(bytes))
}

fn render_events<'a>(events: impl Iterator<Item = &'a RawEvent>) -> String {
    events
        .map(|event| render_event(event.stream, &event.bytes))
        .collect()
}

fn reader_thread<R: Read + Send + 'static>(
    mut reader: R,
    stream: Stream,
    chunk_bytes: usize,
    sender: SyncSender<RawEvent>,
) -> JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let mut buffer = vec![0_u8; chunk_bytes];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            if sender
                .send(RawEvent {
                    stream,
                    bytes: buffer[..count].to_vec(),
                })
                .is_err()
            {
                return Ok(());
            }
        }
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

fn drain_one(receiver: &Receiver<RawEvent>, retention: &mut OutputRetention) -> bool {
    match receiver.recv_timeout(POLL_INTERVAL) {
        Ok(event) => {
            retention.push(event.stream, &event.bytes);
            true
        }
        Err(RecvTimeoutError::Timeout) => true,
        Err(RecvTimeoutError::Disconnected) => false,
    }
}

fn terminate_group(
    process_group: Pid,
    grace: Duration,
    receiver: &Receiver<RawEvent>,
    retention: &mut OutputRetention,
) -> Result<(), String> {
    if !group_exists(process_group)? {
        return Ok(());
    }
    send_group_signal(process_group, Signal::SIGTERM)?;
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline && group_exists(process_group)? {
        drain_one(receiver, retention);
    }
    if group_exists(process_group)? {
        send_group_signal(process_group, Signal::SIGKILL)?;
    }
    Ok(())
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

fn wait_direct_child(child: &mut Child) -> Result<ExitStatus, String> {
    child
        .wait()
        .map_err(|error| format!("cannot reap language runner: {error}"))
}

struct CommandSpec<'a> {
    executable: &'a Path,
    arguments: &'a [&'a str],
    current_dir: &'a Path,
    database_path: Option<&'a Path>,
}

fn execute_command<F>(
    spec: CommandSpec<'_>,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    mut callback: F,
) -> Result<ExecutionResult, String>
where
    F: FnMut(ExecutionEvent) -> Result<(), String>,
{
    limits.validate()?;
    validate_runner(spec.executable)?;
    let mut command = Command::new(spec.executable);
    command
        .args(spec.arguments)
        .current_dir(spec.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(database_path) = spec.database_path {
        command
            .env("PRACTICE_NO_RECORD", "1")
            .env("PRACTICE_DB_PATH", database_path);
    }
    // SAFETY: setpgid is async-signal-safe and the closure accesses no shared state.
    unsafe {
        command.pre_exec(|| setpgid(Pid::from_raw(0), Pid::from_raw(0)).map_err(io::Error::other));
    }

    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot execute {}: {error}", spec.executable.display()))?;
    let process_group = Pid::from_raw(
        i32::try_from(child.id()).map_err(|_| "runner process id exceeds i32".to_string())?,
    );
    let stdout = child.stdout.take().ok_or_else(|| {
        "language runner stdout was not captured after successful spawn".to_string()
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        "language runner stderr was not captured after successful spawn".to_string()
    })?;
    let (sender, receiver) = mpsc::sync_channel(limits.event_queue_capacity);
    let stdout_reader = reader_thread(
        stdout,
        Stream::Stdout,
        limits.read_chunk_bytes,
        sender.clone(),
    );
    let stderr_reader = reader_thread(
        stderr,
        Stream::Stderr,
        limits.read_chunk_bytes,
        sender.clone(),
    );
    drop(sender);

    let mut retention = OutputRetention::new(limits.display_output_bytes);
    let mut observed_status = None;
    let mut requested_termination = None;
    loop {
        if cancellation.is_cancelled() {
            requested_termination = Some(Termination::Cancelled);
            break;
        }
        if started.elapsed() >= limits.wall_timeout {
            requested_termination = Some(Termination::TimedOut);
            break;
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot inspect language runner: {error}"))?
        {
            observed_status = Some(status);
            break;
        }
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(event) => {
                retention.push(event.stream, &event.bytes);
                let delivered = match event.stream {
                    Stream::Stdout => ExecutionEvent::Stdout(sanitize(&event.bytes)),
                    Stream::Stderr => ExecutionEvent::Stderr(sanitize(&event.bytes)),
                };
                if let Err(error) = callback(delivered) {
                    requested_termination = Some(Termination::EventDeliveryFailed(error));
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                observed_status = Some(wait_direct_child(&mut child)?);
                break;
            }
        }
    }

    terminate_group(process_group, limits.term_grace, &receiver, &mut retention)?;
    let final_status = if let Some(status) = observed_status {
        status
    } else {
        wait_direct_child(&mut child)?
    };
    while let Ok(event) = receiver.recv() {
        retention.push(event.stream, &event.bytes);
        if requested_termination.is_none() {
            let delivered = match event.stream {
                Stream::Stdout => ExecutionEvent::Stdout(sanitize(&event.bytes)),
                Stream::Stderr => ExecutionEvent::Stderr(sanitize(&event.bytes)),
            };
            if let Err(error) = callback(delivered) {
                requested_termination = Some(Termination::EventDeliveryFailed(error));
            }
        }
    }
    stdout_reader
        .join()
        .map_err(|_| "stdout reader thread panicked".to_string())?
        .map_err(|error| format!("cannot drain runner stdout: {error}"))?;
    stderr_reader
        .join()
        .map_err(|_| "stderr reader thread panicked".to_string())?
        .map_err(|error| format!("cannot drain runner stderr: {error}"))?;

    let termination = if let Some(termination) = requested_termination {
        termination
    } else {
        termination_from_status(final_status)?
    };
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let (display_output, omitted_bytes) = retention.finish();
    Ok(ExecutionResult {
        termination,
        display_output,
        omitted_bytes,
        duration_ms,
    })
}

pub fn execute<F>(
    plan: &ExecutionPlan,
    database_path: &Path,
    limits: &ExecutionLimits,
    cancellation: &CancellationToken,
    callback: F,
) -> Result<ExecutionResult, String>
where
    F: FnMut(ExecutionEvent) -> Result<(), String>,
{
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
        },
        limits,
        cancellation,
        callback,
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
    let mut stdout = String::new();
    let result = execute_command(
        CommandSpec {
            executable: runner_path,
            arguments: &["--list"],
            current_dir: parent,
            database_path: None,
        },
        limits,
        cancellation,
        |event| {
            if let ExecutionEvent::Stdout(text) = event {
                let mut remaining = limits.display_output_bytes.saturating_sub(stdout.len());
                for character in text.chars() {
                    if character.len_utf8() > remaining {
                        break;
                    }
                    stdout.push(character);
                    remaining -= character.len_utf8();
                }
            }
            Ok(())
        },
    )?;
    if result.termination != Termination::Exited(0) {
        return Err(format!(
            "language runner discovery failed: {:?}",
            result.termination
        ));
    }
    if result.omitted_bytes != 0 {
        return Err(format!(
            "language runner discovery exceeded {} output bytes",
            limits.display_output_bytes
        ));
    }
    let mut adapters = Vec::new();
    for line in stdout.lines() {
        let slug = line.trim();
        if !slug.is_empty() {
            adapters.push(slug.to_string());
        }
    }
    Ok(adapters)
}

pub fn record_execution(
    connection: &Connection,
    plan: &ExecutionPlan,
    result: &ExecutionResult,
) -> Result<(), String> {
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
