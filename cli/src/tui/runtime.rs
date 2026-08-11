use crate::app::model::{SolvePane, SolveSession};
use crate::app::{AppState, Effect, Event, LoadScope, Repository, reduce};
use crate::runner::{self, CancellationToken, ExecutionLimits};
use crate::source;
use crate::tui::{input, render};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event as TerminalEvent};
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
use std::time::Duration;

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("cannot enable terminal raw mode: {e}"))?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
            return Err(format!("cannot enter terminal screen: {e}"));
        }
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

enum WorkerCommand {
    Run {
        operation: crate::app::model::OperationId,
        revision: u64,
        intent: crate::app::RunIntent,
        plan: runner::ExecutionPlan,
        source: String,
        write_source: bool,
        cancellation: CancellationToken,
    },
    Shutdown,
}
type SaveService = dyn Fn(&Path, &Path, &str) -> Result<(), String> + Send + Sync;
type ExecuteService = dyn Fn(&runner::ExecutionPlan, &CancellationToken) -> Result<runner::ExecutionResult, String>
    + Send
    + Sync;
type RecordService =
    dyn Fn(&runner::ExecutionPlan, &runner::ExecutionResult) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
struct RunnerServices {
    save: Arc<SaveService>,
    execute: Arc<ExecuteService>,
    record: Arc<RecordService>,
}

enum CodexWorkerCommand {
    Connect,
    Turn {
        operation: crate::app::model::OperationId,
        revision: u64,
        mode: crate::codex::prompt::Mode,
        statement: String,
        source: String,
        output: String,
        question: String,
        solved: bool,
    },
    Reset,
    Cancel,
    Shutdown,
}

struct CodexWorker {
    sender: SyncSender<CodexWorkerCommand>,
    events: Receiver<Event>,
    join: Option<JoinHandle<()>>,
    control_pid: Arc<std::sync::atomic::AtomicI32>,
}
impl CodexWorker {
    fn start() -> Self {
        let (sender, commands) = mpsc::sync_channel(2);
        let (event_sender, events) = mpsc::sync_channel(64);
        let control_pid = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let thread_control_pid = control_pid.clone();
        let join = thread::spawn(move || {
            let mut session: Option<crate::codex::CodexSession> = None;
            while let Ok(command) = commands.recv() {
                match command {
                    CodexWorkerCommand::Connect => {
                        let control_pid = thread_control_pid.clone();
                        let result = std::panic::catch_unwind(move || {
                            crate::codex::CodexSession::connect_with_control(control_pid)
                        })
                        .unwrap_or_else(|_| Err("Codex worker panicked".into()));
                        let event = match result {
                            Ok(connected) => {
                                session = Some(connected);
                                Event::CodexConnected(Ok(()))
                            }
                            Err(error) => Event::CodexConnected(Err(error)),
                        };
                        if event_sender.send(event).is_err() {
                            break;
                        }
                    }
                    CodexWorkerCommand::Turn {
                        operation,
                        revision,
                        mode,
                        statement,
                        source,
                        output,
                        question,
                        solved,
                    } => {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let session = session.as_mut().ok_or("Codex is not connected")?;
                            session.ask(crate::codex::InterviewRequest {
                                mode,
                                statement: &statement,
                                source: &source,
                                latest_output: &output,
                                question: &question,
                                source_revision: revision,
                                solved,
                            })
                        }))
                        .unwrap_or_else(|_| Err("Codex worker panicked".into()));
                        if result.is_err() {
                            session = None;
                        }
                        if event_sender
                            .send(Event::CodexFinished(operation, revision, mode, result))
                            .is_err()
                        {
                            break;
                        }
                    }
                    CodexWorkerCommand::Reset | CodexWorkerCommand::Cancel => session = None,
                    CodexWorkerCommand::Shutdown => break,
                }
            }
        });
        Self {
            sender,
            events,
            join: Some(join),
            control_pid,
        }
    }
    fn poll(&mut self) -> Vec<Event> {
        let mut result = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            result.push(event);
        }
        result
    }
    fn send(&self, command: CodexWorkerCommand) -> Result<(), String> {
        if matches!(
            command,
            CodexWorkerCommand::Cancel | CodexWorkerCommand::Reset
        ) {
            let pid = self.control_pid.load(std::sync::atomic::Ordering::SeqCst);
            if pid > 0 {
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
        }
        self.sender
            .try_send(command)
            .map_err(|_| "Codex worker is busy or stopped".into())
    }
    fn shutdown(mut self) {
        let pid = self.control_pid.load(std::sync::atomic::Ordering::SeqCst);
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
        let _ = self.sender.send(CodexWorkerCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct RunnerWorker {
    sender: SyncSender<WorkerCommand>,
    events: Receiver<Event>,
    join: Option<JoinHandle<()>>,
    active: Option<(
        crate::app::model::OperationId,
        u64,
        crate::app::RunIntent,
        CancellationToken,
    )>,
}
impl RunnerWorker {
    fn start(root: PathBuf, database_path: PathBuf) -> Self {
        let save_root = root.clone();
        let execute_database = database_path.clone();
        let record_root = root;
        let record_database = database_path;
        Self::start_with_services(RunnerServices {
            save: Arc::new(move |_, solution_path, source| {
                source::atomic_save(&save_root, solution_path, source)
            }),
            execute: Arc::new(move |plan, cancellation| {
                runner::execute(
                    plan,
                    &execute_database,
                    &ExecutionLimits::default(),
                    cancellation,
                    None,
                )
            }),
            record: Arc::new(move |plan, result| {
                let connection = crate::database::open_database(&record_database, &record_root)?;
                runner::record_execution(&connection, plan, result).map(|_| ())
            }),
        })
    }

    fn start_with_services(services: RunnerServices) -> Self {
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
                                            let result = (services.execute)(&plan, &cancellation)?;
                                            if intent == crate::app::RunIntent::Submit {
                                                (services.record)(&plan, &result)?;
                                            }
                                            Ok(result)
                                        })(),
                                    ),
                                }
                            }));
                        let (saved, result) = outcome
                            .unwrap_or_else(|_| (None, Err("runner worker panicked".into())));
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
        let cancellation = CancellationToken::new();
        self.sender
            .send(WorkerCommand::Run {
                operation,
                revision,
                intent,
                plan,
                source,
                write_source,
                cancellation: cancellation.clone(),
            })
            .map_err(|_| "runner worker stopped".to_string())?;
        self.active = Some((operation, revision, intent, cancellation));
        Ok(())
    }
    fn cancel(&mut self, operation: crate::app::model::OperationId) {
        if let Some((active, _, _, token)) = &self.active
            && *active == operation
        {
            token.cancel();
        }
    }
    fn leave(&mut self) {
        if let Some((_, _, _, token)) = &self.active {
            token.cancel();
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
        if let Some((_, _, _, token)) = &self.active {
            token.cancel();
        }
        let _ = self.sender.send(WorkerCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn apply_effects(
    state: &mut AppState,
    repository: &Repository,
    root: &Path,
    worker: &mut RunnerWorker,
    codex_worker: &mut CodexWorker,
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
                        pane: SolvePane::Editor,
                        output: "No test run yet".into(),
                        output_scroll: 0,
                        problem_scroll: 0,
                        running: None,
                        cancellation: None,
                        pending_save: None,
                        stale: false,
                        latest_run_revision: None,
                        quit_after_save: None,
                        discard_confirmation: None,
                        refresh_after_submit: false,
                    }))
                })();
                effects.extend(reduce(state, Event::SolveOpened(operation, result)));
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
            Effect::CancelRun { operation } => worker.cancel(operation),
            Effect::ConnectCodex => {
                if let Err(error) = codex_worker.send(CodexWorkerCommand::Connect) {
                    effects.extend(reduce(state, Event::CodexConnected(Err(error))));
                }
            }
            Effect::CodexTurn {
                operation,
                revision,
                mode,
                statement,
                source,
                output,
                question,
                solved,
            } => {
                if let Err(error) = codex_worker.send(CodexWorkerCommand::Turn {
                    operation,
                    revision,
                    mode,
                    statement,
                    source,
                    output,
                    question,
                    solved,
                }) {
                    effects.extend(reduce(
                        state,
                        Event::CodexFinished(operation, revision, mode, Err(error)),
                    ));
                }
            }
            Effect::CancelCodex => {
                let _ = codex_worker.send(CodexWorkerCommand::Cancel);
            }
            Effect::ResetCodex => {
                let _ = codex_worker.send(CodexWorkerCommand::Reset);
            }
            Effect::LeaveSolve => worker.leave(),
        }
    }
}

pub fn run(
    mut state: AppState,
    repository: Repository,
    requested_set: Option<String>,
    root: PathBuf,
    database_path: PathBuf,
) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("interview requires an interactive terminal".into());
    }
    let mut worker = RunnerWorker::start(root.clone(), database_path);
    let mut codex_worker = CodexWorker::start();
    let initial = requested_set.map_or(Event::Command(crate::app::Action::Reload), Event::OpenSet);
    let effects = reduce(&mut state, initial);
    apply_effects(
        &mut state,
        &repository,
        &root,
        &mut worker,
        &mut codex_worker,
        effects,
    );
    let result = (|| {
        let _guard = TerminalGuard::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal =
            Terminal::new(backend).map_err(|e| format!("cannot initialize terminal: {e}"))?;
        terminal
            .clear()
            .map_err(|e| format!("cannot clear terminal: {e}"))?;
        let mut needs_draw = true;
        while !state.quit {
            for event in worker.poll().into_iter().chain(codex_worker.poll()) {
                let effects = reduce(&mut state, event);
                apply_effects(
                    &mut state,
                    &repository,
                    &root,
                    &mut worker,
                    &mut codex_worker,
                    effects,
                );
                needs_draw = true
            }
            if needs_draw {
                terminal
                    .draw(|frame| render::render(frame, &state))
                    .map_err(|e| format!("cannot draw terminal: {e}"))?;
                needs_draw = false
            }
            if !event::poll(Duration::from_millis(50))
                .map_err(|e| format!("cannot poll terminal: {e}"))?
            {
                continue;
            }
            match event::read().map_err(|e| format!("cannot read terminal: {e}"))? {
                TerminalEvent::Key(key) => {
                    if let Some(action) = input::action_for_key(key, &mut state) {
                        let effects = reduce(&mut state, Event::Command(action));
                        apply_effects(
                            &mut state,
                            &repository,
                            &root,
                            &mut worker,
                            &mut codex_worker,
                            effects,
                        );
                        needs_draw = true
                    }
                }
                TerminalEvent::Resize(_, _) => needs_draw = true,
                TerminalEvent::Paste(text) => {
                    let effects = reduce(
                        &mut state,
                        Event::Command(crate::app::Action::Editor(
                            crate::app::EditorAction::Paste(text),
                        )),
                    );
                    apply_effects(
                        &mut state,
                        &repository,
                        &root,
                        &mut worker,
                        &mut codex_worker,
                        effects,
                    );
                    needs_draw = true
                }
                TerminalEvent::FocusGained | TerminalEvent::FocusLost | TerminalEvent::Mouse(_) => {
                }
            }
        }
        Ok(())
    })();
    worker.shutdown();
    codex_worker.shutdown();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RunIntent;
    use crate::app::model::OperationId;
    use crate::runner::{ExecutionPlan, ExecutionResult, Termination};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
                Ok(())
            }),
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
    fn worker_reports_save_and_record_failures() {
        let save_failure = RunnerServices {
            save: Arc::new(|_, _, _| Err("save failed".into())),
            execute: Arc::new(|_, _| panic!("execute must not run")),
            record: Arc::new(|_, _| panic!("record must not run")),
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
            record: Arc::new(|_, _| Ok(())),
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
            pane: SolvePane::Editor,
            output: String::new(),
            output_scroll: 0,
            problem_scroll: 0,
            running: Some((OperationId(3), 0, RunIntent::Test)),
            cancellation: None,
            pending_save: None,
            stale: false,
            latest_run_revision: None,
            quit_after_save: None,
            discard_confirmation: None,
            refresh_after_submit: false,
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
        worker.active = Some((OperationId(9), 4, RunIntent::Test, CancellationToken::new()));
        worker.sender.send(WorkerCommand::Shutdown).unwrap();
        assert!(worker.events.recv_timeout(Duration::from_secs(2)).is_err());
        assert!(
            matches!(worker.poll().as_slice(), [Event::RunFinished(OperationId(9), 4, RunIntent::Test, None, Err(error))] if error == "runner worker disconnected")
        );
        assert!(worker.active.is_none());
        assert!(worker.join.is_none());
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
            record: Arc::new(|_, _| Ok(())),
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
