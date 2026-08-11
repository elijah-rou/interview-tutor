use super::effects::{Action, EditorAction, Effect, Event, LoadScope, RunIntent};
use super::model::{
    AppState, CodexStatus, DiscardAction, Focus, MAX_COMPOSER_BYTES, MAX_SCROLL, OperationId,
    Screen, SolvePane,
};
use crate::codex::prompt::Mode as CodexMode;
use crate::editor::{EditorCommand, Mode};

fn load_effect(state: &mut AppState) -> Vec<Effect> {
    let Some(language_slug) = state.language_slug().map(str::to_string) else {
        state.error = Some("no enabled languages".to_string());
        return Vec::new();
    };
    let scope = match state.screen {
        Screen::SetMenu => LoadScope::Global,
        Screen::ProblemList | Screen::ProblemDetail | Screen::Solve => match &state.selected_set_id
        {
            Some(slug) => LoadScope::ProblemSet(slug.clone()),
            None => LoadScope::Global,
        },
    };
    let operation = OperationId(state.next_operation);
    state.next_operation = state
        .next_operation
        .checked_add(1)
        .expect("operation id overflow");
    state.active_operation = Some(operation);
    state.status = "Loading…".to_string();
    state.error = None;
    vec![Effect::Load {
        operation,
        scope,
        problem_id: matches!(state.screen, Screen::ProblemDetail | Screen::Solve)
            .then_some(state.selected_problem_id)
            .flatten(),
        language_slug,
    }]
}

fn restore_selection(state: &mut AppState) {
    state.set_index = state
        .selected_set_id
        .as_ref()
        .and_then(|id| state.data.sets.iter().position(|row| &row.slug == id))
        .unwrap_or(0)
        .min(state.data.sets.len().saturating_sub(1));
    state.selected_set_id = state
        .data
        .sets
        .get(state.set_index)
        .map(|row| row.slug.clone());

    state.problem_index = state
        .selected_problem_id
        .and_then(|id| state.data.problems.iter().position(|row| row.id == id))
        .unwrap_or(0)
        .min(state.data.problems.len().saturating_sub(1));
    state.selected_problem_id = state
        .data
        .problems
        .get(state.problem_index)
        .map(|row| row.id);
}

fn next_operation(state: &mut AppState) -> OperationId {
    let operation = OperationId(state.next_operation);
    state.next_operation = state
        .next_operation
        .checked_add(1)
        .expect("operation id overflow");
    operation
}

fn start_codex_connect(state: &mut AppState) -> Vec<Effect> {
    let operation = next_operation(state);
    state.codex.connecting = Some(operation);
    state.codex.status = CodexStatus::Connecting;
    state.error = None;
    vec![Effect::ConnectCodex { operation }]
}

fn solve_command(state: &mut AppState, action: Action) -> Vec<Effect> {
    let Some(solve) = state.solve.as_mut() else {
        return Vec::new();
    };
    state.error = None;
    solve.editor.error = None;

    let requested_discard = match action {
        Action::Back => Some(DiscardAction::Back),
        Action::Quit => Some(DiscardAction::Quit),
        _ => None,
    };
    if let Some(discard) = requested_discard
        && solve.editor.mode == Mode::Normal
        && solve.editor.dirty()
    {
        if solve.discard_confirmation != Some(discard) {
            solve.discard_confirmation = Some(discard);
            state.status = match discard {
                DiscardAction::Back => "Unsaved changes · Space-b again to discard".into(),
                DiscardAction::Quit => "Unsaved changes · Space-q again to quit".into(),
            };
            state.error = Some("unsaved changes; repeat the same guarded action to discard".into());
            return Vec::new();
        }
    } else {
        solve.discard_confirmation = None;
    }

    match action {
        Action::InterviewFocus => {
            solve.pane = SolvePane::Interview;
            if state.codex.disclosure_accepted {
                state.codex.composer_focused = true;
                if matches!(
                    state.codex.status,
                    CodexStatus::Offline | CodexStatus::Disconnected | CodexStatus::ProtocolError
                ) {
                    return start_codex_connect(state);
                }
            } else {
                state.codex.status = CodexStatus::Disclosure;
            }
            Vec::new()
        }
        Action::InterviewDisclosure(accepted) if state.codex.status == CodexStatus::Disclosure => {
            if accepted {
                state.codex.disclosure_accepted = true;
                state.codex.composer_focused = true;
                start_codex_connect(state)
            } else {
                state.codex.status = CodexStatus::Declined;
                state.codex.composer_focused = false;
                Vec::new()
            }
        }
        Action::InterviewChar(character) if state.codex.composer_focused => {
            if state
                .codex
                .composer
                .len()
                .saturating_add(character.len_utf8())
                <= MAX_COMPOSER_BYTES
            {
                state.codex.composer.push(character);
            }
            Vec::new()
        }
        Action::InterviewBackspace if state.codex.composer_focused => {
            state.codex.composer.pop();
            Vec::new()
        }
        Action::InterviewEscape => {
            state.codex.composer_focused = false;
            Vec::new()
        }
        Action::InterviewSend
            if state.codex.composer_focused
                && matches!(
                    state.codex.status,
                    CodexStatus::Ready | CodexStatus::Feedback
                ) =>
        {
            let question = state.codex.composer.trim().to_string();
            if question.is_empty() {
                return Vec::new();
            }
            let operation = next_operation(state);
            let solve = state.solve.as_ref().expect("solve exists");
            let revision = solve.editor.revision;
            state.codex.composer.clear();
            state.codex.composer_focused = false;
            state.codex.push_message("You".into(), question.clone());
            state.codex.status = CodexStatus::Thinking;
            state.codex.active = Some((operation, revision, CodexMode::Interviewer));
            vec![Effect::CodexTurn {
                operation,
                revision,
                mode: CodexMode::Interviewer,
                statement: solve.statement.clone(),
                source: solve.editor.text().to_string(),
                output: solve
                    .output
                    .chars()
                    .rev()
                    .take(16 * 1024)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
                question,
                solved: state.codex.submission_recorded,
            }]
        }
        Action::Hint
            if state.codex.disclosure_accepted
                && matches!(
                    state.codex.status,
                    CodexStatus::Ready | CodexStatus::Feedback
                ) =>
        {
            let revision = solve.editor.revision;
            if state.codex.hint_revision != Some(revision) {
                state.codex.hint_revision = Some(revision);
                state.codex.hint_count = 0;
            }
            if state.codex.hint_count >= 3 {
                state.error = Some("maximum three hints reached for this revision".into());
                return Vec::new();
            }
            let level = state.codex.hint_count + 1;
            let operation = next_operation(state);
            let solve = state.solve.as_ref().expect("solve exists");
            state.codex.status = CodexStatus::Thinking;
            state.codex.active = Some((operation, revision, CodexMode::Hint(level)));
            vec![Effect::CodexTurn {
                operation,
                revision,
                mode: CodexMode::Hint(level),
                statement: solve.statement.clone(),
                source: solve.editor.text().to_string(),
                output: solve
                    .output
                    .chars()
                    .rev()
                    .take(16 * 1024)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
                question: String::new(),
                solved: false,
            }]
        }
        Action::ResetInterview => {
            solve.submitted_source = None;
            state.codex.clear_session();
            vec![Effect::ResetCodex]
        }
        Action::SaveTest | Action::Submit => {
            let intent = if action == Action::Submit {
                RunIntent::Submit
            } else {
                RunIntent::Test
            };
            if solve.running.is_some() {
                if intent == RunIntent::Test {
                    solve.pending_save =
                        Some((solve.editor.revision, solve.editor.text().to_string()));
                    state.status = "Running · newest test pending".into();
                } else {
                    state.error = Some("a run is already active".into());
                }
                return Vec::new();
            }
            let operation = next_operation(state);
            let solve = state.solve.as_mut().expect("solve exists");
            let revision = solve.editor.revision;
            let source = solve.editor.text().to_string();
            solve.running = Some((operation, revision, intent));
            if intent == RunIntent::Submit {
                solve.submitted_source = Some(super::model::SubmittedSource::new(
                    operation,
                    revision,
                    source.clone(),
                ));
            }
            if solve.quit_after_save == Some((None, revision)) {
                solve.quit_after_save = Some((Some(operation), revision));
            }
            state.status = if intent == RunIntent::Submit {
                "Submitting…"
            } else {
                "Testing…"
            }
            .into();
            vec![Effect::SaveRun {
                operation,
                plan: solve.plan.clone(),
                source,
                revision,
                write_source: solve.editor.dirty(),
                intent,
            }]
        }
        Action::Cancel => {
            let codex_operation = state
                .codex
                .active
                .map(|(operation, _, _)| operation)
                .or(state.codex.connecting);
            let runner_active = solve.running.map(|(operation, _, _)| operation);
            if let Some(operation) = codex_operation
                && (solve.pane == SolvePane::Interview || runner_active.is_none())
            {
                state.codex.status = CodexStatus::Disconnected;
                state.codex.connecting = None;
                state.codex.active = None;
                vec![Effect::CancelCodex { operation }]
            } else if let Some(operation) = runner_active {
                state.status = "Cancelling…".into();
                vec![Effect::CancelRun { operation }]
            } else {
                Vec::new()
            }
        }
        Action::Back => {
            let effect = solve
                .running
                .map(|(operation, _, _)| Effect::CancelRun { operation });
            state.solve = None;
            state.codex.clear_session();
            state.screen = Screen::ProblemDetail;
            state.status = "Ready".into();
            let mut effects = effect.into_iter().collect::<Vec<_>>();
            effects.push(Effect::ResetCodex);
            effects.push(Effect::LeaveSolve);
            effects
        }
        Action::NextFocus | Action::PreviousFocus => {
            let forward = action == Action::NextFocus;
            solve.pane = match (solve.pane, forward) {
                (SolvePane::Editor, true) => SolvePane::Problem,
                (SolvePane::Problem, true) => SolvePane::Output,
                (SolvePane::Output, true) => SolvePane::Interview,
                (SolvePane::Interview, true) => SolvePane::Editor,
                (SolvePane::Editor, false) => SolvePane::Interview,
                (SolvePane::Interview, false) => SolvePane::Output,
                (SolvePane::Output, false) => SolvePane::Problem,
                (SolvePane::Problem, false) => SolvePane::Editor,
            };
            Vec::new()
        }
        Action::Editor(editor_action) if solve.pane == SolvePane::Editor => {
            let revision_before = solve.editor.revision;
            let result = match editor_action {
                EditorAction::Normal(key) => solve.editor.normal(key),
                EditorAction::Insert(character) => solve.editor.insert_char(character),
                EditorAction::Paste(text) => match solve.editor.mode {
                    Mode::Insert => solve.editor.insert_text(&text),
                    Mode::Command => solve.editor.command_text(&text),
                    Mode::Normal => Err("paste ignored in Normal mode".into()),
                },
                EditorAction::CommandChar(character) => {
                    solve.editor.command_text(&character.to_string())
                }
                EditorAction::ExecuteCommand => match solve.editor.execute_command() {
                    Ok(command) => {
                        return solve_command(
                            state,
                            Action::Editor(EditorAction::Command(command)),
                        );
                    }
                    Err(error) => Err(error),
                },
                EditorAction::Escape => {
                    solve.editor.escape();
                    state.error = None;
                    Ok(())
                }
                EditorAction::Enter => solve.editor.enter(),
                EditorAction::Backspace => solve.editor.backspace(),
                EditorAction::CommandBackspace => {
                    solve.editor.command_buffer.pop();
                    Ok(())
                }
                EditorAction::Delete => solve.editor.delete(),
                EditorAction::Left => {
                    solve.editor.move_left();
                    Ok(())
                }
                EditorAction::Right => {
                    solve.editor.move_right();
                    Ok(())
                }
                EditorAction::Up => solve.editor.normal('k'),
                EditorAction::Down => solve.editor.normal('j'),
                EditorAction::Redo => {
                    solve.editor.redo();
                    Ok(())
                }
                EditorAction::Command(command) => {
                    let action = match command {
                        EditorCommand::Write => Action::SaveTest,
                        EditorCommand::WriteQuit => {
                            solve.quit_after_save = Some((None, solve.editor.revision));
                            Action::SaveTest
                        }
                        EditorCommand::Submit => Action::Submit,
                        EditorCommand::Quit if solve.editor.dirty() => {
                            state.error = Some("unsaved changes: use :wq".into());
                            return Vec::new();
                        }
                        EditorCommand::Quit => Action::Back,
                    };
                    return solve_command(state, action);
                }
            };
            if let Err(error) = result {
                if solve.editor.mode == Mode::Command {
                    solve.editor.escape();
                }
                solve.editor.error = Some(error.clone());
                state.error = Some(error);
            } else {
                state.error = None;
            }
            if solve.editor.revision != revision_before {
                solve.stale = solve
                    .latest_run_revision
                    .is_some_and(|revision| revision != solve.editor.revision);
            }
            Vec::new()
        }
        Action::Up => {
            match solve.pane {
                SolvePane::Problem => solve.problem_scroll = solve.problem_scroll.saturating_sub(1),
                SolvePane::Output => solve.output_scroll = solve.output_scroll.saturating_sub(1),
                SolvePane::Interview => state.codex.scroll = state.codex.scroll.saturating_add(1),
                SolvePane::Editor => {}
            }
            Vec::new()
        }
        Action::Down => {
            match solve.pane {
                SolvePane::Problem => solve.problem_scroll = solve.problem_scroll.saturating_add(1),
                SolvePane::Output => {
                    solve.output_scroll = solve
                        .output_scroll
                        .saturating_add(1)
                        .min(solve.output_scroll_max())
                }
                SolvePane::Interview => state.codex.scroll = state.codex.scroll.saturating_sub(1),
                SolvePane::Editor => {}
            }
            Vec::new()
        }
        Action::Quit
            if solve.editor.mode == Mode::Normal
                && (!solve.editor.dirty()
                    || solve.discard_confirmation == Some(DiscardAction::Quit)) =>
        {
            state.codex.clear_session();
            state.quit = true;
            vec![Effect::ResetCodex]
        }
        _ => Vec::new(),
    }
}

fn start_pending_test(state: &mut AppState) -> Vec<Effect> {
    let Some((revision, source)) = state
        .solve
        .as_mut()
        .and_then(|solve| solve.pending_save.take())
    else {
        return Vec::new();
    };
    let operation = next_operation(state);
    let solve = state.solve.as_mut().expect("solve exists");
    solve.running = Some((operation, revision, RunIntent::Test));
    state.status = "Testing pending revision…".into();
    if solve.quit_after_save == Some((None, revision)) {
        solve.quit_after_save = Some((Some(operation), revision));
    }
    vec![Effect::SaveRun {
        operation,
        plan: solve.plan.clone(),
        source,
        revision,
        write_source: true,
        intent: RunIntent::Test,
    }]
}

pub fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    if state.screen == Screen::Solve {
        match event {
            Event::Command(action) => return solve_command(state, action),
            Event::CodexConnected(operation, result) => {
                if state.codex.connecting != Some(operation) {
                    return Vec::new();
                }
                state.codex.connecting = None;
                match result {
                    Ok(()) => {
                        state.codex.status = CodexStatus::Ready;
                        state.error = None;
                    }
                    Err(error) if error.contains("authentication required") => {
                        state.codex.status = CodexStatus::AuthRequired;
                        state.error = Some(error);
                    }
                    Err(error) => {
                        state.codex.status = CodexStatus::Disconnected;
                        state.error = Some(error);
                    }
                }
                return Vec::new();
            }
            Event::CodexFinished(operation, revision, mode, result) => {
                if state.codex.active != Some((operation, revision, mode)) {
                    return vec![Effect::FinalizeCodexTurn {
                        operation,
                        revision,
                        mode,
                        accepted: false,
                    }];
                }
                state.codex.active = None;
                let message = match result {
                    Ok(message) => message,
                    Err(error) => {
                        state.codex.status = CodexStatus::ProtocolError;
                        state.error = Some(error);
                        return vec![Effect::FinalizeCodexTurn {
                            operation,
                            revision,
                            mode,
                            accepted: false,
                        }];
                    }
                };
                let current_revision = state.solve.as_ref().expect("solve exists").editor.revision;
                if current_revision != revision {
                    state.codex.status = if state.codex.messages.len() > 1 {
                        CodexStatus::Feedback
                    } else {
                        CodexStatus::Ready
                    };
                    state.error = Some(
                        "Codex response ignored because the source changed during the turn".into(),
                    );
                    return vec![Effect::FinalizeCodexTurn {
                        operation,
                        revision,
                        mode,
                        accepted: false,
                    }];
                }
                let label = match mode {
                    CodexMode::Interviewer => "Interviewer",
                    CodexMode::Hint(_) => "Hinter",
                    CodexMode::SubmissionReview => "Submission review",
                };
                if matches!(mode, CodexMode::Hint(_)) {
                    state.codex.hint_count = state.codex.hint_count.saturating_add(1);
                }
                state.codex.push_message(label.into(), message);
                state.codex.status = CodexStatus::Feedback;
                return vec![Effect::FinalizeCodexTurn {
                    operation,
                    revision,
                    mode,
                    accepted: true,
                }];
            }
            Event::CodexDisconnected(error) => {
                state.codex.connecting = None;
                state.codex.active = None;
                state.codex.composer_focused = false;
                state.codex.status = CodexStatus::Disconnected;
                state.error = Some(error);
                return Vec::new();
            }
            Event::RunFinished(operation, revision, intent, saved_source, result) => {
                let Some(solve) = state.solve.as_mut() else {
                    return Vec::new();
                };
                if solve.running != Some((operation, revision, intent)) {
                    return Vec::new();
                }
                solve.running = None;
                solve.cancellation = None;
                let submitted_source = if intent == RunIntent::Submit {
                    solve.submitted_source.take().and_then(|submitted| {
                        (submitted.operation == operation && submitted.revision == revision)
                            .then(|| submitted.source().to_string())
                    })
                } else {
                    None
                };
                if let Some(saved_source) = saved_source.as_deref() {
                    solve.editor.mark_saved(revision, saved_source);
                }
                let quit_matches = solve.quit_after_save == Some((Some(operation), revision));
                let succeeded = result.is_ok();
                solve.latest_run_revision = Some(revision);
                solve.stale = revision != solve.editor.revision;
                solve.output_scroll = 0;
                match result {
                    Ok(result) => {
                        solve.bounded_output(format!(
                            "{:?} ({} ms){}\n{}",
                            result.termination,
                            result.duration_ms,
                            if solve.stale { " · STALE" } else { "" },
                            result.display_output
                        ));
                        state.status = if solve.stale {
                            "Run complete · stale"
                        } else {
                            "Run complete"
                        }
                        .into();
                    }
                    Err(error) => {
                        solve.bounded_output(format!("Runner error: {error}"));
                        state.status = "Run failed".into();
                        state.error = Some(error);
                    }
                }
                if quit_matches {
                    solve.quit_after_save = None;
                    if succeeded
                        && saved_source.as_deref() == Some(solve.editor.text())
                        && revision == solve.editor.revision
                    {
                        state.quit = true;
                    } else if succeeded {
                        state.status =
                            "Saved revision finished, but newer edits remain open".into();
                        state.error =
                            Some("buffer changed during :wq; review and save again".into());
                    }
                }
                if intent == RunIntent::Submit && succeeded {
                    solve.refresh_after_submit = true;
                    state.codex.submission_recorded = true;
                    let codex_ready = state.codex.disclosure_accepted
                        && matches!(
                            state.codex.status,
                            CodexStatus::Ready | CodexStatus::Feedback
                        );
                    let review = match (codex_ready, submitted_source) {
                        (true, Some(source)) => {
                            let operation = next_operation(state);
                            let solve = state.solve.as_ref().expect("solve exists");
                            state.codex.status = CodexStatus::Thinking;
                            state.codex.active =
                                Some((operation, revision, CodexMode::SubmissionReview));
                            Some(Effect::CodexTurn {
                                operation,
                                revision,
                                mode: CodexMode::SubmissionReview,
                                statement: solve.statement.clone(),
                                source,
                                output: solve
                                    .output
                                    .chars()
                                    .rev()
                                    .take(16 * 1024)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect(),
                                question: String::new(),
                                solved: true,
                            })
                        }
                        (false, _) | (true, None) => None,
                    };
                    let mut effects = load_effect(state);
                    effects.extend(review);
                    return effects;
                }
                return start_pending_test(state);
            }
            Event::Loaded(operation, result) => {
                /* progress refresh after submit */
                if state.active_operation != Some(operation) {
                    return Vec::new();
                }
                state.active_operation = None;
                match result {
                    Ok(data) => {
                        state.data = *data;
                        state.status = "Submit recorded · progress refreshed".into();
                        state.error = None;
                    }
                    Err(error) => {
                        state.status = "Progress refresh failed".into();
                        state.error = Some(error);
                    }
                }
                if let Some(solve) = state.solve.as_mut() {
                    solve.refresh_after_submit = false;
                }
                return start_pending_test(state);
            }
            Event::SolveOpened(_, _) | Event::OpenSet(_) => return Vec::new(),
        }
    }
    if state.show_help {
        match event {
            Event::Command(Action::Quit) => {
                state.codex.clear_session();
                state.quit = true;
                return vec![Effect::ResetCodex];
            }
            Event::Command(Action::Back | Action::Help) => state.show_help = false,
            _ => {}
        }
        return Vec::new();
    }

    match event {
        Event::Command(Action::Up) if state.focus == Focus::Progress => {
            state.progress_scroll = state.progress_scroll.saturating_sub(1);
        }
        Event::Command(Action::Down) if state.focus == Focus::Progress => {
            state.progress_scroll = state.progress_scroll.checked_add(1).unwrap_or(MAX_SCROLL);
        }
        Event::Command(Action::Up) => match state.screen {
            Screen::SetMenu => {
                state.set_index = state.set_index.saturating_sub(1);
                state.selected_set_id = state
                    .data
                    .sets
                    .get(state.set_index)
                    .map(|row| row.slug.clone());
            }
            Screen::ProblemList => {
                state.problem_index = state.problem_index.saturating_sub(1);
                state.selected_problem_id = state
                    .data
                    .problems
                    .get(state.problem_index)
                    .map(|row| row.id);
            }
            Screen::ProblemDetail => state.detail_scroll = state.detail_scroll.saturating_sub(1),
            Screen::Solve => unreachable!("solve events handled above"),
        },
        Event::Command(Action::Down) => match state.screen {
            Screen::SetMenu => {
                state.set_index =
                    (state.set_index + 1).min(state.data.sets.len().saturating_sub(1));
                state.selected_set_id = state
                    .data
                    .sets
                    .get(state.set_index)
                    .map(|row| row.slug.clone());
            }
            Screen::ProblemList => {
                state.problem_index =
                    (state.problem_index + 1).min(state.data.problems.len().saturating_sub(1));
                state.selected_problem_id = state
                    .data
                    .problems
                    .get(state.problem_index)
                    .map(|row| row.id);
            }
            Screen::ProblemDetail => {
                state.detail_scroll = state.detail_scroll.checked_add(1).unwrap_or(MAX_SCROLL);
            }
            Screen::Solve => unreachable!("solve events handled above"),
        },
        Event::Command(Action::Open) if state.focus == Focus::Progress => {}
        Event::Command(Action::Open) => match state.screen {
            Screen::SetMenu => {
                if let Some(row) = state.data.sets.get(state.set_index) {
                    state.selected_set_id = Some(row.slug.clone());
                    state.selected_problem_id = None;
                    state.problem_index = 0;
                    state.detail_scroll = 0;
                    state.progress_scroll = 0;
                    state.screen = Screen::ProblemList;
                    return load_effect(state);
                }
            }
            Screen::ProblemList => {
                if let Some(row) = state.data.problems.get(state.problem_index) {
                    state.selected_problem_id = Some(row.id);
                    state.detail_scroll = 0;
                    state.progress_scroll = 0;
                    state.screen = Screen::ProblemDetail;
                    return load_effect(state);
                }
            }
            Screen::ProblemDetail => {
                let Some(problem_slug) =
                    state.data.detail.as_ref().map(|detail| detail.slug.clone())
                else {
                    state.error = Some("problem detail is unavailable".into());
                    return Vec::new();
                };
                let operation = next_operation(state);
                state.active_operation = Some(operation);
                state.status = "Loading source…".into();
                return vec![Effect::OpenSolve {
                    operation,
                    problem_slug,
                    set_slug: state.selected_set_id.clone(),
                    language_slug: state.language_slug().unwrap_or("").to_string(),
                }];
            }
            Screen::Solve => unreachable!("solve events handled above"),
        },
        Event::CodexFinished(operation, revision, mode, _) => {
            return vec![Effect::FinalizeCodexTurn {
                operation,
                revision,
                mode,
                accepted: false,
            }];
        }
        Event::CodexDisconnected(error) => {
            state.codex.connecting = None;
            state.codex.active = None;
            state.codex.composer_focused = false;
            state.codex.status = CodexStatus::Disconnected;
            state.error = Some(error);
        }
        Event::CodexConnected(_, _) => {}
        Event::OpenSet(slug) => {
            state.selected_set_id = Some(slug);
            state.selected_problem_id = None;
            state.detail_scroll = 0;
            state.progress_scroll = 0;
            state.screen = Screen::ProblemList;
            return load_effect(state);
        }
        Event::Command(Action::Back) => {
            state.detail_scroll = 0;
            state.progress_scroll = 0;
            state.focus = Focus::Main;
            match state.screen {
                Screen::ProblemDetail => state.screen = Screen::ProblemList,
                Screen::ProblemList => {
                    state.screen = Screen::SetMenu;
                    state.selected_problem_id = None;
                    return load_effect(state);
                }
                Screen::SetMenu => {}
                Screen::Solve => unreachable!("solve events handled above"),
            }
        }
        Event::Command(Action::NextFocus | Action::PreviousFocus) => {
            state.focus = match state.focus {
                Focus::Main => Focus::Progress,
                Focus::Progress => Focus::Main,
            };
        }
        Event::Command(Action::CycleLanguage) => {
            if !state.languages.is_empty() {
                state.language_index = (state.language_index + 1) % state.languages.len();
                state.detail_scroll = 0;
                state.progress_scroll = 0;
                return load_effect(state);
            }
        }
        Event::Command(Action::Reload) => {
            state.detail_scroll = 0;
            state.progress_scroll = 0;
            return load_effect(state);
        }
        Event::Command(Action::Help) => state.show_help = true,
        Event::Command(Action::Quit) => {
            state.codex.clear_session();
            state.quit = true;
            return vec![Effect::ResetCodex];
        }
        Event::Command(
            Action::SaveTest
            | Action::Submit
            | Action::Cancel
            | Action::InterviewFocus
            | Action::InterviewChar(_)
            | Action::InterviewBackspace
            | Action::InterviewSend
            | Action::InterviewEscape
            | Action::InterviewDisclosure(_)
            | Action::Hint
            | Action::ResetInterview
            | Action::Editor(_),
        ) => {}
        Event::SolveOpened(operation, result) => {
            if state.active_operation != Some(operation) {
                return Vec::new();
            }
            state.active_operation = None;
            match result {
                Ok(solve) => {
                    state.solve = Some(*solve);
                    state.screen = Screen::Solve;
                    state.status = "Ready".into();
                    state.error = None
                }
                Err(error) => {
                    state.status = "Source load failed".into();
                    state.error = Some(error)
                }
            }
        }
        Event::RunFinished(_, _, _, _, _) => {}
        Event::Loaded(operation, result) => {
            if state.active_operation != Some(operation) {
                return Vec::new();
            }
            state.active_operation = None;
            match result {
                Ok(data) => {
                    data.assert_bounded();
                    state.data = *data;
                    restore_selection(state);
                    state.detail_scroll = 0;
                    state.progress_scroll = 0;
                    state.status = "Ready".to_string();
                    state.error = None;
                }
                Err(error) => {
                    state.status = "Load failed".to_string();
                    state.error = Some(error);
                }
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::{AppData, ProblemRow, SetRow};
    use crate::database::{Difficulty, EnabledLanguage};

    fn state() -> AppState {
        let mut state = AppState::new(
            vec![EnabledLanguage {
                slug: "python".into(),
                display_name: "Python".into(),
                runner_path: "python/run".into(),
            }],
            0,
        );
        state.data.sets = vec![
            SetRow {
                slug: "a".into(),
                name: "A".into(),
                description: String::new(),
                member_count: 1,
                completed_count: 0,
            },
            SetRow {
                slug: "b".into(),
                name: "B".into(),
                description: String::new(),
                member_count: 0,
                completed_count: 0,
            },
        ];
        state.selected_set_id = Some("a".into());
        state
    }

    fn load_scope(effects: Vec<Effect>) -> LoadScope {
        let Effect::Load { scope, .. } = effects.into_iter().next().unwrap() else {
            panic!("expected load effect")
        };
        scope
    }

    #[test]
    fn load_scope_follows_screen_not_highlighted_set() {
        let mut state = state();
        assert_eq!(
            load_scope(reduce(&mut state, Event::Command(Action::Reload))),
            LoadScope::Global
        );
        assert_eq!(
            load_scope(reduce(&mut state, Event::Command(Action::CycleLanguage))),
            LoadScope::Global
        );
        reduce(&mut state, Event::Command(Action::Open));
        assert_eq!(
            load_scope(reduce(&mut state, Event::Command(Action::Reload))),
            LoadScope::ProblemSet("a".into())
        );
        assert_eq!(
            load_scope(reduce(&mut state, Event::Command(Action::Back))),
            LoadScope::Global
        );
    }

    #[test]
    fn menus_clamp_and_progress_focus_routes_navigation() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Up));
        assert_eq!(state.set_index, 0);
        for _ in 0..4 {
            reduce(&mut state, Event::Command(Action::Down));
        }
        assert_eq!(state.set_index, 1);
        reduce(&mut state, Event::Command(Action::NextFocus));
        reduce(&mut state, Event::Command(Action::Up));
        assert_eq!(state.set_index, 1);
        reduce(&mut state, Event::Command(Action::Open));
        assert_eq!(state.screen, Screen::SetMenu);
        reduce(&mut state, Event::Command(Action::Down));
        assert_eq!(state.progress_scroll, 1);
    }

    #[test]
    fn detail_scroll_is_bounded_and_resets_on_screen_changes() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Open));
        state.data.problems = vec![ProblemRow {
            id: 7,
            ordinal: Some(1),
            slug: "p".into(),
            title: "P".into(),
            difficulty: Difficulty::Easy,
            topic: "T".into(),
            completed: false,
        }];
        reduce(&mut state, Event::Command(Action::Open));
        state.detail_scroll = MAX_SCROLL;
        reduce(&mut state, Event::Command(Action::Down));
        assert_eq!(state.detail_scroll, MAX_SCROLL);
        reduce(&mut state, Event::Command(Action::Back));
        assert_eq!(state.detail_scroll, 0);
    }

    #[test]
    fn help_blocks_navigation_and_back_only_dismisses_help() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Help));
        reduce(&mut state, Event::Command(Action::Down));
        reduce(&mut state, Event::Command(Action::Back));
        assert_eq!(state.set_index, 0);
        assert_eq!(state.screen, Screen::SetMenu);
        assert!(!state.show_help);
        assert!(!state.quit);
    }

    #[test]
    fn quit_always_exits_when_help_is_open() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Help));
        reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
    }

    fn solve_state() -> AppState {
        use crate::app::model::{SolvePane, SolveSession};
        use crate::editor::EditorDocument;
        use crate::runner::ExecutionPlan;
        use std::path::PathBuf;
        let mut state = state();
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: "Example".into(),
            language: "python".into(),
            plan: ExecutionPlan {
                root: PathBuf::from("/tmp"),
                language: "python".into(),
                problem_slug: "p".into(),
                set_slug: Some("a".into()),
                runner_path: PathBuf::from("/tmp/run"),
                solution_path: PathBuf::from("/tmp/p.py"),
            },
            editor: EditorDocument::new("print(1)".into()).unwrap(),
            pane: SolvePane::Editor,
            output: String::new(),
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
    fn command_errors_leave_command_mode_and_next_valid_action_dismisses_them() {
        use crate::editor::{MAX_COMMAND_BYTES, Mode};

        for command in ["bogus".to_string(), "x".repeat(MAX_COMMAND_BYTES + 1)] {
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
            assert!(solve.editor.error.is_some());
            assert!(state.error.is_some());

            reduce(
                &mut state,
                Event::Command(Action::Editor(EditorAction::Normal('h'))),
            );
            let solve = state.solve.as_ref().unwrap();
            assert!(solve.editor.error.is_none());
            assert!(state.error.is_none());
        }
    }

    #[test]
    fn submit_refreshes_progress_before_starting_queued_test() {
        let mut state = solve_state();
        let submit = reduce(&mut state, Event::Command(Action::Submit));
        let Effect::SaveRun {
            operation,
            revision,
            intent: RunIntent::Submit,
            ..
        } = submit[0]
        else {
            panic!("expected submit run")
        };
        reduce(&mut state, Event::Command(Action::SaveTest));

        let reload = reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Submit,
                Some("print(1)".into()),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "PASS",
                )),
            ),
        );
        let [
            Effect::Load {
                operation: reload_operation,
                ..
            },
        ] = reload.as_slice()
        else {
            panic!("expected progress reload before queued test")
        };
        assert_eq!(state.status, "Loading…");
        assert!(state.solve.as_ref().unwrap().running.is_none());
        assert!(state.solve.as_ref().unwrap().pending_save.is_some());

        let queued = reduce(
            &mut state,
            Event::Loaded(*reload_operation, Ok(Box::new(AppData::empty()))),
        );
        let [
            Effect::SaveRun {
                intent: RunIntent::Test,
                ..
            },
        ] = queued.as_slice()
        else {
            panic!("expected queued test after progress reload")
        };
        assert_eq!(state.status, "Testing pending revision…");
        assert!(state.solve.as_ref().unwrap().pending_save.is_none());
    }

    #[test]
    fn submit_record_failure_has_failure_status_and_does_not_reload_or_review() {
        let mut state = solve_state();
        state.codex.disclosure_accepted = true;
        state.codex.status = CodexStatus::Ready;
        let submit = reduce(&mut state, Event::Command(Action::Submit));
        let Effect::SaveRun {
            operation,
            revision,
            intent: RunIntent::Submit,
            ..
        } = submit[0]
        else {
            panic!("expected submit run")
        };

        let effects = reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Submit,
                Some("print(1)".into()),
                Err("record failed".into()),
            ),
        );
        assert!(effects.is_empty());
        assert_eq!(state.status, "Run failed");
        assert_eq!(state.error.as_deref(), Some("record failed"));
        assert!(!state.status.contains("recorded"));
        assert!(!state.solve.as_ref().unwrap().refresh_after_submit);
        assert!(state.active_operation.is_none());
        assert!(state.solve.as_ref().unwrap().submitted_source.is_none());
        assert_eq!(state.codex.status, CodexStatus::Ready);
    }

    #[test]
    fn solve_keeps_only_newest_pending_test_and_ignores_old_operation() {
        let mut state = solve_state();
        let first = reduce(&mut state, Event::Command(Action::SaveTest));
        let Effect::SaveRun {
            operation,
            revision,
            ..
        } = first[0].clone()
        else {
            panic!("expected run")
        };
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        reduce(&mut state, Event::Command(Action::SaveTest));
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('y'))),
        );
        reduce(&mut state, Event::Command(Action::SaveTest));
        assert_eq!(
            state
                .solve
                .as_ref()
                .unwrap()
                .pending_save
                .as_ref()
                .unwrap()
                .0,
            2
        );
        reduce(
            &mut state,
            Event::RunFinished(
                OperationId(999),
                revision,
                RunIntent::Test,
                Some("print(1)".into()),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "ignored",
                )),
            ),
        );
        assert_eq!(state.solve.as_ref().unwrap().running.unwrap().0, operation);
        let queued = reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Test,
                Some("print(1)".into()),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "PASS",
                )),
            ),
        );
        assert!(state.solve.as_ref().unwrap().stale);
        let Effect::SaveRun { revision, .. } = queued[0] else {
            panic!("expected queued run")
        };
        assert_eq!(revision, 2);
    }

    #[test]
    fn solve_submit_intent_cancel_and_leave_cleanup_are_explicit() {
        let mut state = solve_state();
        let effects = reduce(&mut state, Event::Command(Action::Submit));
        let Effect::SaveRun {
            operation, intent, ..
        } = effects[0]
        else {
            panic!("expected submit")
        };
        assert_eq!(intent, RunIntent::Submit);
        let cancel = reduce(&mut state, Event::Command(Action::Cancel));
        assert!(
            matches!(cancel.as_slice(),[Effect::CancelRun{operation: cancelled}] if *cancelled==operation)
        );
        let effects = reduce(&mut state, Event::Command(Action::Back));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LeaveSolve))
        );
        assert!(state.solve.is_none());
        assert_eq!(state.screen, Screen::ProblemDetail);
    }

    #[test]
    fn completion_requires_full_operation_revision_intent_tuple() {
        let mut state = solve_state();
        let effects = reduce(&mut state, Event::Command(Action::SaveTest));
        let Effect::SaveRun {
            operation,
            revision,
            ..
        } = effects[0]
        else {
            panic!("expected run")
        };
        let ignored = Event::RunFinished(
            operation,
            revision + 1,
            RunIntent::Test,
            Some("wrong".into()),
            Ok(crate::runner::ExecutionResult::test_result(
                crate::runner::Termination::Exited(0),
                "wrong",
            )),
        );
        assert!(reduce(&mut state, ignored).is_empty());
        assert_eq!(
            state.solve.as_ref().unwrap().running,
            Some((operation, revision, RunIntent::Test))
        );
        let ignored = Event::RunFinished(
            operation,
            revision,
            RunIntent::Submit,
            Some("wrong".into()),
            Ok(crate::runner::ExecutionResult::test_result(
                crate::runner::Termination::Exited(0),
                "wrong",
            )),
        );
        assert!(reduce(&mut state, ignored).is_empty());
        assert_eq!(
            state.solve.as_ref().unwrap().running,
            Some((operation, revision, RunIntent::Test))
        );
    }

    #[test]
    fn first_edit_before_a_run_is_not_stale_and_dirty_quit_is_guarded() {
        let mut state = solve_state();
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        assert!(!state.solve.as_ref().unwrap().stale);
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Escape)),
        );

        reduce(&mut state, Event::Command(Action::Quit));
        assert!(!state.quit);
        assert_eq!(
            state.solve.as_ref().unwrap().discard_confirmation,
            Some(DiscardAction::Quit)
        );
        state
            .codex
            .push_message("Interviewer".into(), "private".into());
        let quit = reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
        assert!(state.codex.messages.is_empty());
        assert!(matches!(quit.as_slice(), [Effect::ResetCodex]));
    }

    #[test]
    fn write_quit_does_not_quit_when_buffer_changes_during_run() {
        let mut state = solve_state();
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        let effects = reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Command(
                EditorCommand::WriteQuit,
            ))),
        );
        let Effect::SaveRun {
            operation,
            revision,
            source,
            ..
        } = effects[0].clone()
        else {
            panic!("expected run")
        };
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Test,
                Some(source),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "PASS",
                )),
            ),
        );
        assert!(!state.quit);
        assert!(state.solve.as_ref().unwrap().editor.dirty());
        assert!(
            state
                .error
                .as_deref()
                .unwrap()
                .contains("changed during :wq")
        );
    }

    #[test]
    fn write_quit_binds_to_queued_save_and_only_success_quits() {
        let mut state = solve_state();
        let first = reduce(&mut state, Event::Command(Action::SaveTest));
        let Effect::SaveRun {
            operation: first_operation,
            revision: first_revision,
            ..
        } = first[0]
        else {
            panic!("expected run")
        };
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Command(
                EditorCommand::WriteQuit,
            ))),
        );
        assert_eq!(
            state.solve.as_ref().unwrap().quit_after_save,
            Some((None, 0))
        );
        let queued = reduce(
            &mut state,
            Event::RunFinished(
                first_operation,
                first_revision,
                RunIntent::Test,
                Some("print(1)".into()),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "PASS",
                )),
            ),
        );
        let Effect::SaveRun {
            operation,
            revision,
            ..
        } = queued[0]
        else {
            panic!("expected queued run")
        };
        assert_eq!(
            state.solve.as_ref().unwrap().quit_after_save,
            Some((Some(operation), revision))
        );
        reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Test,
                None,
                Err("save failed".into()),
            ),
        );
        assert!(!state.quit);
    }

    #[test]
    fn codex_disclosure_question_hint_stale_response_and_failure_preserve_solve() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert_eq!(state.codex.status, CodexStatus::Disclosure);
        let effects = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let [Effect::ConnectCodex { operation }] = effects.as_slice() else {
            panic!("expected connect")
        };
        reduce(&mut state, Event::CodexConnected(*operation, Ok(())));
        reduce(&mut state, Event::Command(Action::InterviewChar('W')));
        let effects = reduce(&mut state, Event::Command(Action::InterviewSend));
        let Effect::CodexTurn {
            operation,
            revision,
            mode: CodexMode::Interviewer,
            ..
        } = effects[0]
        else {
            panic!("expected interview turn")
        };
        let stale = reduce(
            &mut state,
            Event::CodexFinished(
                OperationId(operation.0 + 1),
                revision,
                CodexMode::Interviewer,
                Ok("stale".into()),
            ),
        );
        assert!(matches!(
            stale.as_slice(),
            [Effect::FinalizeCodexTurn {
                accepted: false,
                ..
            }]
        ));
        assert_eq!(state.codex.messages.len(), 1);
        assert!(
            !state
                .codex
                .messages
                .iter()
                .any(|(_, message)| message == "stale")
        );
        let failure = reduce(
            &mut state,
            Event::CodexFinished(
                operation,
                revision,
                CodexMode::Interviewer,
                Err("protocol failed".into()),
            ),
        );
        assert!(matches!(
            failure.as_slice(),
            [Effect::FinalizeCodexTurn {
                accepted: false,
                ..
            }]
        ));
        assert_eq!(state.codex.status, CodexStatus::ProtocolError);
        assert!(state.solve.is_some());
        state
            .codex
            .messages
            .push(("Interviewer".into(), "secret".into()));
        let reset = reduce(&mut state, Event::Command(Action::ResetInterview));
        assert!(matches!(reset.as_slice(), [Effect::ResetCodex]));
        assert!(state.codex.messages.is_empty());
        assert_eq!(state.codex.status, CodexStatus::Offline);
    }

    #[test]
    fn codex_completion_requires_operation_revision_mode_and_current_editor_revision() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        let connect = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let Effect::ConnectCodex {
            operation: connect_operation,
        } = connect[0]
        else {
            panic!("expected connect")
        };
        reduce(&mut state, Event::CodexConnected(connect_operation, Ok(())));
        reduce(&mut state, Event::Command(Action::InterviewChar('W')));
        let effects = reduce(&mut state, Event::Command(Action::InterviewSend));
        let Effect::CodexTurn {
            operation,
            revision,
            mode,
            ..
        } = effects[0]
        else {
            panic!("expected interview turn")
        };

        let wrong_mode = reduce(
            &mut state,
            Event::CodexFinished(
                operation,
                revision,
                CodexMode::Hint(1),
                Ok("wrong mode".into()),
            ),
        );
        assert!(matches!(
            wrong_mode.as_slice(),
            [Effect::FinalizeCodexTurn {
                accepted: false,
                ..
            }]
        ));
        assert!(state.codex.active.is_some());
        assert!(
            !state
                .codex
                .messages
                .iter()
                .any(|(_, text)| text == "wrong mode")
        );

        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        reduce(
            &mut state,
            Event::CodexFinished(operation, revision, mode, Ok("stale response".into())),
        );
        assert!(state.codex.active.is_none());
        assert!(
            !state
                .codex
                .messages
                .iter()
                .any(|(_, text)| text == "stale response")
        );
        assert!(state.error.as_deref().unwrap().contains("source changed"));
    }

    #[test]
    fn submission_review_uses_successfully_recorded_source_even_after_edit() {
        let mut state = solve_state();
        state.codex.disclosure_accepted = true;
        state.codex.status = CodexStatus::Ready;
        let submit = reduce(&mut state, Event::Command(Action::Submit));
        let Effect::SaveRun {
            operation,
            revision,
            source: submitted_source,
            ..
        } = submit[0].clone()
        else {
            panic!("expected submit")
        };
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );

        let effects = reduce(
            &mut state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Submit,
                Some(submitted_source.clone()),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    "PASS",
                )),
            ),
        );
        let review = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::CodexTurn {
                    revision,
                    mode: CodexMode::SubmissionReview,
                    source,
                    ..
                } => Some((*revision, source)),
                _ => None,
            })
            .expect("submission review");
        assert_eq!(review.0, revision);
        assert_eq!(review.1, &submitted_source);
        assert_ne!(review.1, state.solve.as_ref().unwrap().editor.text());
        assert!(state.solve.as_ref().unwrap().submitted_source.is_none());
    }

    #[test]
    fn reset_invalidates_delayed_connect_completion() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        let connect = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let Effect::ConnectCodex { operation } = connect[0] else {
            panic!("expected connect")
        };
        reduce(&mut state, Event::Command(Action::ResetInterview));
        reduce(&mut state, Event::CodexConnected(operation, Ok(())));
        assert_eq!(state.codex.status, CodexStatus::Offline);
        assert!(!state.codex.composer_focused);

        let reconnect = reduce(&mut state, Event::Command(Action::InterviewFocus));
        let Effect::ConnectCodex {
            operation: reconnect_operation,
        } = reconnect[0]
        else {
            panic!("expected reconnect")
        };
        assert_ne!(operation, reconnect_operation);
        reduce(
            &mut state,
            Event::CodexConnected(operation, Err("stale".into())),
        );
        assert_eq!(state.codex.status, CodexStatus::Connecting);
        reduce(
            &mut state,
            Event::CodexConnected(reconnect_operation, Ok(())),
        );
        assert_eq!(state.codex.status, CodexStatus::Ready);
    }

    #[test]
    fn codex_protocol_error_can_explicitly_reconnect_without_reset() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.codex.disclosure_accepted = true;
        state.codex.status = CodexStatus::ProtocolError;
        let effects = reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert!(matches!(effects.as_slice(), [Effect::ConnectCodex { .. }]));
        assert_eq!(state.codex.status, CodexStatus::Connecting);
    }

    #[test]
    fn interview_scroll_is_bounded_and_resets_on_append_and_clear() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.codex.scroll = MAX_SCROLL;
        reduce(&mut state, Event::Command(Action::Up));
        assert_eq!(state.codex.scroll, MAX_SCROLL);
        reduce(&mut state, Event::Command(Action::Down));
        assert_eq!(state.codex.scroll, MAX_SCROLL - 1);
        state.codex.push_message("Interviewer".into(), "new".into());
        assert_eq!(state.codex.scroll, 0);
        state.codex.scroll = 10;
        state.codex.clear_session();
        assert_eq!(state.codex.scroll, 0);
    }

    #[test]
    fn cancel_prefers_focused_codex_then_runner_and_uses_sole_active_operation() {
        let mut state = solve_state();
        let run = reduce(&mut state, Event::Command(Action::SaveTest));
        let Effect::SaveRun {
            operation: run_operation,
            ..
        } = run[0]
        else {
            panic!("expected runner")
        };
        state.codex.active = Some((OperationId(99), 0, CodexMode::Interviewer));
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelRun { operation }] if *operation == run_operation
        ));
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelCodex { operation }] if *operation == OperationId(99)
        ));
        state.codex.active = Some((OperationId(100), 0, CodexMode::Interviewer));
        state.solve.as_mut().unwrap().running = None;
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelCodex { operation }] if *operation == OperationId(100)
        ));
    }

    #[test]
    fn hints_are_limited_to_three_per_revision_and_reset_after_edit() {
        let mut state = solve_state();
        state.codex.disclosure_accepted = true;
        state.codex.status = CodexStatus::Ready;
        for level in 1..=3 {
            let effects = reduce(&mut state, Event::Command(Action::Hint));
            let Effect::CodexTurn {
                operation,
                revision,
                mode,
                ..
            } = effects[0]
            else {
                panic!("expected hint")
            };
            assert_eq!(mode, CodexMode::Hint(level));
            assert!(matches!(
                reduce(
                    &mut state,
                    Event::CodexFinished(operation, revision, mode, Ok(format!("hint-{level}")))
                )
                .as_slice(),
                [Effect::FinalizeCodexTurn { accepted: true, .. }]
            ));
        }
        assert!(reduce(&mut state, Event::Command(Action::Hint)).is_empty());
        assert!(
            state
                .error
                .as_deref()
                .unwrap()
                .contains("maximum three hints")
        );

        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        state.codex.status = CodexStatus::Ready;
        let next = reduce(&mut state, Event::Command(Action::Hint));
        assert!(matches!(
            next.as_slice(),
            [Effect::CodexTurn {
                mode: CodexMode::Hint(1),
                ..
            }]
        ));
    }

    #[test]
    fn codex_disclosure_decline_keeps_local_solve_available() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert!(
            reduce(
                &mut state,
                Event::Command(Action::InterviewDisclosure(false))
            )
            .is_empty()
        );
        assert_eq!(state.codex.status, CodexStatus::Declined);
        let effects = reduce(&mut state, Event::Command(Action::SaveTest));
        assert!(matches!(effects.as_slice(), [Effect::SaveRun { .. }]));
    }

    #[test]
    fn stale_completion_is_ignored() {
        let mut state = state();
        let first = reduce(&mut state, Event::Command(Action::Reload));
        let second = reduce(&mut state, Event::Command(Action::Reload));
        let Effect::Load { operation: old, .. } = first[0].clone() else {
            panic!("expected load effect")
        };
        reduce(
            &mut state,
            Event::Loaded(old, Ok(Box::new(AppData::empty()))),
        );
        assert_eq!(state.selected_set_id.as_deref(), Some("a"));
        let Effect::Load { operation, .. } = second[0].clone() else {
            panic!("expected load effect")
        };
        reduce(
            &mut state,
            Event::Loaded(operation, Ok(Box::new(AppData::empty()))),
        );
        assert!(state.active_operation.is_none());
    }
}
