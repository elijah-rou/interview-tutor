pub mod grid;
pub mod key;
mod msgpack;
mod process;

use grid::GridSnapshot;
use process::{OriginEvent, RpcProcess, Snapshot};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub use process::resolve_executable;

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 16;
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TutorAction {
    Test,
    Submit,
    Back,
    Collapse,
    Quit,
    Hint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentUpdate {
    pub text: String,
    pub mode: String,
    pub changedtick: u64,
}

#[derive(Default)]
struct WorkerShared {
    latest_grid: Option<Arc<GridSnapshot>>,
    latest_document: Option<DocumentUpdate>,
    events: VecDeque<WorkerEvent>,
}

#[derive(Clone, Debug)]
pub struct ActionUpdate {
    pub action: TutorAction,
    pub document: DocumentUpdate,
}

#[derive(Clone, Debug)]
pub struct BarrierUpdate {
    pub id: u64,
    pub document: DocumentUpdate,
}

#[derive(Clone, Debug)]
pub enum SourceEvent {
    Action(ActionUpdate),
    Barrier(BarrierUpdate),
}

#[derive(Clone, Debug)]
pub enum WorkerEvent {
    Started(Result<(), String>),
    Source(SourceEvent),
    Warning(String),
    Failed(String),
}

#[derive(Default)]
pub struct PollResult {
    pub grid: Option<Arc<GridSnapshot>>,
    pub document: Option<DocumentUpdate>,
    pub events: Vec<WorkerEvent>,
}

enum Command {
    Start {
        source: String,
        synthetic_name: String,
        language: String,
        width: u16,
        height: u16,
    },
    Input(String),
    Paste(String),
    Mouse {
        button: String,
        action: String,
        modifiers: String,
        row: u16,
        column: u16,
    },
    Resize(u16, u16),
    Barrier(u64),
    AcknowledgeSaved {
        changedtick: u64,
        source: String,
    },
    Stop,
}

#[derive(Clone)]
struct SessionConfig {
    source: String,
    synthetic_name: String,
    language: String,
    width: u16,
    height: u16,
    accepted_changedtick: u64,
    cancellation: Arc<AtomicBool>,
}

pub struct Worker {
    sender: Option<SyncSender<Command>>,
    shared: Arc<Mutex<WorkerShared>>,
    join: Option<JoinHandle<()>>,
    done: Receiver<()>,
    cancellation: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(executable: PathBuf) -> Self {
        let (sender, commands) = mpsc::sync_channel(COMMAND_CAPACITY);
        let shared = Arc::new(Mutex::new(WorkerShared::default()));
        let thread_shared = Arc::clone(&shared);
        let cancellation = Arc::new(AtomicBool::new(false));
        let thread_cancellation = Arc::clone(&cancellation);
        let (done_sender, done) = mpsc::sync_channel(1);
        let join = thread::spawn(move || {
            controller(executable, commands, thread_shared, thread_cancellation);
            let _ = done_sender.try_send(());
        });
        Self {
            sender: Some(sender),
            shared,
            join: Some(join),
            done,
            cancellation,
        }
    }

    pub fn start_session(
        &self,
        source: String,
        synthetic_name: String,
        language: String,
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        self.send(Command::Start {
            source,
            synthetic_name,
            language,
            width,
            height,
        })
    }

    pub fn input(&self, encoded: String) -> Result<(), String> {
        self.send(Command::Input(encoded))
    }

    pub fn paste(&self, text: String) -> Result<(), String> {
        if text.len() > crate::editor::MAX_DOCUMENT_BYTES {
            return Err("paste exceeds editor document byte bound".into());
        }
        if text.bytes().filter(|byte| *byte == b'\n').count() >= crate::editor::MAX_DOCUMENT_LINES {
            return Err("paste exceeds editor document line bound".into());
        }
        self.send(Command::Paste(text))
    }

    pub fn mouse(
        &self,
        button: String,
        action: String,
        modifiers: String,
        row: u16,
        column: u16,
    ) -> Result<(), String> {
        self.send(Command::Mouse {
            button,
            action,
            modifiers,
            row,
            column,
        })
    }

    pub fn resize(&self, width: u16, height: u16) -> Result<(), String> {
        self.send(Command::Resize(width, height))
    }

    pub fn barrier(&self, id: u64) -> Result<(), String> {
        self.send(Command::Barrier(id))
    }

    pub fn acknowledge_saved(&self, changedtick: u64, source: String) -> Result<(), String> {
        self.send(Command::AcknowledgeSaved {
            changedtick,
            source,
        })
    }

    pub fn stop_session(&self) -> Result<(), String> {
        self.send(Command::Stop)
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("Neovim worker is stopped")?
            .try_send(command)
            .map_err(|error| format!("Neovim worker queue is full or stopped: {error}"))
    }

    pub fn poll(&self) -> PollResult {
        take_poll_result(&self.shared)
    }

    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        self.sender.take();
        match self.done.recv_timeout(WORKER_SHUTDOWN_TIMEOUT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                if let Some(join) = self.join.take() {
                    let _ = join.join();
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                self.join.take();
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

fn take_poll_result(shared: &Arc<Mutex<WorkerShared>>) -> PollResult {
    let mut shared = shared.lock().expect("Neovim worker lock");
    let mut result = PollResult {
        grid: shared.latest_grid.take(),
        document: shared.latest_document.take(),
        ..PollResult::default()
    };
    result.events.extend(shared.events.drain(..));
    result
}

fn controller(
    executable: PathBuf,
    commands: mpsc::Receiver<Command>,
    shared: Arc<Mutex<WorkerShared>>,
    cancellation: Arc<AtomicBool>,
) {
    let mut process: Option<RpcProcess> = None;
    let mut config: Option<SessionConfig> = None;
    loop {
        if cancellation.load(Ordering::Acquire) {
            break;
        }
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(Command::Start {
                source,
                synthetic_name,
                language,
                width,
                height,
            }) => {
                if let Some(mut previous) = process.take() {
                    let _ = previous.shutdown();
                }
                let mut next_config = SessionConfig {
                    source,
                    synthetic_name,
                    language,
                    width,
                    height,
                    accepted_changedtick: 0,
                    cancellation: Arc::clone(&cancellation),
                };
                match spawn(&executable, &next_config) {
                    Ok(mut next) => match sync_document(&mut next, &mut next_config, &shared) {
                        Ok(_) => {
                            process = Some(next);
                            config = Some(next_config);
                            push_event(&shared, WorkerEvent::Started(Ok(())))
                        }
                        Err(error) => {
                            let _ = next.shutdown();
                            config = None;
                            push_event(&shared, WorkerEvent::Started(Err(error)))
                        }
                    },
                    Err(error) => {
                        config = None;
                        push_event(&shared, WorkerEvent::Started(Err(error)))
                    }
                }
            }
            Ok(Command::Input(encoded)) => {
                sync_reader_state(&executable, &mut process, &mut config, &shared);
                let input_error = process
                    .as_mut()
                    .and_then(|process| process.feed_key(&encoded).err());
                if let Some(error) = input_error {
                    fail_process(&mut process, &shared, error);
                } else {
                    sync_reader_state(&executable, &mut process, &mut config, &shared);
                }
            }
            Ok(Command::Paste(text)) => {
                run_and_sync(&executable, &mut process, &mut config, &shared, |process| {
                    process.paste(text)
                });
            }
            Ok(Command::Mouse {
                button,
                action,
                modifiers,
                row,
                column,
            }) => {
                let mouse_error = process.as_mut().and_then(|process| {
                    process.mouse(button, action, modifiers, row, column).err()
                });
                if let Some(error) = mouse_error {
                    fail_process(&mut process, &shared, error);
                }
            }
            Ok(Command::Barrier(id)) => {
                sync_reader_state(&executable, &mut process, &mut config, &shared);
                let barrier_error = process
                    .as_mut()
                    .and_then(|process| process.request_barrier(id).err());
                if let Some(error) = barrier_error {
                    fail_process(&mut process, &shared, error);
                }
            }
            Ok(Command::AcknowledgeSaved {
                changedtick,
                source,
            }) => {
                let acknowledgement_error = process
                    .as_mut()
                    .and_then(|process| process.acknowledge_saved(changedtick, source).err());
                if let Some(error) = acknowledgement_error {
                    fail_process(&mut process, &shared, error);
                }
            }
            Ok(Command::Resize(width, height)) => {
                if let Some(config) = config.as_mut() {
                    config.width = width;
                    config.height = height;
                }
                let resize_error = process
                    .as_mut()
                    .and_then(|process| process.resize(width, height).err());
                if let Some(error) = resize_error {
                    fail_process(&mut process, &shared, error);
                }
            }
            Ok(Command::Stop) => {
                if let Some(mut current) = process.take()
                    && let Err(error) = current.shutdown()
                {
                    push_event(&shared, WorkerEvent::Failed(error));
                }
                config = None;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                sync_reader_state(&executable, &mut process, &mut config, &shared);
            }
        }
    }
    if let Some(mut process) = process {
        let _ = process.shutdown();
    }
}

fn spawn(executable: &Path, config: &SessionConfig) -> Result<RpcProcess, String> {
    RpcProcess::start_with_cancellation(
        executable,
        &config.source,
        &config.synthetic_name,
        &config.language,
        config.width,
        config.height,
        Arc::clone(&config.cancellation),
    )
}

fn run_and_sync(
    executable: &Path,
    process: &mut Option<RpcProcess>,
    config: &mut Option<SessionConfig>,
    shared: &Arc<Mutex<WorkerShared>>,
    operation: impl FnOnce(&mut RpcProcess) -> Result<(), String>,
) {
    let Some(current) = process.as_mut() else {
        return;
    };
    if let Err(error) = operation(current) {
        fail_process(process, shared, error);
        return;
    }
    let candidate = current.snapshot();
    let _ = handle_snapshot(executable, process, config, shared, candidate);
    sync_reader_state(executable, process, config, shared);
}

fn accept_snapshot(
    process: &RpcProcess,
    config: &mut SessionConfig,
    shared: &Arc<Mutex<WorkerShared>>,
    snapshot: Snapshot,
) -> Result<DocumentUpdate, String> {
    if !process.is_solution_buffer(&snapshot.buffer) {
        return Err("Neovim snapshot targeted a non-solution buffer".into());
    }
    let text = snapshot.text.ok_or("document exceeds host source bounds")?;
    if snapshot.changedtick < config.accepted_changedtick {
        if text == config.source {
            return Ok(DocumentUpdate {
                text,
                mode: snapshot.mode,
                changedtick: config.accepted_changedtick,
            });
        }
        return Err("Neovim snapshot changedtick moved backwards".into());
    }
    if snapshot.changedtick == config.accepted_changedtick && text != config.source {
        return Err("Neovim changed bytes without advancing changedtick".into());
    }
    let update = DocumentUpdate {
        text: text.clone(),
        mode: snapshot.mode,
        changedtick: snapshot.changedtick,
    };
    config.source = text;
    config.accepted_changedtick = snapshot.changedtick;
    shared.lock().expect("Neovim worker lock").latest_document = Some(update.clone());
    Ok(update)
}

fn recover_overflow(
    executable: &Path,
    process: &mut Option<RpcProcess>,
    config: &mut Option<SessionConfig>,
    shared: &Arc<Mutex<WorkerShared>>,
    error: String,
) -> Result<DocumentUpdate, String> {
    let mut last_valid = config
        .clone()
        .ok_or("Neovim overflow recovery has no valid host snapshot")?;
    if let Some(mut invalid) = process.take() {
        let _ = invalid.shutdown();
    }
    last_valid.accepted_changedtick = 0;
    let mut replacement = spawn(executable, &last_valid)
        .map_err(|restart| format!("{error}; cannot restart Neovim: {restart}"))?;
    let snapshot = replacement.snapshot()?;
    if snapshot.text.as_deref() != Some(last_valid.source.as_str()) {
        let _ = replacement.shutdown();
        return Err("Neovim overflow recovery changed source bytes".into());
    }
    let update = accept_snapshot(&replacement, &mut last_valid, shared, snapshot)?;
    *config = Some(last_valid);
    *process = Some(replacement);
    push_event(
        shared,
        WorkerEvent::Warning(format!(
            "{error}; Neovim restarted from the last valid source"
        )),
    );
    Ok(update)
}

fn handle_snapshot(
    executable: &Path,
    process: &mut Option<RpcProcess>,
    config: &mut Option<SessionConfig>,
    shared: &Arc<Mutex<WorkerShared>>,
    candidate: Result<Snapshot, String>,
) -> Option<(DocumentUpdate, bool)> {
    let overflow = match &candidate {
        Ok(snapshot) => snapshot.text.is_none(),
        Err(error) => error.contains("document exceeds"),
    };
    if overflow {
        let reason = candidate
            .err()
            .unwrap_or_else(|| "document exceeds host source bounds".into());
        return match recover_overflow(executable, process, config, shared, reason) {
            Ok(update) => Some((update, true)),
            Err(error) => {
                fail_process(process, shared, error);
                None
            }
        };
    }
    let snapshot = match candidate {
        Ok(snapshot) => snapshot,
        Err(error) => {
            fail_process(process, shared, error);
            return None;
        }
    };
    let result = process
        .as_ref()
        .zip(config.as_mut())
        .ok_or_else(|| "Neovim session is unavailable".to_string())
        .and_then(|(process, config)| accept_snapshot(process, config, shared, snapshot));
    match result {
        Ok(update) => Some((update, false)),
        Err(error) => {
            fail_process(process, shared, error);
            None
        }
    }
}

fn event_changedtick(event: &OriginEvent) -> u64 {
    match event {
        OriginEvent::Dirty { changedtick, .. } => *changedtick,
        OriginEvent::Action { snapshot, .. } | OriginEvent::Barrier { snapshot, .. } => {
            snapshot.changedtick
        }
    }
}

fn sync_reader_state(
    executable: &Path,
    process: &mut Option<RpcProcess>,
    config: &mut Option<SessionConfig>,
    shared: &Arc<Mutex<WorkerShared>>,
) {
    let Some(current) = process.as_ref() else {
        return;
    };
    let reader_shared = current.shared();
    let (grid, events, error) = {
        let mut reader = reader_shared.lock().expect("Neovim reader lock");
        (
            reader.latest_grid.take(),
            std::mem::take(&mut reader.origin_events),
            reader.error.take(),
        )
    };
    if let Some(grid) = grid {
        shared.lock().expect("Neovim worker lock").latest_grid = Some(grid);
    }
    if let Some(error) = error {
        fail_process(process, shared, error);
        return;
    }
    let events = events.into_iter().collect::<Vec<_>>();
    for (index, event) in events.iter().cloned().enumerate() {
        let event_tick = event_changedtick(&event);
        if let OriginEvent::Dirty { buffer, .. } = &event {
            let valid_buffer = process
                .as_ref()
                .is_some_and(|process| process.is_solution_buffer(buffer));
            if !valid_buffer {
                fail_process(
                    process,
                    shared,
                    "Neovim dirty event targeted a non-solution buffer".into(),
                );
                return;
            }
            if events[index + 1..]
                .iter()
                .any(|later| event_changedtick(later) >= event_tick)
            {
                continue;
            }
        }
        let candidate = match event.clone() {
            OriginEvent::Dirty { .. } => process
                .as_mut()
                .ok_or_else(|| "Neovim session is unavailable".to_string())
                .and_then(RpcProcess::snapshot),
            OriginEvent::Action { snapshot, .. } | OriginEvent::Barrier { snapshot, .. } => {
                Ok(snapshot)
            }
        };
        let Some((document, recovered)) =
            handle_snapshot(executable, process, config, shared, candidate)
        else {
            return;
        };
        match event {
            OriginEvent::Dirty { .. } => {}
            OriginEvent::Action { action, .. } => match parse_action(&action) {
                Ok(action) => push_event(
                    shared,
                    WorkerEvent::Source(SourceEvent::Action(ActionUpdate { action, document })),
                ),
                Err(error) => {
                    fail_process(process, shared, error);
                    return;
                }
            },
            OriginEvent::Barrier { id, .. } => push_event(
                shared,
                WorkerEvent::Source(SourceEvent::Barrier(BarrierUpdate { id, document })),
            ),
        }
        if recovered {
            break;
        }
    }
}

fn sync_document(
    process: &mut RpcProcess,
    config: &mut SessionConfig,
    shared: &Arc<Mutex<WorkerShared>>,
) -> Result<DocumentUpdate, String> {
    let snapshot = process.snapshot()?;
    accept_snapshot(process, config, shared, snapshot)
}

fn parse_action(action: &str) -> Result<TutorAction, String> {
    match action {
        "test" => Ok(TutorAction::Test),
        "submit" => Ok(TutorAction::Submit),
        "back" => Ok(TutorAction::Back),
        "collapse" => Ok(TutorAction::Collapse),
        "quit" => Ok(TutorAction::Quit),
        "hint" => Ok(TutorAction::Hint),
        _ => Err("Neovim emitted an unknown Tutor action".into()),
    }
}

fn fail_process(
    process: &mut Option<RpcProcess>,
    shared: &Arc<Mutex<WorkerShared>>,
    error: String,
) {
    if let Some(mut process) = process.take() {
        let stderr = process.stderr_tail();
        let _ = process.shutdown();
        let error = if stderr.trim().is_empty() {
            error
        } else {
            format!(
                "{error}; Neovim stderr: {}",
                stderr.chars().take(512).collect::<String>()
            )
        };
        push_event(shared, WorkerEvent::Failed(error));
    } else {
        push_event(shared, WorkerEvent::Failed(error));
    }
}

fn push_event(shared: &Arc<Mutex<WorkerShared>>, event: WorkerEvent) {
    let mut shared = shared.lock().expect("Neovim worker lock");
    if shared.events.len() == EVENT_CAPACITY {
        shared.events.pop_front();
        shared.events.push_back(WorkerEvent::Failed(
            "Neovim event queue exceeded bound".into(),
        ));
    } else {
        shared.events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn wait_until_started(worker: &Worker, deadline: Instant) {
        loop {
            let poll = worker.poll();
            for event in poll.events {
                match event {
                    WorkerEvent::Started(started) => {
                        started.unwrap();
                        return;
                    }
                    WorkerEvent::Failed(error) => panic!("Neovim failed during startup: {error}"),
                    WorkerEvent::Source(_) | WorkerEvent::Warning(_) => {}
                }
            }
            assert!(Instant::now() < deadline, "Neovim startup timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn out_of_band_shutdown_ignores_a_full_queue_and_cancels_startup() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "interview-worker-shutdown-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let marker = root.join("embedded-started");
        let executable = root.join("fake-nvim");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nif [ \"${{1-}}\" = --version ]; then printf 'NVIM v0.11.5\\n'; exit 0; fi\nprintf started > '{}'\nsleep 30\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let worker = Worker::start(executable);
        worker
            .start_session(
                "x\n".into(),
                "interview://cancel.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "fake Neovim did not start");
            thread::sleep(Duration::from_millis(10));
        }
        let mut queue_full = false;
        for _ in 0..=COMMAND_CAPACITY {
            if worker.input("x".into()).is_err() {
                queue_full = true;
                break;
            }
        }
        assert!(queue_full, "test did not fill the worker command queue");
        let started = Instant::now();
        worker.shutdown();
        assert!(started.elapsed() < Duration::from_millis(1500));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_candidate_restarts_from_exact_last_valid_document() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        let source = "x".repeat(crate::editor::MAX_DOCUMENT_BYTES - 1);
        let mut host_document = crate::editor::EditorDocument::new(source.clone()).unwrap();
        let saved_revision = host_document.revision;
        worker
            .start_session(
                source.clone(),
                "interview://overflow.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        wait_until_started(&worker, deadline);

        worker.paste("zz".into()).unwrap();
        let mut warning = None;
        let mut restored = None;
        while Instant::now() < deadline && (warning.is_none() || restored.is_none()) {
            let poll = worker.poll();
            if let Some(document) = poll.document {
                restored = Some(document.text);
            }
            for event in poll.events {
                match event {
                    WorkerEvent::Warning(message) => warning = Some(message),
                    WorkerEvent::Failed(error) => {
                        panic!("overflow recovery must not fail the editor: {error}")
                    }
                    WorkerEvent::Started(_) | WorkerEvent::Source(_) => {}
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            warning
                .as_deref()
                .is_some_and(|message| message.contains("restarted from the last valid source")),
            "overflow recovery warning was not published"
        );
        assert_eq!(restored.as_deref(), Some(source.as_str()));
        host_document
            .install_neovim_snapshot(restored.unwrap(), "n", 1)
            .unwrap();
        assert_eq!(host_document.text(), source);
        assert_eq!(host_document.revision, saved_revision);
        assert_eq!(host_document.saved_text(), source);
        assert!(!host_document.dirty());
        worker.shutdown();
    }

    #[test]
    fn oversized_external_paste_is_rejected_before_queueing_without_mutation() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        let source = "saved bytes\n".to_string();
        let mut host_document = crate::editor::EditorDocument::new(source.clone()).unwrap();
        let saved_revision = host_document.revision;
        worker
            .start_session(
                source.clone(),
                "interview://external-paste.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        wait_until_started(&worker, deadline);

        assert!(
            worker
                .paste("z".repeat(crate::editor::MAX_DOCUMENT_BYTES + 1))
                .unwrap_err()
                .contains("byte bound")
        );
        assert!(
            worker
                .paste("\n".repeat(crate::editor::MAX_DOCUMENT_LINES))
                .unwrap_err()
                .contains("line bound")
        );
        worker.barrier(7).unwrap();

        let barrier = loop {
            let poll = worker.poll();
            let mut barrier = None;
            for event in poll.events {
                match event {
                    WorkerEvent::Source(SourceEvent::Barrier(update)) => barrier = Some(update),
                    WorkerEvent::Failed(error) => panic!("paste rejection killed Neovim: {error}"),
                    WorkerEvent::Started(_)
                    | WorkerEvent::Source(SourceEvent::Action(_))
                    | WorkerEvent::Warning(_) => {}
                }
            }
            if let Some(barrier) = barrier {
                break barrier;
            }
            assert!(
                Instant::now() < deadline,
                "post-rejection barrier timed out"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(barrier.id, 7);
        host_document
            .install_neovim_snapshot(
                barrier.document.text,
                &barrier.document.mode,
                barrier.document.changedtick,
            )
            .unwrap();
        assert_eq!(host_document.text(), source);
        assert_eq!(host_document.revision, saved_revision);
        assert_eq!(host_document.saved_text(), source);
        assert!(!host_document.dirty());
        worker.shutdown();
    }

    #[test]
    fn poll_preserves_source_barrier_before_following_failure() {
        let shared = Arc::new(Mutex::new(WorkerShared::default()));
        let document = DocumentUpdate {
            text: "processed\n".into(),
            mode: "n".into(),
            changedtick: 3,
        };
        push_event(
            &shared,
            WorkerEvent::Source(SourceEvent::Barrier(BarrierUpdate { id: 9, document })),
        );
        push_event(&shared, WorkerEvent::Failed("crashed after barrier".into()));

        let poll = take_poll_result(&shared);
        assert!(matches!(
            poll.events.as_slice(),
            [
                WorkerEvent::Source(SourceEvent::Barrier(BarrierUpdate { id: 9, .. })),
                WorkerEvent::Failed(error)
            ] if error == "crashed after barrier"
        ));

        let mut host_document = crate::editor::EditorDocument::new("saved\n".into()).unwrap();
        let saved_revision = host_document.revision;
        for event in poll.events {
            match event {
                WorkerEvent::Source(SourceEvent::Barrier(update)) => host_document
                    .install_neovim_snapshot(
                        update.document.text,
                        &update.document.mode,
                        update.document.changedtick,
                    )
                    .unwrap(),
                WorkerEvent::Failed(_) => {}
                WorkerEvent::Started(_)
                | WorkerEvent::Source(SourceEvent::Action(_))
                | WorkerEvent::Warning(_) => unreachable!(),
            }
        }
        assert_eq!(host_document.text(), "processed\n");
        assert_eq!(host_document.revision, saved_revision + 1);
        assert_eq!(host_document.saved_text(), "saved\n");
        assert!(host_document.dirty());
    }

    #[test]
    fn serialized_barrier_captures_immediately_preceding_input() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        worker
            .start_session(
                "x\n".into(),
                "interview://barrier.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        wait_until_started(&worker, deadline);
        worker.input("iZ".into()).unwrap();
        worker.barrier(42).unwrap();
        worker.input("!<Esc>".into()).unwrap();
        let mut barrier = None;
        let mut latest_text = None;
        while barrier.is_none() || latest_text.as_deref() != Some("Z!x\n") {
            let poll = worker.poll();
            if let Some(document) = poll.document {
                latest_text = Some(document.text);
            }
            for event in poll.events {
                match event {
                    WorkerEvent::Source(SourceEvent::Barrier(update)) => barrier = Some(update),
                    WorkerEvent::Failed(error) => panic!("source barrier failed: {error}"),
                    WorkerEvent::Started(_)
                    | WorkerEvent::Source(SourceEvent::Action(_))
                    | WorkerEvent::Warning(_) => {}
                }
            }
            assert!(Instant::now() < deadline, "source barrier timed out");
            thread::sleep(Duration::from_millis(10));
        }
        let barrier = barrier.unwrap();
        assert_eq!(barrier.id, 42);
        assert_eq!(barrier.document.text, "Zx\n");
        assert_eq!(latest_text.as_deref(), Some("Z!x\n"));
        worker.shutdown();
    }

    #[test]
    fn tutor_action_snapshot_is_not_replaced_by_a_later_edit() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        worker
            .start_session(
                "x\n".into(),
                "interview://action.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        wait_until_started(&worker, deadline);
        worker.input("iZ<Esc><Space>tA!<Esc>".into()).unwrap();
        let mut action = None;
        let mut latest_text = None;
        while action.is_none() || latest_text.as_deref() != Some("Zx!\n") {
            let poll = worker.poll();
            if let Some(document) = poll.document {
                latest_text = Some(document.text);
            }
            for event in poll.events {
                match event {
                    WorkerEvent::Source(SourceEvent::Action(update)) => action = Some(update),
                    WorkerEvent::Failed(error) => panic!("Tutor action failed: {error}"),
                    WorkerEvent::Started(_)
                    | WorkerEvent::Source(SourceEvent::Barrier(_))
                    | WorkerEvent::Warning(_) => {}
                }
            }
            assert!(Instant::now() < deadline, "Tutor action timed out");
            thread::sleep(Duration::from_millis(10));
        }
        let action = action.unwrap();
        assert_eq!(action.action, TutorAction::Test);
        assert_eq!(action.document.text, "Zx\n");
        assert_eq!(latest_text.as_deref(), Some("Zx!\n"));
        worker.shutdown();
    }

    #[test]
    fn oversized_normal_input_uses_the_same_restart_recovery() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        let source = "x".repeat(crate::editor::MAX_DOCUMENT_BYTES);
        let mut host_document = crate::editor::EditorDocument::new(source.clone()).unwrap();
        let saved_revision = host_document.revision;
        worker
            .start_session(
                source.clone(),
                "interview://normal-overflow.py".into(),
                "python".into(),
                20,
                8,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        wait_until_started(&worker, deadline);
        worker.input("iZ<Esc>".into()).unwrap();
        let mut warning = None;
        let mut restored = None;
        while Instant::now() < deadline && warning.is_none() {
            let poll = worker.poll();
            restored = poll.document.map(|document| document.text).or(restored);
            for event in poll.events {
                match event {
                    WorkerEvent::Warning(message) => warning = Some(message),
                    WorkerEvent::Failed(error) => {
                        panic!("normal input overflow must recover: {error}")
                    }
                    WorkerEvent::Started(_) | WorkerEvent::Source(_) => {}
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(warning.is_some(), "normal input overflow warning missing");
        assert_eq!(restored.as_deref(), Some(source.as_str()));
        host_document
            .install_neovim_snapshot(restored.unwrap(), "n", 1)
            .unwrap();
        assert_eq!(host_document.text(), source);
        assert_eq!(host_document.revision, saved_revision);
        assert_eq!(host_document.saved_text(), source);
        assert!(!host_document.dirty());
        worker.shutdown();
    }
}
