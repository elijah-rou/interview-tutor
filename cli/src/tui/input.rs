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
    let mode = state.solve.as_ref()?.editor.mode;
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
    if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT)
        || key.code == KeyCode::BackTab
    {
        return Some(Action::PreviousFocus);
    }
    if key.code == KeyCode::Tab {
        return Some(Action::NextFocus);
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
                    _ => None,
                };
            }
            if key.code == KeyCode::Char(' ') {
                state.leader_pending = true;
                return None;
            }
            match key.code {
                KeyCode::Esc => Some(Action::Back),
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
    use crate::database::EnabledLanguage;
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
