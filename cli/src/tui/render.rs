use crate::app::model::{
    AppState, Focus, MAX_RENDERED_MARKDOWN_CHARS, MAX_ROWS, Screen, SolvePane,
};
use crate::editor::Mode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
        let mut row = String::new();
        let mut row_width: usize = 0;
        for character in content.chars() {
            let character_width = character.width().unwrap_or(0);
            if row_width > 0 && row_width.saturating_add(character_width) > width {
                wrapped.push(Line::from(row));
                row = String::new();
                row_width = 0;
            }
            row.push(character);
            row_width = row_width.saturating_add(character_width);
        }
        if !row.is_empty() {
            wrapped.push(Line::from(row));
        }
    }
    Text::from(wrapped)
}

fn wrapped_dialog_text(dialog: &str, width: usize) -> Text<'static> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in markdown_text(dialog).lines {
        let content = line
            .spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>();
        if content.trim().is_empty() {
            wrapped.push(Line::default());
            continue;
        }

        let mut row = String::new();
        let mut row_width = 0usize;
        for word in content.split_whitespace() {
            let word_width = UnicodeWidthStr::width(word);
            if word_width <= width {
                let separator_width = usize::from(!row.is_empty());
                if !row.is_empty()
                    && row_width
                        .saturating_add(separator_width)
                        .saturating_add(word_width)
                        > width
                {
                    wrapped.push(Line::from(std::mem::take(&mut row)));
                    row_width = 0;
                }
                if !row.is_empty() {
                    row.push(' ');
                    row_width = row_width.saturating_add(1);
                }
                row.push_str(word);
                row_width = row_width.saturating_add(word_width);
                continue;
            }

            if !row.is_empty() {
                wrapped.push(Line::from(std::mem::take(&mut row)));
                row_width = 0;
            }
            for grapheme in UnicodeSegmentation::graphemes(word, true) {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                if !row.is_empty() && row_width.saturating_add(grapheme_width) > width {
                    wrapped.push(Line::from(std::mem::take(&mut row)));
                    row_width = 0;
                }
                row.push_str(grapheme);
                row_width = row_width.saturating_add(grapheme_width);
            }
        }
        if !row.is_empty() {
            wrapped.push(Line::from(row));
        }
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
            " {}: {} · memory only · {} mode",
            state.interviewer.backend.display_name(),
            state.interviewer.status.label(),
            state.interviewer.guidance_mode.display_name()
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

fn first_fitting_footer(candidates: impl IntoIterator<Item = String>, width: u16) -> String {
    candidates
        .into_iter()
        .find(|candidate| UnicodeWidthStr::width(candidate.as_str()) <= usize::from(width))
        .unwrap_or_default()
}

fn solve_footer_text(state: &AppState, width: u16) -> String {
    let solve = state
        .solve
        .as_ref()
        .expect("solve footer requires solve state");
    let collapse_verb = match solve.pane {
        SolvePane::Problem if !solve.accessory_panes.problem_expanded => "expand",
        SolvePane::Output if !solve.accessory_panes.output_expanded => "expand",
        SolvePane::Interview if !solve.accessory_panes.interview_expanded => "expand",
        SolvePane::Problem | SolvePane::Output | SolvePane::Interview => "collapse",
        SolvePane::Editor => "pane",
    };
    let mut candidates = if solve.pane == SolvePane::Interview
        && state.interviewer.status == crate::app::model::InterviewerStatus::Disclosure
    {
        vec![
            "y/Enter accept · n/Esc decline · Space t/s/b/c/?".to_string(),
            "y accept · n decline · Space actions/?".to_string(),
            "y accept · n decline".to_string(),
        ]
    } else if solve.pane == SolvePane::Interview && state.interviewer.composer_focused {
        vec![
            "Type question · ↑/↓/PgUp/PgDn/Home/End scroll · Enter send · Esc close · Tab panes"
                .to_string(),
            "Enter send · Esc close · ↑/↓/PgUp/PgDn scroll · Tab panes".to_string(),
        ]
    } else {
        match (solve.pane, solve.editor.mode) {
            (SolvePane::Editor, Mode::Normal) => vec![
                "Space t test · s submit · b back · ? help · Tab panes".to_string(),
                "Space t/s/b/? · Tab panes".to_string(),
            ],
            (SolvePane::Editor, Mode::Insert) => vec![
                "Tab/Shift-Tab to Neovim · Esc returns to Normal · Ctrl-Q quit".to_string(),
                "Tab to Neovim · Esc normal · Ctrl-Q quit".to_string(),
            ],
            (SolvePane::Editor, Mode::Visual) => vec![
                "Neovim Visual · Esc then Space actions · Ctrl-Q quit · Tab panes".to_string(),
                "Visual · Esc normal · Ctrl-Q quit · Tab panes".to_string(),
            ],
            (SolvePane::Editor, Mode::Command) => vec![
                "Neovim command · Esc then Space actions · Ctrl-Q quit · Tab panes".to_string(),
                "Command · Esc normal · Ctrl-Q quit · Tab panes".to_string(),
            ],
            (SolvePane::Interview, _)
                if matches!(
                    state.interviewer.status,
                    crate::app::model::InterviewerStatus::ProtocolError
                        | crate::app::model::InterviewerStatus::Disconnected
                ) =>
            {
                vec![
                    format!(
                        "i retry · Space-m mode · t test · s submit · b back · c {collapse_verb} · ? help · Tab panes"
                    ),
                    format!("Space t/s/b/? · c {collapse_verb} · Tab panes"),
                ]
            }
            (SolvePane::Interview, _) => vec![
                format!(
                    "i ask · Space-m mode · ↑/↓ row · PgUp/PgDn page · Home/End · Space t/s/b/c/? · Tab panes"
                ),
                format!(
                    "i ask · Space-m mode · PgUp/PgDn/Home/End · Space t/s/b/? · c {collapse_verb} · Tab panes"
                ),
                format!("Space t/s/b/? · c {collapse_verb} · Tab panes"),
            ],
            (SolvePane::Problem | SolvePane::Output, _) => vec![
                format!(
                    "i interview · ↑/↓ scroll · Space t test · s submit · b back · c {collapse_verb} · ? help · Tab panes"
                ),
                format!("Space t/s/b/? · c {collapse_verb} · Tab panes"),
            ],
        }
    };
    let interviewer_active =
        state.interviewer.active.is_some() || state.interviewer.connecting.is_some();
    let runner_active = solve.running.is_some();
    let cancel = if interviewer_active && (solve.pane == SolvePane::Interview || !runner_active) {
        Some("Interviewer")
    } else if runner_active {
        Some("runner")
    } else {
        None
    };
    if let Some(target) = cancel {
        let mut with_cancel = candidates
            .iter()
            .map(|candidate| format!("{candidate} · Ctrl-C {target}"))
            .collect::<Vec<_>>();
        with_cancel.push(format!("Space actions · Ctrl-C {target} · Tab panes"));
        with_cancel.append(&mut candidates);
        candidates = with_cancel;
    }
    candidates.push("Space actions/? · Tab panes".into());
    candidates.push("Tab panes".into());
    first_fitting_footer(candidates, width)
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
            "Problem / Examples [active] [-]"
        } else {
            "Problem / Examples [-]"
        }))
        .wrap(Wrap { trim: false })
        .scroll((solve.problem_scroll, 0))
}
fn solve_output(state: &AppState) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    Paragraph::new(solve.output.clone())
        .block(block(if solve.pane == SolvePane::Output {
            "Output / Test [active] [-]"
        } else {
            "Output / Test [-]"
        }))
        .wrap(Wrap { trim: false })
        .scroll((solve.output_scroll, 0))
}
fn speaker_badge(label: &str, backend_name: &str) -> Line<'static> {
    let badge = |text: String, color: Color| {
        Line::styled(
            text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )
    };
    if label.starts_with("You · ") {
        badge(format!(" {} ", label.to_uppercase()), Color::Cyan)
    } else if label == "Hinter · Interview" {
        badge(" HINTER · INTERVIEW ".into(), Color::Yellow)
    } else if label == "Interviewer · Interview" {
        badge(
            format!(
                " {} · INTERVIEWER · INTERVIEW ",
                backend_name.to_uppercase()
            ),
            Color::Magenta,
        )
    } else if label == "Tutor" || label == "Tutor help" {
        badge(
            format!(
                " {} · {} ",
                backend_name.to_uppercase(),
                label.to_uppercase()
            ),
            Color::Green,
        )
    } else if label.starts_with("Submission review · ") {
        badge(format!(" {label} "), Color::Green)
    } else {
        badge(format!(" {label} "), Color::White)
    }
}
fn solve_interview(state: &AppState, area: Rect) -> Paragraph<'static> {
    let solve = state.solve.as_ref().unwrap();
    let backend_name = state.interviewer.backend.display_name();
    let mut lines = vec![Line::from(format!(
        "{backend_name} {} · Transcript memory only · {} mode · not saved",
        state.interviewer.status.label(),
        state.interviewer.guidance_mode.display_name()
    ))];
    match state.interviewer.status {
        crate::app::model::InterviewerStatus::Disabled => lines.push(Line::from(
            "Interviewer is Disabled. Local editor, tests, and submission remain available.",
        )),
        crate::app::model::InterviewerStatus::Disclosure => {
            lines.push(Line::from("Privacy disclosure"));
            if area.width < 58 && state.interviewer.backend == crate::interviewer::Backend::Pi {
                lines.push(Line::from(
                    "Sends selected statement/current source/bounded test output/transcript/question to Pi's selected provider.",
                ));
                lines.push(Line::from(
                    "Reads Pi settings/models/auth and allowlisted provider credential environment; auth !command entries execute.",
                ));
                lines.push(Line::from(
                    "Disables sessions, tools/bash, extensions, skills, templates, themes,",
                ));
                lines.push(Line::from(
                    "context files, approvals, telemetry, updates, and startup network.",
                ));
                lines.push(Line::from(
                    "User-trusted executable; version checks compatibility, not provenance. y accept · n decline",
                ));
            } else {
                lines.push(Line::from(
                    "The selected statement, current source, bounded latest test output,",
                ));
                match state.interviewer.backend {
                    crate::interviewer::Backend::Pi => {
                        lines.push(Line::from(
                        "in-memory transcript, and your question go to Pi's configured default model/provider.",
                    ));
                        lines.push(Line::from(
                            "Configured executable is user-selected and trusted.",
                        ));
                        lines.push(Line::from("Version checks compatibility,"));
                        lines.push(Line::from("not provenance."));
                        lines.push(Line::from(
                        "Pi may read its settings, models, auth file, and allowlisted provider auth environment.",
                    ));
                        lines.push(Line::from(
                            "Leading !command credentials in auth.json execute via Pi's shell.",
                        ));
                        lines.push(Line::from(
                        "Sessions, tools/bash, extensions, skills, templates, themes, context files,",
                    ));
                        lines.push(Line::from(
                        "approvals, telemetry, update checks, and startup network operations are disabled.",
                    ));
                    }
                    crate::interviewer::Backend::Codex => {
                        lines.push(Line::from(
                            "in-memory transcript, and your question use Codex's configured default model (not reported to Interview Tutor) through OpenAI.",
                        ));
                        lines.push(Line::from("Configured executable is"));
                        lines.push(Line::from("user-selected and trusted."));
                        lines.push(Line::from("Version checks compatibility,"));
                        lines.push(Line::from("not provenance."));
                        lines.push(Line::from(
                            "Codex may read local config, MCP, or sandbox-readable paths.",
                        ));
                        lines.push(Line::from("Dedicated profile/home is safer."));
                    }
                    crate::interviewer::Backend::None => {
                        unreachable!("disabled backend has no disclosure")
                    }
                }
                lines.push(Line::from(
                    "Provider account controls apply. y/Enter accept · n/Esc decline",
                ));
            }
        }
        crate::app::model::InterviewerStatus::AuthRequired => {
            lines.push(Line::from(match state.interviewer.backend {
                crate::interviewer::Backend::Pi => {
                    "Authentication required. Configure Pi provider credentials and retry."
                }
                crate::interviewer::Backend::Codex => {
                    "Authentication required. Exit and run: codex login"
                }
                crate::interviewer::Backend::None => {
                    "Authentication is unavailable while interviewer is disabled."
                }
            }))
        }
        crate::app::model::InterviewerStatus::Declined => lines.push(Line::from(format!(
            "{backend_name} declined. Local editor, tests, and submission remain available."
        ))),
        _ => {
            if state.interviewer.omitted_messages > 0 {
                let suffix = if state.interviewer.omitted_messages == 1 {
                    "message"
                } else {
                    "messages"
                };
                lines.push(Line::styled(
                    format!(
                        "… {} earlier {suffix} omitted from this bounded display …",
                        state.interviewer.omitted_messages
                    ),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ));
                lines.push(Line::default());
            }
            let body_width = usize::from(area.width.saturating_sub(4)).max(1);
            for (index, (label, message)) in state.interviewer.messages.iter().enumerate() {
                if index > 0 {
                    lines.push(Line::default());
                }
                lines.push(speaker_badge(label, backend_name));
                for mut row in wrapped_dialog_text(message, body_width).lines {
                    row.spans.insert(0, Span::raw("  "));
                    lines.push(row);
                }
            }
            lines.push(Line::from(""));
            let cursor = if state.interviewer.composer_focused {
                "▌"
            } else {
                ""
            };
            lines.push(Line::from(format!(
                "Question: {}{cursor}",
                state.interviewer.composer
            )));
            lines.push(Line::from(
                "i ask · Enter send · ↑/↓ row · PgUp/PgDn page · Home/End · Space-h help",
            ));
            lines.push(Line::from("Space-m mode · Space-r reset · Ctrl-C cancel"));
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
    let retained_below = usize::from(state.interviewer.scroll).min(latest_offset);
    let offset = latest_offset.saturating_sub(retained_below);
    Paragraph::new(lines)
        .block(block(&format!(
            "Interview{} [-] · {} mode",
            if solve.pane == SolvePane::Interview {
                " [active]"
            } else {
                ""
            },
            state.interviewer.guidance_mode.display_name(),
        )))
        .wrap(Wrap { trim: false })
        .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0))
}
const FULL_SOLVE_WIDTH: u16 = 100;
const FULL_SOLVE_CONTENT_HEIGHT: u16 = 27;
const SIDE_RAIL_WIDTH: u16 = 6;
const FOCUSED_SIDE_PERCENT: u16 = 45;
const UNFOCUSED_SIDE_PERCENT: u16 = 28;

#[derive(Clone, Copy)]
struct FullSolveLayout {
    problem: Rect,
    editor: Rect,
    interview: Rect,
    output: Rect,
}

fn accessory_expanded(state: &AppState, pane: SolvePane) -> bool {
    let solve = state.solve.as_ref().expect("solve state");
    match pane {
        SolvePane::Editor => true,
        SolvePane::Problem => solve.accessory_panes.problem_expanded,
        SolvePane::Output => solve.accessory_panes.output_expanded,
        SolvePane::Interview => solve.accessory_panes.interview_expanded,
    }
}

fn side_width(area_width: u16, expanded: bool, focused: bool) -> u16 {
    if !expanded {
        return SIDE_RAIL_WIDTH;
    }
    let percent = if focused {
        FOCUSED_SIDE_PERCENT
    } else {
        UNFOCUSED_SIDE_PERCENT
    };
    area_width.saturating_mul(percent) / 100
}

fn full_solve_layout(state: &AppState, area: Rect) -> Option<FullSolveLayout> {
    if area.width < FULL_SOLVE_WIDTH || area.height < FULL_SOLVE_CONTENT_HEIGHT {
        return None;
    }
    let solve = state.solve.as_ref()?;
    let problem_width = side_width(
        area.width,
        solve.accessory_panes.problem_expanded,
        solve.pane == SolvePane::Problem,
    );
    let interview_width = side_width(
        area.width,
        solve.accessory_panes.interview_expanded,
        solve.pane == SolvePane::Interview,
    );
    assert!(problem_width.saturating_add(interview_width) < area.width);
    let columns =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(interview_width)]).split(area);
    let left = if solve.accessory_panes.output_expanded {
        Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(columns[0])
    } else {
        Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(columns[0])
    };
    let upper =
        Layout::horizontal([Constraint::Length(problem_width), Constraint::Min(1)]).split(left[0]);
    Some(FullSolveLayout {
        problem: upper[0],
        editor: upper[1],
        interview: columns[1],
        output: left[1],
    })
}

fn collapsed_rail(label: &str, active: bool) -> Paragraph<'static> {
    let mut lines = Vec::new();
    if active {
        lines.push(Line::from("*"));
    }
    lines.push(Line::from("[+]"));
    Paragraph::new(lines).block(block(label))
}

fn compact_pane_label(label: &str, expanded: bool) -> Line<'static> {
    Line::from(format!("{label} {}", if expanded { "[-]" } else { "[+]" }))
}

fn render_solve(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let solve = state.solve.as_ref().unwrap();
    if let Some(layout) = full_solve_layout(state, area) {
        if solve.accessory_panes.problem_expanded {
            frame.render_widget(solve_problem(state), layout.problem);
        } else {
            frame.render_widget(
                collapsed_rail("Prob", solve.pane == SolvePane::Problem),
                layout.problem,
            );
        }
        solve_editor(frame, state, layout.editor);
        if solve.accessory_panes.interview_expanded {
            frame.render_widget(solve_interview(state, layout.interview), layout.interview);
        } else {
            frame.render_widget(
                collapsed_rail("Intv", solve.pane == SolvePane::Interview),
                layout.interview,
            );
        }
        if solve.accessory_panes.output_expanded {
            frame.render_widget(solve_output(state), layout.output);
        } else {
            frame.render_widget(
                collapsed_rail("Outp", solve.pane == SolvePane::Output),
                layout.output,
            );
        }
    } else {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
        let panes = vec![
            Line::from("Editor"),
            compact_pane_label("Problem", solve.accessory_panes.problem_expanded),
            compact_pane_label("Output", solve.accessory_panes.output_expanded),
            compact_pane_label("Interview", solve.accessory_panes.interview_expanded),
        ];
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
            SolvePane::Problem if solve.accessory_panes.problem_expanded => {
                frame.render_widget(solve_problem(state), chunks[1])
            }
            SolvePane::Output if solve.accessory_panes.output_expanded => {
                frame.render_widget(solve_output(state), chunks[1])
            }
            SolvePane::Interview if solve.accessory_panes.interview_expanded => {
                frame.render_widget(solve_interview(state, chunks[1]), chunks[1])
            }
            SolvePane::Problem | SolvePane::Output | SolvePane::Interview => {
                solve_editor(frame, state, chunks[1])
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
    let editor = if let Some(layout) = full_solve_layout(state, content) {
        layout.editor
    } else if solve.pane == SolvePane::Editor || !accessory_expanded(state, solve.pane) {
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
            "Ctrl-Q"
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
        let popup_height = if state.screen == Screen::Solve {
            70
        } else {
            55
        };
        let popup = centered(70, popup_height, area);
        frame.render_widget(Clear, popup);
        let help = if state.screen == Screen::Solve {
            format!(
                "Solve help\n\nCurrent: {}\nSpace leader in Editor Normal/accessories: t test · s submit · b autosave back · c toggle pane · ? help\nCtrl-Q quits from every pane and editor mode; dirty source requires confirmation\nEditor cannot collapse; Tab/Shift-Tab switches panes in Normal/Visual/Command and accessories, but goes to Neovim in Insert/Replace/terminal modes\nProblem/Output/Interview: i expands and focuses Interview; focused accessories widen\nInterview Space-m toggles Interview/Tutor; Tutor gives direct help and may show complete solutions\nComposer captures typed spaces; Interview uses ↑/↓ by row, PgUp/PgDn by page, Home/End oldest/latest\nCompatibility aliases: F5 test · F9 submit",
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

    fn buffer_rows_in(buffer: &ratatui::buffer::Buffer, area: Rect) -> Vec<String> {
        let right = area.x.saturating_add(area.width);
        let bottom = area.y.saturating_add(area.height);
        (area.y..bottom)
            .map(|y| {
                (area.x..right)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn buffer_text_in(buffer: &ratatui::buffer::Buffer, area: Rect) -> String {
        buffer_rows_in(buffer, area).join("\n")
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
    fn wrapped_dialog_preserves_whole_words_and_splits_only_oversized_tokens() {
        let rows = |text: &str, width| {
            wrapped_dialog_text(text, width)
                .lines
                .into_iter()
                .map(|line| {
                    line.spans
                        .into_iter()
                        .map(|span| span.content.into_owned())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            rows("hello are you fine", 8),
            vec!["hello", "are you", "fine"]
        );
        assert_eq!(rows("hello\nare you", 8), vec!["hello", "are you"]);
        assert_eq!(rows("extraordinary", 5), vec!["extra", "ordin", "ary"]);
        assert!(
            rows("café déjà", 6)
                .iter()
                .all(|row| UnicodeWidthStr::width(row.as_str()) <= 6)
        );
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
            generation: crate::neovim::SessionGeneration(1),
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
            accessory_panes: crate::app::model::AccessoryPaneState::default(),
            output: "compiler error".into(),
            output_scroll: 0,
            problem_scroll: 0,
            running: None,
            cancellation: None,
            pending_save: None,
            pending_draft_save: None,
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
    fn undersized_solve_instructs_the_global_guarded_quit_binding() {
        let mut state = solve_state();
        for pane in [
            SolvePane::Editor,
            SolvePane::Problem,
            SolvePane::Output,
            SolvePane::Interview,
        ] {
            state.solve.as_mut().unwrap().pane = pane;
            for mode in ["n", "i", "v", "c"] {
                state
                    .solve
                    .as_mut()
                    .unwrap()
                    .editor
                    .update_neovim_mode(mode);
                let view = rendered(&state, 59, 19);
                assert!(
                    view.contains("Press Ctrl-Q to quit"),
                    "pane {pane:?} mode {mode}"
                );
                assert!(view.contains("press twice to confirm"));
            }
        }
    }

    #[test]
    fn interview_disclosure_and_labels_render_at_supported_sizes() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.disable_interviewer();
        let disabled = rendered(&state, 120, 40);
        assert!(disabled.contains("Pi Disabled"));

        state.interviewer.enabled = true;
        state.interviewer.status = crate::app::model::InterviewerStatus::Disclosure;
        let disclosure = rendered(&state, 120, 40);
        assert!(disclosure.contains("Privacy disclosure"));
        assert!(disclosure.contains("provider"));
        assert!(disclosure.contains("settings"));
        assert!(disclosure.contains("!command"));
        assert!(disclosure.contains("tools/bash"));
        assert!(disclosure.contains("provenance"));
        let compact_disclosure = rendered(&state, 80, 24);
        assert!(compact_disclosure.contains("Privacy disclosure"));
        assert!(compact_disclosure.contains("settings"));
        assert!(compact_disclosure.contains("tools/bash"));
        assert!(compact_disclosure.contains("provenance"));
        state.interviewer.backend = crate::interviewer::Backend::Codex;
        let codex_disclosure = rendered(&state, 120, 40);
        assert!(codex_disclosure.contains("OpenAI"));
        assert!(codex_disclosure.contains("MCP"));
        assert!(codex_disclosure.contains("Dedicated profile/home"));
        assert!(rendered(&state, 59, 19).contains("Terminal too small"));
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        state
            .interviewer
            .messages
            .push(("Interviewer · Interview".into(), "Question".into()));
        state
            .interviewer
            .messages
            .push(("Hinter · Interview".into(), "Hint".into()));
        state.interviewer.messages.push((
            "Submission review · recorded revision 7".into(),
            "Review".into(),
        ));
        let view = rendered(&state, 80, 24);
        assert!(
            view.contains("INTERVIEWER")
                && view.contains("HINTER")
                && view.contains("Submission review · recorded revision 7")
        );
    }

    #[test]
    fn transcript_renders_distinct_styled_speaker_blocks() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.backend = crate::interviewer::Backend::Pi;
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        state.interviewer.push_message(
            "You · Interview".into(),
            format!("{}界界", "my question ".repeat(8)),
        );
        state
            .interviewer
            .push_message("Interviewer · Interview".into(), "interviewer reply".into());
        state
            .interviewer
            .push_message("Hinter · Interview".into(), "hint body".into());
        state.interviewer.push_message(
            "Submission review · recorded revision 7".into(),
            "review body".into(),
        );

        let area = Rect::new(0, 0, 56, 30);
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| frame.render_widget(solve_interview(&state, area), area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content = buffer.content();
        let rows = buffer_rows_in(buffer, area);
        let interior_rows = buffer_rows_in(
            buffer,
            Rect::new(
                area.x.saturating_add(1),
                area.y,
                area.width.saturating_sub(2),
                area.height,
            ),
        );
        let find_row = |needle: &str| rows.iter().position(|row| row.contains(needle));
        let styled_text = |color: Color| -> String {
            content
                .iter()
                .filter(|cell| cell.style().fg == Some(color))
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        let bold_at = |color: Color| -> bool {
            content.iter().any(|cell| {
                cell.style().fg == Some(color) && cell.style().add_modifier.contains(Modifier::BOLD)
            })
        };

        assert!(styled_text(Color::Cyan).contains("YOU"));
        assert!(styled_text(Color::Magenta).contains("PI · INTERVIEWER"));
        assert!(styled_text(Color::Yellow).contains("HINTER"));
        assert!(styled_text(Color::Green).contains("Submission review · recorded revision 7"));
        for color in [Color::Cyan, Color::Magenta, Color::Yellow, Color::Green] {
            assert!(bold_at(color), "{color:?} badge renders bold");
        }

        let you_row = find_row("YOU").expect("YOU badge visible");
        let interviewer_row = find_row("INTERVIEWER").expect("interviewer badge visible");
        let question_body = find_row("  my question").expect("prefixed question body");
        assert!(
            interior_rows[you_row + 1..interviewer_row]
                .iter()
                .any(|row| row.trim().is_empty()),
            "turns are separated by blank rows"
        );
        assert!(question_body > you_row && question_body < interviewer_row);
        for row in interior_rows[you_row + 1..interviewer_row]
            .iter()
            .filter(|row| !row.trim().is_empty())
        {
            assert!(row.starts_with("  "), "body row lacks prefix: {row:?}");
        }
    }

    #[test]
    fn guidance_mode_and_originating_turn_labels_are_unmistakable_at_supported_widths() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        state.interviewer.guidance_mode = crate::interviewer::GuidanceMode::Tutor;
        state
            .interviewer
            .push_message("You · Tutor".into(), "show the implementation".into());
        state
            .interviewer
            .push_message("Tutor".into(), "complete direct answer".into());
        for (width, height) in [(60, 24), (80, 24), (120, 40)] {
            let view = rendered(&state, width, height);
            assert!(view.contains("Interview [active] [-]"), "{width}x{height}");
            assert!(view.contains("Tutor mode"), "{width}x{height}");
            assert!(view.contains("YOU · TUTOR"), "{width}x{height}");
            assert!(view.contains("PI · TUTOR"), "{width}x{height}");
        }
    }

    #[test]
    fn focused_interview_displays_more_transcript_than_unfocused() {
        let mut state = solve_state();
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        for index in 0..40 {
            state.interviewer.push_message(
                "Interviewer".into(),
                format!("message-{index}: {}", "wrapped content ".repeat(8)),
            );
        }
        let mut visible_messages = |pane: SolvePane| {
            state.solve.as_mut().unwrap().pane = pane;
            let view = rendered(&state, 120, 40);
            (0..40)
                .filter(|index| view.contains(&format!("message-{index}:")))
                .count()
        };
        let unfocused = visible_messages(SolvePane::Editor);
        let focused = visible_messages(SolvePane::Interview);
        assert!(
            focused > unfocused,
            "focused {focused} vs unfocused {unfocused}"
        );
    }

    #[test]
    fn full_layout_sides_are_focus_responsive_with_compact_rails() {
        let mut state = solve_state();
        let area = Rect::new(0, 0, 120, 30);
        let layout = |state: &AppState| full_solve_layout(state, area).unwrap();
        let base = layout(&state);
        assert_eq!(base.problem.width, 33, "unfocused expanded problem width");
        assert_eq!(base.interview.y, area.y);
        assert_eq!(base.interview.height, area.height);
        assert_eq!(base.output.right(), base.interview.x);
        assert_eq!(base.output.width, area.width - base.interview.width);
        assert_eq!(
            base.interview.width, 33,
            "unfocused expanded interview width"
        );
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        let focused_interview = layout(&state);
        assert_eq!(focused_interview.interview.width, 54, "45% of upper row");
        assert_eq!(focused_interview.problem.width, 33);
        assert_eq!(
            focused_interview.editor.width,
            area.width - focused_interview.problem.width - focused_interview.interview.width
        );
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        let focused_problem = layout(&state);
        assert_eq!(focused_problem.problem.width, 54, "focused problem widens");
        assert_eq!(focused_problem.interview.width, 33);
        assert!(
            focused_problem.editor.width >= 20,
            "editor keeps usable space"
        );

        // Collapsing returns substantial width to the editor; rails stay compact.
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .problem_expanded = false;
        let collapsed_problem = layout(&state);
        assert_eq!(collapsed_problem.problem.width, SIDE_RAIL_WIDTH);
        assert_eq!(SIDE_RAIL_WIDTH, 6);
        assert!(collapsed_problem.editor.width > base.editor.width + 20);
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .interview_expanded = false;
        let collapsed_both = layout(&state);
        assert_eq!(collapsed_both.interview.width, SIDE_RAIL_WIDTH);
        assert!(collapsed_both.editor.width > collapsed_problem.editor.width + 20);

        let view = rendered(&state, 100, 30);
        assert!(view.contains("Prob") && view.contains("Intv") && view.contains("Outp"));
        assert!(view.contains("[+]"));
    }

    #[test]
    fn compact_rails_mark_the_focused_pane() {
        let mut state = solve_state();
        {
            let solve = state.solve.as_mut().unwrap();
            solve.accessory_panes.problem_expanded = false;
            solve.accessory_panes.output_expanded = false;
            solve.accessory_panes.interview_expanded = false;
            solve.pane = SolvePane::Problem;
        }
        let rail_text = |state: &AppState| {
            let width = 100;
            let height = 30;
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| render(frame, state)).unwrap();
            let content_area = Rect::new(0, 2, width, height.saturating_sub(3));
            let problem_area = full_solve_layout(state, content_area).unwrap().problem;
            buffer_text_in(terminal.backend().buffer(), problem_area)
        };
        let active = rail_text(&state);
        assert!(
            active.contains('*'),
            "focused rail shows a marker: {active}"
        );
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        let inactive = rail_text(&state);
        assert!(!inactive.contains('*'));
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
        let grid_area = neovim_grid_area(&state, 120, 40).unwrap();
        let keyword = &terminal.backend().buffer()[(grid_area.x, grid_area.y)];
        assert_eq!(keyword.symbol(), "d");
        assert_eq!(keyword.fg, Color::Rgb(0xff, 0x00, 0xff));
    }

    #[test]
    fn collapse_combinations_render_full_rails_compact_markers_and_editor_fallback() {
        for mask in 0_u8..8 {
            let mut state = solve_state();
            let problem_expanded = mask & 1 == 0;
            let output_expanded = mask & 2 == 0;
            let interview_expanded = mask & 4 == 0;
            state.solve.as_mut().unwrap().accessory_panes = crate::app::model::AccessoryPaneState {
                problem_expanded,
                output_expanded,
                interview_expanded,
            };

            let width: u16 = 100;
            let height: u16 = 30;
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| render(frame, &state)).unwrap();
            let content_area = Rect::new(0, 2, width, height.saturating_sub(3));
            let layout = full_solve_layout(&state, content_area).unwrap();
            let buffer = terminal.backend().buffer();
            assert!(!buffer_text_in(buffer, content_area).contains("Solve panes"));
            for (expanded, area, expanded_title, rail_title) in [
                (
                    problem_expanded,
                    layout.problem,
                    "Problem / Examples",
                    "Prob",
                ),
                (output_expanded, layout.output, "Output / Test", "Outp"),
                (interview_expanded, layout.interview, "Interview", "Intv"),
            ] {
                let pane = buffer_text_in(buffer, area);
                if expanded {
                    assert!(pane.contains(expanded_title), "mask {mask}: {pane}");
                    assert!(!pane.contains("[+]"), "mask {mask}: {pane}");
                } else {
                    assert!(pane.contains(rail_title), "mask {mask}: {pane}");
                    assert!(pane.contains("[+]"), "mask {mask}: {pane}");
                }
            }

            let compact = rendered(&state, 80, 24);
            assert!(compact.contains("Solve panes"), "mask {mask}");
            assert_eq!(compact.contains("Problem [+]"), !problem_expanded);
            assert_eq!(compact.contains("Output [+]"), !output_expanded);
            assert_eq!(compact.contains("Interview [+]"), !interview_expanded);
            assert!(rendered(&state, 59, 19).contains("Terminal too small"));
        }

        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .problem_expanded = false;
        let compact = rendered(&state, 80, 24);
        assert!(compact.contains("Problem [+]"));
        assert!(compact.contains("Waiting for Neovim redraw"));
        assert!(neovim_grid_area(&state, 80, 24).is_some());
    }

    #[test]
    fn collapsed_accessories_redistribute_full_editor_space() {
        let mut state = solve_state();
        let base = neovim_grid_size(&state, 100, 30).unwrap();
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .problem_expanded = false;
        let side_collapsed = neovim_grid_size(&state, 100, 30).unwrap();
        assert!(side_collapsed.0 > base.0);
        assert_eq!(side_collapsed.1, base.1);
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .output_expanded = false;
        let output_collapsed = neovim_grid_size(&state, 100, 30).unwrap();
        assert!(output_collapsed.0 >= side_collapsed.0);
        assert!(output_collapsed.1 > side_collapsed.1);

        let preserved = state.solve.as_ref().unwrap().accessory_panes;
        assert!(rendered(&state, 80, 24).contains("Problem [+]"));
        assert!(rendered(&state, 100, 30).contains("Prob"));
        assert!(rendered(&state, 100, 30).contains("[+]"));
        assert_eq!(state.solve.as_ref().unwrap().accessory_panes, preserved);
    }

    #[test]
    fn solve_header_preserves_exact_interviewer_state_and_memory_badge_at_supported_widths() {
        let mut state = solve_state();
        for status in [
            crate::app::model::InterviewerStatus::Disabled,
            crate::app::model::InterviewerStatus::Offline,
            crate::app::model::InterviewerStatus::AuthRequired,
            crate::app::model::InterviewerStatus::Ready,
            crate::app::model::InterviewerStatus::Thinking,
            crate::app::model::InterviewerStatus::ProtocolError,
        ] {
            state.interviewer.status = status;
            for width in [60_u16, 80, 120] {
                let row = rendered_row(&state, width, 24, 1);
                assert_eq!(
                    row,
                    format!(" Pi: {} · memory only · Interview mode", status.label()),
                    "width {width}"
                );
            }
        }
    }

    #[test]
    fn oldest_interview_view_marks_messages_omitted_by_the_display_bound() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        for index in 0..=crate::app::model::MAX_VISIBLE_TRANSCRIPT_ENTRIES {
            state
                .interviewer
                .push_message("Interviewer".into(), format!("message-{index}"));
        }
        assert_eq!(state.interviewer.omitted_messages, 1);
        state.interviewer.scroll = u16::MAX;

        let oldest = rendered(&state, 80, 24);
        assert!(oldest.contains("1 earlier message omitted"));
        assert!(oldest.contains("message-1"));
        assert!(!oldest.contains("message-0"));
    }

    #[test]
    fn interview_scroll_reaches_oldest_and_latest_messages_at_both_layouts() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.status = crate::app::model::InterviewerStatus::Feedback;
        for index in 0..48 {
            let message = if index == 0 {
                "OLDEST-SENTINEL".into()
            } else if index == 47 {
                "LATEST-SENTINEL".into()
            } else {
                format!("message-{index}: {}", "wrapped content ".repeat(3))
            };
            state
                .interviewer
                .push_message("Interviewer".into(), message);
        }
        for (width, height) in [(120, 40), (80, 24)] {
            state.interviewer.scroll = 0;
            let latest = rendered(&state, width, height);
            assert!(latest.contains("LATEST-SENTINEL"), "{width}x{height}");
            assert!(!latest.contains("OLDEST-SENTINEL"), "{width}x{height}");
            state.interviewer.scroll = u16::MAX;
            let oldest = rendered(&state, width, height);
            assert!(oldest.contains("OLDEST-SENTINEL"), "{width}x{height}");
            assert!(!oldest.contains("LATEST-SENTINEL"), "{width}x{height}");
        }
    }

    #[test]
    fn solve_footer_reports_context_and_cancel_target_without_clipping() {
        let mut state = solve_state();
        assert!(rendered_footer(&state, 80).contains("Space t test"));
        assert!(!rendered_footer(&state, 80).contains("F5"));
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert!(rendered_footer(&state, 80).contains("Space t/s/b"));
        state.solve.as_mut().unwrap().running = Some((
            crate::app::model::OperationId(1),
            0,
            crate::app::RunIntent::Test,
        ));
        assert!(rendered_footer(&state, 80).contains("Ctrl-C runner"));
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.active = Some((
            crate::app::model::OperationId(2),
            0,
            crate::interviewer::Mode::Interviewer,
            crate::interviewer::GuidanceMode::Interview,
        ));
        assert!(rendered_footer(&state, 80).contains("Ctrl-C Interviewer"));
        for width in [1_u16, 10, 20, 60, 80, 120] {
            assert!(rendered_footer(&state, width).width() <= usize::from(width));
        }

        state.interviewer.active = None;
        state.solve.as_mut().unwrap().running = None;
        state.show_help = true;
        let help = rendered(&state, 100, 30);
        assert!(help.contains("Space leader"));
        assert!(help.contains("F5 test · F9 submit"));
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
