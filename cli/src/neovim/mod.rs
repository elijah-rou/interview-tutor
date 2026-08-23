pub mod grid;
pub mod key;
mod msgpack;
mod process;

use grid::GridSnapshot;
use process::RpcProcess;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub use process::resolve_executable;

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 16;

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
enum WorkerEvent {
    Started(Result<(), String>),
    Action(TutorAction),
    Warning(String),
    Failed(String),
}

#[derive(Default)]
pub struct PollResult {
    pub grid: Option<Arc<GridSnapshot>>,
    pub document: Option<DocumentUpdate>,
    pub started: Option<Result<(), String>>,
    pub actions: Vec<TutorAction>,
    pub warning: Option<String>,
    pub failure: Option<String>,
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
    Stop,
    Shutdown,
}

#[derive(Clone)]
struct SessionConfig {
    source: String,
    synthetic_name: String,
    language: String,
    width: u16,
    height: u16,
}

pub struct Worker {
    sender: Option<SyncSender<Command>>,
    shared: Arc<Mutex<WorkerShared>>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(executable: PathBuf) -> Self {
        let (sender, commands) = mpsc::sync_channel(COMMAND_CAPACITY);
        let shared = Arc::new(Mutex::new(WorkerShared::default()));
        let thread_shared = Arc::clone(&shared);
        let join = thread::spawn(move || controller(executable, commands, thread_shared));
        Self {
            sender: Some(sender),
            shared,
            join: Some(join),
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
        let mut shared = self.shared.lock().expect("Neovim worker lock");
        let mut result = PollResult {
            grid: shared.latest_grid.take(),
            document: shared.latest_document.take(),
            ..PollResult::default()
        };
        while let Some(event) = shared.events.pop_front() {
            match event {
                WorkerEvent::Started(started) => result.started = Some(started),
                WorkerEvent::Action(action) => result.actions.push(action),
                WorkerEvent::Warning(error) => result.warning = Some(error),
                WorkerEvent::Failed(error) => result.failure = Some(error),
            }
        }
        result
    }

    pub fn shutdown(mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(Command::Shutdown);
            drop(sender);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(Command::Shutdown);
            drop(sender);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn controller(
    executable: PathBuf,
    commands: mpsc::Receiver<Command>,
    shared: Arc<Mutex<WorkerShared>>,
) {
    let mut process: Option<RpcProcess> = None;
    let mut config: Option<SessionConfig> = None;
    loop {
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
                let next_config = SessionConfig {
                    source,
                    synthetic_name,
                    language,
                    width,
                    height,
                };
                match spawn(&executable, &next_config) {
                    Ok(mut next) => {
                        let result = sync_document(&mut next, &shared);
                        process = Some(next);
                        config = Some(next_config);
                        push_event(&shared, WorkerEvent::Started(result.map(|_| ())))
                    }
                    Err(error) => {
                        config = None;
                        push_event(&shared, WorkerEvent::Started(Err(error)))
                    }
                }
            }
            Ok(Command::Input(encoded)) => {
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
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
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
    RpcProcess::start(
        executable,
        &config.source,
        &config.synthetic_name,
        &config.language,
        config.width,
        config.height,
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
    match sync_document(current, shared) {
        Ok(update) => {
            if let Some(config) = config.as_mut() {
                config.source = update.text;
            }
            sync_reader_state(executable, process, config, shared);
        }
        Err(error) if error.contains("document exceeds") => {
            let Some(last_valid) = config.clone() else {
                fail_process(process, shared, error);
                return;
            };
            if let Some(mut invalid) = process.take() {
                let _ = invalid.shutdown();
            }
            match spawn(executable, &last_valid) {
                Ok(mut replacement) => {
                    let restored = sync_document(&mut replacement, shared).and_then(|update| {
                        (update.text == last_valid.source)
                            .then_some(())
                            .ok_or("Neovim overflow recovery changed source bytes".into())
                    });
                    process.replace(replacement);
                    match restored {
                        Ok(()) => push_event(
                            shared,
                            WorkerEvent::Warning(format!(
                                "{error}; Neovim restarted from the last valid source"
                            )),
                        ),
                        Err(restart_error) => fail_process(process, shared, restart_error),
                    }
                }
                Err(restart_error) => fail_process(
                    process,
                    shared,
                    format!("{error}; cannot restart Neovim: {restart_error}"),
                ),
            }
        }
        Err(error) => fail_process(process, shared, error),
    }
}

fn sync_reader_state(
    _executable: &Path,
    process: &mut Option<RpcProcess>,
    config: &mut Option<SessionConfig>,
    shared: &Arc<Mutex<WorkerShared>>,
) {
    let Some(current) = process.as_mut() else {
        return;
    };
    let reader_shared = current.shared();
    let (grid, dirty, actions, error) = {
        let mut reader = reader_shared.lock().expect("Neovim reader lock");
        (
            reader.latest_grid.take(),
            std::mem::take(&mut reader.document_dirty),
            std::mem::take(&mut reader.pending_actions),
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
    if dirty || !actions.is_empty() {
        match sync_document(current, shared) {
            Ok(update) => {
                if let Some(config) = config.as_mut() {
                    config.source = update.text;
                }
            }
            Err(error) => {
                fail_process(process, shared, error);
                return;
            }
        }
    }
    for action in actions {
        match parse_action(&action) {
            Ok(action) => push_event(shared, WorkerEvent::Action(action)),
            Err(error) => {
                fail_process(process, shared, error);
                return;
            }
        }
    }
}

fn sync_document(
    process: &mut RpcProcess,
    shared: &Arc<Mutex<WorkerShared>>,
) -> Result<DocumentUpdate, String> {
    let (text, mode, changedtick) = process.snapshot()?;
    let update = DocumentUpdate {
        text,
        mode,
        changedtick,
    };
    shared.lock().expect("Neovim worker lock").latest_document = Some(update.clone());
    Ok(update)
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
    use std::time::{Duration, Instant};

    #[test]
    fn oversized_candidate_restarts_from_exact_last_valid_document() {
        let executable = resolve_executable(None).unwrap();
        let worker = Worker::start(executable);
        let source = "x".repeat(crate::editor::MAX_DOCUMENT_BYTES - 1);
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
        loop {
            let poll = worker.poll();
            if let Some(started) = poll.started {
                started.unwrap();
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Neovim did not start before deadline"
            );
            thread::sleep(Duration::from_millis(10));
        }

        worker.paste("zz".into()).unwrap();
        let mut warning = None;
        let mut restored = None;
        while Instant::now() < deadline && (warning.is_none() || restored.is_none()) {
            let poll = worker.poll();
            warning = warning.or(poll.warning);
            if let Some(document) = poll.document {
                restored = Some(document.text);
            }
            assert!(
                poll.failure.is_none(),
                "overflow recovery must not fail the editor"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            warning
                .as_deref()
                .is_some_and(|message| message.contains("restarted from the last valid source")),
            "overflow recovery warning was not published"
        );
        assert_eq!(restored.as_deref(), Some(source.as_str()));
        worker.shutdown();
    }
}
