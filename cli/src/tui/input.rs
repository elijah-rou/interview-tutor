use crate::app::{Action, AppState, EditorAction, Screen};
use crate::editor::Mode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn action_for_key(key: KeyEvent, state: &mut AppState) -> Option<Action> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if state.screen != Screen::Solve {
        return browser_key(key);
    }
    let solve = state.solve.as_ref()?;
    let mode = solve.editor.mode;
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Cancel);
    }
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::SaveTest);
    }
    if key.code == KeyCode::F(5) {
        return Some(Action::SaveTest);
    }
    if key.code == KeyCode::F(9) {
        return Some(Action::Submit);
    }
    if solve.pane == crate::app::model::SolvePane::Interview {
        if state.codex.status == crate::app::model::CodexStatus::Disclosure {
            return match key.code {
                KeyCode::Enter | KeyCode::Char('y') => Some(Action::InterviewDisclosure(true)),
                KeyCode::Esc | KeyCode::Char('n') => Some(Action::InterviewDisclosure(false)),
                _ => None,
            };
        }
        if state.codex.composer_focused {
            return match key.code {
                KeyCode::Esc => Some(Action::InterviewEscape),
                KeyCode::Enter => Some(Action::InterviewSend),
                KeyCode::Backspace => Some(Action::InterviewBackspace),
                KeyCode::Char(character)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    Some(Action::InterviewChar(character))
                }
                _ => None,
            };
        }
        if state.leader_pending {
            state.leader_pending = false;
            return match key.code {
                KeyCode::Char('h') => Some(Action::Hint),
                KeyCode::Char('r') => Some(Action::ResetInterview),
                _ => None,
            };
        }
        if key.code == KeyCode::Char(' ') {
            state.leader_pending = true;
            return None;
        }
        if key.code == KeyCode::Char('i') {
            return Some(Action::InterviewFocus);
        }
    }
    if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT)
        || key.code == KeyCode::BackTab
    {
        return Some(Action::PreviousFocus);
    }
    if key.code == KeyCode::Tab {
        return Some(Action::NextFocus);
    }
    if solve.pane != crate::app::model::SolvePane::Editor {
        return match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            _ => None,
        };
    }
    match mode {
        Mode::Insert => match key.code {
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            KeyCode::Left => Some(Action::Editor(EditorAction::Left)),
            KeyCode::Right => Some(Action::Editor(EditorAction::Right)),
            KeyCode::Up => Some(Action::Editor(EditorAction::Up)),
            KeyCode::Down => Some(Action::Editor(EditorAction::Down)),
            KeyCode::Backspace => Some(Action::Editor(EditorAction::Backspace)),
            KeyCode::Delete => Some(Action::Editor(EditorAction::Delete)),
            KeyCode::Enter => Some(Action::Editor(EditorAction::Enter)),
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(Action::Editor(EditorAction::Insert(c)))
            }
            _ => None,
        },
        Mode::Command => match key.code {
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            KeyCode::Enter => Some(Action::Editor(EditorAction::ExecuteCommand)),
            KeyCode::Backspace => Some(Action::Editor(EditorAction::CommandBackspace)),
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(Action::Editor(EditorAction::CommandChar(c)))
            }
            _ => None,
        },
        Mode::Normal => {
            if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Some(Action::Editor(EditorAction::Redo));
            }
            if state.leader_pending {
                state.leader_pending = false;
                return match key.code {
                    KeyCode::Char('q') => Some(Action::Quit),
                    KeyCode::Char('b') => Some(Action::Back),
                    _ => None,
                };
            }
            if key.code == KeyCode::Char(' ') {
                state.leader_pending = true;
                return None;
            }
            match key.code {
                KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
                KeyCode::Up => Some(Action::Editor(EditorAction::Up)),
                KeyCode::Down => Some(Action::Editor(EditorAction::Down)),
                KeyCode::Left => Some(Action::Editor(EditorAction::Left)),
                KeyCode::Right => Some(Action::Editor(EditorAction::Right)),
                KeyCode::Char(c) => Some(Action::Editor(EditorAction::Normal(c))),
                _ => None,
            }
        }
    }
}
fn browser_key(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => Some(Action::Quit),
        (KeyCode::Char('?'), _) => Some(Action::Help),
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => Some(Action::Down),
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => Some(Action::Up),
        (KeyCode::Enter, _) => Some(Action::Open),
        (KeyCode::Esc | KeyCode::Backspace, _) => Some(Action::Back),
        (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => Some(Action::PreviousFocus),
        (KeyCode::Tab, _) => Some(Action::NextFocus),
        (KeyCode::Char('l'), _) => Some(Action::CycleLanguage),
        (KeyCode::Char('r'), _) => Some(Action::Reload),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::{SolvePane, SolveSession};
    use crate::database::EnabledLanguage;
    use crate::editor::EditorDocument;
    use crate::runner::ExecutionPlan;
    use std::path::PathBuf;

    fn solve_state() -> AppState {
        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: String::new(),
            language: "python".into(),
            plan: ExecutionPlan {
                root: PathBuf::from("/tmp"),
                language: "python".into(),
                problem_slug: "p".into(),
                set_slug: None,
                runner_path: PathBuf::from("/tmp/run"),
                solution_path: PathBuf::from("/tmp/p.py"),
            },
            editor: EditorDocument::new("x".into()).unwrap(),
            pane: SolvePane::Editor,
            output: String::new(),
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
        });
        state
    }

    #[test]
    fn solve_run_leader_and_pane_keys_are_mapped() {
        let mut state = solve_state();
        for (key, expected) in [
            (
                KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
                Action::SaveTest,
            ),
            (
                KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
                Action::Submit,
            ),
            (
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                Action::SaveTest,
            ),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Action::Cancel,
            ),
            (
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                Action::NextFocus,
            ),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                Action::PreviousFocus,
            ),
        ] {
            assert_eq!(action_for_key(key, &mut state), Some(expected));
        }
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Back)
        );
        state.solve.as_mut().unwrap().pane = SolvePane::Output;
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state),
            Some(Action::Down)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Down)
        );
    }

    #[test]
    fn documented_keys_are_mapped() {
        let mut state = AppState::new(
            vec![EnabledLanguage {
                slug: "python".into(),
                display_name: "Python".into(),
                runner_path: "python/run".into(),
            }],
            0,
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Quit)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                &mut state
            ),
            Some(Action::PreviousFocus)
        );
    }
}
