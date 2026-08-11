use crate::app::Action;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn action_for_key(key: KeyEvent) -> Option<Action> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
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
    #[test]
    fn documented_keys_are_mapped() {
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::PreviousFocus)
        );
    }
}
