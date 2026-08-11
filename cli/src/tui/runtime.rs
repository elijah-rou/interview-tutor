use crate::app::{AppState, Effect, Event, Repository, reduce};
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
use std::time::Duration;

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|error| format!("cannot enable terminal raw mode: {error}"))?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
            return Err(format!("cannot enter terminal screen: {error}"));
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

fn apply_effects(state: &mut AppState, repository: &Repository, mut effects: Vec<Effect>) {
    while let Some(effect) = effects.pop() {
        match effect {
            Effect::Load {
                operation,
                set_slug,
                problem_id,
                language_slug,
            } => {
                let result = repository
                    .load(set_slug.as_deref(), problem_id, &language_slug)
                    .map(Box::new);
                effects.extend(reduce(state, Event::Loaded(operation, result)));
            }
        }
    }
}

pub fn run(
    mut state: AppState,
    repository: Repository,
    requested_set: Option<String>,
) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("interview requires an interactive terminal".to_string());
    }
    let initial = requested_set.map_or(Event::Command(crate::app::Action::Reload), Event::OpenSet);
    let effects = reduce(&mut state, initial);
    apply_effects(&mut state, &repository, effects);

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal =
        Terminal::new(backend).map_err(|error| format!("cannot initialize terminal: {error}"))?;
    terminal
        .clear()
        .map_err(|error| format!("cannot clear terminal: {error}"))?;
    while !state.quit {
        terminal
            .draw(|frame| render::render(frame, &state))
            .map_err(|error| format!("cannot draw terminal: {error}"))?;
        if !event::poll(Duration::from_millis(250))
            .map_err(|error| format!("cannot poll terminal: {error}"))?
        {
            continue;
        }
        match event::read().map_err(|error| format!("cannot read terminal: {error}"))? {
            TerminalEvent::Key(key) => {
                if let Some(action) = input::action_for_key(key) {
                    let effects = reduce(&mut state, Event::Command(action));
                    apply_effects(&mut state, &repository, effects);
                }
            }
            TerminalEvent::Resize(_, _) => {}
            TerminalEvent::FocusGained
            | TerminalEvent::FocusLost
            | TerminalEvent::Paste(_)
            | TerminalEvent::Mouse(_) => {}
        }
    }
    Ok(())
}
