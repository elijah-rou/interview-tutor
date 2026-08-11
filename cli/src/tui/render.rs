use crate::app::model::{
    AppState, Focus, MAX_RENDERED_MARKDOWN_CHARS, MAX_ROWS, Screen, SolvePane,
};
use crate::editor::{Mode, highlight_line, highlight_style};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use unicode_segmentation::UnicodeSegmentation;
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
            if solve.stale { " · STALE" } else { "" }
        )
    });
    Paragraph::new(format!(
        " Interview Tutor  Language: {language}  Progress: {}/{}  {}{}",
        state.data.progress.completed, state.data.progress.total, state.status, solve_badges
    ))
    .style(
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
    let visible = usize::from(area.height.saturating_sub(3)).max(1);
    let start = if solve.editor.row >= solve.editor.viewport_row.saturating_add(visible) {
        solve.editor.row.saturating_add(1).saturating_sub(visible)
    } else {
        solve.editor.viewport_row.min(solve.editor.row)
    };
    let visible_columns = usize::from(area.width.saturating_sub(2)).max(1);
    let cursor_line = solve.editor.line(solve.editor.row);
    let mut viewport_column = solve.editor.viewport_column.min(solve.editor.column);
    while viewport_column < solve.editor.column {
        let visible_prefix = cursor_line
            .graphemes(true)
            .skip(viewport_column)
            .take(solve.editor.column - viewport_column)
            .collect::<String>();
        if UnicodeWidthStr::width(visible_prefix.as_str()) < visible_columns {
            break;
        }
        viewport_column += 1;
    }
    let mut lines = solve
        .editor
        .text()
        .split('\n')
        .skip(start)
        .take(visible)
        .map(|line| {
            let bounded = line
                .graphemes(true)
                .skip(viewport_column)
                .take(visible_columns.saturating_mul(2))
                .collect::<String>();
            let spans = highlight_line(&solve.language, &bounded)
                .into_iter()
                .map(|item| {
                    Span::styled(
                        bounded[item.start..item.end].to_string(),
                        highlight_style(item.kind),
                    )
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    let inline = if solve.editor.mode == Mode::Command {
        format!(":{}", solve.editor.command_buffer)
    } else if let Some(error) = &solve.editor.error {
        format!("Error: {error}")
    } else {
        String::new()
    };
    lines.push(Line::styled(inline, Style::default().fg(Color::Red)));
    frame.render_widget(
        Paragraph::new(lines).block(block(if solve.pane == SolvePane::Editor {
            "Editor [active]"
        } else {
            "Editor"
        })),
        area,
    );
    if solve.pane == SolvePane::Editor && matches!(solve.editor.mode, Mode::Insert | Mode::Normal) {
        let visible_prefix = cursor_line
            .graphemes(true)
            .skip(viewport_column)
            .take(solve.editor.column.saturating_sub(viewport_column))
            .collect::<String>();
        let cursor_width = UnicodeWidthStr::width(visible_prefix.as_str());
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(cursor_width).unwrap_or(u16::MAX))
            .min(area.right().saturating_sub(2));
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(solve.editor.row.saturating_sub(start) as u16)
            .min(area.bottom().saturating_sub(2));
        frame.set_cursor_position((x, y));
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
fn solve_interview(state: &AppState) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    Paragraph::new("Interview is offline until Stack 7.\nLocal edit, test, cancel, and submit remain available.").block(block(if solve.pane==SolvePane::Interview{"Interview [active]"}else{"Interview"})).wrap(Wrap{trim:true})
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
        frame.render_widget(solve_interview(state), upper[2]);
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
            SolvePane::Interview => frame.render_widget(solve_interview(state), chunks[1]),
        }
    }
}

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(header(state), vertical[0]);
    let footer = if state.screen == Screen::Solve {
        if area.width >= 80 {
            "Ctrl-S/F5 test  F9 submit  Ctrl-C cancel  Tab panes  Space-b back  Space-q quit"
        } else {
            "F5 test F9 submit Tab panes Space-b back Space-q quit"
        }
    } else {
        footer_text(area.width)
    };
    frame.render_widget(Paragraph::new(footer), vertical[2]);

    if area.width < 60 || area.height < 20 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nResize to at least 60 × 20\nPress Space-q to quit")
                .alignment(ratatui::layout::Alignment::Center)
                .block(block("Resize required")),
            vertical[1],
        );
        return;
    }

    let content = vertical[1];
    if state.screen == Screen::Solve {
        render_solve(frame, state, content);
        return;
    }
    if area.width >= 100 && area.height >= 30 {
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
                Screen::SetMenu => frame.render_widget(sets(state, chunks[1].height), chunks[1]),
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

    if state.show_help {
        let popup = centered(70, 55, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("Help\n\n↑/k up  ↓/j down  Enter open\nEsc back  Tab/Shift-Tab pane\nl language  r reload  ? close  q quit").block(block("Keyboard help")).wrap(Wrap { trim: true }), popup);
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
        assert!(view.contains("Press Space-q to quit"));
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

    #[test]
    fn solve_layouts_offline_placeholder_and_syntax_style() {
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
            pane: SolvePane::Editor,
            output: "compiler error".into(),
            output_scroll: 0,
            problem_scroll: 0,
            running: None,
            cancellation: None,
            pending_save: None,
            stale: false,
            quit_after_save: None,
            refresh_after_submit: false,
        });
        let full = rendered(&state, 120, 40);
        assert!(full.contains("Problem / Examples"));
        assert!(full.contains("compiler error"));
        assert!(full.contains("Stack 7"));
        let compact = rendered(&state, 80, 24);
        assert!(compact.contains("Solve panes"));
        assert!(compact.contains("def solve"));
        assert!(rendered(&state, 59, 19).contains("Terminal too small"));
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &state)).unwrap();
        let keyword = &terminal.backend().buffer()[(37, 2)];
        assert_eq!(keyword.symbol(), "d");
        assert_eq!(keyword.fg, Color::Magenta);
        state.solve.as_mut().unwrap().editor =
            EditorDocument::new(format!("{}END", "界".repeat(100))).unwrap();
        state.solve.as_mut().unwrap().editor.normal('$').unwrap();
        assert!(rendered(&state, 80, 24).contains("END"));
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
