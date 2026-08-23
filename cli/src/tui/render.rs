use crate::app::model::{
    AppState, Focus, MAX_RENDERED_MARKDOWN_CHARS, MAX_ROWS, Screen, SolvePane,
};
use crate::editor::Mode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use unicode_width::UnicodeWidthStr;

fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
}

fn markdown_text(markdown: &str) -> Text<'static> {
    let mut remaining = MAX_RENDERED_MARKDOWN_CHARS;
    let mut in_code = false;
    let lines = markdown
        .lines()
        .take(MAX_ROWS)
        .map_while(|source| {
            if remaining == 0 {
                return None;
            }
            let source = source.trim_end_matches('\r');
            let (prefix, content) = if source.trim_start().starts_with("```") {
                in_code = !in_code;
                ("", if in_code { "Code:" } else { "" })
            } else {
                let plain = source.trim_start_matches('#').trim_start();
                (if in_code { "  " } else { "" }, plain)
            };
            let prefix: String = prefix.chars().take(remaining).collect();
            remaining -= prefix.chars().count();
            let content: String = content.chars().take(remaining).collect();
            remaining -= content.chars().count();
            Some(Line::from(format!("{prefix}{content}")))
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn wrapped_markdown_text(markdown: &str, width: usize) -> Text<'static> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in markdown_text(markdown).lines {
        let content = line
            .spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>();
        if content.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let chars = content.chars().collect::<Vec<_>>();
        wrapped.extend(
            chars
                .chunks(width)
                .map(|chunk| Line::from(chunk.iter().collect::<String>())),
        );
    }
    Text::from(wrapped)
}

fn header(state: &AppState) -> Paragraph<'static> {
    let language = state
        .languages
        .get(state.language_index)
        .map_or("none", |item| item.display_name.as_str());
    let solve_badges = state.solve.as_ref().map_or(String::new(), |solve| {
        format!(
            "  {:?}{}{}",
            solve.editor.mode,
            if solve.editor.dirty() {
                " · DIRTY"
            } else {
                " · SAVED"
            },
            if solve.stale { " · STALE" } else { "" },
        )
    });
    let mut lines = vec![Line::from(format!(
        " Interview Tutor  Language: {language}  Progress: {}/{}  {}{}",
        state.data.progress.completed, state.data.progress.total, state.status, solve_badges
    ))];
    if state.solve.is_some() {
        lines.push(Line::from(format!(
            " Codex: {} · memory only",
            state.codex.status.label()
        )));
    }
    Paragraph::new(lines).style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_text(width: u16) -> &'static str {
    const FULL: &str = "j/k move Enter open Esc back Tab pane l lang r reload ? help q quit";
    const COMPACT: &str = "j/k move Enter open Esc back Tab pane ? help q quit";

    if usize::from(width) >= FULL.len() {
        FULL
    } else {
        COMPACT
    }
}

fn solve_footer_text(state: &AppState, width: u16) -> String {
    let solve = state
        .solve
        .as_ref()
        .expect("solve footer requires solve state");
    let base = if solve.pane == SolvePane::Interview
        && state.codex.status == crate::app::model::CodexStatus::Disclosure
    {
        "y/Enter accept · n/Esc decline".to_string()
    } else if solve.pane == SolvePane::Interview && state.codex.composer_focused {
        "Type question · Enter send · Esc close".to_string()
    } else {
        match (solve.pane, solve.editor.mode) {
            (SolvePane::Editor, Mode::Normal) => {
                "i insert · Space-h hint · F5 test · F9 submit · Tab panes".into()
            }
            (SolvePane::Editor, Mode::Insert) => {
                "Esc normal · F5 test · F9 submit · Tab panes".into()
            }
            (SolvePane::Editor, Mode::Visual) => {
                "Neovim Visual · F5 test · F9 submit · Tab panes".into()
            }
            (SolvePane::Editor, Mode::Command) => "Neovim command · Esc normal · Tab panes".into(),
            (SolvePane::Interview, _)
                if matches!(
                    state.codex.status,
                    crate::app::model::CodexStatus::ProtocolError
                        | crate::app::model::CodexStatus::Disconnected
                ) =>
            {
                "i retry · Space-h hint · Space-r reset · Tab panes".into()
            }
            (SolvePane::Interview, _) => "i ask · ↑/↓ scroll · Space-h hint · Space-r reset".into(),
            (SolvePane::Problem | SolvePane::Output, _) => {
                "i interview · ↑/↓ scroll · Space-h hint · Tab panes".into()
            }
        }
    };
    let codex_active = state.codex.active.is_some() || state.codex.connecting.is_some();
    let runner_active = solve.running.is_some();
    let cancel = if codex_active && (solve.pane == SolvePane::Interview || !runner_active) {
        Some("Codex")
    } else if runner_active {
        Some("runner")
    } else {
        None
    };
    let expanded = cancel.map_or_else(
        || base.clone(),
        |target| format!("{base} · Ctrl-C {target}"),
    );
    if UnicodeWidthStr::width(expanded.as_str()) <= usize::from(width) {
        return expanded;
    }
    if let Some(target) = cancel {
        let compact = format!("F5 test · F9 submit · Tab panes · Ctrl-C {target}");
        if UnicodeWidthStr::width(compact.as_str()) <= usize::from(width) {
            return compact;
        }
    }
    assert!(UnicodeWidthStr::width(base.as_str()) <= 60);
    base
}

fn progress(state: &AppState) -> Paragraph<'static> {
    let mut lines = vec![Line::from(format!(
        "Total  {}/{}",
        state.data.progress.completed, state.data.progress.total
    ))];
    lines.extend(state.data.progress.by_difficulty.iter().map(|item| {
        Line::from(format!(
            "{}  {}/{}",
            item.difficulty, item.completed, item.total
        ))
    }));
    lines.extend(
        state
            .data
            .progress
            .by_topic
            .iter()
            .map(|item| Line::from(format!("{}  {}/{}", item.topic, item.completed, item.total))),
    );
    let title = if state.focus == Focus::Progress {
        "Progress [active]"
    } else {
        "Progress"
    };
    Paragraph::new(lines)
        .block(block(title))
        .wrap(Wrap { trim: true })
        .scroll((state.progress_scroll, 0))
}

fn viewport_start(selected: usize, area_height: u16) -> usize {
    let visible_rows = usize::from(area_height.saturating_sub(4)).max(1);
    selected.saturating_sub(visible_rows.saturating_sub(1))
}

fn sets(state: &AppState, area_height: u16) -> Table<'static> {
    let start = viewport_start(state.set_index, area_height);
    let rows = state
        .data
        .sets
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, item)| {
            let style = if index == state.set_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(item.name.clone()),
                Cell::from(format!("{}/{}", item.completed_count, item.member_count)),
                Cell::from(item.description.clone()),
            ])
            .style(style)
        });
    Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(10),
            Constraint::Percentage(70),
        ],
    )
    .header(
        Row::new(["Problem set", "Done", "Description"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(block(if state.focus == Focus::Main {
        "Problem sets [active]"
    } else {
        "Problem sets"
    }))
}

fn problems(state: &AppState, area_height: u16) -> Table<'static> {
    let start = viewport_start(state.problem_index, area_height);
    let rows = state
        .data
        .problems
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, item)| {
            let style = if index == state.problem_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(
                    item.ordinal
                        .map_or_else(|| "-".into(), |value| value.to_string()),
                ),
                Cell::from(if item.completed { "✓" } else { "·" }),
                Cell::from(item.title.clone()),
                Cell::from(item.difficulty.to_string()),
                Cell::from(item.topic.clone()),
            ])
            .style(style)
        });
    Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Percentage(45),
            Constraint::Length(9),
            Constraint::Percentage(35),
        ],
    )
    .header(
        Row::new(["#", "", "Problem", "Level", "Topic"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(block(if state.focus == Focus::Main {
        "Problems [active]"
    } else {
        "Problems"
    }))
}

fn detail(state: &AppState, area_width: u16) -> Paragraph<'static> {
    match &state.data.detail {
        Some(item) => Paragraph::new(wrapped_markdown_text(
            &item.statement_markdown,
            usize::from(area_width.saturating_sub(2)),
        ))
        .block(block(&format!(
            "{} · {} · {}",
            item.title, item.difficulty, item.topic
        )))
        .wrap(Wrap { trim: false })
        .scroll((state.detail_scroll, 0)),
        None => Paragraph::new("No problem detail available").block(block("Problem")),
    }
}

fn solve_editor(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let Some(solve) = &state.solve else { return };
    let title = if solve.pane == SolvePane::Editor {
        "Editor · Neovim [active]"
    } else {
        "Editor · Neovim"
    };
    let editor_block = block(title);
    let inner = editor_block.inner(area);
    frame.render_widget(editor_block, area);
    let Some(view) = solve.editor_view.as_ref() else {
        let message = match solve.editor_status {
            crate::app::model::EditorRuntimeStatus::Starting => "Starting required Neovim…",
            crate::app::model::EditorRuntimeStatus::Ready => "Waiting for Neovim redraw…",
            crate::app::model::EditorRuntimeStatus::Failed => "Neovim unavailable",
        };
        frame.render_widget(Paragraph::new(message), inner);
        return;
    };
    let rows = usize::from(inner.height).min(view.height);
    let columns = usize::from(inner.width).min(view.width);
    let buffer = frame.buffer_mut();
    for row in 0..rows {
        for column in 0..columns {
            let cell = view.cell(row, column).expect("bounded Neovim grid cell");
            let x = inner.x + u16::try_from(column).expect("grid column fits u16");
            let y = inner.y + u16::try_from(row).expect("grid row fits u16");
            buffer[(x, y)]
                .set_symbol(&cell.text)
                .set_style(view.style(cell.highlight));
        }
    }
    if solve.pane == SolvePane::Editor
        && view.cursor_visible
        && view.cursor_row < rows
        && view.cursor_column < columns
    {
        frame.set_cursor_position((
            inner.x + u16::try_from(view.cursor_column).expect("cursor column fits u16"),
            inner.y + u16::try_from(view.cursor_row).expect("cursor row fits u16"),
        ));
    }
}
fn solve_problem(state: &AppState) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    Paragraph::new(markdown_text(&solve.statement))
        .block(block(if solve.pane == SolvePane::Problem {
            "Problem / Examples [active]"
        } else {
            "Problem / Examples"
        }))
        .wrap(Wrap { trim: false })
        .scroll((solve.problem_scroll, 0))
}
fn solve_output(state: &AppState) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    Paragraph::new(solve.output.clone())
        .block(block(if solve.pane == SolvePane::Output {
            "Output / Test [active]"
        } else {
            "Output / Test"
        }))
        .wrap(Wrap { trim: false })
        .scroll((solve.output_scroll, 0))
}
fn solve_interview(state: &AppState, area: Rect) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    let mut lines = vec![Line::from(format!(
        "Codex {} · Transcript memory only · not saved",
        state.codex.status.label()
    ))];
    match state.codex.status {
        crate::app::model::CodexStatus::Disabled => lines.push(Line::from(
            "Codex is Disabled. Local editor, tests, and submission remain available.",
        )),
        crate::app::model::CodexStatus::Disclosure => {
            lines.push(Line::from("Privacy disclosure"));
            lines.push(Line::from(
                "The selected statement, current source, bounded latest test output,",
            ));
            lines.push(Line::from(
                "in-memory transcript, and your question may be sent to OpenAI.",
            ));
            lines.push(Line::from("Configured executable is"));
            lines.push(Line::from("user-selected and trusted."));
            lines.push(Line::from("Version checks compatibility,"));
            lines.push(Line::from("not provenance."));
            lines.push(Line::from(
                "Codex may read local config, MCP, or sandbox-readable paths.",
            ));
            lines.push(Line::from("Dedicated profile/home is safer."));
            lines.push(Line::from(
                "Account controls apply. y/Enter accept · n/Esc decline",
            ));
        }
        crate::app::model::CodexStatus::AuthRequired => lines.push(Line::from(
            "Authentication required. Exit and run: codex login",
        )),
        crate::app::model::CodexStatus::Declined => lines.push(Line::from(
            "Codex declined. Local editor, tests, and submission remain available.",
        )),
        _ => {
            for (label, message) in &state.codex.messages {
                lines.push(Line::styled(
                    format!("{label}:"),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                lines.extend(message.lines().map(|line| Line::from(line.to_string())));
            }
            lines.push(Line::from(""));
            let cursor = if state.codex.composer_focused {
                "▌"
            } else {
                ""
            };
            lines.push(Line::from(format!(
                "Question: {}{cursor}",
                state.codex.composer
            )));
            lines.push(Line::from(
                "i ask · Enter send · Space-h hint · Space-r reset · Ctrl-C cancel",
            ));
        }
    }
    let inner_width = usize::from(area.width.saturating_sub(2)).max(1);
    let visible_rows = usize::from(area.height.saturating_sub(2)).max(1);
    let wrapped_rows = lines
        .iter()
        .map(|line| {
            let width = line
                .spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            width.max(1).div_ceil(inner_width)
        })
        .sum::<usize>();
    let latest_offset = wrapped_rows.saturating_sub(visible_rows);
    let retained_below = usize::from(state.codex.scroll).min(latest_offset);
    let offset = latest_offset.saturating_sub(retained_below);
    Paragraph::new(lines)
        .block(block(if solve.pane == SolvePane::Interview {
            "Interview [active]"
        } else {
            "Interview"
        }))
        .wrap(Wrap { trim: false })
        .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0))
}
fn render_solve(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let solve = state.solve.as_ref().unwrap();
    if area.width >= 100 && area.height >= 28 {
        let vertical =
            Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(area);
        let upper = Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ])
        .split(vertical[0]);
        frame.render_widget(solve_problem(state), upper[0]);
        solve_editor(frame, state, upper[1]);
        frame.render_widget(solve_interview(state, upper[2]), upper[2]);
        frame.render_widget(solve_output(state), vertical[1]);
    } else {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
        let panes = ["Editor", "Problem", "Output", "Interview"]
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        let selected = match solve.pane {
            SolvePane::Editor => 0,
            SolvePane::Problem => 1,
            SolvePane::Output => 2,
            SolvePane::Interview => 3,
        };
        frame.render_widget(
            Tabs::new(panes)
                .select(selected)
                .block(block("Solve panes")),
            chunks[0],
        );
        match solve.pane {
            SolvePane::Editor => solve_editor(frame, state, chunks[1]),
            SolvePane::Problem => frame.render_widget(solve_problem(state), chunks[1]),
            SolvePane::Output => frame.render_widget(solve_output(state), chunks[1]),
            SolvePane::Interview => {
                frame.render_widget(solve_interview(state, chunks[1]), chunks[1])
            }
        }
    }
}

pub fn neovim_grid_area(state: &AppState, width: u16, height: u16) -> Option<Rect> {
    let solve = state.solve.as_ref()?;
    if width < 60 || height < 20 {
        return None;
    }
    let content = Rect::new(0, 2, width, height.saturating_sub(3));
    let editor = if content.width >= 100 && content.height >= 28 {
        let vertical = Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(content);
        Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ])
        .split(vertical[0])[1]
    } else if solve.pane == SolvePane::Editor {
        Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(content)[1]
    } else {
        return None;
    };
    Some(Rect::new(
        editor.x.saturating_add(1),
        editor.y.saturating_add(1),
        editor.width.saturating_sub(2).max(1),
        editor.height.saturating_sub(2).max(1),
    ))
}

pub fn neovim_grid_size(state: &AppState, width: u16, height: u16) -> Option<(u16, u16)> {
    neovim_grid_area(state, width, height).map(|area| (area.width, area.height))
}

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    let header_height = if state.screen == Screen::Solve { 2 } else { 1 };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(header(state), vertical[0]);
    let footer = if state.screen == Screen::Solve {
        solve_footer_text(state, area.width)
    } else {
        footer_text(area.width).to_string()
    };
    frame.render_widget(Paragraph::new(footer), vertical[2]);

    let undersized = area.width < 60 || area.height < 20;
    if undersized {
        let quit_key = if state.screen == Screen::Solve {
            "Space-q"
        } else {
            "q"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal too small\nResize to at least 60 × 20\nPress {quit_key} to quit\nDirty editor: press twice to confirm"
            ))
            .alignment(ratatui::layout::Alignment::Center)
            .block(block("Resize required")),
            vertical[1],
        );
    } else {
        let content = vertical[1];
        if state.screen == Screen::Solve {
            render_solve(frame, state, content);
        } else if area.width >= 100 && area.height >= 30 {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
                .split(content);
            match state.screen {
                Screen::SetMenu => frame.render_widget(sets(state, columns[0].height), columns[0]),
                Screen::ProblemList => {
                    frame.render_widget(problems(state, columns[0].height), columns[0])
                }
                Screen::ProblemDetail => {
                    frame.render_widget(detail(state, columns[0].width), columns[0])
                }
                Screen::Solve => unreachable!("solve rendered above"),
            }
            frame.render_widget(progress(state), columns[1]);
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(1)])
                .split(content);
            let labels: Vec<Line<'static>> = ["Sets", "Problems", "Detail", "Progress"]
                .into_iter()
                .map(Line::from)
                .collect();
            let selected = if state.focus == Focus::Progress {
                3
            } else {
                match state.screen {
                    Screen::SetMenu => 0,
                    Screen::ProblemList => 1,
                    Screen::ProblemDetail => 2,
                    Screen::Solve => unreachable!("solve rendered above"),
                }
            };
            frame.render_widget(
                Tabs::new(labels).select(selected).block(block("View")),
                chunks[0],
            );
            if state.focus == Focus::Progress {
                frame.render_widget(progress(state), chunks[1]);
            } else {
                match state.screen {
                    Screen::SetMenu => {
                        frame.render_widget(sets(state, chunks[1].height), chunks[1])
                    }
                    Screen::ProblemList => {
                        frame.render_widget(problems(state, chunks[1].height), chunks[1])
                    }
                    Screen::ProblemDetail => {
                        frame.render_widget(detail(state, chunks[1].width), chunks[1])
                    }
                    Screen::Solve => unreachable!("solve rendered above"),
                }
            }
        }
    }

    if state.show_help && !undersized {
        let popup = centered(70, 55, area);
        frame.render_widget(Clear, popup);
        let help = if state.screen == Screen::Solve {
            format!(
                "Solve help\n\nCurrent: {}\nEditor Normal: i inserts\nProblem/Output/Interview: i focuses Interview\nSpace-h hints outside Insert/Command/composer\n↑/↓ scrolls the focused non-editor pane\nTab/Shift-Tab changes pane · F5 test · F9 submit",
                solve_footer_text(state, popup.width.saturating_sub(2))
            )
        } else {
            "Help\n\n↑/k up  ↓/j down  Enter open\nEsc back  Tab/Shift-Tab pane\nl language  r reload  ? close  q quit".into()
        };
        frame.render_widget(
            Paragraph::new(help)
                .block(block("Keyboard help"))
                .wrap(Wrap { trim: true }),
            popup,
        );
    } else if let Some(error) = &state.error {
        let popup = centered(70, 35, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .block(block("Error"))
                .wrap(Wrap { trim: true }),
            popup,
        );
    }
}

fn centered(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::{AppState, ProblemDetail, ProblemRow, SetRow};
    use crate::app::{Action, Event, reduce};
    use crate::database::{Difficulty, TopicProgress};
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn rendered_row(state: &AppState, width: u16, height: u16, row: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .nth(usize::from(row))
            .unwrap()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn rendered_footer(state: &AppState, width: u16) -> String {
        let backend = TestBackend::new(width, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .last()
            .unwrap()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn footer_is_not_clipped_at_supported_widths() {
        let state = AppState::new(Vec::new(), 0);
        for width in [60, 68, 80, 120] {
            let footer = rendered_footer(&state, width);
            assert_eq!(footer, footer_text(width));
            assert!(footer.contains("? help"));
            assert!(footer.contains("q quit"));
        }
    }

    #[test]
    fn undersized_terminal_replaces_help_with_truthful_quit_cue() {
        let mut state = AppState::new(Vec::new(), 0);
        state.show_help = true;
        let view = rendered(&state, 59, 19);
        assert!(view.contains("Terminal too small"));
        assert!(view.contains("Press q to quit"));
        assert!(view.contains("press twice to confirm"));
        assert!(!view.contains("Keyboard help"));
        reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
    }

    #[test]
    fn deterministic_full_compact_and_resize_views() {
        let mut state = AppState::new(Vec::new(), 0);
        state.data.sets.push(SetRow {
            slug: "unicode".into(),
            name: "Unicode 🦀".into(),
            description: "界".repeat(200),
            member_count: 0,
            completed_count: 0,
        });
        assert!(rendered(&state, 120, 40).contains("Unicode 🦀"));
        assert!(rendered(&state, 80, 24).contains("View"));
        assert!(rendered(&state, 59, 19).contains("Terminal too small"));

        state.screen = Screen::ProblemList;
        assert!(rendered(&state, 120, 40).contains("Problems"));
        state.data.problems.push(ProblemRow {
            id: 1,
            ordinal: Some(1),
            slug: "two-sum".into(),
            title: "Two Sum".into(),
            difficulty: Difficulty::Easy,
            topic: "Arrays".into(),
            completed: true,
        });
        state.screen = Screen::ProblemDetail;
        state.data.detail = Some(ProblemDetail {
            id: 1,
            slug: "two-sum".into(),
            title: "Two Sum".into(),
            difficulty: Difficulty::Easy,
            topic: "Arrays".into(),
            statement_markdown: "# Statement\n\n- item\n```\ncode\n```".into(),
            implementations: Vec::new(),
        });
        assert!(rendered(&state, 80, 24).contains("Statement"));
        state.show_help = true;
        assert!(rendered(&state, 120, 40).contains("Keyboard help"));
        state.show_help = false;
        state.error = Some("deterministic error".into());
        assert!(rendered(&state, 120, 40).contains("deterministic error"));
        state.error = None;
        state.focus = Focus::Progress;
        assert!(rendered(&state, 80, 24).contains("Total"));
    }

    #[test]
    fn selected_rows_remain_visible_in_oversized_tables() {
        let mut state = AppState::new(Vec::new(), 0);
        state.data.sets = (0..75)
            .map(|index| SetRow {
                slug: format!("set-{index}"),
                name: format!("S{index}"),
                description: String::new(),
                member_count: 1,
                completed_count: 0,
            })
            .collect();
        state.set_index = 74;
        assert!(rendered(&state, 120, 40).contains("S74"));
        assert!(rendered(&state, 80, 24).contains("S74"));

        state.screen = Screen::ProblemList;
        state.data.problems = (0..75)
            .map(|index| ProblemRow {
                id: index,
                ordinal: Some(index + 1),
                slug: format!("problem-{index}"),
                title: format!("Selected problem {index}"),
                difficulty: Difficulty::Easy,
                topic: "Arrays".into(),
                completed: false,
            })
            .collect();
        state.problem_index = 74;
        assert!(rendered(&state, 120, 40).contains("Selected problem 74"));
        assert!(rendered(&state, 80, 24).contains("Selected problem 74"));
        reduce(&mut state, Event::Command(Action::Open));
        assert_eq!(state.screen, Screen::ProblemDetail);
        assert_eq!(state.selected_problem_id, Some(74));
    }

    #[test]
    fn detail_and_progress_scroll_reach_trailing_content() {
        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::ProblemDetail;
        state.data.detail = Some(ProblemDetail {
            id: 1,
            slug: "long".into(),
            title: "Long".into(),
            difficulty: Difficulty::Easy,
            topic: "Unicode".into(),
            statement_markdown: format!(
                "{}\nTRAILING-界-SENTINEL",
                "wrapped 界 content ".repeat(200)
            ),
            implementations: Vec::new(),
        });
        assert!((0..100).any(|scroll| {
            state.detail_scroll = scroll;
            rendered(&state, 80, 24).contains("SENTINEL")
        }));

        state.focus = Focus::Progress;
        state.data.progress.by_topic = (0..18)
            .map(|index| TopicProgress {
                topic: format!("topic-{index}"),
                completed: 0,
                total: 1,
            })
            .collect();
        state.progress_scroll = 6;
        assert!(rendered(&state, 80, 24).contains("topic-17"));
    }

    fn solve_state() -> AppState {
        use crate::app::model::{SolvePane, SolveSession};
        use crate::editor::EditorDocument;
        use crate::runner::ExecutionPlan;
        use std::path::PathBuf;
        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: "Statement\nExample".into(),
            language: "python".into(),
            plan: ExecutionPlan {
                root: PathBuf::from("/tmp"),
                language: "python".into(),
                problem_slug: "p".into(),
                set_slug: None,
                runner_path: PathBuf::from("/tmp/run"),
                solution_path: PathBuf::from("/tmp/p.py"),
            },
            editor: EditorDocument::new("def solve():\n    return \"界\" # comment".into())
                .unwrap(),
            editor_view: None,
            editor_status: crate::app::model::EditorRuntimeStatus::Ready,
            pane: SolvePane::Editor,
            output: "compiler error".into(),
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
            submitted_source: None,
        });
        state
    }

    #[test]
    fn interview_disclosure_and_labels_render_at_supported_sizes() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.disable_codex();
        let disabled = rendered(&state, 120, 40);
        assert!(disabled.contains("Codex Disabled"));

        state.codex.enabled = true;
        state.codex.status = crate::app::model::CodexStatus::Disclosure;
        let disclosure = rendered(&state, 120, 40);
        assert!(disclosure.contains("Privacy disclosure"));
        assert!(disclosure.contains("Configured executable is"));
        assert!(disclosure.contains("user-selected and trusted"));
        assert!(disclosure.contains("not provenance"));
        assert!(disclosure.contains("Codex may read local"));
        assert!(disclosure.contains("Dedicated profile/home"));
        assert!(rendered(&state, 80, 24).contains("Privacy disclosure"));
        assert!(rendered(&state, 59, 19).contains("Terminal too small"));
        state.codex.status = crate::app::model::CodexStatus::Feedback;
        state
            .codex
            .messages
            .push(("Interviewer".into(), "Question".into()));
        state.codex.messages.push(("Hinter".into(), "Hint".into()));
        state.codex.messages.push((
            "Submission review · recorded revision 7".into(),
            "Review".into(),
        ));
        let view = rendered(&state, 80, 24);
        assert!(
            view.contains("Interviewer")
                && view.contains("Hinter")
                && view.contains("Submission review · recorded revision 7")
        );
    }

    #[test]
    fn solve_layouts_render_neovim_grid_and_native_highlights() {
        use crate::neovim::grid::{GridCell, GridSnapshot, Highlight};
        use std::collections::HashMap;
        use std::sync::Arc;

        let mut state = solve_state();
        let width = 20;
        let height = 3;
        let mut cells = vec![GridCell::default(); width * height];
        for (column, character) in "def solve():".chars().enumerate() {
            cells[column] = GridCell {
                text: character.to_string(),
                highlight: if column < 3 { 7 } else { 0 },
            };
        }
        let mut highlights = HashMap::new();
        highlights.insert(
            7,
            Highlight {
                foreground: Some(0xff00ff),
                ..Highlight::default()
            },
        );
        state.solve.as_mut().unwrap().editor_view = Some(Arc::new(GridSnapshot {
            width,
            height,
            cells,
            highlights: Arc::new(highlights),
            default_foreground: None,
            default_background: None,
            cursor_row: 0,
            cursor_column: 0,
            cursor_visible: true,
            cursor_shape: crate::neovim::grid::CursorShape::Block,
            cursor_blink: false,
            cursor_cell_percentage: 0,
            mode: "normal".into(),
        }));

        let full = rendered(&state, 120, 40);
        assert!(full.contains("Problem / Examples"));
        assert!(full.contains("compiler error"));
        assert!(full.contains("Question:"));
        let compact = rendered(&state, 80, 24);
        assert!(compact.contains("Solve panes"));
        assert!(compact.contains("def solve"));
        assert!(rendered(&state, 59, 19).contains("Terminal too small"));
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &state)).unwrap();
        let keyword = &terminal.backend().buffer()[(37, 3)];
        assert_eq!(keyword.symbol(), "d");
        assert_eq!(keyword.fg, Color::Rgb(0xff, 0x00, 0xff));
    }

    #[test]
    fn solve_header_preserves_exact_codex_state_and_memory_badge_at_supported_widths() {
        let mut state = solve_state();
        for status in [
            crate::app::model::CodexStatus::Disabled,
            crate::app::model::CodexStatus::Offline,
            crate::app::model::CodexStatus::AuthRequired,
            crate::app::model::CodexStatus::Ready,
            crate::app::model::CodexStatus::Thinking,
            crate::app::model::CodexStatus::ProtocolError,
        ] {
            state.codex.status = status;
            for width in [60_u16, 80, 120] {
                let row = rendered_row(&state, width, 24, 1);
                assert_eq!(
                    row,
                    format!(" Codex: {} · memory only", status.label()),
                    "width {width}"
                );
            }
        }
    }

    #[test]
    fn interview_scroll_reaches_oldest_and_latest_messages_at_both_layouts() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.codex.status = crate::app::model::CodexStatus::Feedback;
        for index in 0..48 {
            let message = if index == 0 {
                "OLDEST-SENTINEL".into()
            } else if index == 47 {
                "LATEST-SENTINEL".into()
            } else {
                format!("message-{index}: {}", "wrapped content ".repeat(3))
            };
            state.codex.push_message("Interviewer".into(), message);
        }
        for (width, height) in [(120, 40), (80, 24)] {
            state.codex.scroll = 0;
            let latest = rendered(&state, width, height);
            assert!(latest.contains("LATEST-SENTINEL"), "{width}x{height}");
            assert!(!latest.contains("OLDEST-SENTINEL"), "{width}x{height}");
            state.codex.scroll = u16::MAX;
            let oldest = rendered(&state, width, height);
            assert!(oldest.contains("OLDEST-SENTINEL"), "{width}x{height}");
            assert!(!oldest.contains("LATEST-SENTINEL"), "{width}x{height}");
        }
    }

    #[test]
    fn solve_footer_reports_context_and_cancel_target_without_clipping() {
        let mut state = solve_state();
        assert!(rendered_footer(&state, 80).contains("i insert"));
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert!(rendered_footer(&state, 80).contains("i interview"));
        state.solve.as_mut().unwrap().running = Some((
            crate::app::model::OperationId(1),
            0,
            crate::app::RunIntent::Test,
        ));
        assert!(rendered_footer(&state, 80).contains("Ctrl-C runner"));
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.codex.active = Some((
            crate::app::model::OperationId(2),
            0,
            crate::codex::prompt::Mode::Interviewer,
        ));
        assert!(rendered_footer(&state, 80).contains("Ctrl-C Codex"));
        for width in [60_u16, 80, 120] {
            assert!(rendered_footer(&state, width).width() <= usize::from(width));
        }
    }

    #[test]
    fn invalid_and_overlength_commands_render_bounded_errors() {
        use crate::app::EditorAction;
        use crate::editor::{MAX_COMMAND_BYTES, Mode};

        for (command, expected) in [
            ("bogus".to_string(), "unsupported command: :bogus"),
            (
                "x".repeat(MAX_COMMAND_BYTES + 1),
                "command exceeds 256 bytes",
            ),
        ] {
            let mut state = solve_state();
            reduce(
                &mut state,
                Event::Command(Action::Editor(EditorAction::Normal(':'))),
            );
            for character in command.chars() {
                reduce(
                    &mut state,
                    Event::Command(Action::Editor(EditorAction::CommandChar(character))),
                );
            }
            if command == "bogus" {
                reduce(
                    &mut state,
                    Event::Command(Action::Editor(EditorAction::ExecuteCommand)),
                );
            }

            let solve = state.solve.as_ref().unwrap();
            assert_eq!(solve.editor.mode, Mode::Normal);
            assert_eq!(solve.editor.error.as_deref(), Some(expected));
            assert!(solve.editor.error.as_ref().unwrap().len() <= MAX_COMMAND_BYTES + 32);
            let view = rendered(&state, 80, 24);
            assert!(view.contains(expected));
        }
    }

    #[test]
    fn unicode_and_long_markdown_are_bounded() {
        let text = markdown_text(&format!(
            "# Héading 🦀\n```rust\n{}\n```",
            "界".repeat(MAX_RENDERED_MARKDOWN_CHARS + 10)
        ));
        assert!(text.lines.len() >= 3);
        let rendered_chars = text
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.chars().count())
            .sum::<usize>();
        assert!(rendered_chars <= MAX_RENDERED_MARKDOWN_CHARS);
    }
}
