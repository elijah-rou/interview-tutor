use super::msgpack::Value;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::sync::Arc;

pub const MAX_GRID_WIDTH: usize = 512;
pub const MAX_GRID_HEIGHT: usize = 256;
pub const MAX_GRID_CELLS: usize = MAX_GRID_WIDTH * MAX_GRID_HEIGHT;
pub const MAX_HIGHLIGHTS: usize = 65_536;
const MAX_FLUSHES_PER_BATCH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GridCell {
    pub text: String,
    pub highlight: u64,
}

impl Default for GridCell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            highlight: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Highlight {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    pub dim: bool,
    pub strikethrough: bool,
}

impl Highlight {
    pub fn style(self, default_foreground: Option<u32>, default_background: Option<u32>) -> Style {
        let mut foreground = self.foreground.or(default_foreground);
        let mut background = self.background.or(default_background);
        if self.reverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        let mut style = Style::default();
        if let Some(color) = foreground {
            style = style.fg(rgb(color));
        }
        if let Some(color) = background {
            style = style.bg(rgb(color));
        }
        let mut modifiers = Modifier::empty();
        if self.bold {
            modifiers |= Modifier::BOLD;
        }
        if self.italic {
            modifiers |= Modifier::ITALIC;
        }
        if self.underline {
            modifiers |= Modifier::UNDERLINED;
        }
        if self.dim {
            modifiers |= Modifier::DIM;
        }
        if self.strikethrough {
            modifiers |= Modifier::CROSSED_OUT;
        }
        style.add_modifier(modifiers)
    }
}

fn rgb(value: u32) -> Color {
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorShape {
    #[default]
    Block,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ModeCursor {
    shape: CursorShape,
    blink: bool,
    cell_percentage: u8,
}

#[derive(Clone, Debug)]
pub struct GridSnapshot {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<GridCell>,
    pub highlights: Arc<HashMap<u64, Highlight>>,
    pub default_foreground: Option<u32>,
    pub default_background: Option<u32>,
    pub cursor_row: usize,
    pub cursor_column: usize,
    pub cursor_visible: bool,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub cursor_cell_percentage: u8,
    pub mode: String,
}

impl GridSnapshot {
    pub fn cell(&self, row: usize, column: usize) -> Option<&GridCell> {
        let index = row.checked_mul(self.width)?.checked_add(column)?;
        self.cells.get(index)
    }

    pub fn style(&self, highlight: u64) -> Style {
        self.highlights
            .get(&highlight)
            .copied()
            .unwrap_or_default()
            .style(self.default_foreground, self.default_background)
    }
}

pub struct GridState {
    width: usize,
    height: usize,
    cells: Vec<GridCell>,
    highlights: HashMap<u64, Highlight>,
    default_foreground: Option<u32>,
    default_background: Option<u32>,
    cursor_row: usize,
    cursor_column: usize,
    cursor_visible: bool,
    mode_cursors: Vec<ModeCursor>,
    mode_cursor_index: usize,
    mode: String,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            width: 1,
            height: 1,
            cells: vec![GridCell::default()],
            highlights: HashMap::new(),
            default_foreground: None,
            default_background: None,
            cursor_row: 0,
            cursor_column: 0,
            cursor_visible: true,
            mode_cursors: Vec::new(),
            mode_cursor_index: 0,
            mode: "normal".into(),
        }
    }
}

impl GridState {
    pub fn apply_redraw(&mut self, batch: &Value) -> Result<Option<Arc<GridSnapshot>>, String> {
        let events = batch
            .as_array()
            .ok_or("Neovim redraw batch is not an array")?;
        let mut flush_count = 0;
        let mut latest_snapshot = None;
        for event in events {
            let event = event
                .as_array()
                .ok_or("Neovim redraw event is not an array")?;
            let (name, calls) = event.split_first().ok_or("Neovim redraw event is empty")?;
            let name = name
                .as_str()
                .ok_or("Neovim redraw event name is not a string")?;
            for call in calls {
                let arguments = call
                    .as_array()
                    .ok_or("Neovim redraw event arguments are not an array")?;
                match name {
                    "grid_resize" => self.resize(arguments)?,
                    "grid_clear" => self.clear(arguments)?,
                    "grid_destroy" => self.destroy(arguments)?,
                    "grid_cursor_goto" => self.cursor(arguments)?,
                    "grid_line" => self.line(arguments)?,
                    "grid_scroll" => self.scroll(arguments)?,
                    "default_colors_set" => self.default_colors(arguments)?,
                    "hl_attr_define" => self.define_highlight(arguments)?,
                    "mode_info_set" => self.set_mode_info(arguments)?,
                    "mode_change" => self.change_mode(arguments)?,
                    "busy_start" => self.cursor_visible = false,
                    "busy_stop" => self.cursor_visible = true,
                    "flush" => {
                        if flush_count == MAX_FLUSHES_PER_BATCH {
                            return Err("Neovim redraw batch exceeds flush bound".into());
                        }
                        flush_count += 1;
                        // A batch may flush 64 times; release first to retain at most one full grid.
                        drop(latest_snapshot.take());
                        latest_snapshot = Some(Arc::new(self.snapshot()));
                    }
                    "option_set" | "set_title" | "set_icon" | "mouse_on" | "mouse_off" | "bell"
                    | "visual_bell" | "hl_group_set" | "chdir" | "update_menu" | "suspend" => {}
                    "connect" | "restart" => {
                        return Err(format!("unsupported Neovim UI event: {name}"));
                    }
                    _ => {}
                }
            }
        }
        Ok(latest_snapshot)
    }

    fn snapshot(&self) -> GridSnapshot {
        assert_eq!(self.cells.len(), self.width * self.height);
        GridSnapshot {
            width: self.width,
            height: self.height,
            cells: self.cells.clone(),
            highlights: Arc::new(self.highlights.clone()),
            default_foreground: self.default_foreground,
            default_background: self.default_background,
            cursor_row: self.cursor_row,
            cursor_column: self.cursor_column,
            cursor_visible: self.cursor_visible,
            cursor_shape: self.current_mode_cursor().shape,
            cursor_blink: self.current_mode_cursor().blink,
            cursor_cell_percentage: self.current_mode_cursor().cell_percentage,
            mode: self.mode.clone(),
        }
    }

    fn grid(arguments: &[Value]) -> Result<(), String> {
        if arguments.first().and_then(Value::as_i64) == Some(1) {
            Ok(())
        } else {
            Err("Neovim emitted an unexpected grid id".into())
        }
    }

    fn resize(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        let width = usize_value(arguments.get(1), "grid width")?;
        let height = usize_value(arguments.get(2), "grid height")?;
        if width == 0 || height == 0 || width > MAX_GRID_WIDTH || height > MAX_GRID_HEIGHT {
            return Err("Neovim grid dimensions exceed bounds".into());
        }
        let cell_count = width
            .checked_mul(height)
            .ok_or("Neovim grid size overflow")?;
        if cell_count > MAX_GRID_CELLS {
            return Err("Neovim grid exceeds cell bound".into());
        }
        self.width = width;
        self.height = height;
        self.cells = vec![GridCell::default(); cell_count];
        self.cursor_row = self.cursor_row.min(height - 1);
        self.cursor_column = self.cursor_column.min(width - 1);
        Ok(())
    }

    fn clear(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        self.cells.fill(GridCell::default());
        Ok(())
    }

    fn destroy(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        Err("Neovim destroyed the root UI grid".into())
    }

    fn cursor(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        let row = usize_value(arguments.get(1), "cursor row")?;
        let column = usize_value(arguments.get(2), "cursor column")?;
        if row >= self.height || column >= self.width {
            return Err("Neovim cursor is outside the grid".into());
        }
        self.cursor_row = row;
        self.cursor_column = column;
        Ok(())
    }

    fn line(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        let row = usize_value(arguments.get(1), "grid line row")?;
        let mut column = usize_value(arguments.get(2), "grid line column")?;
        if row >= self.height || column > self.width {
            return Err("Neovim grid line starts outside the grid".into());
        }
        let encoded_cells = arguments
            .get(3)
            .and_then(Value::as_array)
            .ok_or("Neovim grid line cells are not an array")?;
        let mut inherited_highlight = 0_u64;
        for encoded in encoded_cells {
            let encoded = encoded
                .as_array()
                .ok_or("Neovim grid cell is not an array")?;
            let text = encoded
                .first()
                .and_then(Value::as_str)
                .ok_or("Neovim grid cell text is not a string")?;
            if text.len() > 64 {
                return Err("Neovim grid cell text exceeds bound".into());
            }
            if let Some(highlight) = encoded.get(1) {
                inherited_highlight = highlight
                    .as_u64()
                    .ok_or("Neovim grid highlight is invalid")?;
            }
            let repeat = match encoded.get(2) {
                Some(value) => {
                    usize::try_from(value.as_u64().ok_or("Neovim grid repeat is invalid")?)
                        .map_err(|_| "Neovim grid repeat does not fit usize")?
                }
                None => 1,
            };
            if column
                .checked_add(repeat)
                .is_none_or(|end| end > self.width)
            {
                return Err(format!(
                    "Neovim grid line exceeds row bounds: row={row} column={column} repeat={repeat} width={} text={text:?}",
                    self.width
                ));
            }
            for _ in 0..repeat {
                let index = row * self.width + column;
                self.cells[index] = GridCell {
                    text: sanitize_symbol(text),
                    highlight: inherited_highlight,
                };
                column += 1;
            }
        }
        Ok(())
    }

    fn scroll(&mut self, arguments: &[Value]) -> Result<(), String> {
        Self::grid(arguments)?;
        let top = usize_value(arguments.get(1), "scroll top")?;
        let bottom = usize_value(arguments.get(2), "scroll bottom")?;
        let left = usize_value(arguments.get(3), "scroll left")?;
        let right = usize_value(arguments.get(4), "scroll right")?;
        let rows = arguments
            .get(5)
            .and_then(Value::as_i64)
            .ok_or("Neovim scroll rows are invalid")?;
        let columns = arguments
            .get(6)
            .and_then(Value::as_i64)
            .ok_or("Neovim scroll columns are invalid")?;
        if top >= bottom || bottom > self.height || left >= right || right > self.width {
            return Err("Neovim scroll region is invalid".into());
        }
        let old = self.cells.clone();
        for row in top..bottom {
            for column in left..right {
                let source_row = i64::try_from(row).unwrap().checked_add(rows);
                let source_column = i64::try_from(column).unwrap().checked_add(columns);
                let value = source_row
                    .zip(source_column)
                    .and_then(|(source_row, source_column)| {
                        let source_row = usize::try_from(source_row).ok()?;
                        let source_column = usize::try_from(source_column).ok()?;
                        (top..bottom)
                            .contains(&source_row)
                            .then_some(())
                            .and((left..right).contains(&source_column).then_some(()))?;
                        old.get(source_row * self.width + source_column).cloned()
                    })
                    .unwrap_or_default();
                self.cells[row * self.width + column] = value;
            }
        }
        Ok(())
    }

    fn default_colors(&mut self, arguments: &[Value]) -> Result<(), String> {
        self.default_foreground = color(arguments.first())?;
        self.default_background = color(arguments.get(1))?;
        Ok(())
    }

    fn define_highlight(&mut self, arguments: &[Value]) -> Result<(), String> {
        let id = arguments
            .first()
            .and_then(Value::as_u64)
            .ok_or("Neovim highlight id is invalid")?;
        if !self.highlights.contains_key(&id) && self.highlights.len() == MAX_HIGHLIGHTS {
            return Err("Neovim highlight table exceeds bound".into());
        }
        let attributes = arguments
            .get(1)
            .and_then(Value::as_map)
            .ok_or("Neovim RGB highlight is not a map")?;
        let lookup = |name: &str| {
            attributes
                .iter()
                .find_map(|(key, value)| (key.as_str() == Some(name)).then_some(value))
        };
        self.highlights.insert(
            id,
            Highlight {
                foreground: color(lookup("foreground"))?,
                background: color(lookup("background"))?,
                bold: boolean(lookup("bold"))?,
                italic: boolean(lookup("italic"))?,
                underline: boolean(lookup("underline"))?
                    || boolean(lookup("undercurl"))?
                    || boolean(lookup("underdouble"))?
                    || boolean(lookup("underdotted"))?
                    || boolean(lookup("underdashed"))?,
                reverse: boolean(lookup("reverse"))?,
                dim: boolean(lookup("dim"))?,
                strikethrough: boolean(lookup("strikethrough"))?,
            },
        );
        Ok(())
    }

    fn current_mode_cursor(&self) -> ModeCursor {
        self.mode_cursors
            .get(self.mode_cursor_index)
            .copied()
            .unwrap_or_default()
    }

    fn set_mode_info(&mut self, arguments: &[Value]) -> Result<(), String> {
        let enabled = arguments
            .first()
            .and_then(Value::as_bool)
            .ok_or("Neovim mode cursor enable flag is invalid")?;
        let modes = arguments
            .get(1)
            .and_then(Value::as_array)
            .ok_or("Neovim mode cursor table is invalid")?;
        if modes.len() > 64 {
            return Err("Neovim mode cursor table exceeds bound".into());
        }
        self.mode_cursors.clear();
        if !enabled {
            return Ok(());
        }
        for mode in modes {
            let attributes = mode.as_map().ok_or("Neovim mode cursor entry is invalid")?;
            let lookup = |name: &str| {
                attributes
                    .iter()
                    .find_map(|(key, value)| (key.as_str() == Some(name)).then_some(value))
            };
            let shape = match lookup("cursor_shape").and_then(Value::as_str) {
                None | Some("block") => CursorShape::Block,
                Some("horizontal") => CursorShape::Horizontal,
                Some("vertical") => CursorShape::Vertical,
                Some(_) => return Err("Neovim mode cursor shape is invalid".into()),
            };
            let cell_percentage = lookup("cell_percentage")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cell_percentage = u8::try_from(cell_percentage)
                .map_err(|_| "Neovim mode cursor percentage exceeds bound")?;
            if cell_percentage > 100 {
                return Err("Neovim mode cursor percentage exceeds bound".into());
            }
            let blink = lookup("blinkon").and_then(Value::as_u64).unwrap_or(0) > 0;
            self.mode_cursors.push(ModeCursor {
                shape,
                blink,
                cell_percentage,
            });
        }
        self.mode_cursor_index = self
            .mode_cursor_index
            .min(self.mode_cursors.len().saturating_sub(1));
        Ok(())
    }

    fn change_mode(&mut self, arguments: &[Value]) -> Result<(), String> {
        let mode = arguments
            .first()
            .and_then(Value::as_str)
            .ok_or("Neovim mode name is invalid")?;
        if mode.len() > 64 {
            return Err("Neovim mode name exceeds bound".into());
        }
        let index = usize_value(arguments.get(1), "mode cursor index")?;
        if !self.mode_cursors.is_empty() && index >= self.mode_cursors.len() {
            return Err("Neovim mode cursor index exceeds table".into());
        }
        self.mode = mode.into();
        self.mode_cursor_index = index;
        Ok(())
    }
}

fn sanitize_symbol(text: &str) -> String {
    text.chars()
        .map(|character| match character as u32 {
            0x00..=0x1f | 0x7f..=0x9f => '�',
            _ => character,
        })
        .collect()
}

fn usize_value(value: Option<&Value>, name: &str) -> Result<usize, String> {
    usize::try_from(
        value
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("Neovim {name} is invalid"))?,
    )
    .map_err(|_| format!("Neovim {name} does not fit usize"))
}

fn color(value: Option<&Value>) -> Result<Option<u32>, String> {
    let Some(value) = value else { return Ok(None) };
    let raw = value.as_i64().ok_or("Neovim color is invalid")?;
    if raw < 0 {
        return Ok(None);
    }
    let raw = u32::try_from(raw).map_err(|_| "Neovim color exceeds RGB range")?;
    if raw > 0x00ff_ffff {
        return Err("Neovim color exceeds RGB range".into());
    }
    Ok(Some(raw))
}

fn boolean(value: Option<&Value>) -> Result<bool, String> {
    value.map_or(Ok(false), |value| {
        value
            .as_bool()
            .ok_or("Neovim highlight flag is invalid".into())
    })
}

#[cfg(test)]
mod tests {
    use super::super::msgpack::{array, map};
    use super::*;

    fn redraw(events: impl IntoIterator<Item = Value>) -> Value {
        Value::Array(events.into_iter().collect())
    }

    fn event(name: &str, calls: impl IntoIterator<Item = Value>) -> Value {
        let mut values = vec![Value::String(name.into())];
        values.extend(calls);
        Value::Array(values)
    }

    #[test]
    fn line_highlights_repeats_scroll_and_flush_are_transactional() {
        let mut grid = GridState::default();
        let no_flush = redraw([
            event(
                "grid_resize",
                [array([
                    Value::Unsigned(1),
                    Value::Unsigned(4),
                    Value::Unsigned(3),
                ])],
            ),
            event(
                "hl_attr_define",
                [array([
                    Value::Unsigned(7),
                    map([
                        ("foreground", Value::Unsigned(0x112233)),
                        ("bold", Value::Bool(true)),
                    ]),
                    Value::Map(Vec::new()),
                    Value::Array(Vec::new()),
                ])],
            ),
            event(
                "grid_line",
                [array([
                    Value::Unsigned(1),
                    Value::Unsigned(0),
                    Value::Unsigned(0),
                    array([
                        array([Value::String("a".into()), Value::Unsigned(7)]),
                        array([
                            Value::String(" ".into()),
                            Value::Unsigned(0),
                            Value::Unsigned(3),
                        ]),
                    ]),
                    Value::Bool(false),
                ])],
            ),
        ]);
        assert!(grid.apply_redraw(&no_flush).unwrap().is_none());
        let flushed = grid
            .apply_redraw(&redraw([event("flush", [Value::Array(Vec::new())])]))
            .unwrap()
            .unwrap();
        assert_eq!(flushed.cell(0, 0).unwrap().text, "a");
        assert_eq!(flushed.cell(0, 0).unwrap().highlight, 7);
        assert_eq!(flushed.style(7).fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
        assert!(flushed.style(7).add_modifier.contains(Modifier::BOLD));

        grid.apply_redraw(&redraw([event(
            "grid_line",
            [array([
                Value::Unsigned(1),
                Value::Unsigned(1),
                Value::Unsigned(0),
                array([array([
                    Value::String("b".into()),
                    Value::Unsigned(0),
                    Value::Unsigned(4),
                ])]),
                Value::Bool(false),
            ])],
        )]))
        .unwrap();
        let snapshot = grid
            .apply_redraw(&redraw([
                event(
                    "grid_scroll",
                    [array([
                        Value::Unsigned(1),
                        Value::Unsigned(0),
                        Value::Unsigned(3),
                        Value::Unsigned(0),
                        Value::Unsigned(4),
                        Value::Integer(1),
                        Value::Integer(0),
                    ])],
                ),
                event("flush", [Value::Array(Vec::new())]),
            ]))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.cell(0, 0).unwrap().text, "b");
    }

    #[test]
    fn mode_info_selects_cursor_shape_blink_and_percentage() {
        let mut grid = GridState::default();
        let snapshot = grid
            .apply_redraw(&redraw([
                event(
                    "mode_info_set",
                    [array([
                        Value::Bool(true),
                        array([map([
                            ("cursor_shape", Value::String("vertical".into())),
                            ("cell_percentage", Value::Unsigned(25)),
                            ("blinkon", Value::Unsigned(300)),
                        ])]),
                    ])],
                ),
                event(
                    "mode_change",
                    [array([Value::String("insert".into()), Value::Unsigned(0)])],
                ),
                event("flush", [Value::Array(Vec::new())]),
            ]))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.cursor_shape, CursorShape::Vertical);
        assert!(snapshot.cursor_blink);
        assert_eq!(snapshot.cursor_cell_percentage, 25);
        assert_eq!(snapshot.mode, "insert");
    }

    #[test]
    fn maximal_grid_multi_flush_retains_only_the_latest_exact_flush() {
        let mut grid = GridState::default();
        let mut events = vec![event(
            "grid_resize",
            [array([
                Value::Unsigned(1),
                Value::Unsigned(MAX_GRID_WIDTH as u64),
                Value::Unsigned(MAX_GRID_HEIGHT as u64),
            ])],
        )];
        for index in 0..MAX_FLUSHES_PER_BATCH {
            events.push(event(
                "grid_line",
                [array([
                    Value::Unsigned(1),
                    Value::Unsigned(0),
                    Value::Unsigned(0),
                    array([array([Value::String(index.to_string())])]),
                    Value::Bool(false),
                ])],
            ));
            events.push(event("flush", [Value::Array(Vec::new())]));
        }
        events.push(event(
            "grid_line",
            [array([
                Value::Unsigned(1),
                Value::Unsigned(0),
                Value::Unsigned(0),
                array([array([Value::String("unflushed".into())])]),
                Value::Bool(false),
            ])],
        ));

        let snapshot = grid.apply_redraw(&redraw(events)).unwrap().unwrap();
        assert_eq!(snapshot.cells.len(), MAX_GRID_CELLS);
        assert_eq!(snapshot.cell(0, 0).unwrap().text, "63");
    }

    #[test]
    fn grid_symbols_do_not_publish_terminal_controls() {
        let mut grid = GridState::default();
        let snapshot = grid
            .apply_redraw(&redraw([
                event(
                    "grid_line",
                    [array([
                        Value::Unsigned(1),
                        Value::Unsigned(0),
                        Value::Unsigned(0),
                        array([array([Value::String("\u{1b}\u{85}x".into())])]),
                        Value::Bool(false),
                    ])],
                ),
                event("flush", [Value::Array(Vec::new())]),
            ]))
            .unwrap()
            .unwrap();
        let text = &snapshot.cell(0, 0).unwrap().text;
        assert_eq!(text, "��x");
        assert!(!text.chars().any(char::is_control));
    }

    #[test]
    fn exact_grid_highlight_and_flush_bounds_are_enforced() {
        let mut grid = GridState::default();
        grid.resize(&[
            Value::Unsigned(1),
            Value::Unsigned(MAX_GRID_WIDTH as u64),
            Value::Unsigned(MAX_GRID_HEIGHT as u64),
        ])
        .unwrap();
        assert_eq!(grid.cells.len(), MAX_GRID_CELLS);
        grid.line(&[
            Value::Unsigned(1),
            Value::Unsigned((MAX_GRID_HEIGHT - 1) as u64),
            Value::Unsigned((MAX_GRID_WIDTH - 1) as u64),
            array([array([Value::String("x".into())])]),
        ])
        .unwrap();
        assert!(
            grid.line(&[
                Value::Unsigned(1),
                Value::Unsigned((MAX_GRID_HEIGHT - 1) as u64),
                Value::Unsigned((MAX_GRID_WIDTH - 1) as u64),
                array([array([
                    Value::String("x".into()),
                    Value::Unsigned(0),
                    Value::Unsigned(2),
                ])]),
            ])
            .unwrap_err()
            .contains("exceeds row bounds")
        );
        assert!(
            grid.resize(&[
                Value::Unsigned(1),
                Value::Unsigned((MAX_GRID_WIDTH + 1) as u64),
                Value::Unsigned(1),
            ])
            .is_err()
        );

        let mut highlights = GridState::default();
        for id in 0..MAX_HIGHLIGHTS as u64 {
            highlights
                .define_highlight(&[Value::Unsigned(id), Value::Map(Vec::new())])
                .unwrap();
        }
        highlights
            .define_highlight(&[Value::Unsigned(0), Value::Map(Vec::new())])
            .unwrap();
        assert!(
            highlights
                .define_highlight(&[
                    Value::Unsigned(MAX_HIGHLIGHTS as u64),
                    Value::Map(Vec::new()),
                ])
                .unwrap_err()
                .contains("highlight table")
        );

        let flushes = (0..=MAX_FLUSHES_PER_BATCH)
            .map(|_| event("flush", [Value::Array(Vec::new())]))
            .collect::<Vec<_>>();
        assert!(
            GridState::default()
                .apply_redraw(&redraw(flushes))
                .unwrap_err()
                .contains("flush bound")
        );
    }

    #[test]
    fn wide_continuation_cursor_colors_busy_and_clear_publish_at_flush() {
        let mut grid = GridState::default();
        let busy = grid
            .apply_redraw(&redraw([
                event(
                    "grid_resize",
                    [array([
                        Value::Unsigned(1),
                        Value::Unsigned(4),
                        Value::Unsigned(2),
                    ])],
                ),
                event(
                    "default_colors_set",
                    [array([
                        Value::Unsigned(0x010203),
                        Value::Unsigned(0x040506),
                        Value::Integer(-1),
                        Value::Unsigned(0),
                        Value::Unsigned(0),
                    ])],
                ),
                event(
                    "grid_line",
                    [array([
                        Value::Unsigned(1),
                        Value::Unsigned(0),
                        Value::Unsigned(0),
                        array([
                            array([Value::String("界".into()), Value::Unsigned(0)]),
                            array([Value::String(String::new())]),
                        ]),
                        Value::Bool(false),
                    ])],
                ),
                event(
                    "grid_cursor_goto",
                    [array([
                        Value::Unsigned(1),
                        Value::Unsigned(0),
                        Value::Unsigned(1),
                    ])],
                ),
                event("busy_start", [Value::Array(Vec::new())]),
                event("flush", [Value::Array(Vec::new())]),
            ]))
            .unwrap()
            .unwrap();
        assert_eq!(busy.cell(0, 0).unwrap().text, "界");
        assert_eq!(busy.cell(0, 1).unwrap().text, "");
        assert_eq!(busy.default_foreground, Some(0x010203));
        assert_eq!(busy.default_background, Some(0x040506));
        assert_eq!((busy.cursor_row, busy.cursor_column), (0, 1));
        assert!(!busy.cursor_visible);

        let cleared = grid
            .apply_redraw(&redraw([
                event("busy_stop", [Value::Array(Vec::new())]),
                event("grid_clear", [array([Value::Unsigned(1)])]),
                event("flush", [Value::Array(Vec::new())]),
            ]))
            .unwrap()
            .unwrap();
        assert!(cleared.cursor_visible);
        assert!(
            cleared
                .cells
                .iter()
                .all(|cell| cell == &GridCell::default())
        );
    }

    #[test]
    fn overlapping_vertical_and_horizontal_scroll_reads_original_region() {
        let mut grid = GridState::default();
        grid.resize(&[Value::Unsigned(1), Value::Unsigned(4), Value::Unsigned(3)])
            .unwrap();
        for (row, text) in ["abcd", "efgh", "ijkl"].into_iter().enumerate() {
            grid.line(&[
                Value::Unsigned(1),
                Value::Unsigned(row as u64),
                Value::Unsigned(0),
                array(
                    text.chars()
                        .map(|character| array([Value::String(character.to_string())])),
                ),
            ])
            .unwrap();
        }
        grid.scroll(&[
            Value::Unsigned(1),
            Value::Unsigned(0),
            Value::Unsigned(3),
            Value::Unsigned(0),
            Value::Unsigned(4),
            Value::Integer(1),
            Value::Integer(1),
        ])
        .unwrap();
        let rows = (0..3)
            .map(|row| {
                (0..4)
                    .map(|column| grid.cells[row * 4 + column].text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, ["fgh ", "jkl ", "    "]);
    }

    #[test]
    fn malformed_known_events_fail_and_unknown_future_events_are_ignored() {
        let mut grid = GridState::default();
        assert!(
            grid.apply_redraw(&redraw([event(
                "grid_resize",
                [array([
                    Value::Unsigned(2),
                    Value::Unsigned(10),
                    Value::Unsigned(10),
                ])]
            )]))
            .is_err()
        );
        assert!(
            grid.apply_redraw(&redraw([event(
                "future_metadata",
                [Value::Array(Vec::new())]
            )]))
            .unwrap()
            .is_none()
        );
    }
}
