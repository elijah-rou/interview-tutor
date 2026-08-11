use ratatui::style::{Color, Style};
use std::collections::VecDeque;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_DOCUMENT_LINES: usize = 100_000;
pub const MAX_UNDO_SNAPSHOTS: usize = 32;
pub const MAX_COMMAND_BYTES: usize = 256;

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
}

#[derive(Clone, Debug)]
pub struct EditorDocument {
    text: String,
    saved_text: String,
    pub mode: Mode,
    pub row: usize,
    pub column: usize,
    pub viewport_row: usize,
    pub viewport_column: usize,
    /// Monotonic content identity. Revisions are never restored from undo history.
    pub revision: u64,
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
            saved_text: text.clone(),
            text,
            mode: Mode::Normal,
            row: 0,
            column: 0,
            viewport_row: 0,
            viewport_column: 0,
            revision: 0,
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
        self.text != self.saved_text
    }

    pub fn mark_saved(&mut self, revision: u64, saved_text: &str) {
        if revision <= self.revision {
            self.saved_text = saved_text.to_string();
        }
    }

    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }
    pub fn line(&self, row: usize) -> &str {
        self.text.split('\n').nth(row).unwrap_or("")
    }
    fn line_graphemes(&self) -> usize {
        self.line(self.row).graphemes(true).count()
    }

    fn current_snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            row: self.row,
            column: self.column,
        }
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo.len() == MAX_UNDO_SNAPSHOTS {
            self.undo.pop_front();
        }
        self.undo.push_back(snapshot);
        self.redo.clear();
    }

    fn install_snapshot(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.row = snapshot.row;
        self.column = snapshot.column;
    }

    fn next_revision(&self) -> Result<u64, String> {
        self.revision
            .checked_add(1)
            .ok_or_else(|| "editor revision overflow".into())
    }

    fn mutate(
        &mut self,
        operation: impl FnOnce(&mut String, &mut usize, &mut usize),
    ) -> Result<(), String> {
        let old = self.current_snapshot();
        let mut candidate = self.text.clone();
        let mut row = self.row;
        let mut column = self.column;
        operation(&mut candidate, &mut row, &mut column);
        validate_text(&candidate)?;
        if candidate == self.text {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.push_undo(old);
        self.text = candidate;
        self.row = row;
        self.column = column;
        self.revision = revision;
        self.clamp();
        Ok(())
    }

    fn clamp(&mut self) {
        self.row = self.row.min(self.line_count().saturating_sub(1));
        let count = self.line_graphemes();
        self.column = match self.mode {
            Mode::Insert => self.column.min(count),
            Mode::Normal | Mode::Command => self.column.min(count.saturating_sub(1)),
        };
    }

    fn byte_offset(&self) -> usize {
        offset(&self.text, self.row, self.column)
    }

    pub fn insert_char(&mut self, character: char) -> Result<(), String> {
        let mut encoded = [0_u8; 4];
        self.insert_text(character.encode_utf8(&mut encoded))
    }

    pub fn insert_text(&mut self, inserted: &str) -> Result<(), String> {
        if inserted.is_empty() {
            self.error = None;
            return Ok(());
        }
        let at = self.byte_offset();
        let endpoint = at
            .checked_add(inserted.len())
            .ok_or_else(|| "document position overflow".to_string())?;
        let result = self.mutate(|text, row, column| {
            text.insert_str(at, inserted);
            let prefix = &text[..endpoint];
            *row = prefix.bytes().filter(|byte| *byte == b'\n').count();
            let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
            let line_end = text[endpoint..]
                .find('\n')
                .map_or(text.len(), |index| endpoint + index);
            let line = &text[line_start..line_end];
            let endpoint_in_line = endpoint - line_start;
            *column = line
                .grapheme_indices(true)
                .take_while(|(index, grapheme)| index + grapheme.len() <= endpoint_in_line)
                .count();
            if line.grapheme_indices(true).any(|(index, grapheme)| {
                index < endpoint_in_line && endpoint_in_line < index + grapheme.len()
            }) {
                *column += 1;
            }
        });
        if result.is_ok() {
            self.error = None;
        }
        result
    }

    pub fn command_text(&mut self, text: &str) -> Result<(), String> {
        let Some(length) = self.command_buffer.len().checked_add(text.len()) else {
            return Err("command exceeds 256 bytes".into());
        };
        if length > MAX_COMMAND_BYTES {
            return Err("command exceeds 256 bytes".into());
        }
        self.command_buffer.push_str(text);
        self.error = None;
        Ok(())
    }

    pub fn enter(&mut self) -> Result<(), String> {
        self.insert_text("\n")
    }

    pub fn backspace(&mut self) -> Result<(), String> {
        let at = self.byte_offset();
        if at == 0 {
            self.error = None;
            return Ok(());
        }
        self.mutate(|text, row, column| {
            let previous = text[..at]
                .grapheme_indices(true)
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            let newline = &text[previous..at] == "\n";
            text.replace_range(previous..at, "");
            if newline {
                *row = row.saturating_sub(1);
                *column = text[..previous]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .graphemes(true)
                    .count();
            } else {
                *column = column.saturating_sub(1);
            }
        })?;
        self.error = None;
        Ok(())
    }

    pub fn delete(&mut self) -> Result<(), String> {
        let at = self.byte_offset();
        let line_end = at + self.text[at..].find('\n').unwrap_or(self.text.len() - at);
        if at >= line_end {
            self.error = None;
            return Ok(());
        }
        self.mutate(|text, _, _| {
            let end = at
                + text[at..line_end]
                    .graphemes(true)
                    .next()
                    .expect("cursor addresses grapheme")
                    .len();
            text.replace_range(at..end, "");
        })?;
        self.error = None;
        Ok(())
    }

    fn delete_line(&mut self) -> Result<(), String> {
        let row = self.row;
        self.mutate(|text, current, column| {
            let mut lines = text.split('\n').map(str::to_string).collect::<Vec<_>>();
            if lines.len() == 1 {
                lines[0].clear();
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
            let Ok(revision) = self.next_revision() else {
                self.error = Some("editor revision overflow".into());
                return;
            };
            if self.redo.len() == MAX_UNDO_SNAPSHOTS {
                self.redo.pop_front();
            }
            self.redo.push_back(self.current_snapshot());
            self.install_snapshot(previous);
            self.revision = revision;
            self.clamp();
            self.error = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop_back() {
            let Ok(revision) = self.next_revision() else {
                self.error = Some("editor revision overflow".into());
                return;
            };
            if self.undo.len() == MAX_UNDO_SNAPSHOTS {
                self.undo.pop_front();
            }
            self.undo.push_back(self.current_snapshot());
            self.install_snapshot(next);
            self.revision = revision;
            self.clamp();
            self.error = None;
        }
    }

    pub fn escape(&mut self) {
        if self.mode == Mode::Insert {
            self.column = self.column.saturating_sub(1);
        }
        self.mode = Mode::Normal;
        self.command_buffer.clear();
        self.pending_d = false;
        self.pending_g = false;
        self.error = None;
        self.clamp();
    }

    pub fn move_left(&mut self) {
        self.column = self.column.saturating_sub(1);
        self.error = None;
    }

    pub fn move_right(&mut self) {
        let maximum = match self.mode {
            Mode::Insert => self.line_graphemes(),
            Mode::Normal | Mode::Command => self.line_graphemes().saturating_sub(1),
        };
        self.column = self.column.saturating_add(1).min(maximum);
        self.error = None;
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
            'l' => self.column = (self.column + 1).min(self.line_graphemes().saturating_sub(1)),
            'j' => self.row = (self.row + 1).min(self.line_count() - 1),
            'k' => self.row = self.row.saturating_sub(1),
            '0' => self.column = 0,
            '$' => self.column = self.line_graphemes().saturating_sub(1),
            'g' => self.pending_g = true,
            'G' => {
                self.row = self.line_count() - 1;
                self.column = 0;
            }
            'i' => self.mode = Mode::Insert,
            'a' => {
                self.column = (self.column + 1).min(self.line_graphemes());
                self.mode = Mode::Insert;
            }
            'o' => {
                self.column = self.line_graphemes();
                self.mode = Mode::Insert;
                self.enter()?;
            }
            'O' => {
                self.column = 0;
                self.mode = Mode::Insert;
                self.enter()?;
                self.row = self.row.saturating_sub(1);
            }
            'x' => self.delete()?,
            'd' => self.pending_d = true,
            'u' => self.undo(),
            'w' => self.word_forward(),
            'b' => self.word_backward(),
            ':' => {
                self.mode = Mode::Command;
                self.command_buffer.clear();
            }
            _ => self.error = Some(format!("unsupported normal command: {key}")),
        }
        self.clamp();
        Ok(())
    }

    fn word_forward(&mut self) {
        let graphemes = self.text.graphemes(true).collect::<Vec<_>>();
        let mut index = self.text[..self.byte_offset()].graphemes(true).count();
        while index < graphemes.len() && word_grapheme(graphemes[index]) {
            index += 1;
        }
        while index < graphemes.len() && !word_grapheme(graphemes[index]) {
            index += 1;
        }
        self.set_from_grapheme_index(index.min(graphemes.len().saturating_sub(1)));
    }

    fn word_backward(&mut self) {
        let graphemes = self.text.graphemes(true).collect::<Vec<_>>();
        if graphemes.is_empty() {
            return;
        }
        let mut index = self.text[..self.byte_offset()]
            .graphemes(true)
            .count()
            .saturating_sub(1);
        while index > 0 && !word_grapheme(graphemes[index]) {
            index -= 1;
        }
        while index > 0 && word_grapheme(graphemes[index - 1]) {
            index -= 1;
        }
        self.set_from_grapheme_index(index);
    }

    fn set_from_grapheme_index(&mut self, index: usize) {
        let byte = self
            .text
            .grapheme_indices(true)
            .nth(index)
            .map_or(self.text.len(), |(at, _)| at);
        let prefix = &self.text[..byte];
        self.row = prefix.bytes().filter(|byte| *byte == b'\n').count();
        self.column = prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .graphemes(true)
            .count();
        self.clamp();
    }

    pub fn command_char(&mut self, character: char) {
        if let Err(error) = self.command_text(&character.to_string()) {
            self.error = Some(error);
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
        self.escape();
        Ok(command)
    }
}

fn word_grapheme(grapheme: &str) -> bool {
    grapheme == "_" || grapheme.chars().next().is_some_and(char::is_alphanumeric)
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"));
    }
    let lines = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    if lines > MAX_DOCUMENT_LINES {
        return Err(format!("document exceeds {MAX_DOCUMENT_LINES} lines"));
    }
    Ok(())
}

pub fn offset(text: &str, row: usize, column: usize) -> usize {
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
            .grapheme_indices(true)
            .nth(column)
            .map_or_else(
                || text[start..].split('\n').next().unwrap_or("").len(),
                |(index, _)| index,
            )
}

fn push_span(spans: &mut Vec<HighlightSpan>, start: usize, end: usize, kind: HighlightKind) {
    if start == end {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.end == start
        && last.kind == kind
    {
        last.end = end;
        return;
    }
    spans.push(HighlightSpan { start, end, kind });
}

/// A deliberately lexical, single-line highlighter. It does not parse raw strings or nested syntax.
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
    let mut index = 0;
    while index < bytes.len() {
        if line[index..].starts_with(comment) {
            push_span(&mut spans, index, line.len(), HighlightKind::Comment);
            break;
        }
        let character = line[index..]
            .chars()
            .next()
            .expect("valid character boundary");
        if character == '\'' || character == '"' {
            let quote = character;
            let start = index;
            index += character.len_utf8();
            let mut escaped = false;
            while index < bytes.len() {
                let next = line[index..]
                    .chars()
                    .next()
                    .expect("valid character boundary");
                index += next.len_utf8();
                if next == quote && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
                if next != '\\' {
                    escaped = false;
                }
            }
            push_span(&mut spans, start, index, HighlightKind::String);
        } else if character.is_alphabetic() || character == '_' {
            let start = index;
            index += character.len_utf8();
            while index < bytes.len() {
                let next = line[index..]
                    .chars()
                    .next()
                    .expect("valid character boundary");
                if !(next.is_alphanumeric() || next == '_') {
                    break;
                }
                index += next.len_utf8();
            }
            let kind = if keywords.contains(&&line[start..index]) {
                HighlightKind::Keyword
            } else {
                HighlightKind::Plain
            };
            push_span(&mut spans, start, index, kind);
        } else {
            let start = index;
            index += character.len_utf8();
            push_span(&mut spans, start, index, HighlightKind::Plain);
        }
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
    fn revisions_never_repeat_and_saved_bytes_define_dirty() {
        let mut document = EditorDocument::new("a".into()).unwrap();
        document.normal('a').unwrap();
        document.insert_text("b").unwrap();
        let saved_revision = document.revision;
        let saved = document.text().to_string();
        document.mark_saved(saved_revision, &saved);
        document.escape();
        document.undo();
        let undo_revision = document.revision;
        assert!(undo_revision > saved_revision);
        assert!(document.dirty());
        document.redo();
        assert!(document.revision > undo_revision);
        assert!(!document.dirty());
    }

    #[test]
    fn grapheme_motion_delete_and_insert_escape_are_vim_like() {
        let family = "👩‍👩‍👧‍👦";
        let mut document = EditorDocument::new(format!("e\u{301}{family}x\nnext")).unwrap();
        document.normal('$').unwrap();
        document.normal('x').unwrap();
        assert_eq!(document.text(), format!("e\u{301}{family}\nnext"));
        document.normal('x').unwrap();
        assert_eq!(document.text(), "e\u{301}\nnext");
        document.normal('i').unwrap();
        document.insert_text(family).unwrap();
        document.escape();
        assert_eq!(document.column, 0);
    }

    #[test]
    fn insertion_cursor_uses_resulting_grapheme_boundaries() {
        let mut document = EditorDocument::new(String::new()).unwrap();
        document.normal('i').unwrap();
        document.insert_char('a').unwrap();
        document.insert_char('\u{301}').unwrap();
        document.insert_char('b').unwrap();
        assert_eq!(document.text(), "a\u{301}b");
        assert_eq!(document.column, 2);

        let mut document = EditorDocument::new("👩b".into()).unwrap();
        document.normal('a').unwrap();
        document.insert_text("\u{200d}💻").unwrap();
        assert_eq!(document.text(), "👩‍💻b");
        assert_eq!(document.column, 1);
        document.move_right();
        assert_eq!(document.column, 2);
        document.move_left();
        document.move_left();
        document.move_left();
        assert_eq!(document.column, 0);
    }

    #[test]
    fn paste_is_one_revision_and_multiline_cursor_is_correct() {
        let mut document = EditorDocument::new("x".into()).unwrap();
        document.normal('i').unwrap();
        document.insert_text("a\n👩‍💻b").unwrap();
        assert_eq!(
            (document.revision, document.row, document.column),
            (1, 1, 2)
        );
        document.undo();
        assert_eq!(document.text(), "x");
        let oversized = "z".repeat(MAX_DOCUMENT_BYTES + 1);
        let revision = document.revision;
        assert!(document.insert_text(&oversized).is_err());
        assert_eq!(document.revision, revision);
    }

    #[test]
    fn underscore_is_a_word_character_and_spans_coalesce() {
        let mut document = EditorDocument::new("one_two three".into()).unwrap();
        document.normal('w').unwrap();
        assert_eq!(document.column, 8);
        let spans = highlight_line("rust", "...fn");
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0],
            HighlightSpan {
                start: 0,
                end: 3,
                kind: HighlightKind::Plain
            }
        );
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
                .map(|span| span.kind)
                .collect::<Vec<_>>();
            assert!(kinds.contains(&HighlightKind::Keyword));
            assert!(kinds.contains(&HighlightKind::String));
            assert!(kinds.contains(&HighlightKind::Comment));
        }
    }
}
