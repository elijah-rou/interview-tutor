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
                                    source::atomic_save(&root, &plan.solution_path, &source)
                                } else {
                                    Ok(())
                                };
                                match save_result {
                                    Err(error) => (None, Err(error)),
                                    Ok(()) => (
                                        Some(source_for_save),
                                        (|| {
                                            let result = runner::execute(
                                                &plan,
                                                &database_path,
                                                &ExecutionLimits::default(),
                                                &cancellation,
                                                None,
                                            )?;
                                            if intent == crate::app::RunIntent::Submit {
                                                let connection = crate::database::open_database(
                                                    &database_path,
                                                    &root,
                                                )?;
                                                runner::record_execution(
                                                    &connection,
                                                    &plan,
                                                    &result,
                                                )?;
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
                        quit_after_save: None,
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
    let initial = requested_set.map_or(Event::Command(crate::app::Action::Reload), Event::OpenSet);
    let effects = reduce(&mut state, initial);
    apply_effects(&mut state, &repository, &root, &mut worker, effects);
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
            for event in worker.poll() {
                let effects = reduce(&mut state, event);
                apply_effects(&mut state, &repository, &root, &mut worker, effects);
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
                        apply_effects(&mut state, &repository, &root, &mut worker, effects);
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
                    apply_effects(&mut state, &repository, &root, &mut worker, effects);
                    needs_draw = true
                }
                TerminalEvent::FocusGained | TerminalEvent::FocusLost | TerminalEvent::Mouse(_) => {
                }
            }
        }
        Ok(())
    })();
    worker.shutdown();
    result
}
