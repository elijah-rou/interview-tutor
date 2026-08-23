use crate::app::model::{SolvePane, SolveSession};
use crate::app::{AppState, Effect, Event, LoadScope, Repository, reduce};
use crate::runner::{self, CancellationToken, ExecutionLimits};
use crate::signals::{ScopedSignalHandlers, SignalState};
use crate::source;
use crate::tui::{input, render};
use crossterm::cursor::{Hide, SetCursorStyle, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as TerminalEvent,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const NEOVIM_SOURCE_BARRIER_TIMEOUT: Duration = Duration::from_secs(3);

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("cannot enable terminal raw mode: {e}"))?;
        if let Err(e) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            Hide,
            EnableMouseCapture,
            EnableBracketedPaste
        ) {
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                DisableBracketedPaste,
                DisableMouseCapture,
                SetCursorStyle::DefaultUserShape,
                Show,
                LeaveAlternateScreen
            );
            return Err(format!("cannot enter terminal screen: {e}"));
        }
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            SetCursorStyle::DefaultUserShape,
            Show,
            LeaveAlternateScreen
        );
    }
}

enum WorkerCommand {
    Run {
        operation: crate::app::model::OperationId,
        revision: u64,
        intent: crate::app::RunIntent,
        plan: Box<runner::ExecutionPlan>,
        source: String,
        write_source: bool,
        cancellation: RunCancellation,
    },
    Shutdown,
}
type SaveService = dyn Fn(&Path, &Path, &str) -> Result<(), String> + Send + Sync;
type ExecuteService = dyn Fn(&runner::ExecutionPlan, &CancellationToken) -> Result<runner::ExecutionResult, String>
    + Send
    + Sync;
type RecordService =
    dyn Fn(&runner::ExecutionPlan, &runner::ExecutionResult) -> Result<i64, String> + Send + Sync;
type FinalizeCancelledService = dyn Fn(i64, i32) -> Result<(), String> + Send + Sync;

#[derive(Default)]
struct RunCancellationState {
    exit_code: Option<i32>,
    completed: bool,
}

#[derive(Clone, Default)]
struct RunCancellation {
    token: CancellationToken,
    state: Arc<std::sync::Mutex<RunCancellationState>>,
}

impl RunCancellation {
    fn with_signal_state(signal_state: Option<SignalState>) -> Self {
        let token =
            signal_state.map_or_else(CancellationToken::new, CancellationToken::with_signal_state);
        Self {
            token,
            state: Arc::new(std::sync::Mutex::new(RunCancellationState::default())),
        }
    }

    fn cancel(&self, exit_code: i32) {
        assert!(matches!(exit_code, 130 | 143));
        let mut state = self.state.lock().expect("run cancellation lock");
        if state.completed {
            return;
        }
        state.exit_code.get_or_insert(exit_code);
        self.token.cancel();
    }

    fn finish(&self) -> Option<i32> {
        let mut state = self.state.lock().expect("run cancellation lock");
        if state.completed {
            return state.exit_code;
        }
        if let Some(exit_code) = self.token.signal_exit_code() {
            state.exit_code.get_or_insert(exit_code);
        }
        state.completed = true;
        state.exit_code
    }

    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

#[derive(Clone)]
struct RunnerServices {
    save: Arc<SaveService>,
    execute: Arc<ExecuteService>,
    record: Arc<RecordService>,
    finalize_cancelled: Arc<FinalizeCancelledService>,
}

enum InterviewerWorkerCommand {
    Connect {
        operation: crate::app::model::OperationId,
        generation: u64,
        cancellation: CancellationToken,
    },
    Turn {
        operation: crate::app::model::OperationId,
        generation: u64,
        revision: u64,
        mode: crate::interviewer::Mode,
        statement: String,
        source: String,
        output: String,
        question: String,
        solved: bool,
        cancellation: CancellationToken,
    },
    Reset,
    Cancel {
        operation: crate::app::model::OperationId,
    },
    #[cfg(test)]
    Panic,
    Shutdown,
}

struct InterviewerTurnRequest {
    revision: u64,
    mode: crate::interviewer::Mode,
    statement: String,
    source: String,
    output: String,
    question: String,
    solved: bool,
}

trait InterviewerWorkerBackend: Send {
    fn backend(&self) -> crate::interviewer::Backend;

    fn connect(
        &mut self,
        control_pid: Arc<std::sync::atomic::AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<(), crate::interviewer::InterviewerError>;
    fn turn(
        &mut self,
        request: &InterviewerTurnRequest,
        cancellation: &CancellationToken,
    ) -> Result<String, crate::interviewer::InterviewerError>;
    fn commit(&mut self, pending: PendingInterviewerResponse);
    fn reset(&mut self);
}

struct SessionInterviewerBackend {
    backend: crate::interviewer::Backend,
    session: Option<crate::interviewer::InterviewerSession>,
}

impl InterviewerWorkerBackend for SessionInterviewerBackend {
    fn backend(&self) -> crate::interviewer::Backend {
        self.backend
    }

    fn connect(
        &mut self,
        control_pid: Arc<std::sync::atomic::AtomicI32>,
        cancellation: &CancellationToken,
    ) -> Result<(), crate::interviewer::InterviewerError> {
        if let Some(session) = self.session.as_mut() {
            session.prepare_next_operation(cancellation)
        } else {
            self.session = Some(crate::interviewer::InterviewerSession::connect(
                self.backend,
                control_pid,
                cancellation,
            )?);
            Ok(())
        }
    }

    fn turn(
        &mut self,
        request: &InterviewerTurnRequest,
        cancellation: &CancellationToken,
    ) -> Result<String, crate::interviewer::InterviewerError> {
        let session = self.session.as_mut().ok_or_else(|| {
            crate::interviewer::InterviewerError::transport(
                self.backend,
                "Interviewer is not connected",
            )
        })?;
        session.ask_deferred_with_cancellation(
            crate::interviewer::InterviewRequest {
                mode: request.mode,
                statement: &request.statement,
                source: &request.source,
                latest_output: &request.output,
                question: &request.question,
                source_revision: request.revision,
                solved: request.solved,
            },
            cancellation,
        )
    }

    fn commit(&mut self, pending: PendingInterviewerResponse) {
        self.session
            .as_mut()
            .expect("successful turn requires session")
            .commit_response(
                pending.mode,
                pending.revision,
                &pending.question,
                pending.response,
            );
    }

    fn reset(&mut self) {
        if self
            .session
            .as_ref()
            .is_some_and(crate::interviewer::InterviewerSession::requires_restart)
        {
            self.session.as_mut().expect("session checked").clear();
        } else {
            self.session = None;
        }
    }
}

struct PendingInterviewerResponse {
    operation: crate::app::model::OperationId,
    revision: u64,
    mode: crate::interviewer::Mode,
    question: String,
    response: String,
}

#[derive(Clone, Copy)]
struct InterviewerTurnFinalization {
    operation: crate::app::model::OperationId,
    revision: u64,
    mode: crate::interviewer::Mode,
    accepted: bool,
}

type InterviewerBackendFactory = dyn Fn() -> Box<dyn InterviewerWorkerBackend> + Send + Sync;

fn catch_interviewer_worker_panic<T>(
    backend: crate::interviewer::Backend,
    operation: impl FnOnce() -> Result<T, crate::interviewer::InterviewerError>,
) -> Result<T, crate::interviewer::InterviewerError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_or_else(|_| {
        Err(crate::interviewer::InterviewerError::transport(
            backend,
            "Interviewer worker panicked",
        ))
    })
}

struct InterviewerWorker {
    enabled: bool,
    sender: Option<SyncSender<InterviewerWorkerCommand>>,
    events: Option<Receiver<Event>>,
    join: Option<JoinHandle<()>>,
    control_pid: Arc<std::sync::atomic::AtomicI32>,
    active_cancellation:
        Arc<std::sync::Mutex<Option<(crate::app::model::OperationId, CancellationToken)>>>,
    pending_response: Arc<std::sync::Mutex<Option<PendingInterviewerResponse>>>,
    finalization: Arc<std::sync::Mutex<Option<InterviewerTurnFinalization>>>,
    cancelled_operation: Arc<std::sync::Mutex<Option<crate::app::model::OperationId>>>,
    reset_generation: Arc<std::sync::atomic::AtomicU64>,
    backend_factory: Arc<InterviewerBackendFactory>,
    #[cfg(test)]
    queued_event_operation: Arc<std::sync::atomic::AtomicU64>,
}
impl InterviewerWorker {
    fn start(backend: crate::interviewer::Backend) -> Self {
        assert!(backend.enabled());
        Self::new(
            Arc::new(move || {
                Box::new(SessionInterviewerBackend {
                    backend,
                    session: None,
                })
            }),
            true,
        )
    }

    fn disabled() -> Self {
        Self::new(
            Arc::new(|| {
                Box::new(SessionInterviewerBackend {
                    backend: crate::interviewer::Backend::Pi,
                    session: None,
                })
            }),
            false,
        )
    }

    #[cfg(test)]
    fn start_with_backend(backend_factory: Arc<InterviewerBackendFactory>) -> Self {
        Self::new(backend_factory, true)
    }

    fn new(backend_factory: Arc<InterviewerBackendFactory>, enabled: bool) -> Self {
        let mut worker = Self {
            enabled,
            sender: None,
            events: None,
            join: None,
            control_pid: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            active_cancellation: Arc::new(std::sync::Mutex::new(None)),
            pending_response: Arc::new(std::sync::Mutex::new(None)),
            finalization: Arc::new(std::sync::Mutex::new(None)),
            cancelled_operation: Arc::new(std::sync::Mutex::new(None)),
            reset_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            backend_factory,
            #[cfg(test)]
            queued_event_operation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };
        if enabled {
            worker.spawn();
        }
        worker
    }

    fn spawn(&mut self) {
        assert!(self.enabled, "disabled Interviewer worker must not spawn");
        assert!(self.join.is_none());
        assert!(self.sender.is_none());
        assert!(self.events.is_none());
        let (sender, commands) = mpsc::sync_channel(2);
        let (event_sender, events) = mpsc::sync_channel(64);
        let thread_control_pid = Arc::clone(&self.control_pid);
        let thread_cancellation = Arc::clone(&self.active_cancellation);
        let thread_pending_response = Arc::clone(&self.pending_response);
        let thread_finalization = Arc::clone(&self.finalization);
        let thread_cancelled_operation = Arc::clone(&self.cancelled_operation);
        let thread_reset_generation = Arc::clone(&self.reset_generation);
        let mut backend = (self.backend_factory)();
        #[cfg(test)]
        let thread_queued_event_operation = Arc::clone(&self.queued_event_operation);
        let join = thread::spawn(move || {
            let mut observed_generation =
                thread_reset_generation.load(std::sync::atomic::Ordering::Acquire);
            while let Ok(command) = commands.recv() {
                let generation = thread_reset_generation.load(std::sync::atomic::Ordering::Acquire);
                if generation != observed_generation {
                    *thread_pending_response
                        .lock()
                        .expect("Interviewer pending response lock") = None;
                    *thread_finalization
                        .lock()
                        .expect("Interviewer finalization lock") = None;
                    backend.reset();
                    observed_generation = generation;
                }
                let cancelled = thread_cancelled_operation
                    .lock()
                    .expect("Interviewer cancelled operation lock")
                    .take();
                if let Some(cancelled) = cancelled {
                    let mut pending = thread_pending_response
                        .lock()
                        .expect("Interviewer pending response lock");
                    if pending
                        .as_ref()
                        .is_some_and(|response| response.operation == cancelled)
                    {
                        *pending = None;
                    }
                }
                let finalization = thread_finalization
                    .lock()
                    .expect("Interviewer finalization lock")
                    .take();
                if let Some(finalization) = finalization {
                    let pending = {
                        let mut pending = thread_pending_response
                            .lock()
                            .expect("Interviewer pending response lock");
                        if pending.as_ref().is_some_and(|response| {
                            (response.operation, response.revision, response.mode)
                                == (
                                    finalization.operation,
                                    finalization.revision,
                                    finalization.mode,
                                )
                        }) {
                            pending.take()
                        } else {
                            None
                        }
                    };
                    if finalization.accepted
                        && let Some(pending) = pending
                    {
                        backend.commit(pending);
                    }
                }
                match command {
                    InterviewerWorkerCommand::Connect {
                        operation,
                        generation,
                        cancellation,
                    } => {
                        let result = if generation == observed_generation {
                            let control_pid = Arc::clone(&thread_control_pid);
                            let backend_kind = backend.backend();
                            catch_interviewer_worker_panic(backend_kind, || {
                                backend.connect(control_pid, &cancellation)
                            })
                        } else {
                            Err(crate::interviewer::InterviewerError::cancelled(
                                backend.backend(),
                                "Interviewer connection discarded after reset",
                            ))
                        };
                        let mut active = thread_cancellation
                            .lock()
                            .expect("Interviewer cancellation lock");
                        if active
                            .as_ref()
                            .is_some_and(|(active_operation, _)| *active_operation == operation)
                        {
                            *active = None;
                        }
                        drop(active);
                        let event = Event::InterviewerConnected(operation, result);
                        if event_sender.send(event).is_err() {
                            break;
                        }
                        #[cfg(test)]
                        thread_queued_event_operation
                            .store(operation.0, std::sync::atomic::Ordering::Release);
                    }
                    InterviewerWorkerCommand::Turn {
                        operation,
                        generation,
                        revision,
                        mode,
                        statement,
                        source,
                        output,
                        question,
                        solved,
                        cancellation,
                    } => {
                        // A prior completion can outlive its UI operation when cancellation and
                        // polling race. It must never block or contaminate a newer turn.
                        *thread_pending_response
                            .lock()
                            .expect("Interviewer pending response lock") = None;
                        let request = InterviewerTurnRequest {
                            revision,
                            mode,
                            statement,
                            source,
                            output,
                            question,
                            solved,
                        };
                        let mut result = if generation == observed_generation {
                            let backend_kind = backend.backend();
                            catch_interviewer_worker_panic(backend_kind, || {
                                backend.turn(&request, &cancellation)
                            })
                        } else {
                            Err(crate::interviewer::InterviewerError::cancelled(
                                backend.backend(),
                                "Interviewer turn discarded after reset",
                            ))
                        };
                        let mut active = thread_cancellation
                            .lock()
                            .expect("Interviewer cancellation lock");
                        if active
                            .as_ref()
                            .is_some_and(|(active_operation, _)| *active_operation == operation)
                        {
                            *active = None;
                        }
                        drop(active);

                        let mut pending = thread_pending_response
                            .lock()
                            .expect("Interviewer pending response lock");
                        let current_generation =
                            thread_reset_generation.load(std::sync::atomic::Ordering::Acquire);
                        let cancelled = thread_cancelled_operation
                            .lock()
                            .expect("Interviewer cancelled operation lock")
                            .as_ref()
                            == Some(&operation);
                        if current_generation != generation || cancelled {
                            result = Err(crate::interviewer::InterviewerError::cancelled(
                                backend.backend(),
                                if cancelled {
                                    "Interviewer operation cancelled"
                                } else {
                                    "Interviewer turn discarded after reset"
                                },
                            ));
                        } else if let Ok(response) = result.as_ref() {
                            *pending = Some(PendingInterviewerResponse {
                                operation,
                                revision,
                                mode,
                                question: request.question,
                                response: response.clone(),
                            });
                        }
                        drop(pending);
                        if event_sender
                            .send(Event::InterviewerFinished(
                                operation, revision, mode, result,
                            ))
                            .is_err()
                        {
                            break;
                        }
                        #[cfg(test)]
                        thread_queued_event_operation
                            .store(operation.0, std::sync::atomic::Ordering::Release);
                    }
                    InterviewerWorkerCommand::Reset => {
                        *thread_pending_response
                            .lock()
                            .expect("Interviewer pending response lock") = None;
                        backend.reset();
                    }
                    InterviewerWorkerCommand::Cancel { .. } => {}
                    #[cfg(test)]
                    InterviewerWorkerCommand::Panic => panic!("injected Interviewer worker panic"),
                    InterviewerWorkerCommand::Shutdown => break,
                }
            }
        });
        self.sender = Some(sender);
        self.events = Some(events);
        self.join = Some(join);
    }
    fn poll(&mut self) -> Vec<Event> {
        let mut result = Vec::new();
        let mut disconnected = false;
        if let Some(events) = self.events.as_ref() {
            loop {
                match events.try_recv() {
                    Ok(event) => result.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.events = None;
            self.sender = None;
            *self
                .active_cancellation
                .lock()
                .expect("Interviewer cancellation lock") = None;
            *self
                .pending_response
                .lock()
                .expect("Interviewer pending response lock") = None;
            *self
                .finalization
                .lock()
                .expect("Interviewer finalization lock") = None;
            let panicked = self.join.take().is_some_and(|join| join.join().is_err());
            self.kill_control_process();
            result.push(Event::InterviewerDisconnected(if panicked {
                "Interviewer worker panicked and disconnected".into()
            } else {
                "Interviewer worker disconnected".into()
            }));
        }
        result
    }

    fn generation(&self) -> u64 {
        self.reset_generation
            .load(std::sync::atomic::Ordering::Acquire)
    }

    fn send(&mut self, command: InterviewerWorkerCommand) -> Result<(), String> {
        if !self.enabled {
            return Err("Interviewer is disabled".into());
        }
        if self.join.is_none() && matches!(&command, InterviewerWorkerCommand::Connect { .. }) {
            self.spawn();
        }
        if self.sender.is_none() {
            return Err("Interviewer worker is stopped".into());
        }
        let replacement = match &command {
            InterviewerWorkerCommand::Connect {
                operation,
                cancellation,
                ..
            }
            | InterviewerWorkerCommand::Turn {
                operation,
                cancellation,
                ..
            } => Some((*operation, cancellation.clone())),
            InterviewerWorkerCommand::Cancel { operation } => {
                *self
                    .cancelled_operation
                    .lock()
                    .expect("Interviewer cancelled operation lock") = Some(*operation);
                let mut pending = self
                    .pending_response
                    .lock()
                    .expect("Interviewer pending response lock");
                if pending
                    .as_ref()
                    .is_some_and(|response| response.operation == *operation)
                {
                    *pending = None;
                }
                drop(pending);
                if let Some((active_operation, cancellation)) = self
                    .active_cancellation
                    .lock()
                    .expect("Interviewer cancellation lock")
                    .as_ref()
                    && active_operation == operation
                {
                    cancellation.cancel();
                }
                None
            }
            InterviewerWorkerCommand::Reset => {
                if let Some((_, cancellation)) = self
                    .active_cancellation
                    .lock()
                    .expect("Interviewer cancellation lock")
                    .as_ref()
                {
                    cancellation.cancel();
                }
                None
            }
            #[cfg(test)]
            InterviewerWorkerCommand::Panic => None,
            InterviewerWorkerCommand::Shutdown => None,
        };
        let previous = replacement.as_ref().map(|replacement| {
            let mut active = self
                .active_cancellation
                .lock()
                .expect("Interviewer cancellation lock");
            let previous = active.take();
            *active = Some(replacement.clone());
            previous
        });
        let sender = self.sender.as_ref().expect("sender checked");
        if let Err(error) = sender.try_send(command) {
            if let (Some((_, replacement)), Some(previous)) = (replacement, previous) {
                let mut active = self
                    .active_cancellation
                    .lock()
                    .expect("Interviewer cancellation lock");
                let replacement_flag = replacement.signal_flag();
                if active.as_ref().is_some_and(|(_, current)| {
                    Arc::ptr_eq(&current.signal_flag(), &replacement_flag)
                }) {
                    *active = previous;
                }
            }
            return Err(format!("Interviewer worker is busy or stopped: {error}"));
        }
        Ok(())
    }

    fn finalize_turn(
        &self,
        operation: crate::app::model::OperationId,
        revision: u64,
        mode: crate::interviewer::Mode,
        accepted: bool,
    ) -> Result<(), String> {
        if !accepted {
            let mut pending = self
                .pending_response
                .lock()
                .expect("Interviewer pending response lock");
            if pending.as_ref().is_some_and(|response| {
                (response.operation, response.revision, response.mode)
                    == (operation, revision, mode)
            }) {
                *pending = None;
            }
            return Ok(());
        }
        if self.join.is_none() {
            return Err("Interviewer worker stopped before finalizing the turn".into());
        }
        let mut finalization = self
            .finalization
            .lock()
            .expect("Interviewer finalization lock");
        if finalization.is_some() {
            return Err("Interviewer worker has an unconsumed turn finalization".into());
        }
        *finalization = Some(InterviewerTurnFinalization {
            operation,
            revision,
            mode,
            accepted,
        });
        Ok(())
    }

    fn reset(&mut self) {
        let previous = self
            .reset_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        assert_ne!(previous, u64::MAX, "Interviewer reset generation exhausted");
        if let Some((_, cancellation)) = self
            .active_cancellation
            .lock()
            .expect("Interviewer cancellation lock")
            .as_ref()
        {
            cancellation.cancel();
        }
        *self
            .pending_response
            .lock()
            .expect("Interviewer pending response lock") = None;
        *self
            .finalization
            .lock()
            .expect("Interviewer finalization lock") = None;
        *self
            .cancelled_operation
            .lock()
            .expect("Interviewer cancelled operation lock") = None;
        if let Some(sender) = self.sender.as_ref() {
            let _ = sender.try_send(InterviewerWorkerCommand::Reset);
        }
    }

    fn kill_control_process(&self) {
        let pid = self
            .control_pid
            .swap(0, std::sync::atomic::Ordering::SeqCst);
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }

    fn shutdown(mut self) {
        self.reset();
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(InterviewerWorkerCommand::Shutdown);
            drop(sender);
        }
        self.kill_control_process();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn runtime_execution_limits() -> ExecutionLimits {
    let limits = ExecutionLimits::default();
    #[cfg(debug_assertions)]
    if let Some(milliseconds) = std::env::var_os("INTERVIEW_TUTOR_TEST_RUN_TIMEOUT_MS") {
        let milliseconds = milliseconds
            .to_str()
            .expect("test runner timeout must be UTF-8")
            .parse::<u64>()
            .expect("test runner timeout must be an integer");
        assert!((50..=20_000).contains(&milliseconds));
        return ExecutionLimits {
            wall_timeout: Duration::from_millis(milliseconds),
            term_grace: Duration::from_millis(100),
            ..limits
        };
    }
    limits
}

struct RunnerWorker {
    sender: SyncSender<WorkerCommand>,
    events: Receiver<Event>,
    join: Option<JoinHandle<()>>,
    signal_state: Option<SignalState>,
    active: Option<(
        crate::app::model::OperationId,
        u64,
        crate::app::RunIntent,
        RunCancellation,
    )>,
}
impl RunnerWorker {
    fn start(root: PathBuf, database_path: PathBuf, signal_state: SignalState) -> Self {
        let save_root = root.clone();
        let execute_database = database_path.clone();
        let execution_limits = runtime_execution_limits();
        let record_root = root;
        let finalize_root = record_root.clone();
        let record_database = database_path;
        let finalize_database = record_database.clone();
        Self::start_with_services_and_signal(
            RunnerServices {
                save: Arc::new(move |_, solution_path, source| {
                    source::atomic_save(&save_root, solution_path, source)
                }),
                execute: Arc::new(move |plan, cancellation| {
                    runner::execute(
                        plan,
                        &execute_database,
                        &execution_limits,
                        cancellation,
                        None,
                    )
                }),
                record: Arc::new(move |plan, result| {
                    let connection =
                        crate::database::open_database(&record_database, &record_root)?;
                    runner::record_execution(&connection, plan, result)
                }),
                finalize_cancelled: Arc::new(move |attempt_id, exit_code| {
                    let connection =
                        crate::database::open_database(&finalize_database, &finalize_root)?;
                    crate::database::finalize_attempt_cancelled(&connection, attempt_id, exit_code)
                }),
            },
            Some(signal_state),
        )
    }

    #[cfg(test)]
    fn start_with_services(services: RunnerServices) -> Self {
        Self::start_with_services_and_signal(services, None)
    }

    fn start_with_services_and_signal(
        services: RunnerServices,
        signal_state: Option<SignalState>,
    ) -> Self {
        let (sender, commands) = mpsc::sync_channel(2);
        let (event_sender, events) = mpsc::sync_channel(64);
        let join = thread::spawn(move || {
            while let Ok(command) = commands.recv() {
                match command {
                    WorkerCommand::Shutdown => break,
                    WorkerCommand::Run {
                        operation,
                        revision,
                        intent,
                        plan,
                        source,
                        write_source,
                        cancellation,
                    } => {
                        let source_for_save = source.clone();
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let save_result = if write_source {
                                    (services.save)(&plan.root, &plan.solution_path, &source)
                                } else {
                                    Ok(())
                                };
                                match save_result {
                                    Err(error) => (None, Err(error)),
                                    Ok(()) => (
                                        Some(source_for_save),
                                        (|| {
                                            let mut result =
                                                (services.execute)(&plan, &cancellation.token)?;
                                            if cancellation.is_cancelled() {
                                                result.termination = runner::Termination::Cancelled;
                                            }
                                            if intent == crate::app::RunIntent::Submit {
                                                let attempt_id = (services.record)(&plan, &result)?;
                                                assert!(attempt_id > 0);
                                                // This immediate post-record signal observation is
                                                // the completion cutoff. Signals seen here rewrite
                                                // this exact attempt before any completion event is
                                                // published; later signals apply to runtime teardown.
                                                if let Some(exit_code) = cancellation.finish() {
                                                    (services.finalize_cancelled)(
                                                        attempt_id, exit_code,
                                                    )?;
                                                    result.termination =
                                                        runner::Termination::Cancelled;
                                                }
                                            } else if cancellation.finish().is_some() {
                                                result.termination = runner::Termination::Cancelled;
                                            }
                                            Ok(result)
                                        })(),
                                    ),
                                }
                            }));
                        let (saved, result) = outcome
                            .unwrap_or_else(|_| (None, Err("runner worker panicked".into())));
                        if result.is_err() {
                            cancellation.finish();
                        }
                        if event_sender
                            .send(Event::RunFinished(
                                operation, revision, intent, saved, result,
                            ))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            sender,
            events,
            join: Some(join),
            signal_state,
            active: None,
        }
    }
    fn run(
        &mut self,
        operation: crate::app::model::OperationId,
        revision: u64,
        intent: crate::app::RunIntent,
        plan: runner::ExecutionPlan,
        source: String,
        write_source: bool,
    ) -> Result<(), String> {
        let cancellation = RunCancellation::with_signal_state(self.signal_state.clone());
        self.sender
            .send(WorkerCommand::Run {
                operation,
                revision,
                intent,
                plan: Box::new(plan),
                source,
                write_source,
                cancellation: cancellation.clone(),
            })
            .map_err(|_| "runner worker stopped".to_string())?;
        self.active = Some((operation, revision, intent, cancellation));
        Ok(())
    }
    fn cancel(&mut self, operation: crate::app::model::OperationId) {
        if let Some((active, _, _, cancellation)) = &self.active
            && *active == operation
        {
            cancellation.cancel(130);
        }
    }
    fn interrupt_active(&mut self, exit_code: i32) {
        if let Some((_, _, _, cancellation)) = &self.active {
            cancellation.cancel(exit_code);
        }
    }
    fn leave(&mut self) {
        if let Some((_, _, _, cancellation)) = &self.active {
            cancellation.cancel(130);
        }
        if self.active.is_some() {
            let _ = self.events.recv();
            self.active = None;
        }
    }
    fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    self.active = None;
                    events.push(event);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some((operation, revision, intent, _)) = self.active.take() {
                        events.push(Event::RunFinished(
                            operation,
                            revision,
                            intent,
                            None,
                            Err("runner worker disconnected".into()),
                        ));
                    }
                    if let Some(join) = self.join.take() {
                        let _ = join.join();
                    }
                    break;
                }
            }
        }
        events
    }
    fn shutdown(mut self) {
        if let Some((_, _, _, cancellation)) = &self.active {
            cancellation.cancel(130);
        }
        let _ = self.sender.try_send(WorkerCommand::Shutdown);
        let join = self.join.take();
        drop(self.sender);
        if let Some(join) = join {
            let _ = join.join();
        }
    }
}

struct RuntimeWorkers {
    runner: Option<RunnerWorker>,
    interviewer: Option<InterviewerWorker>,
    neovim: Option<crate::neovim::Worker>,
}

impl RuntimeWorkers {
    fn start(
        root: PathBuf,
        database_path: PathBuf,
        interviewer_backend: crate::interviewer::Backend,
        signal_state: SignalState,
        neovim_executable: PathBuf,
    ) -> Self {
        let mut workers = Self {
            runner: Some(RunnerWorker::start(root, database_path, signal_state)),
            interviewer: None,
            neovim: Some(crate::neovim::Worker::start(neovim_executable)),
        };
        workers.interviewer = Some(if interviewer_backend.enabled() {
            InterviewerWorker::start(interviewer_backend)
        } else {
            InterviewerWorker::disabled()
        });
        workers
    }

    fn parts(
        &mut self,
    ) -> (
        &mut RunnerWorker,
        &mut InterviewerWorker,
        &mut crate::neovim::Worker,
    ) {
        (
            self.runner.as_mut().expect("runtime runner exists"),
            self.interviewer
                .as_mut()
                .expect("runtime Interviewer worker exists"),
            self.neovim.as_mut().expect("runtime Neovim worker exists"),
        )
    }

    fn shutdown(&mut self) {
        if let Some(worker) = self.runner.take() {
            worker.shutdown();
        }
        if let Some(worker) = self.interviewer.take() {
            worker.shutdown();
        }
        if let Some(worker) = self.neovim.take() {
            worker.shutdown();
        }
    }
}

impl Drop for RuntimeWorkers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn apply_effects(
    state: &mut AppState,
    repository: &Repository,
    root: &Path,
    worker: &mut RunnerWorker,
    interviewer_worker: &mut InterviewerWorker,
    neovim_worker: &mut crate::neovim::Worker,
    mut effects: Vec<Effect>,
) {
    while let Some(effect) = effects.pop() {
        match effect {
            Effect::Load {
                operation,
                scope,
                problem_id,
                language_slug,
            } => {
                let set = match &scope {
                    LoadScope::Global => None,
                    LoadScope::ProblemSet(slug) => Some(slug.as_str()),
                };
                let result = repository
                    .load(set, problem_id, &language_slug)
                    .map(Box::new);
                effects.extend(reduce(state, Event::Loaded(operation, result)));
            }
            Effect::OpenSolve {
                operation,
                problem_slug,
                set_slug,
                language_slug,
            } => {
                let result = (|| {
                    let plan = repository.prepare_execution(
                        root,
                        &problem_slug,
                        set_slug.as_deref(),
                        &language_slug,
                    )?;
                    let editor = source::load(root, &plan.solution_path)?;
                    let detail = state
                        .data
                        .detail
                        .as_ref()
                        .ok_or("problem detail unavailable")?;
                    Ok(Box::new(SolveSession {
                        problem_id: detail.id,
                        problem_slug: detail.slug.clone(),
                        problem_title: detail.title.clone(),
                        statement: detail.statement_markdown.clone(),
                        language: language_slug,
                        plan,
                        editor,
                        editor_view: None,
                        editor_status: crate::app::model::EditorRuntimeStatus::Starting,
                        pane: SolvePane::Editor,
                        accessory_panes: crate::app::model::AccessoryPaneState::default(),
                        output: "No test run yet".into(),
                        output_scroll: 0,
                        problem_scroll: 0,
                        running: None,
                        cancellation: None,
                        pending_save: None,
                        pending_draft_save: None,
                        stale: false,
                        latest_run_revision: None,
                        quit_after_save: None,
                        discard_confirmation: None,
                        refresh_after_submit: false,
                        submitted_source: None,
                    }))
                })();
                effects.extend(reduce(state, Event::SolveOpened(operation, result)));
            }
            Effect::StartNeovim {
                source,
                synthetic_name,
                language,
            } => {
                let grid_size = crossterm::terminal::size()
                    .ok()
                    .and_then(|(width, height)| render::neovim_grid_size(state, width, height))
                    .unwrap_or((1, 1));
                if let Err(error) = neovim_worker.start_session(
                    source,
                    synthetic_name,
                    language,
                    grid_size.0,
                    grid_size.1,
                ) {
                    effects.extend(reduce(state, Event::NeovimStarted(Err(error))));
                }
            }
            Effect::SaveRun {
                operation,
                plan,
                source,
                revision,
                write_source,
                intent,
            } => {
                if let Err(error) =
                    worker.run(operation, revision, intent, plan, source, write_source)
                {
                    effects.extend(reduce(
                        state,
                        Event::RunFinished(operation, revision, intent, None, Err(error)),
                    ))
                }
            }
            Effect::SaveDraft {
                operation,
                plan,
                source,
                revision,
            } => {
                let result = source::atomic_save(&plan.root, &plan.solution_path, &source);
                effects.extend(reduce(
                    state,
                    Event::DraftSaved(operation, revision, source, result),
                ));
            }
            Effect::CancelRun { operation } => worker.cancel(operation),
            Effect::ConnectInterviewer { operation } => {
                if let Err(error) = interviewer_worker.send(InterviewerWorkerCommand::Connect {
                    operation,
                    generation: interviewer_worker.generation(),
                    cancellation: CancellationToken::new(),
                }) {
                    effects.extend(reduce(
                        state,
                        Event::InterviewerConnected(
                            operation,
                            Err(crate::interviewer::InterviewerError::transport(
                                state.interviewer.backend,
                                error,
                            )),
                        ),
                    ));
                }
            }
            Effect::InterviewerTurn {
                operation,
                revision,
                mode,
                statement,
                source,
                output,
                question,
                solved,
            } => {
                if let Err(error) = interviewer_worker.send(InterviewerWorkerCommand::Turn {
                    operation,
                    generation: interviewer_worker.generation(),
                    revision,
                    mode,
                    statement,
                    source,
                    output,
                    question,
                    solved,
                    cancellation: CancellationToken::new(),
                }) {
                    effects.extend(reduce(
                        state,
                        Event::InterviewerFinished(
                            operation,
                            revision,
                            mode,
                            Err(crate::interviewer::InterviewerError::transport(
                                state.interviewer.backend,
                                error,
                            )),
                        ),
                    ));
                }
            }
            Effect::FinalizeInterviewerTurn {
                operation,
                revision,
                mode,
                accepted,
            } => {
                if let Err(error) =
                    interviewer_worker.finalize_turn(operation, revision, mode, accepted)
                {
                    state.interviewer.status = crate::app::model::InterviewerStatus::ProtocolError;
                    state.error = Some(error);
                }
            }
            Effect::CancelInterviewer { operation } => {
                let _ = interviewer_worker.send(InterviewerWorkerCommand::Cancel { operation });
            }
            Effect::ResetInterviewer => interviewer_worker.reset(),
            Effect::LeaveSolve => worker.leave(),
            Effect::StopNeovim => {
                if let Err(error) = neovim_worker.stop_session() {
                    state.error = Some(error);
                }
            }
        }
    }
}

fn apply_neovim_cursor_shape(state: &AppState) -> Result<(), String> {
    use crate::neovim::grid::CursorShape;
    let cursor = state.solve.as_ref().and_then(|solve| {
        (state.screen == crate::app::Screen::Solve && solve.pane == SolvePane::Editor)
            .then_some(())
            .and(solve.editor_view.as_deref())
    });
    let style = match cursor.map(|view| (view.cursor_shape, view.cursor_blink)) {
        Some((CursorShape::Block, true)) => SetCursorStyle::BlinkingBlock,
        Some((CursorShape::Block, false)) => SetCursorStyle::SteadyBlock,
        Some((CursorShape::Horizontal, true)) => SetCursorStyle::BlinkingUnderScore,
        Some((CursorShape::Horizontal, false)) => SetCursorStyle::SteadyUnderScore,
        Some((CursorShape::Vertical, true)) => SetCursorStyle::BlinkingBar,
        Some((CursorShape::Vertical, false)) => SetCursorStyle::SteadyBar,
        None => SetCursorStyle::DefaultUserShape,
    };
    execute!(io::stdout(), style)
        .map_err(|error| format!("cannot set Neovim cursor shape: {error}"))
}

fn action_requires_neovim_barrier(state: &AppState, action: &crate::app::Action) -> bool {
    state.screen == crate::app::Screen::Solve
        && state.solve.as_ref().is_some_and(|solve| {
            solve.editor_status != crate::app::model::EditorRuntimeStatus::Failed
        })
        && matches!(
            action,
            crate::app::Action::SaveTest
                | crate::app::Action::Submit
                | crate::app::Action::Back
                | crate::app::Action::Quit
                | crate::app::Action::Hint
                | crate::app::Action::InterviewSend
        )
}

fn resolve_tutor_action(
    update: crate::neovim::ActionUpdate,
    coalesced_document: &mut Option<crate::neovim::DocumentUpdate>,
) -> (crate::neovim::DocumentUpdate, crate::app::Action) {
    let crate::neovim::ActionUpdate { action, document } = update;
    let app_action = match action {
        crate::neovim::TutorAction::Test => crate::app::Action::SaveTest,
        crate::neovim::TutorAction::Submit => crate::app::Action::Submit,
        crate::neovim::TutorAction::Back => crate::app::Action::Back,
        crate::neovim::TutorAction::Collapse => crate::app::Action::EditorCollapse,
        crate::neovim::TutorAction::Quit => crate::app::Action::Quit,
        crate::neovim::TutorAction::Hint => crate::app::Action::Hint,
        crate::neovim::TutorAction::Help => crate::app::Action::Help,
    };
    if action != crate::neovim::TutorAction::Back {
        return (document, app_action);
    }
    if let Some(newest) = coalesced_document.as_ref() {
        assert!(
            newest.changedtick >= document.changedtick,
            "coalesced Neovim document precedes its source action"
        );
        return (
            coalesced_document
                .take()
                .expect("coalesced Neovim document exists"),
            app_action,
        );
    }
    (document, app_action)
}

fn matching_save_acknowledgement(state: &AppState, event: &Event) -> Option<(u64, String)> {
    let Event::RunFinished(operation, revision, intent, Some(source), _) = event else {
        return None;
    };
    let solve = state.solve.as_ref()?;
    if solve.running != Some((*operation, *revision, *intent))
        || solve.editor.revision != *revision
        || solve.editor.text() != source
    {
        return None;
    }
    solve
        .editor
        .neovim_changedtick()
        .map(|changedtick| (changedtick, source.clone()))
}

fn neovim_mouse_kind(
    kind: crossterm::event::MouseEventKind,
) -> Option<(&'static str, &'static str)> {
    use crossterm::event::{MouseButton, MouseEventKind};
    match kind {
        MouseEventKind::Down(MouseButton::Left) => Some(("left", "press")),
        MouseEventKind::Down(MouseButton::Right) => Some(("right", "press")),
        MouseEventKind::Down(MouseButton::Middle) => Some(("middle", "press")),
        MouseEventKind::Up(MouseButton::Left) => Some(("left", "release")),
        MouseEventKind::Up(MouseButton::Right) => Some(("right", "release")),
        MouseEventKind::Up(MouseButton::Middle) => Some(("middle", "release")),
        MouseEventKind::Drag(MouseButton::Left) => Some(("left", "drag")),
        MouseEventKind::Drag(MouseButton::Right) => Some(("right", "drag")),
        MouseEventKind::Drag(MouseButton::Middle) => Some(("middle", "drag")),
        MouseEventKind::ScrollUp => Some(("wheel", "up")),
        MouseEventKind::ScrollDown => Some(("wheel", "down")),
        MouseEventKind::ScrollLeft => Some(("wheel", "left")),
        MouseEventKind::ScrollRight => Some(("wheel", "right")),
        MouseEventKind::Moved => None,
    }
}

fn apply_event(
    state: &mut AppState,
    repository: &Repository,
    root: &Path,
    workers: &mut RuntimeWorkers,
    event: Event,
) {
    let effects = reduce(state, event);
    let (runner_worker, interviewer_worker, neovim_worker) = workers.parts();
    apply_effects(
        state,
        repository,
        root,
        runner_worker,
        interviewer_worker,
        neovim_worker,
        effects,
    );
}

pub fn run(
    mut state: AppState,
    repository: Repository,
    requested_set: Option<String>,
    root: PathBuf,
    database_path: PathBuf,
    neovim_executable: PathBuf,
) -> Result<u8, String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("interview requires an interactive terminal".into());
    }
    let signal_handlers = ScopedSignalHandlers::register()?;
    let signal_state = signal_handlers.state();
    let mut workers = RuntimeWorkers::start(
        root.clone(),
        database_path,
        state.interviewer.backend,
        signal_state.clone(),
        neovim_executable,
    );
    let initial = requested_set.map_or(Event::Command(crate::app::Action::Reload), Event::OpenSet);
    let effects = reduce(&mut state, initial);
    let (runner_worker, interviewer_worker, neovim_worker) = workers.parts();
    apply_effects(
        &mut state,
        &repository,
        &root,
        runner_worker,
        interviewer_worker,
        neovim_worker,
        effects,
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = TerminalGuard::enter()?;
        #[cfg(debug_assertions)]
        if std::env::var_os("INTERVIEW_TUTOR_TEST_PANIC_AFTER_ENTER").is_some() {
            panic!("injected TUI runtime panic");
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("INTERVIEW_TUTOR_TEST_ERROR_AFTER_ENTER").is_some() {
            return Err("injected TUI startup error".into());
        }
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal =
            Terminal::new(backend).map_err(|e| format!("cannot initialize terminal: {e}"))?;
        terminal
            .clear()
            .map_err(|e| format!("cannot clear terminal: {e}"))?;
        let mut needs_draw = true;
        let mut next_neovim_barrier_id = 1_u64;
        let mut pending_neovim_barriers: std::collections::VecDeque<(
            u64,
            crate::app::Action,
            Option<Instant>,
        )> = std::collections::VecDeque::new();
        let mut last_neovim_grid_size = None;
        while !state.quit {
            if let Some(signal) = signal_state.received() {
                let (runner_worker, interviewer_worker, _) = workers.parts();
                runner_worker.interrupt_active(128 + signal);
                interviewer_worker.reset();
                state.quit = true;
                continue;
            }
            let events = {
                let (runner_worker, interviewer_worker, _) = workers.parts();
                runner_worker
                    .poll()
                    .into_iter()
                    .chain(interviewer_worker.poll())
                    .collect::<Vec<_>>()
            };
            for event in events {
                let save_acknowledgement = matching_save_acknowledgement(&state, &event);
                apply_event(&mut state, &repository, &root, &mut workers, event);
                if let Some((changedtick, source)) = save_acknowledgement {
                    workers
                        .neovim
                        .as_ref()
                        .expect("runtime Neovim worker exists")
                        .acknowledge_saved(changedtick, source)?;
                }
                needs_draw = true;
            }
            let neovim_poll = workers
                .neovim
                .as_ref()
                .expect("runtime Neovim worker exists")
                .poll();
            if let Some(view) = neovim_poll.grid {
                apply_event(
                    &mut state,
                    &repository,
                    &root,
                    &mut workers,
                    Event::NeovimView(view),
                );
                needs_draw = true;
            }
            let mut coalesced_document = neovim_poll.document;
            for worker_event in neovim_poll.events {
                match worker_event {
                    crate::neovim::WorkerEvent::Started(started) => {
                        if started.is_ok() {
                            let deadline = Instant::now() + NEOVIM_SOURCE_BARRIER_TIMEOUT;
                            for (_, _, pending_deadline) in &mut pending_neovim_barriers {
                                if pending_deadline.is_none() {
                                    *pending_deadline = Some(deadline);
                                }
                            }
                        } else {
                            pending_neovim_barriers.clear();
                        }
                        apply_event(
                            &mut state,
                            &repository,
                            &root,
                            &mut workers,
                            Event::NeovimStarted(started),
                        );
                    }
                    crate::neovim::WorkerEvent::Warning(error) => apply_event(
                        &mut state,
                        &repository,
                        &root,
                        &mut workers,
                        Event::NeovimWarning(error),
                    ),
                    crate::neovim::WorkerEvent::Failed(error) => {
                        pending_neovim_barriers.clear();
                        apply_event(
                            &mut state,
                            &repository,
                            &root,
                            &mut workers,
                            Event::NeovimFailed(error),
                        );
                    }
                    crate::neovim::WorkerEvent::Source(source_event) => match source_event {
                        crate::neovim::SourceEvent::Barrier(barrier) => {
                            apply_event(
                                &mut state,
                                &repository,
                                &root,
                                &mut workers,
                                Event::NeovimDocument(barrier.document),
                            );
                            let Some((expected_id, action, _deadline)) =
                                pending_neovim_barriers.pop_front()
                            else {
                                return Err("Neovim returned an unexpected source barrier".into());
                            };
                            if expected_id != barrier.id {
                                return Err(format!(
                                    "Neovim source barrier mismatch: expected {expected_id}, got {}",
                                    barrier.id
                                ));
                            }
                            apply_event(
                                &mut state,
                                &repository,
                                &root,
                                &mut workers,
                                Event::Command(action),
                            );
                        }
                        crate::neovim::SourceEvent::Action(action_update) => {
                            let (document, action) =
                                resolve_tutor_action(action_update, &mut coalesced_document);
                            apply_event(
                                &mut state,
                                &repository,
                                &root,
                                &mut workers,
                                Event::NeovimDocument(document),
                            );
                            apply_event(
                                &mut state,
                                &repository,
                                &root,
                                &mut workers,
                                Event::Command(action),
                            );
                        }
                    },
                }
                needs_draw = true;
            }
            if pending_neovim_barriers
                .front()
                .and_then(|(_, _, deadline)| *deadline)
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err("Neovim source barrier timed out".into());
            }
            if let Some(document) = coalesced_document {
                apply_event(
                    &mut state,
                    &repository,
                    &root,
                    &mut workers,
                    Event::NeovimDocument(document),
                );
                needs_draw = true;
            }
            let terminal_size = terminal
                .size()
                .map_err(|error| format!("cannot read terminal size: {error}"))?;
            let expected_grid_size =
                render::neovim_grid_size(&state, terminal_size.width, terminal_size.height);
            if expected_grid_size != last_neovim_grid_size {
                if let Some((width, height)) = expected_grid_size {
                    workers
                        .neovim
                        .as_ref()
                        .expect("runtime Neovim worker exists")
                        .resize(width, height)?;
                }
                last_neovim_grid_size = expected_grid_size;
            }
            if needs_draw {
                terminal
                    .draw(|frame| render::render(frame, &state))
                    .map_err(|e| format!("cannot draw terminal: {e}"))?;
                apply_neovim_cursor_shape(&state)?;
                needs_draw = false;
            }
            if !event::poll(Duration::from_millis(50))
                .map_err(|e| format!("cannot poll terminal: {e}"))?
            {
                continue;
            }
            match event::read().map_err(|e| format!("cannot read terminal: {e}"))? {
                TerminalEvent::Key(key) => {
                    if input::routes_to_neovim(key, &state) {
                        if let Some(encoded) = crate::neovim::key::encode_key(key)? {
                            workers
                                .neovim
                                .as_ref()
                                .expect("runtime Neovim worker exists")
                                .input(encoded)?;
                        }
                        needs_draw = true;
                    } else if let Some(action) = input::action_for_key(key, &mut state) {
                        if action_requires_neovim_barrier(&state, &action) {
                            if pending_neovim_barriers.len() == 16 {
                                return Err("Neovim source barrier queue exceeded bound".into());
                            }
                            let id = next_neovim_barrier_id;
                            next_neovim_barrier_id = next_neovim_barrier_id
                                .checked_add(1)
                                .ok_or("Neovim source barrier id overflow")?;
                            workers
                                .neovim
                                .as_ref()
                                .expect("runtime Neovim worker exists")
                                .barrier(id)?;
                            let deadline = state
                                .solve
                                .as_ref()
                                .is_some_and(|solve| {
                                    solve.editor_status
                                        == crate::app::model::EditorRuntimeStatus::Ready
                                })
                                .then(|| Instant::now() + NEOVIM_SOURCE_BARRIER_TIMEOUT);
                            pending_neovim_barriers.push_back((id, action, deadline));
                        } else {
                            apply_event(
                                &mut state,
                                &repository,
                                &root,
                                &mut workers,
                                Event::Command(action),
                            );
                        }
                        needs_draw = true;
                    }
                }
                TerminalEvent::Resize(width, height) => {
                    if let Some((grid_width, grid_height)) =
                        render::neovim_grid_size(&state, width, height)
                    {
                        workers
                            .neovim
                            .as_ref()
                            .expect("runtime Neovim worker exists")
                            .resize(grid_width, grid_height)?;
                        last_neovim_grid_size = Some((grid_width, grid_height));
                    }
                    needs_draw = true;
                }
                TerminalEvent::Paste(text) => {
                    if state.screen == crate::app::Screen::Solve
                        && state
                            .solve
                            .as_ref()
                            .is_some_and(|solve| solve.pane == SolvePane::Editor)
                    {
                        let paste = workers
                            .neovim
                            .as_ref()
                            .expect("runtime Neovim worker exists")
                            .paste(text);
                        match paste {
                            Ok(()) => needs_draw = true,
                            Err(error) => {
                                apply_event(
                                    &mut state,
                                    &repository,
                                    &root,
                                    &mut workers,
                                    Event::NeovimWarning(error),
                                );
                                needs_draw = true;
                            }
                        }
                    }
                }
                TerminalEvent::Mouse(mouse) => {
                    let size = terminal
                        .size()
                        .map_err(|error| format!("cannot read terminal size: {error}"))?;
                    if let Some(area) = render::neovim_grid_area(&state, size.width, size.height)
                        && mouse.column >= area.x
                        && mouse.column < area.right()
                        && mouse.row >= area.y
                        && mouse.row < area.bottom()
                        && let Some((button, action)) = neovim_mouse_kind(mouse.kind)
                    {
                        let mut modifiers = String::new();
                        if mouse
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::SHIFT)
                        {
                            modifiers.push('S');
                        }
                        if mouse
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            modifiers.push('C');
                        }
                        if mouse
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::ALT)
                        {
                            modifiers.push('A');
                        }
                        workers
                            .neovim
                            .as_ref()
                            .expect("runtime Neovim worker exists")
                            .mouse(
                                button.into(),
                                action.into(),
                                modifiers,
                                mouse.row - area.y,
                                mouse.column - area.x,
                            )?;
                    }
                }
                TerminalEvent::FocusGained | TerminalEvent::FocusLost => {}
            }
        }
        Ok(())
    }));
    workers.shutdown();
    let exit_code = signal_state
        .exit_code()
        .map(|code| u8::try_from(code).expect("signal exit codes fit u8"))
        .unwrap_or(0);
    signal_handlers.restore()?;
    match result {
        Ok(Ok(())) => Ok(exit_code),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("TUI runtime panicked; terminal state was restored".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RunIntent;
    use crate::app::model::OperationId;
    use crate::runner::{ExecutionPlan, ExecutionResult, Termination};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn solve_open_aliases_and_source_actions_wait_for_starting_neovim() {
        use crate::app::model::EditorRuntimeStatus;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut state = interviewer_solve_state();
        let solve = state.solve.as_mut().unwrap();
        solve.pane = SolvePane::Editor;
        solve.editor_status = EditorRuntimeStatus::Starting;

        for (key, expected) in [
            (
                KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
                crate::app::Action::SaveTest,
            ),
            (
                KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
                crate::app::Action::Submit,
            ),
            (
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                crate::app::Action::SaveTest,
            ),
        ] {
            let action = input::action_for_key(key, &mut state).unwrap();
            assert_eq!(action, expected);
            assert!(action_requires_neovim_barrier(&state, &action));
        }
        for action in [
            crate::app::Action::SaveTest,
            crate::app::Action::Submit,
            crate::app::Action::Back,
            crate::app::Action::Quit,
            crate::app::Action::Hint,
            crate::app::Action::InterviewSend,
        ] {
            assert!(action_requires_neovim_barrier(&state, &action));
        }

        state.solve.as_mut().unwrap().editor_status = EditorRuntimeStatus::Failed;
        assert!(!action_requires_neovim_barrier(
            &state,
            &crate::app::Action::SaveTest
        ));
    }

    #[test]
    fn tutor_back_uses_newest_coalesced_document_without_changing_run_key_moments() {
        use crate::neovim::{ActionUpdate, DocumentUpdate, TutorAction};

        let mut back_state = interviewer_solve_state();
        let mut coalesced = Some(DocumentUpdate {
            text: "newest accepted bytes".into(),
            mode: "n".into(),
            changedtick: 3,
        });
        let (document, action) = resolve_tutor_action(
            ActionUpdate {
                action: TutorAction::Back,
                document: DocumentUpdate {
                    text: "bytes at back notification".into(),
                    mode: "n".into(),
                    changedtick: 2,
                },
            },
            &mut coalesced,
        );
        reduce(&mut back_state, Event::NeovimDocument(document));
        let effects = reduce(&mut back_state, Event::Command(action));
        assert!(coalesced.is_none());
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::SaveDraft { source, .. },
                Effect::LeaveSolve
            ] if source == "newest accepted bytes"
        ));

        for (tutor_action, expected_action, expected_intent) in [
            (
                TutorAction::Test,
                crate::app::Action::SaveTest,
                RunIntent::Test,
            ),
            (
                TutorAction::Submit,
                crate::app::Action::Submit,
                RunIntent::Submit,
            ),
        ] {
            let mut state = interviewer_solve_state();
            let mut coalesced = Some(DocumentUpdate {
                text: "bytes accepted after key moment".into(),
                mode: "n".into(),
                changedtick: 3,
            });
            let (document, action) = resolve_tutor_action(
                ActionUpdate {
                    action: tutor_action,
                    document: DocumentUpdate {
                        text: "exact bytes at key moment".into(),
                        mode: "n".into(),
                        changedtick: 2,
                    },
                },
                &mut coalesced,
            );
            assert_eq!(action, expected_action);
            reduce(&mut state, Event::NeovimDocument(document));
            let effects = reduce(&mut state, Event::Command(action));
            assert!(coalesced.is_some());
            assert!(matches!(
                effects.as_slice(),
                [Effect::SaveRun {
                    source,
                    intent,
                    ..
                }] if source == "exact bytes at key moment" && *intent == expected_intent
            ));
        }
    }

    #[test]
    fn tutor_collapse_and_help_keep_their_editor_origin() {
        use crate::neovim::{ActionUpdate, DocumentUpdate, TutorAction};

        for (tutor_action, expected) in [
            (TutorAction::Collapse, crate::app::Action::EditorCollapse),
            (TutorAction::Help, crate::app::Action::Help),
        ] {
            let mut coalesced = None;
            let (_, action) = resolve_tutor_action(
                ActionUpdate {
                    action: tutor_action,
                    document: DocumentUpdate {
                        text: "source".into(),
                        mode: "n".into(),
                        changedtick: 2,
                    },
                },
                &mut coalesced,
            );
            assert_eq!(action, expected);
        }
    }

    #[test]
    fn mouse_events_map_to_neovim_protocol_actions() {
        use crossterm::event::{MouseButton, MouseEventKind};
        assert_eq!(
            neovim_mouse_kind(MouseEventKind::Down(MouseButton::Left)),
            Some(("left", "press"))
        );
        assert_eq!(
            neovim_mouse_kind(MouseEventKind::Drag(MouseButton::Right)),
            Some(("right", "drag"))
        );
        assert_eq!(
            neovim_mouse_kind(MouseEventKind::ScrollUp),
            Some(("wheel", "up"))
        );
        assert_eq!(neovim_mouse_kind(MouseEventKind::Moved), None);
    }

    fn plan() -> ExecutionPlan {
        ExecutionPlan {
            root: PathBuf::from("/tmp"),
            language: "python".into(),
            problem_slug: "p".into(),
            set_slug: None,
            runner_path: PathBuf::from("/tmp/run"),
            solution_path: PathBuf::from("/tmp/p.py"),
        }
    }

    fn services(termination: Termination, records: Arc<AtomicUsize>) -> RunnerServices {
        RunnerServices {
            save: Arc::new(|_, _, _| Ok(())),
            execute: Arc::new(move |_, _| {
                Ok(ExecutionResult::test_result(termination.clone(), "result"))
            }),
            record: Arc::new(move |_, _| {
                records.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            }),
            finalize_cancelled: Arc::new(|_, _| Ok(())),
        }
    }

    fn current_signal_mask() -> (bool, bool) {
        // SAFETY: pthread_sigmask initializes mask when the set argument is null, then
        // sigismember only reads that initialized value.
        unsafe {
            let mut mask = std::mem::zeroed();
            let error = libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask);
            assert_eq!(error, 0);
            (
                libc::sigismember(&mask, libc::SIGINT) == 1,
                libc::sigismember(&mask, libc::SIGTERM) == 1,
            )
        }
    }

    fn poll_one(worker: &mut RunnerWorker) -> Event {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = worker.poll().into_iter().next() {
                return event;
            }
            assert!(std::time::Instant::now() < deadline, "worker event timeout");
            thread::yield_now();
        }
    }

    fn run_once(worker: &mut RunnerWorker, operation: u64, intent: RunIntent) -> Event {
        worker
            .run(
                OperationId(operation),
                0,
                intent,
                plan(),
                "source".into(),
                true,
            )
            .unwrap();
        poll_one(worker)
    }

    #[derive(Default)]
    struct TestInterviewerBackendState {
        transcript: String,
        captures: Vec<String>,
        reset_count: usize,
        turn_count: usize,
    }

    #[derive(Default)]
    struct TestTurnGate {
        state: std::sync::Mutex<(bool, bool)>,
        changed: std::sync::Condvar,
    }

    impl TestTurnGate {
        fn wait_until_started(&self) {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().expect("test turn gate lock");
            while !state.0 {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                assert!(!remaining.is_zero(), "Interviewer test turn did not start");
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, remaining)
                    .expect("test turn gate wait");
                state = next;
                assert!(
                    !timeout.timed_out() || state.0,
                    "Interviewer test turn timed out"
                );
            }
        }

        fn block_until_released(&self) {
            let mut state = self.state.lock().expect("test turn gate lock");
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).expect("test turn gate wait");
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().expect("test turn gate lock");
            state.1 = true;
            self.changed.notify_all();
        }
    }

    struct TestInterviewerBackend {
        state: Arc<std::sync::Mutex<TestInterviewerBackendState>>,
        gate: Arc<TestTurnGate>,
        blocking_turn: Option<usize>,
    }

    impl InterviewerWorkerBackend for TestInterviewerBackend {
        fn backend(&self) -> crate::interviewer::Backend {
            crate::interviewer::Backend::Pi
        }

        fn connect(
            &mut self,
            _control_pid: Arc<std::sync::atomic::AtomicI32>,
            _cancellation: &CancellationToken,
        ) -> Result<(), crate::interviewer::InterviewerError> {
            Ok(())
        }

        fn turn(
            &mut self,
            request: &InterviewerTurnRequest,
            _cancellation: &CancellationToken,
        ) -> Result<String, crate::interviewer::InterviewerError> {
            let turn_count = {
                let mut state = self.state.lock().expect("test Interviewer backend lock");
                state.turn_count += 1;
                let turn_count = state.turn_count;
                let transcript = state.transcript.clone();
                state.captures.push(format!(
                    "transcript={transcript}\nstatement={}\nsource={}\nquestion={}",
                    request.statement, request.source, request.question
                ));
                turn_count
            };
            if self.blocking_turn == Some(turn_count) {
                self.gate.block_until_released();
            }
            Ok(format!("response-{turn_count}"))
        }

        fn commit(&mut self, pending: PendingInterviewerResponse) {
            let mut state = self.state.lock().expect("test Interviewer backend lock");
            state.transcript.push_str(&format!(
                "user: {}\ninterviewer: {}\n",
                pending.question, pending.response
            ));
        }

        fn reset(&mut self) {
            let mut state = self.state.lock().expect("test Interviewer backend lock");
            state.transcript.clear();
            state.reset_count += 1;
        }
    }

    fn test_interviewer_worker(
        blocking_turn: Option<usize>,
    ) -> (
        InterviewerWorker,
        Arc<std::sync::Mutex<TestInterviewerBackendState>>,
        Arc<TestTurnGate>,
    ) {
        let state = Arc::new(std::sync::Mutex::new(TestInterviewerBackendState::default()));
        let gate = Arc::new(TestTurnGate::default());
        let factory_state = Arc::clone(&state);
        let factory_gate = Arc::clone(&gate);
        let factory: Arc<InterviewerBackendFactory> = Arc::new(move || {
            Box::new(TestInterviewerBackend {
                state: Arc::clone(&factory_state),
                gate: Arc::clone(&factory_gate),
                blocking_turn,
            })
        });
        (InterviewerWorker::start_with_backend(factory), state, gate)
    }

    fn send_interviewer_connect(worker: &mut InterviewerWorker, operation: u64) {
        let generation = worker.generation();
        worker
            .send(InterviewerWorkerCommand::Connect {
                operation: OperationId(operation),
                generation,
                cancellation: CancellationToken::new(),
            })
            .unwrap();
    }

    fn send_interviewer_turn(
        worker: &mut InterviewerWorker,
        operation: u64,
        source: &str,
        question: &str,
    ) {
        let generation = worker.generation();
        worker
            .send(InterviewerWorkerCommand::Turn {
                operation: OperationId(operation),
                generation,
                revision: 0,
                mode: crate::interviewer::Mode::Interviewer,
                statement: format!("statement-{operation}"),
                source: source.into(),
                output: String::new(),
                question: question.into(),
                solved: false,
                cancellation: CancellationToken::new(),
            })
            .unwrap();
    }

    fn poll_interviewer_one(worker: &mut InterviewerWorker) -> Event {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = worker.poll().into_iter().next() {
                return event;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Interviewer worker event timeout"
            );
            thread::yield_now();
        }
    }

    fn wait_for_queued_interviewer_event(worker: &InterviewerWorker, operation: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while worker
            .queued_event_operation
            .load(std::sync::atomic::Ordering::Acquire)
            != operation
        {
            assert!(
                std::time::Instant::now() < deadline,
                "Interviewer worker did not queue operation {operation}"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn save_acknowledgement_requires_the_active_run_and_exact_editor_snapshot() {
        let mut state = interviewer_solve_state();
        let solve = state.solve.as_mut().unwrap();
        solve
            .editor
            .install_neovim_snapshot("source".into(), "n", 7)
            .unwrap();
        solve.running = Some((OperationId(4), 0, RunIntent::Test));
        let matching = Event::RunFinished(
            OperationId(4),
            0,
            RunIntent::Test,
            Some("source".into()),
            Err("test failed after save".into()),
        );
        assert_eq!(
            matching_save_acknowledgement(&state, &matching),
            Some((7, "source".into()))
        );
        let stale_operation = Event::RunFinished(
            OperationId(3),
            0,
            RunIntent::Test,
            Some("source".into()),
            Ok(ExecutionResult::test_result(Termination::Exited(0), "pass")),
        );
        assert_eq!(
            matching_save_acknowledgement(&state, &stale_operation),
            None
        );
        let wrong_source = Event::RunFinished(
            OperationId(4),
            0,
            RunIntent::Test,
            Some("other".into()),
            Ok(ExecutionResult::test_result(Termination::Exited(0), "pass")),
        );
        assert_eq!(matching_save_acknowledgement(&state, &wrong_source), None);
    }

    fn interviewer_solve_state() -> AppState {
        use crate::app::model::Screen;
        use crate::editor::EditorDocument;

        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: "statement".into(),
            language: "python".into(),
            plan: plan(),
            editor: EditorDocument::new("source".into()).unwrap(),
            editor_view: None,
            editor_status: crate::app::model::EditorRuntimeStatus::Ready,
            pane: SolvePane::Interview,
            accessory_panes: crate::app::model::AccessoryPaneState::default(),
            output: String::new(),
            output_scroll: 0,
            problem_scroll: 0,
            running: None,
            cancellation: None,
            pending_save: None,
            pending_draft_save: None,
            stale: false,
            latest_run_revision: None,
            quit_after_save: None,
            discard_confirmation: None,
            refresh_after_submit: false,
            submitted_source: None,
        });
        state
    }

    #[test]
    fn worker_tests_never_record_and_submit_terminations_record_once() {
        let records = Arc::new(AtomicUsize::new(0));
        let mut worker =
            RunnerWorker::start_with_services(services(Termination::Exited(0), records.clone()));
        assert!(matches!(
            run_once(&mut worker, 1, RunIntent::Test),
            Event::RunFinished(_, _, RunIntent::Test, Some(_), Ok(_))
        ));
        assert_eq!(records.load(Ordering::SeqCst), 0);
        assert!(matches!(
            run_once(&mut worker, 2, RunIntent::Submit),
            Event::RunFinished(_, _, RunIntent::Submit, Some(_), Ok(_))
        ));
        assert_eq!(records.load(Ordering::SeqCst), 1);
        worker.shutdown();

        for (index, termination) in [
            Termination::Exited(1),
            Termination::TimedOut,
            Termination::Cancelled,
            Termination::Signalled(15),
        ]
        .into_iter()
        .enumerate()
        {
            let records = Arc::new(AtomicUsize::new(0));
            let mut worker =
                RunnerWorker::start_with_services(services(termination, records.clone()));
            assert_eq!(records.load(Ordering::SeqCst), 0);
            assert!(matches!(
                run_once(&mut worker, index as u64 + 3, RunIntent::Submit),
                Event::RunFinished(_, _, RunIntent::Submit, Some(_), Ok(_))
            ));
            assert_eq!(records.load(Ordering::SeqCst), 1);
            worker.shutdown();
        }
    }

    #[test]
    fn cancellation_during_recording_finalizes_exact_attempt_with_matching_interrupt() {
        for exit_code in [130, 143] {
            let signal_mask_before = current_signal_mask();
            let (record_started, record_started_receiver) = mpsc::sync_channel(1);
            let release_record = Arc::new(AtomicBool::new(false));
            let record_release = Arc::clone(&release_record);
            let records = Arc::new(AtomicUsize::new(0));
            let record_count = Arc::clone(&records);
            let finalized = Arc::new(std::sync::Mutex::new(Vec::new()));
            let finalized_attempts = Arc::clone(&finalized);
            let services = RunnerServices {
                save: Arc::new(|_, _, _| Ok(())),
                execute: Arc::new(|_, _| {
                    Ok(ExecutionResult::test_result(Termination::Exited(0), "pass"))
                }),
                record: Arc::new(move |_, _| {
                    record_started.send(()).unwrap();
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
                    while !record_release.load(Ordering::Acquire) {
                        assert!(std::time::Instant::now() < deadline);
                        thread::yield_now();
                    }
                    record_count.fetch_add(1, Ordering::SeqCst);
                    Ok(41)
                }),
                finalize_cancelled: Arc::new(move |attempt_id, exit_code| {
                    finalized_attempts
                        .lock()
                        .expect("finalized attempts lock")
                        .push((attempt_id, exit_code));
                    Ok(())
                }),
            };
            let mut worker = RunnerWorker::start_with_services(services);
            worker
                .run(
                    OperationId(17),
                    4,
                    RunIntent::Submit,
                    plan(),
                    "source".into(),
                    false,
                )
                .unwrap();
            record_started_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            if exit_code == 130 {
                worker.cancel(OperationId(17));
            } else {
                worker.interrupt_active(exit_code);
            }
            release_record.store(true, Ordering::Release);
            assert!(matches!(
                poll_one(&mut worker),
                Event::RunFinished(
                    OperationId(17),
                    4,
                    RunIntent::Submit,
                    Some(_),
                    Ok(ExecutionResult {
                        termination: Termination::Cancelled,
                        ..
                    })
                )
            ));
            assert_eq!(records.load(Ordering::SeqCst), 1);
            assert_eq!(
                finalized
                    .lock()
                    .expect("finalized attempts lock")
                    .as_slice(),
                &[(41, exit_code)]
            );
            assert_eq!(current_signal_mask(), signal_mask_before);
            worker.shutdown();
        }
    }

    #[test]
    fn worker_reports_save_and_record_failures() {
        let save_failure = RunnerServices {
            save: Arc::new(|_, _, _| Err("save failed".into())),
            execute: Arc::new(|_, _| panic!("execute must not run")),
            record: Arc::new(|_, _| panic!("record must not run")),
            finalize_cancelled: Arc::new(|_, _| panic!("finalize must not run")),
        };
        let mut worker = RunnerWorker::start_with_services(save_failure);
        assert!(
            matches!(run_once(&mut worker, 1, RunIntent::Test), Event::RunFinished(_, _, _, None, Err(error)) if error == "save failed")
        );
        worker.shutdown();

        let record_failure = RunnerServices {
            save: Arc::new(|_, _, _| Ok(())),
            execute: Arc::new(|_, _| {
                Ok(ExecutionResult::test_result(Termination::Exited(0), "pass"))
            }),
            record: Arc::new(|_, _| Err("record failed".into())),
            finalize_cancelled: Arc::new(|_, _| panic!("finalize must not run")),
        };
        let mut worker = RunnerWorker::start_with_services(record_failure);
        assert!(
            matches!(run_once(&mut worker, 2, RunIntent::Submit), Event::RunFinished(_, _, _, Some(_), Err(error)) if error == "record failed")
        );
        worker.shutdown();
    }

    #[test]
    fn worker_panic_clears_reducer_state_renders_error_and_joins() {
        use crate::app::model::Screen;
        use crate::editor::EditorDocument;
        use ratatui::{Terminal, backend::TestBackend};

        let panic_services = RunnerServices {
            save: Arc::new(|_, _, _| Ok(())),
            execute: Arc::new(|_, _| panic!("injected panic")),
            record: Arc::new(|_, _| Ok(1)),
            finalize_cancelled: Arc::new(|_, _| Ok(())),
        };
        let mut worker = RunnerWorker::start_with_services(panic_services);
        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: "statement".into(),
            language: "python".into(),
            plan: plan(),
            editor: EditorDocument::new("source".into()).unwrap(),
            editor_view: None,
            editor_status: crate::app::model::EditorRuntimeStatus::Ready,
            pane: SolvePane::Editor,
            accessory_panes: crate::app::model::AccessoryPaneState::default(),
            output: String::new(),
            output_scroll: 0,
            problem_scroll: 0,
            running: Some((OperationId(3), 0, RunIntent::Test)),
            cancellation: None,
            pending_save: None,
            pending_draft_save: None,
            stale: false,
            latest_run_revision: None,
            quit_after_save: None,
            discard_confirmation: None,
            refresh_after_submit: false,
            submitted_source: None,
        });
        worker
            .run(
                OperationId(3),
                0,
                RunIntent::Test,
                plan(),
                "source".into(),
                false,
            )
            .unwrap();

        let event = poll_one(&mut worker);
        assert!(worker.active.is_none());
        let effects = reduce(&mut state, event);
        assert!(effects.is_empty());
        assert!(state.solve.as_ref().unwrap().running.is_none());
        assert_eq!(state.status, "Run failed");
        assert_eq!(state.error.as_deref(), Some("runner worker panicked"));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render::render(frame, &state))
            .unwrap();
        let view = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(view.contains("runner worker panicked"));

        worker.shutdown();
    }

    #[test]
    fn worker_disconnect_clears_active_operation() {
        let records = Arc::new(AtomicUsize::new(0));
        let mut worker =
            RunnerWorker::start_with_services(services(Termination::Exited(0), records));
        worker.active = Some((
            OperationId(9),
            4,
            RunIntent::Test,
            RunCancellation::default(),
        ));
        worker.sender.send(WorkerCommand::Shutdown).unwrap();
        assert!(worker.events.recv_timeout(Duration::from_secs(2)).is_err());
        assert!(
            matches!(worker.poll().as_slice(), [Event::RunFinished(OperationId(9), 4, RunIntent::Test, None, Err(error))] if error == "runner worker disconnected")
        );
        assert!(worker.active.is_none());
        assert!(worker.join.is_none());
    }

    #[test]
    fn disabled_interviewer_worker_has_no_thread_and_never_constructs_backend() {
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&factory_calls);
        let factory: Arc<InterviewerBackendFactory> = Arc::new(move || {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            Box::new(SessionInterviewerBackend {
                backend: crate::interviewer::Backend::Pi,
                session: None,
            })
        });
        let mut worker = InterviewerWorker::new(factory, false);
        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
        assert!(worker.join.is_none());
        assert!(worker.sender.is_none());
        assert!(worker.poll().is_empty());
        assert_eq!(
            worker
                .send(InterviewerWorkerCommand::Connect {
                    operation: OperationId(1),
                    generation: worker.generation(),
                    cancellation: CancellationToken::new(),
                })
                .unwrap_err(),
            "Interviewer is disabled"
        );
        assert!(worker.join.is_none());
        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
        worker.shutdown();
        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn interviewer_worker_panic_is_converted_to_a_recoverable_error() {
        let result = catch_interviewer_worker_panic::<()>(crate::interviewer::Backend::Pi, || {
            panic!("injected Interviewer panic")
        });
        assert_eq!(result.unwrap_err().message(), "Interviewer worker panicked");
    }

    #[test]
    fn interviewer_poll_reports_disconnect_once_joins_and_allows_reconnect() {
        let (mut worker, _, _) = test_interviewer_worker(None);
        let mut state = interviewer_solve_state();
        state.interviewer.active = Some((OperationId(8), 0, crate::interviewer::Mode::Interviewer));
        state.interviewer.status = crate::app::model::InterviewerStatus::Thinking;
        worker.send(InterviewerWorkerCommand::Panic).unwrap();

        let event = poll_interviewer_one(&mut worker);
        assert!(
            matches!(event, Event::InterviewerDisconnected(ref error) if error.contains("panicked"))
        );
        assert!(worker.join.is_none());
        assert!(worker.poll().is_empty());
        assert!(reduce(&mut state, event).is_empty());
        assert!(state.interviewer.active.is_none());
        assert_eq!(
            state.interviewer.status,
            crate::app::model::InterviewerStatus::Disconnected
        );

        send_interviewer_connect(&mut worker, 9);
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerConnected(OperationId(9), Ok(()))
        ));
        assert!(worker.join.is_some());
        worker.shutdown();
    }

    #[test]
    fn queued_completion_cancel_race_discards_pending_and_next_turn_succeeds() {
        let (mut worker, backend, _) = test_interviewer_worker(None);
        send_interviewer_connect(&mut worker, 1);
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerConnected(OperationId(1), Ok(()))
        ));

        let mut state = interviewer_solve_state();
        state.interviewer.active = Some((OperationId(2), 0, crate::interviewer::Mode::Interviewer));
        state.interviewer.status = crate::app::model::InterviewerStatus::Thinking;
        state
            .interviewer
            .push_message("You".into(), "cancelled-question".into());
        send_interviewer_turn(&mut worker, 2, "cancelled-source", "cancelled-question");
        wait_for_queued_interviewer_event(&worker, 2);

        let cancel = reduce(&mut state, Event::Command(crate::app::Action::Cancel));
        let [Effect::CancelInterviewer { operation }] = cancel.as_slice() else {
            panic!("expected Interviewer cancellation")
        };
        worker
            .send(InterviewerWorkerCommand::Cancel {
                operation: *operation,
            })
            .unwrap();
        assert!(
            worker
                .pending_response
                .lock()
                .expect("Interviewer pending response lock")
                .is_none()
        );

        let event = poll_interviewer_one(&mut worker);
        let effects = reduce(&mut state, event);
        let [
            Effect::FinalizeInterviewerTurn {
                operation,
                revision,
                mode,
                accepted: false,
            },
        ] = effects.as_slice()
        else {
            panic!("cancelled completion must be explicitly discarded")
        };
        worker
            .finalize_turn(*operation, *revision, *mode, false)
            .unwrap();
        assert!(
            !state
                .interviewer
                .messages
                .iter()
                .any(|(_, message)| message == "response-1")
        );

        send_interviewer_connect(&mut worker, 3);
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerConnected(OperationId(3), Ok(()))
        ));
        send_interviewer_turn(&mut worker, 4, "new-source", "new-question");
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerFinished(OperationId(4), 0, _, Ok(ref response))
                if response == "response-2"
        ));
        {
            let backend = backend.lock().expect("test Interviewer backend lock");
            let next_capture = backend.captures.last().expect("next turn capture");
            assert!(!next_capture.contains("cancelled-question"));
            assert!(!next_capture.contains("cancelled-source"));
            assert!(!next_capture.contains("response-1"));
        }
        worker.shutdown();
    }

    #[test]
    fn saturated_reset_epoch_clears_ui_and_worker_transcript_before_next_problem() {
        let (mut worker, backend, gate) = test_interviewer_worker(Some(2));
        send_interviewer_connect(&mut worker, 1);
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerConnected(OperationId(1), Ok(()))
        ));
        send_interviewer_turn(&mut worker, 2, "prior-source", "prior-question");
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerFinished(OperationId(2), 0, mode, Ok(_))
                if mode == crate::interviewer::Mode::Interviewer
        ));
        worker
            .finalize_turn(
                OperationId(2),
                0,
                crate::interviewer::Mode::Interviewer,
                true,
            )
            .unwrap();

        send_interviewer_turn(
            &mut worker,
            3,
            "active-prior-source",
            "active-prior-question",
        );
        gate.wait_until_started();
        let old_generation = worker.generation();
        let sender = worker.sender.as_ref().expect("Interviewer sender");
        for operation in [90, 91] {
            sender
                .try_send(InterviewerWorkerCommand::Connect {
                    operation: OperationId(operation),
                    generation: old_generation,
                    cancellation: CancellationToken::new(),
                })
                .unwrap();
        }

        let mut state = interviewer_solve_state();
        state
            .interviewer
            .push_message("Interviewer".into(), "private".into());
        let effects = reduce(&mut state, Event::Command(crate::app::Action::Back));
        assert!(state.interviewer.messages.is_empty());
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::ResetInterviewer))
        );
        worker.reset();
        assert_eq!(worker.generation(), old_generation + 1);
        assert!(
            worker
                .pending_response
                .lock()
                .expect("Interviewer pending response lock")
                .is_none()
        );

        gate.release();
        wait_for_queued_interviewer_event(&worker, 91);
        let events = worker.poll();
        assert!(events.iter().any(|event| {
            matches!(event, Event::InterviewerFinished(OperationId(3), 0, _, Err(error)) if error.contains("reset"))
        }));
        send_interviewer_connect(&mut worker, 10);
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerConnected(OperationId(10), Ok(()))
        ));
        send_interviewer_turn(&mut worker, 11, "next-source", "next-question");
        assert!(matches!(
            poll_interviewer_one(&mut worker),
            Event::InterviewerFinished(OperationId(11), 0, _, Ok(_))
        ));

        let backend = backend.lock().expect("test Interviewer backend lock");
        assert!(backend.reset_count >= 1);
        let next_capture = backend.captures.last().expect("next problem capture");
        assert!(next_capture.contains("next-source"));
        for private in [
            "prior-source",
            "prior-question",
            "response-1",
            "active-prior-source",
            "active-prior-question",
            "private",
        ] {
            assert!(!next_capture.contains(private), "leaked {private}");
        }
        drop(backend);
        worker.shutdown();
    }

    #[test]
    fn interviewer_cancel_command_sets_active_cancellation_token() {
        let mut worker = InterviewerWorker::start(crate::interviewer::Backend::Pi);
        let cancellation = CancellationToken::new();
        *worker
            .active_cancellation
            .lock()
            .expect("Interviewer cancellation lock") = Some((OperationId(7), cancellation.clone()));
        worker
            .send(InterviewerWorkerCommand::Cancel {
                operation: OperationId(7),
            })
            .unwrap();
        assert!(cancellation.is_cancelled());
        worker.shutdown();
    }

    #[test]
    fn shutdown_cancels_and_joins_active_execution() {
        let finished = Arc::new(AtomicBool::new(false));
        let observed = finished.clone();
        let services = RunnerServices {
            save: Arc::new(|_, _, _| Ok(())),
            execute: Arc::new(move |_, cancellation| {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while !cancellation.is_cancelled() && std::time::Instant::now() < deadline {
                    thread::yield_now();
                }
                observed.store(true, Ordering::SeqCst);
                Ok(ExecutionResult::test_result(
                    Termination::Cancelled,
                    "cancelled",
                ))
            }),
            record: Arc::new(|_, _| Ok(1)),
            finalize_cancelled: Arc::new(|_, _| Ok(())),
        };
        let mut worker = RunnerWorker::start_with_services(services);
        worker
            .run(
                OperationId(1),
                0,
                RunIntent::Test,
                plan(),
                "source".into(),
                false,
            )
            .unwrap();
        worker.shutdown();
        assert!(finished.load(Ordering::SeqCst));
    }
}
