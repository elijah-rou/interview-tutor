use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn encode_key(key: KeyEvent) -> Result<Option<String>, String> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Ok(None);
    }
    if key
        .modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::HYPER)
    {
        return Err("Neovim input does not support Super/Hyper modifiers".into());
    }
    let base = match key.code {
        KeyCode::Char(character)
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            return Ok(Some(modified(&character.to_string(), key.modifiers)));
        }
        KeyCode::Char('<') => "LT".to_string(),
        KeyCode::Char(character) => return Ok(Some(character.to_string())),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Enter => "CR".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "S-Tab".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(number) if (1..=37).contains(&number) => format!("F{number}"),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_)
        | KeyCode::F(_) => {
            return Err("unsupported Neovim key".into());
        }
    };
    let modifiers =
        key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
    Ok(Some(if modifiers.is_empty() || base == "S-Tab" {
        format!("<{base}>")
    } else {
        modified(&base, modifiers)
    }))
}

fn modified(base: &str, modifiers: KeyModifiers) -> String {
    let mut prefix = String::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if modifiers.contains(KeyModifiers::ALT) {
        prefix.push_str("M-");
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        prefix.push_str("S-");
    }
    format!("<{prefix}{base}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_literal_special_unicode_and_modifiers() {
        for (key, expected) in [
            (KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT), "A"),
            (
                KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE),
                "<LT>",
            ),
            (KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE), "界"),
            (
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                "<C-r>",
            ),
            (KeyEvent::new(KeyCode::Left, KeyModifiers::ALT), "<M-Left>"),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                "<S-Tab>",
            ),
            (KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE), "<F9>"),
        ] {
            assert_eq!(encode_key(key).unwrap().as_deref(), Some(expected));
        }
    }
}
