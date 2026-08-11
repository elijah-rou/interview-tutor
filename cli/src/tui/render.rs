use crate::app::model::{AppState, Focus, MAX_RENDERED_MARKDOWN_CHARS, MAX_ROWS, Screen};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};

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
            if source.trim_start().starts_with("```") {
                in_code = !in_code;
                return Some(Line::from(if in_code { "Code:" } else { "" }));
            }
            let plain = source.trim_start_matches('#').trim_start();
            let prefix = if in_code { "  " } else { "" };
            let allowed = remaining.saturating_sub(prefix.chars().count());
            let clipped: String = plain.chars().take(allowed).collect();
            remaining = remaining.saturating_sub(prefix.chars().count() + clipped.chars().count());
            Some(Line::from(format!("{prefix}{clipped}")))
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn header(state: &AppState) -> Paragraph<'static> {
    let language = state
        .languages
        .get(state.language_index)
        .map_or("none", |item| item.display_name.as_str());
    Paragraph::new(format!(
        " Interview Tutor  Language: {language}  Progress: {}/{}  {}",
        state.data.progress.completed, state.data.progress.total, state.status
    ))
    .style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
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
    Paragraph::new(lines)
        .block(block("Progress"))
        .wrap(Wrap { trim: true })
}

fn sets(state: &AppState) -> Table<'static> {
    let rows = state.data.sets.iter().enumerate().map(|(index, item)| {
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
    .block(block("Problem sets"))
}

fn problems(state: &AppState) -> Table<'static> {
    let rows = state.data.problems.iter().enumerate().map(|(index, item)| {
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
    .block(block("Problems"))
}

fn detail(state: &AppState) -> Paragraph<'static> {
    match &state.data.detail {
        Some(item) => Paragraph::new(markdown_text(&item.statement_markdown))
            .block(block(&format!(
                "{} · {} · {}",
                item.title, item.difficulty, item.topic
            )))
            .wrap(Wrap { trim: false }),
        None => Paragraph::new("No problem detail available").block(block("Problem")),
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
    frame.render_widget(
        Paragraph::new(
            "j/k navigate  Enter open  Esc back  Tab focus  l language  r reload  ? help  q quit",
        ),
        vertical[2],
    );

    if area.width < 60 || area.height < 20 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nResize to at least 60 × 20\nPress q to quit")
                .alignment(ratatui::layout::Alignment::Center)
                .block(block("Resize required")),
            vertical[1],
        );
        return;
    }

    let content = vertical[1];
    if area.width >= 100 && area.height >= 30 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
            .split(content);
        match state.screen {
            Screen::SetMenu => frame.render_widget(sets(state), columns[0]),
            Screen::ProblemList => frame.render_widget(problems(state), columns[0]),
            Screen::ProblemDetail => frame.render_widget(detail(state), columns[0]),
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
                Screen::SetMenu => frame.render_widget(sets(state), chunks[1]),
                Screen::ProblemList => frame.render_widget(problems(state), chunks[1]),
                Screen::ProblemDetail => frame.render_widget(detail(state), chunks[1]),
            }
        }
    }

    if state.show_help {
        let popup = centered(70, 55, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("Help\n\n↑/k up   ↓/j down   Enter open\nEsc/Backspace back   Tab/Shift-Tab focus\nl cycle language   r reload   ? close   q quit").block(block("Keyboard help")).wrap(Wrap { trim: true }), popup);
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
    use crate::database::Difficulty;
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
        assert!(rendered_chars <= MAX_RENDERED_MARKDOWN_CHARS + 10);
    }
}
