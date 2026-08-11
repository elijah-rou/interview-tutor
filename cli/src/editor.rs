use ratatui::style::{Color, Style};
use std::collections::VecDeque;

pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_DOCUMENT_LINES: usize = 100_000;
pub const MAX_UNDO_SNAPSHOTS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorCommand {
    Write,
    WriteQuit,
    Quit,
    Submit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightKind {
    Plain,
    Keyword,
    String,
    Comment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
    row: usize,
    column: usize,
    generation: u64,
}

#[derive(Clone, Debug)]
pub struct EditorDocument {
    text: String,
    pub mode: Mode,
    pub row: usize,
    pub column: usize,
    pub viewport_row: usize,
    pub viewport_column: usize,
    pub generation: u64,
    pub saved_generation: u64,
    pub command_buffer: String,
    pub error: Option<String>,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    pending_g: bool,
    pending_d: bool,
}

impl EditorDocument {
    pub fn new(text: String) -> Result<Self, String> {
        validate_text(&text)?;
        Ok(Self {
            text,
            mode: Mode::Normal,
            row: 0,
            column: 0,
            viewport_row: 0,
            viewport_column: 0,
            generation: 0,
            saved_generation: 0,
            command_buffer: String::new(),
            error: None,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            pending_g: false,
            pending_d: false,
        })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn dirty(&self) -> bool {
        self.generation != self.saved_generation
    }
    pub fn mark_saved(&mut self, generation: u64) {
        if generation == self.generation {
            self.saved_generation = generation;
        }
    }
    pub fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }
    pub fn line(&self, row: usize) -> &str {
        self.text.split('\n').nth(row).unwrap_or("")
    }
    fn line_chars(&self) -> usize {
        self.line(self.row).chars().count()
    }
    fn snapshot(&mut self) {
        if self.undo.len() == MAX_UNDO_SNAPSHOTS {
            self.undo.pop_front();
        }
        self.undo.push_back(Snapshot {
            text: self.text.clone(),
            row: self.row,
            column: self.column,
            generation: self.generation,
        });
        self.redo.clear();
    }
    fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.row = snapshot.row;
        self.column = snapshot.column;
        self.generation = snapshot.generation;
    }
    fn mutate(
        &mut self,
        operation: impl FnOnce(&mut String, &mut usize, &mut usize),
    ) -> Result<(), String> {
        let old = Snapshot {
            text: self.text.clone(),
            row: self.row,
            column: self.column,
            generation: self.generation,
        };
        let mut candidate = self.text.clone();
        let mut row = self.row;
        let mut column = self.column;
        operation(&mut candidate, &mut row, &mut column);
        validate_text(&candidate)?;
        if candidate != self.text {
            self.snapshot();
            self.text = candidate;
            self.row = row;
            self.column = column;
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("editor generation overflow")?;
        } else {
            self.set_snapshot(old);
        }
        self.clamp();
        Ok(())
    }
    fn clamp(&mut self) {
        self.row = self.row.min(self.line_count().saturating_sub(1));
        self.column = self.column.min(self.line_chars());
    }
    fn byte_offset(&self) -> usize {
        offset(&self.text, self.row, self.column)
    }
    pub fn insert_char(&mut self, character: char) -> Result<(), String> {
        let at = self.byte_offset();
        self.mutate(|text, _, column| {
            text.insert(at, character);
            *column += 1;
        })
    }
    pub fn enter(&mut self) -> Result<(), String> {
        let at = self.byte_offset();
        self.mutate(|text, row, column| {
            text.insert(at, '\n');
            *row += 1;
            *column = 0;
        })
    }
    pub fn backspace(&mut self) -> Result<(), String> {
        let at = self.byte_offset();
        if at == 0 {
            return Ok(());
        }
        self.mutate(|text, row, column| {
            let previous = text[..at]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            let newline = text[previous..at].contains('\n');
            text.replace_range(previous..at, "");
            if newline {
                *row = row.saturating_sub(1);
                *column = text[..previous]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .count()
            } else {
                *column = column.saturating_sub(1)
            }
        })
    }
    pub fn delete(&mut self) -> Result<(), String> {
        let at = self.byte_offset();
        if at == self.text.len() {
            return Ok(());
        }
        self.mutate(|text, _, _| {
            let end = at + text[at..].chars().next().unwrap().len_utf8();
            text.replace_range(at..end, "");
        })
    }
    fn delete_line(&mut self) -> Result<(), String> {
        let row = self.row;
        self.mutate(|text, current, column| {
            let mut lines = text.split('\n').map(str::to_string).collect::<Vec<_>>();
            if lines.len() == 1 {
                lines[0].clear()
            } else {
                lines.remove(row);
            }
            *text = lines.join("\n");
            *current = (*current).min(lines.len().saturating_sub(1));
            *column = 0;
        })
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop_back() {
            if self.redo.len() == MAX_UNDO_SNAPSHOTS {
                self.redo.pop_front();
            }
            self.redo.push_back(Snapshot {
                text: self.text.clone(),
                row: self.row,
                column: self.column,
                generation: self.generation,
            });
            self.set_snapshot(previous);
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop_back() {
            if self.undo.len() == MAX_UNDO_SNAPSHOTS {
                self.undo.pop_front();
            }
            self.undo.push_back(Snapshot {
                text: self.text.clone(),
                row: self.row,
                column: self.column,
                generation: self.generation,
            });
            self.set_snapshot(next);
        }
    }
    pub fn normal(&mut self, key: char) -> Result<(), String> {
        self.error = None;
        if self.pending_g {
            self.pending_g = false;
            if key == 'g' {
                self.row = 0;
                self.column = 0;
                return Ok(());
            }
        }
        if self.pending_d {
            self.pending_d = false;
            if key == 'd' {
                return self.delete_line();
            }
        }
        match key {
            'h' => self.column = self.column.saturating_sub(1),
            'l' => self.column = (self.column + 1).min(self.line_chars()),
            'j' => self.row = (self.row + 1).min(self.line_count() - 1),
            'k' => self.row = self.row.saturating_sub(1),
            '0' => self.column = 0,
            '$' => self.column = self.line_chars(),
            'g' => self.pending_g = true,
            'G' => {
                self.row = self.line_count() - 1;
                self.column = 0
            }
            'i' => self.mode = Mode::Insert,
            'a' => {
                self.column = (self.column + 1).min(self.line_chars());
                self.mode = Mode::Insert
            }
            'o' => {
                self.column = self.line_chars();
                self.enter()?;
                self.mode = Mode::Insert
            }
            'O' => {
                self.column = 0;
                self.enter()?;
                self.row = self.row.saturating_sub(1);
                self.mode = Mode::Insert
            }
            'x' => self.delete()?,
            'd' => self.pending_d = true,
            'u' => self.undo(),
            'w' => self.word_forward(),
            'b' => self.word_backward(),
            ':' => {
                self.mode = Mode::Command;
                self.command_buffer.clear()
            }
            _ => self.error = Some(format!("unsupported normal command: {key}")),
        }
        self.clamp();
        Ok(())
    }
    fn word_forward(&mut self) {
        let chars = self.text.chars().collect::<Vec<_>>();
        let mut index = self.text[..self.byte_offset()].chars().count();
        while index < chars.len() && chars[index].is_alphanumeric() {
            index += 1;
        }
        while index < chars.len() && !chars[index].is_alphanumeric() {
            index += 1;
        }
        self.set_from_char_index(index.min(chars.len()));
    }
    fn word_backward(&mut self) {
        let chars = self.text.chars().collect::<Vec<_>>();
        let mut index = self.text[..self.byte_offset()]
            .chars()
            .count()
            .saturating_sub(1);
        while index > 0 && !chars[index].is_alphanumeric() {
            index -= 1;
        }
        while index > 0 && chars[index - 1].is_alphanumeric() {
            index -= 1;
        }
        self.set_from_char_index(index);
    }
    fn set_from_char_index(&mut self, index: usize) {
        let prefix = self.text.chars().take(index).collect::<String>();
        self.row = prefix.matches('\n').count();
        self.column = prefix.rsplit('\n').next().unwrap_or("").chars().count();
    }
    pub fn command_char(&mut self, character: char) {
        if self.command_buffer.len() < 256 {
            self.command_buffer.push(character);
        } else {
            self.error = Some("command exceeds 256 bytes".into());
        }
    }
    pub fn execute_command(&mut self) -> Result<EditorCommand, String> {
        let command = match self.command_buffer.as_str() {
            "w" => EditorCommand::Write,
            "wq" => EditorCommand::WriteQuit,
            "q" => EditorCommand::Quit,
            "submit" => EditorCommand::Submit,
            other => return Err(format!("unsupported command: :{other}")),
        };
        self.mode = Mode::Normal;
        self.command_buffer.clear();
        Ok(command)
    }
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"));
    }
    let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
    if lines > MAX_DOCUMENT_LINES {
        return Err(format!("document exceeds {MAX_DOCUMENT_LINES} lines"));
    }
    Ok(())
}
fn offset(text: &str, row: usize, column: usize) -> usize {
    let start = text
        .split_inclusive('\n')
        .take(row)
        .map(str::len)
        .sum::<usize>();
    start
        + text[start..]
            .split('\n')
            .next()
            .unwrap_or("")
            .char_indices()
            .nth(column)
            .map_or_else(
                || text[start..].split('\n').next().unwrap_or("").len(),
                |(i, _)| i,
            )
}

pub fn highlight_line(language: &str, line: &str) -> Vec<HighlightSpan> {
    if !matches!(language, "rust" | "python") {
        return vec![HighlightSpan {
            start: 0,
            end: line.len(),
            kind: HighlightKind::Plain,
        }];
    }
    let comment = if language == "rust" { "//" } else { "#" };
    let keywords: &[&str] = if language == "rust" {
        &[
            "fn", "let", "mut", "pub", "impl", "struct", "enum", "match", "use", "return", "self",
            "Self",
        ]
    } else {
        &[
            "def", "class", "if", "elif", "else", "for", "while", "in", "return", "import", "from",
            "True", "False", "None", "self",
        ]
    };
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if line[i..].starts_with(comment) {
            spans.push(HighlightSpan {
                start: i,
                end: line.len(),
                kind: HighlightKind::Comment,
            });
            break;
        }
        let c = line[i..].chars().next().unwrap();
        if c == '\'' || c == '"' {
            let quote = c;
            i += c.len_utf8();
            let start = i - c.len_utf8();
            while i < bytes.len() {
                let next = line[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == quote && bytes.get(i.saturating_sub(2)) != Some(&b'\\') {
                    break;
                }
            }
            spans.push(HighlightSpan {
                start,
                end: i,
                kind: HighlightKind::String,
            });
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            i += c.len_utf8();
            while i < bytes.len() {
                let next = line[i..].chars().next().unwrap();
                if !(next.is_alphanumeric() || next == '_') {
                    break;
                }
                i += next.len_utf8()
            }
            let word = &line[start..i];
            spans.push(HighlightSpan {
                start,
                end: i,
                kind: if keywords.contains(&word) {
                    HighlightKind::Keyword
                } else {
                    HighlightKind::Plain
                },
            });
            continue;
        }
        let start = i;
        i += c.len_utf8();
        spans.push(HighlightSpan {
            start,
            end: i,
            kind: HighlightKind::Plain,
        });
    }
    spans
}
pub fn highlight_style(kind: HighlightKind) -> Style {
    match kind {
        HighlightKind::Plain => Style::default(),
        HighlightKind::Keyword => Style::default().fg(Color::Magenta),
        HighlightKind::String => Style::default().fg(Color::Green),
        HighlightKind::Comment => Style::default().fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_edit_undo_and_dirty() {
        let mut d = EditorDocument::new("ab\n界".into()).unwrap();
        d.normal('G').unwrap();
        d.normal('$').unwrap();
        d.normal('i').unwrap();
        d.insert_char('🦀').unwrap();
        assert_eq!(d.text(), "ab\n界🦀");
        assert!(d.dirty());
        d.mode = Mode::Normal;
        d.undo();
        assert_eq!(d.text(), "ab\n界");
        d.redo();
        assert_eq!(d.text(), "ab\n界🦀");
        d.mark_saved(d.generation);
        assert!(!d.dirty());
    }
    #[test]
    fn motions_edits_and_commands() {
        let mut d = EditorDocument::new("one two\nthree".into()).unwrap();
        d.normal('w').unwrap();
        d.normal('x').unwrap();
        assert_eq!(d.text(), "one wo\nthree");
        d.normal('d').unwrap();
        d.normal('d').unwrap();
        assert_eq!(d.text(), "three");
        d.normal(':').unwrap();
        for c in "submit".chars() {
            d.command_char(c)
        }
        assert_eq!(d.execute_command().unwrap(), EditorCommand::Submit);
    }
    #[test]
    fn bounds_and_highlights() {
        assert!(EditorDocument::new("x".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
        for language in ["rust", "python"] {
            let line = if language == "rust" {
                "fn x() { let s = \"v\"; // c"
            } else {
                "def x(): return \"v\" # c"
            };
            let kinds = highlight_line(language, line)
                .into_iter()
                .map(|s| s.kind)
                .collect::<Vec<_>>();
            assert!(kinds.contains(&HighlightKind::Keyword));
            assert!(kinds.contains(&HighlightKind::String));
            assert!(kinds.contains(&HighlightKind::Comment));
        }
        assert_eq!(highlight_line("text", "abc")[0].kind, HighlightKind::Plain);
    }
}
