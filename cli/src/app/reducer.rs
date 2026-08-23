use super::effects::{Action, EditorAction, Effect, Event, LoadScope, RunIntent};
use super::model::{
    AppState, DiscardAction, DiscardConfirmation, EditorRuntimeStatus, Focus, InterviewerStatus,
    MAX_COMPOSER_BYTES, MAX_SCROLL, OperationId, RecordedSubmissionReview, Screen, SolvePane,
};
use crate::editor::{EditorCommand, Mode};
use crate::interviewer::Mode as InterviewerMode;

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

fn start_interviewer_connect(state: &mut AppState) -> Vec<Effect> {
    assert!(
        state.interviewer.enabled,
        "disabled Interviewer must not connect"
    );
    let operation = next_operation(state);
    state.interviewer.connecting = Some(operation);
    state.interviewer.status = InterviewerStatus::Connecting;
    state.error = None;
    vec![Effect::ConnectInterviewer { operation }]
}

fn interviewer_output_tail(output: &str) -> String {
    const MAX_INTERVIEWER_OUTPUT_BYTES: usize = 16 * 1024;
    if output.len() <= MAX_INTERVIEWER_OUTPUT_BYTES {
        return output.to_string();
    }
    let mut start = output.len() - MAX_INTERVIEWER_OUTPUT_BYTES;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let tail = output[start..].to_string();
    assert!(tail.len() <= MAX_INTERVIEWER_OUTPUT_BYTES);
    tail
}

fn dispatch_pending_submission_review(state: &mut AppState) -> Vec<Effect> {
    if !state.interviewer.enabled
        || !state.interviewer.disclosure_accepted
        || !matches!(
            state.interviewer.status,
            InterviewerStatus::Ready | InterviewerStatus::Feedback
        )
    {
        return Vec::new();
    }
    let Some(review) = state.interviewer.pending_submission_review.take() else {
        return Vec::new();
    };
    let operation = next_operation(state);
    let revision = review.revision;
    let solve = state.solve.as_ref().expect("solve exists");
    let effect = Effect::InterviewerTurn {
        operation,
        revision,
        mode: InterviewerMode::SubmissionReview,
        statement: solve.statement.clone(),
        source: review.source().to_string(),
        output: review.output().to_string(),
        question: String::new(),
        solved: true,
    };
    state.interviewer.status = InterviewerStatus::Thinking;
    state.interviewer.active = Some((operation, revision, InterviewerMode::SubmissionReview));
    state.status = format!("Reviewing recorded revision {revision}…");
    vec![effect]
}

fn finalize_interviewer_turn_and_dispatch_review(
    state: &mut AppState,
    operation: OperationId,
    revision: u64,
    mode: InterviewerMode,
    accepted: bool,
) -> Vec<Effect> {
    let mut effects = dispatch_pending_submission_review(state);
    // Effects execute from the end, so finalization reaches the worker before a queued review.
    effects.push(Effect::FinalizeInterviewerTurn {
        operation,
        revision,
        mode,
        accepted,
    });
    effects
}

fn enter_problem_list(state: &mut AppState) -> Vec<Effect> {
    let neovim_generation = state.solve.as_ref().map(|solve| solve.generation);
    state.solve = None;
    state.interviewer.clear_session();
    state.screen = Screen::ProblemList;
    state.focus = Focus::Main;
    state.detail_scroll = 0;
    state.progress_scroll = 0;
    let mut effects = load_effect(state);
    effects.push(Effect::ResetInterviewer);
    if let Some(generation) = neovim_generation {
        effects.push(Effect::StopNeovim { generation });
    }
    effects
}

fn solve_command(state: &mut AppState, action: Action) -> Vec<Effect> {
    let Some(solve) = state.solve.as_mut() else {
        return Vec::new();
    };
    state.error = None;
    solve.editor.error = None;

    if action == Action::Quit && solve.editor.dirty() {
        let confirmation = DiscardConfirmation {
            action: DiscardAction::Quit,
            revision: solve.editor.revision,
        };
        if solve.discard_confirmation != Some(confirmation) {
            solve.discard_confirmation = Some(confirmation);
            state.status = "Unsaved changes · Ctrl-Q again to quit".into();
            state.error = Some("unsaved changes; repeat quit to discard".into());
            return Vec::new();
        }
    } else {
        solve.discard_confirmation = None;
    }

    match action {
        Action::InterviewFocus => {
            solve.pane = SolvePane::Interview;
            solve.accessory_panes.interview_expanded = true;
            if !state.interviewer.enabled {
                state.interviewer.status = InterviewerStatus::Disabled;
                state.interviewer.composer_focused = false;
                return Vec::new();
            }
            if state.interviewer.disclosure_accepted {
                state.interviewer.composer_focused = true;
                if matches!(
                    state.interviewer.status,
                    InterviewerStatus::Offline
                        | InterviewerStatus::Disconnected
                        | InterviewerStatus::ProtocolError
                ) {
                    return start_interviewer_connect(state);
                }
            } else {
                state.interviewer.status = InterviewerStatus::Disclosure;
            }
            Vec::new()
        }
        Action::InterviewDisclosure(accepted)
            if state.interviewer.status == InterviewerStatus::Disclosure =>
        {
            if accepted {
                solve.accessory_panes.interview_expanded = true;
                state.interviewer.disclosure_accepted = true;
                state.interviewer.composer_focused = true;
                start_interviewer_connect(state)
            } else {
                state.interviewer.status = InterviewerStatus::Declined;
                state.interviewer.composer_focused = false;
                Vec::new()
            }
        }
        Action::InterviewChar(character) if state.interviewer.composer_focused => {
            if state
                .interviewer
                .composer
                .len()
                .saturating_add(character.len_utf8())
                <= MAX_COMPOSER_BYTES
            {
                state.interviewer.composer.push(character);
            }
            Vec::new()
        }
        Action::InterviewBackspace if state.interviewer.composer_focused => {
            state.interviewer.composer.pop();
            Vec::new()
        }
        Action::InterviewEscape => {
            state.interviewer.composer_focused = false;
            Vec::new()
        }
        Action::InterviewSend
            if state.interviewer.composer_focused
                && matches!(
                    state.interviewer.status,
                    InterviewerStatus::Ready | InterviewerStatus::Feedback
                ) =>
        {
            let question = state.interviewer.composer.trim().to_string();
            if question.is_empty() {
                return Vec::new();
            }
            let operation = next_operation(state);
            let solve = state.solve.as_ref().expect("solve exists");
            let revision = solve.editor.revision;
            state.interviewer.composer.clear();
            state.interviewer.composer_focused = false;
            state
                .interviewer
                .push_message("You".into(), question.clone());
            state.interviewer.status = InterviewerStatus::Thinking;
            state.interviewer.active = Some((operation, revision, InterviewerMode::Interviewer));
            vec![Effect::InterviewerTurn {
                operation,
                revision,
                mode: InterviewerMode::Interviewer,
                statement: solve.statement.clone(),
                source: solve.editor.text().to_string(),
                output: interviewer_output_tail(&solve.output),
                question,
                solved: state.interviewer.submission_recorded,
            }]
        }
        Action::Hint
            if state.interviewer.disclosure_accepted
                && matches!(
                    state.interviewer.status,
                    InterviewerStatus::Ready | InterviewerStatus::Feedback
                ) =>
        {
            let revision = solve.editor.revision;
            if state.interviewer.hint_revision != Some(revision) {
                state.interviewer.hint_revision = Some(revision);
                state.interviewer.hint_count = 0;
            }
            if state.interviewer.hint_count >= 3 {
                state.error = Some("maximum three hints reached for this revision".into());
                return Vec::new();
            }
            let level = state.interviewer.hint_count + 1;
            let operation = next_operation(state);
            let solve = state.solve.as_ref().expect("solve exists");
            state.interviewer.status = InterviewerStatus::Thinking;
            state.interviewer.active = Some((operation, revision, InterviewerMode::Hint(level)));
            vec![Effect::InterviewerTurn {
                operation,
                revision,
                mode: InterviewerMode::Hint(level),
                statement: solve.statement.clone(),
                source: solve.editor.text().to_string(),
                output: interviewer_output_tail(&solve.output),
                question: String::new(),
                solved: false,
            }]
        }
        Action::ResetInterview => {
            solve.submitted_source = None;
            state.interviewer.clear_session();
            vec![Effect::ResetInterviewer]
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
            let interviewer_operation = state
                .interviewer
                .active
                .map(|(operation, _, _)| operation)
                .or(state.interviewer.connecting);
            let runner_active = solve.running.map(|(operation, _, _)| operation);
            if let Some(operation) = interviewer_operation
                && (solve.pane == SolvePane::Interview || runner_active.is_none())
            {
                state.interviewer.status = InterviewerStatus::Disconnected;
                state.interviewer.connecting = None;
                state.interviewer.active = None;
                vec![Effect::CancelInterviewer { operation }]
            } else if let Some(operation) = runner_active {
                state.status = "Cancelling…".into();
                vec![Effect::CancelRun { operation }]
            } else {
                Vec::new()
            }
        }
        Action::Back => {
            if !solve.editor.dirty() {
                return vec![Effect::LeaveSolve];
            }
            if solve.pending_draft_save.is_some() {
                state.status = "Draft save already in progress…".into();
                return Vec::new();
            }
            let operation = next_operation(state);
            let solve = state.solve.as_mut().expect("solve exists");
            let revision = solve.editor.revision;
            let source = solve.editor.text().to_string();
            solve.pending_draft_save = Some((operation, revision, source.clone()));
            solve.pending_save = None;
            solve.quit_after_save = None;
            solve.refresh_after_submit = false;
            solve.submitted_source = None;
            state.status = "Saving draft…".into();
            vec![
                Effect::SaveDraft {
                    operation,
                    plan: solve.plan.clone(),
                    source,
                    revision,
                },
                Effect::LeaveSolve,
            ]
        }
        Action::EditorCollapse => {
            state.status = "Editor cannot be collapsed".into();
            Vec::new()
        }
        Action::ToggleCollapse => {
            match solve.pane {
                SolvePane::Editor => {
                    state.status = "Editor cannot be collapsed".into();
                }
                SolvePane::Problem => {
                    solve.accessory_panes.problem_expanded =
                        !solve.accessory_panes.problem_expanded;
                }
                SolvePane::Output => {
                    solve.accessory_panes.output_expanded = !solve.accessory_panes.output_expanded;
                }
                SolvePane::Interview => {
                    solve.accessory_panes.interview_expanded =
                        !solve.accessory_panes.interview_expanded;
                    if !solve.accessory_panes.interview_expanded {
                        state.interviewer.composer_focused = false;
                    }
                }
            }
            Vec::new()
        }
        Action::NextFocus | Action::PreviousFocus => {
            state.interviewer.composer_focused = false;
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
                    Mode::Normal | Mode::Visual => Err("paste ignored outside Insert mode".into()),
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
                solve.discard_confirmation = None;
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
                SolvePane::Interview => {
                    state.interviewer.scroll = state.interviewer.scroll.saturating_add(1)
                }
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
                SolvePane::Interview => {
                    state.interviewer.scroll = state.interviewer.scroll.saturating_sub(1)
                }
                SolvePane::Editor => {}
            }
            Vec::new()
        }
        Action::Quit
            if !solve.editor.dirty()
                || solve.discard_confirmation
                    == Some(DiscardConfirmation {
                        action: DiscardAction::Quit,
                        revision: solve.editor.revision,
                    }) =>
        {
            state.interviewer.clear_session();
            state.quit = true;
            vec![Effect::ResetInterviewer]
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
    if state.show_help {
        match &event {
            Event::Command(Action::Quit) if state.screen == Screen::Solve => {
                state.show_help = false;
                return solve_command(state, Action::Quit);
            }
            Event::Command(Action::Quit) => {
                state.interviewer.clear_session();
                state.quit = true;
                return vec![Effect::ResetInterviewer];
            }
            Event::Command(Action::Back | Action::Help) => {
                state.show_help = false;
                return Vec::new();
            }
            Event::Command(_) => return Vec::new(),
            _ => {}
        }
    }
    if matches!(&event, Event::Command(Action::Help)) {
        state.show_help = true;
        state.leader_pending = false;
        return Vec::new();
    }
    if state.screen == Screen::Solve {
        match event {
            Event::Command(action) => return solve_command(state, action),
            Event::NeovimStarted(result) => {
                let solve = state.solve.as_mut().expect("solve exists");
                match result {
                    Ok(()) => {
                        solve.editor_status = EditorRuntimeStatus::Ready;
                        if state.status == "Starting Neovim…" {
                            state.status = "Neovim ready".into();
                        }
                    }
                    Err(error) => {
                        solve.editor_status = EditorRuntimeStatus::Failed;
                        state.status = "Neovim failed".into();
                        state.error = Some(error);
                    }
                }
                return Vec::new();
            }
            Event::NeovimDocument(update) => {
                let solve = state.solve.as_mut().expect("solve exists");
                let revision_before = solve.editor.revision;
                if let Err(error) = solve.editor.install_neovim_snapshot(
                    update.text,
                    &update.mode,
                    update.changedtick,
                ) {
                    solve.editor_status = EditorRuntimeStatus::Failed;
                    state.error = Some(error);
                } else if solve.editor.revision != revision_before {
                    solve.discard_confirmation = None;
                    solve.stale = solve
                        .latest_run_revision
                        .is_some_and(|revision| revision != solve.editor.revision);
                }
                return Vec::new();
            }
            Event::NeovimView(view) => {
                let solve = state.solve.as_mut().expect("solve exists");
                solve.editor.update_neovim_mode(&view.mode);
                solve.editor_view = Some(view);
                return Vec::new();
            }
            Event::NeovimWarning(error) => {
                state.solve.as_mut().expect("solve exists").editor_status =
                    EditorRuntimeStatus::Ready;
                state.status = "Neovim edit rejected".into();
                state.error = Some(error);
                return Vec::new();
            }
            Event::NeovimFailed(error) => {
                state.solve.as_mut().expect("solve exists").editor_status =
                    EditorRuntimeStatus::Failed;
                state.status = "Neovim failed".into();
                state.error = Some(error);
                return Vec::new();
            }
            Event::InterviewerConnected(operation, result) => {
                if state.interviewer.connecting != Some(operation) {
                    return Vec::new();
                }
                state.interviewer.connecting = None;
                match result {
                    Ok(()) => {
                        state.interviewer.status = InterviewerStatus::Ready;
                        state.error = None;
                        return dispatch_pending_submission_review(state);
                    }
                    Err(error) if error.kind() == crate::interviewer::ErrorKind::Authentication => {
                        state.interviewer.status = InterviewerStatus::AuthRequired;
                        state.interviewer.pending_submission_review = None;
                        state.error = Some(error.to_string());
                    }
                    Err(error) => {
                        state.interviewer.status = InterviewerStatus::Disconnected;
                        state.error = Some(error.to_string());
                    }
                }
                return Vec::new();
            }
            Event::InterviewerFinished(operation, revision, mode, result) => {
                if state.interviewer.active != Some((operation, revision, mode)) {
                    return vec![Effect::FinalizeInterviewerTurn {
                        operation,
                        revision,
                        mode,
                        accepted: false,
                    }];
                }
                state.interviewer.active = None;
                let message = match result {
                    Ok(message) => message,
                    Err(error) => {
                        state.interviewer.status =
                            if error.kind() == crate::interviewer::ErrorKind::Authentication {
                                InterviewerStatus::AuthRequired
                            } else {
                                InterviewerStatus::ProtocolError
                            };
                        state.error = Some(error.to_string());
                        return vec![Effect::FinalizeInterviewerTurn {
                            operation,
                            revision,
                            mode,
                            accepted: false,
                        }];
                    }
                };
                let current_revision = state.solve.as_ref().expect("solve exists").editor.revision;
                if mode != InterviewerMode::SubmissionReview && current_revision != revision {
                    state.interviewer.status = if state.interviewer.messages.len() > 1 {
                        InterviewerStatus::Feedback
                    } else {
                        InterviewerStatus::Ready
                    };
                    state.error = Some(
                        "Interviewer response ignored because the source changed during the turn"
                            .into(),
                    );
                    return finalize_interviewer_turn_and_dispatch_review(
                        state, operation, revision, mode, false,
                    );
                }
                let label = match mode {
                    InterviewerMode::Interviewer => "Interviewer".to_string(),
                    InterviewerMode::Hint(_) => "Hinter".to_string(),
                    InterviewerMode::SubmissionReview => {
                        format!("Submission review · recorded revision {revision}")
                    }
                };
                if matches!(mode, InterviewerMode::Hint(_)) {
                    state.interviewer.hint_count = state.interviewer.hint_count.saturating_add(1);
                }
                state.interviewer.push_message(label, message);
                state.interviewer.status = InterviewerStatus::Feedback;
                return finalize_interviewer_turn_and_dispatch_review(
                    state, operation, revision, mode, true,
                );
            }
            Event::InterviewerDisconnected(error) => {
                state.interviewer.connecting = None;
                state.interviewer.active = None;
                state.interviewer.composer_focused = false;
                state.interviewer.status = InterviewerStatus::Disconnected;
                state.error = Some(error);
                return Vec::new();
            }
            Event::RunnerLeftSolve(result) => match result {
                Err(error) => {
                    if let Some(solve) = state.solve.as_mut() {
                        solve.pending_draft_save = None;
                    }
                    state.status = "Runner cancellation failed · still in Solve".into();
                    state.error = Some(error);
                    return Vec::new();
                }
                Ok(())
                    if state
                        .solve
                        .as_ref()
                        .is_some_and(|solve| solve.pending_draft_save.is_some()) =>
                {
                    return Vec::new();
                }
                Ok(()) => return enter_problem_list(state),
            },
            Event::DraftSaved(operation, revision, source, result) => {
                let Some(solve) = state.solve.as_mut() else {
                    return Vec::new();
                };
                if !solve.pending_draft_save.as_ref().is_some_and(|pending| {
                    (pending.0, pending.1, pending.2.as_str())
                        == (operation, revision, source.as_str())
                }) {
                    return Vec::new();
                }
                solve.pending_draft_save = None;
                match result {
                    Err(error) => {
                        state.status = "Draft save failed · still in Solve".into();
                        state.error = Some(format!(
                            "Draft was not saved: {error}. The editor remains open and dirty; retry Space-b."
                        ));
                        return Vec::new();
                    }
                    Ok(()) => {
                        solve.editor.mark_saved(revision, &source);
                        if solve.editor.revision == revision && solve.editor.text() == source {
                            return enter_problem_list(state);
                        }
                        state.status = "Older draft saved · newer edits remain in Solve".into();
                        state.error = Some(
                            "The source changed while the draft was saving. Newer edits remain open and dirty; retry Space-b."
                                .into(),
                        );
                        return Vec::new();
                    }
                }
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
                    state.interviewer.submission_recorded = true;
                    let source = submitted_source
                        .expect("successful matching submit retains its captured source");
                    let output = interviewer_output_tail(&solve.output);
                    let review_allowed = state.interviewer.enabled
                        && state.interviewer.disclosure_accepted
                        && matches!(
                            state.interviewer.status,
                            InterviewerStatus::Offline
                                | InterviewerStatus::Connecting
                                | InterviewerStatus::Ready
                                | InterviewerStatus::Thinking
                                | InterviewerStatus::Feedback
                                | InterviewerStatus::Disconnected
                                | InterviewerStatus::ProtocolError
                        );
                    let replaced = if review_allowed {
                        let replaced = state
                            .interviewer
                            .pending_submission_review
                            .replace(RecordedSubmissionReview::new(revision, source, output))
                            .is_some();
                        if replaced {
                            state
                                .interviewer
                                .pending_submission_review
                                .as_mut()
                                .expect("newest review inserted")
                                .replaced_older = true;
                        }
                        replaced
                    } else {
                        false
                    };
                    let mut effects = load_effect(state);
                    effects.extend(dispatch_pending_submission_review(state));
                    if let Some(review) = state.interviewer.pending_submission_review.as_ref() {
                        state.status = if replaced {
                            format!(
                                "Submit recorded · queued review replaced by recorded revision {}",
                                review.revision
                            )
                        } else {
                            format!(
                                "Submit recorded · review queued for recorded revision {}",
                                review.revision
                            )
                        };
                    } else if let Some((_, review_revision, InterviewerMode::SubmissionReview)) =
                        state.interviewer.active
                    {
                        state.status = format!(
                            "Submit recorded · reviewing recorded revision {review_revision}"
                        );
                    }
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
                        state.status = if let Some(review) =
                            state.interviewer.pending_submission_review.as_ref()
                        {
                            if review.replaced_older {
                                format!(
                                    "Submit recorded · progress refreshed · queued review replaced by recorded revision {}",
                                    review.revision
                                )
                            } else {
                                format!(
                                    "Submit recorded · progress refreshed · review queued for recorded revision {}",
                                    review.revision
                                )
                            }
                        } else if let Some((_, revision, InterviewerMode::SubmissionReview)) =
                            state.interviewer.active
                        {
                            format!(
                                "Submit recorded · progress refreshed · reviewing recorded revision {revision}"
                            )
                        } else {
                            "Submit recorded · progress refreshed".into()
                        };
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
        Event::InterviewerFinished(operation, revision, mode, _) => {
            return vec![Effect::FinalizeInterviewerTurn {
                operation,
                revision,
                mode,
                accepted: false,
            }];
        }
        Event::InterviewerDisconnected(error) => {
            state.interviewer.connecting = None;
            state.interviewer.active = None;
            state.interviewer.composer_focused = false;
            state.interviewer.status = InterviewerStatus::Disconnected;
            state.error = Some(error);
        }
        Event::InterviewerConnected(_, _) => {}
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
            state.interviewer.clear_session();
            state.quit = true;
            return vec![Effect::ResetInterviewer];
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
            | Action::ToggleCollapse
            | Action::EditorCollapse
            | Action::Editor(_),
        ) => {}
        Event::SolveOpened(operation, result) => {
            if state.active_operation != Some(operation) {
                return Vec::new();
            }
            state.active_operation = None;
            match result {
                Ok(solve) => {
                    let effect = Effect::StartNeovim {
                        generation: solve.generation,
                        source: solve.editor.text().to_string(),
                        synthetic_name: format!(
                            "interview://{}.{}",
                            solve.problem_slug, solve.language
                        ),
                        language: solve.language.clone(),
                    };
                    state.solve = Some(*solve);
                    state.screen = Screen::Solve;
                    state.status = "Starting Neovim…".into();
                    state.error = None;
                    return vec![effect];
                }
                Err(error) => {
                    state.status = "Source load failed".into();
                    state.error = Some(error)
                }
            }
        }
        Event::NeovimStarted(_)
        | Event::NeovimDocument(_)
        | Event::NeovimView(_)
        | Event::NeovimWarning(_)
        | Event::NeovimFailed(_) => {}
        Event::RunFinished(_, _, _, _, _)
        | Event::DraftSaved(_, _, _, _)
        | Event::RunnerLeftSolve(_) => {}
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
    use crate::app::model::{AppData, ProblemRow, SetRow, SolvePane};
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
            generation: crate::neovim::SessionGeneration(1),
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
            editor_view: None,
            editor_status: crate::app::model::EditorRuntimeStatus::Ready,
            pane: SolvePane::Editor,
            accessory_panes: crate::app::model::AccessoryPaneState::default(),
            output: String::new(),
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
    fn accessory_panes_collapse_independently_and_interview_focus_expands() {
        let mut state = solve_state();
        state.interviewer.status = InterviewerStatus::Thinking;
        state.interviewer.active = Some((OperationId(90), 0, InterviewerMode::Interviewer));
        state.interviewer.composer_focused = true;
        state
            .interviewer
            .push_message("Interviewer".into(), "preserved".into());

        for pane in [SolvePane::Problem, SolvePane::Output, SolvePane::Interview] {
            state.solve.as_mut().unwrap().pane = pane;
            reduce(&mut state, Event::Command(Action::ToggleCollapse));
        }
        let solve = state.solve.as_ref().unwrap();
        assert!(!solve.accessory_panes.problem_expanded);
        assert!(!solve.accessory_panes.output_expanded);
        assert!(!solve.accessory_panes.interview_expanded);
        assert!(!state.interviewer.composer_focused);
        assert_eq!(
            state.interviewer.active,
            Some((OperationId(90), 0, InterviewerMode::Interviewer))
        );
        assert_eq!(state.interviewer.messages.last().unwrap().1, "preserved");

        reduce(&mut state, Event::Command(Action::NextFocus));
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Editor);
        assert!(
            !state
                .solve
                .as_ref()
                .unwrap()
                .accessory_panes
                .interview_expanded
        );
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Interview);
        assert!(
            state
                .solve
                .as_ref()
                .unwrap()
                .accessory_panes
                .interview_expanded
        );
        assert_eq!(
            state.interviewer.active,
            Some((OperationId(90), 0, InterviewerMode::Interviewer))
        );

        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        reduce(&mut state, Event::Command(Action::ToggleCollapse));
        assert_eq!(state.status, "Editor cannot be collapsed");
    }

    #[test]
    fn disclosure_acceptance_expands_a_collapsed_interview_before_focusing_composer() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state
            .solve
            .as_mut()
            .unwrap()
            .accessory_panes
            .interview_expanded = false;
        state.interviewer.status = InterviewerStatus::Disclosure;

        let effects = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );

        assert!(
            state
                .solve
                .as_ref()
                .unwrap()
                .accessory_panes
                .interview_expanded
        );
        assert!(state.interviewer.composer_focused);
        assert!(matches!(
            effects.as_slice(),
            [Effect::ConnectInterviewer { .. }]
        ));
    }

    #[test]
    fn delayed_editor_collapse_never_toggles_the_pane_that_gained_focus() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        let before = state.solve.as_ref().unwrap().accessory_panes;

        reduce(&mut state, Event::Command(Action::EditorCollapse));

        assert_eq!(state.solve.as_ref().unwrap().accessory_panes, before);
        assert_eq!(state.status, "Editor cannot be collapsed");
    }

    #[test]
    fn solve_help_is_modal_before_ordinary_commands() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::Help));
        assert!(state.show_help);

        let effects = reduce(&mut state, Event::Command(Action::SaveTest));
        assert!(effects.is_empty());
        assert!(state.solve.as_ref().unwrap().running.is_none());
        assert!(state.show_help);

        reduce(
            &mut state,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "edit accepted behind help".into(),
                mode: "n".into(),
                changedtick: 2,
            }),
        );
        assert_eq!(
            state.solve.as_ref().unwrap().editor.text(),
            "edit accepted behind help"
        );
        assert!(state.show_help);

        reduce(&mut state, Event::Command(Action::Back));
        assert!(!state.show_help);
        assert_eq!(state.screen, Screen::Solve);
    }

    #[test]
    fn dirty_quit_is_guarded_in_non_normal_editor_modes() {
        let mut state = solve_state();
        reduce(
            &mut state,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "dirty source".into(),
                mode: "i".into(),
                changedtick: 2,
            }),
        );

        reduce(&mut state, Event::Command(Action::Quit));
        assert!(!state.quit);
        assert_eq!(
            state.solve.as_ref().unwrap().discard_confirmation,
            Some(DiscardConfirmation {
                action: DiscardAction::Quit,
                revision: 1,
            })
        );
        reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
    }

    #[test]
    fn dirty_quit_confirmation_is_invalidated_by_a_new_source_snapshot() {
        let mut state = solve_state();
        reduce(
            &mut state,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "first dirty source".into(),
                mode: "n".into(),
                changedtick: 2,
            }),
        );
        reduce(&mut state, Event::Command(Action::Quit));
        assert!(!state.quit);

        reduce(
            &mut state,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "second dirty source".into(),
                mode: "n".into(),
                changedtick: 3,
            }),
        );
        reduce(&mut state, Event::Command(Action::Quit));

        assert!(!state.quit);
        assert!(state.solve.as_ref().unwrap().editor.dirty());
    }

    #[test]
    fn dirty_back_drains_before_draft_save_and_success_enters_scoped_list() {
        let mut state = solve_state();
        reduce(
            &mut state,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "newest draft\n".into(),
                mode: "n".into(),
                changedtick: 8,
            }),
        );
        let running = reduce(&mut state, Event::Command(Action::SaveTest));
        assert!(matches!(running.as_slice(), [Effect::SaveRun { .. }]));

        let effects = reduce(&mut state, Event::Command(Action::Back));
        let [
            Effect::SaveDraft {
                operation,
                revision,
                source,
                ..
            },
            Effect::LeaveSolve,
        ] = effects.as_slice()
        else {
            panic!("runner must drain before the LIFO draft-save effect")
        };
        assert_eq!(*revision, 1);
        assert_eq!(source, "newest draft\n");
        assert!(state.solve.as_ref().unwrap().running.is_some());
        assert!(state.solve.as_ref().unwrap().pending_save.is_none());
        assert_eq!(state.screen, Screen::Solve);

        let completion = reduce(
            &mut state,
            Event::DraftSaved(*operation, *revision, source.clone(), Ok(())),
        );
        assert_eq!(state.screen, Screen::ProblemList);
        assert!(state.solve.is_none());
        assert!(
            completion
                .iter()
                .any(|effect| matches!(effect, Effect::Load {
            scope: LoadScope::ProblemSet(slug),
            ..
        } if slug == "a"))
        );
        assert!(
            completion
                .iter()
                .any(|effect| matches!(effect, Effect::ResetInterviewer))
        );
        assert!(
            completion
                .iter()
                .any(|effect| matches!(effect, Effect::StopNeovim { .. }))
        );
    }

    #[test]
    fn draft_save_failure_or_newer_edit_stays_dirty_in_solve() {
        let mut failed = solve_state();
        reduce(
            &mut failed,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "failed draft".into(),
                mode: "n".into(),
                changedtick: 2,
            }),
        );
        let save = reduce(&mut failed, Event::Command(Action::Back));
        let Effect::SaveDraft {
            operation,
            revision,
            source,
            ..
        } = save[0].clone()
        else {
            panic!("expected draft save")
        };
        assert!(
            reduce(
                &mut failed,
                Event::DraftSaved(operation, revision, source, Err("read-only target".into()))
            )
            .is_empty()
        );
        assert_eq!(failed.screen, Screen::Solve);
        assert!(failed.solve.as_ref().unwrap().editor.dirty());
        assert!(failed.error.as_deref().unwrap().contains("retry Space-b"));

        let mut newer = solve_state();
        reduce(
            &mut newer,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "captured draft".into(),
                mode: "n".into(),
                changedtick: 3,
            }),
        );
        let save = reduce(&mut newer, Event::Command(Action::Back));
        let Effect::SaveDraft {
            operation,
            revision,
            source,
            ..
        } = save[0].clone()
        else {
            panic!("expected draft save")
        };
        reduce(
            &mut newer,
            Event::NeovimDocument(crate::neovim::DocumentUpdate {
                text: "newer edit".into(),
                mode: "n".into(),
                changedtick: 4,
            }),
        );
        assert!(
            reduce(
                &mut newer,
                Event::DraftSaved(operation, revision, source, Ok(()))
            )
            .is_empty()
        );
        assert_eq!(newer.screen, Screen::Solve);
        let solve = newer.solve.as_ref().unwrap();
        assert!(solve.editor.dirty());
        assert_eq!(solve.editor.text(), "newer edit");
        assert!(newer.error.as_deref().unwrap().contains("Newer edits"));
    }

    #[test]
    fn clean_back_waits_for_runner_drain_before_leaving_solve() {
        let mut state = solve_state();
        let generation = state.solve.as_ref().unwrap().generation;
        let effects = reduce(&mut state, Event::Command(Action::Back));
        assert_eq!(state.screen, Screen::Solve);
        assert_eq!(state.solve.as_ref().unwrap().generation, generation);
        assert!(matches!(effects.as_slice(), [Effect::LeaveSolve]));

        let completion = reduce(&mut state, Event::RunnerLeftSolve(Ok(())));
        assert_eq!(state.screen, Screen::ProblemList);
        assert!(state.solve.is_none());
        assert!(
            completion
                .iter()
                .any(|effect| matches!(effect, Effect::Load {
            scope: LoadScope::ProblemSet(slug),
            ..
        } if slug == "a"))
        );
        assert!(
            !completion
                .iter()
                .any(|effect| matches!(effect, Effect::LeaveSolve))
        );
    }

    #[test]
    fn pane_focus_changes_close_the_interview_composer() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.composer_focused = true;

        reduce(&mut state, Event::Command(Action::NextFocus));
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Editor);
        assert!(!state.interviewer.composer_focused);

        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.composer_focused = true;
        reduce(&mut state, Event::Command(Action::PreviousFocus));
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Output);
        assert!(!state.interviewer.composer_focused);
    }

    fn finish_successful_submit(state: &mut AppState, output: &str) -> Vec<Effect> {
        let submit = reduce(state, Event::Command(Action::Submit));
        let Effect::SaveRun {
            operation,
            revision,
            source,
            intent: RunIntent::Submit,
            ..
        } = submit[0].clone()
        else {
            panic!("expected submit")
        };
        reduce(
            state,
            Event::RunFinished(
                operation,
                revision,
                RunIntent::Submit,
                Some(source),
                Ok(crate::runner::ExecutionResult::test_result(
                    crate::runner::Termination::Exited(0),
                    output,
                )),
            ),
        )
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
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Ready;
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
        assert_eq!(state.interviewer.status, InterviewerStatus::Ready);
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
        assert!(state.solve.is_some());
        assert_eq!(state.screen, Screen::Solve);
        reduce(&mut state, Event::RunnerLeftSolve(Ok(())));
        assert!(state.solve.is_none());
        assert_eq!(state.screen, Screen::ProblemList);
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
            Some(DiscardConfirmation {
                action: DiscardAction::Quit,
                revision: 1,
            })
        );
        state
            .interviewer
            .push_message("Interviewer".into(), "private".into());
        let quit = reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
        assert!(state.interviewer.messages.is_empty());
        assert!(matches!(quit.as_slice(), [Effect::ResetInterviewer]));
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
    fn interviewer_disclosure_question_hint_stale_response_and_failure_preserve_solve() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert_eq!(state.interviewer.status, InterviewerStatus::Disclosure);
        let effects = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let [Effect::ConnectInterviewer { operation }] = effects.as_slice() else {
            panic!("expected connect")
        };
        reduce(&mut state, Event::InterviewerConnected(*operation, Ok(())));
        reduce(&mut state, Event::Command(Action::InterviewChar('W')));
        let effects = reduce(&mut state, Event::Command(Action::InterviewSend));
        let Effect::InterviewerTurn {
            operation,
            revision,
            mode: InterviewerMode::Interviewer,
            ..
        } = effects[0]
        else {
            panic!("expected interview turn")
        };
        let stale = reduce(
            &mut state,
            Event::InterviewerFinished(
                OperationId(operation.0 + 1),
                revision,
                InterviewerMode::Interviewer,
                Ok("stale".into()),
            ),
        );
        assert!(matches!(
            stale.as_slice(),
            [Effect::FinalizeInterviewerTurn {
                accepted: false,
                ..
            }]
        ));
        assert_eq!(state.interviewer.messages.len(), 1);
        assert!(
            !state
                .interviewer
                .messages
                .iter()
                .any(|(_, message)| message == "stale")
        );
        let failure = reduce(
            &mut state,
            Event::InterviewerFinished(
                operation,
                revision,
                InterviewerMode::Interviewer,
                Err(crate::interviewer::InterviewerError::protocol(
                    crate::interviewer::Backend::Pi,
                    "protocol failed",
                )),
            ),
        );
        assert!(matches!(
            failure.as_slice(),
            [Effect::FinalizeInterviewerTurn {
                accepted: false,
                ..
            }]
        ));
        assert_eq!(state.interviewer.status, InterviewerStatus::ProtocolError);
        assert!(state.solve.is_some());
        state
            .interviewer
            .messages
            .push(("Interviewer".into(), "secret".into()));
        let reset = reduce(&mut state, Event::Command(Action::ResetInterview));
        assert!(matches!(reset.as_slice(), [Effect::ResetInterviewer]));
        assert!(state.interviewer.messages.is_empty());
        assert_eq!(state.interviewer.status, InterviewerStatus::Offline);
    }

    #[test]
    fn interviewer_completion_requires_operation_revision_mode_and_current_editor_revision() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        let connect = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let Effect::ConnectInterviewer {
            operation: connect_operation,
        } = connect[0]
        else {
            panic!("expected connect")
        };
        reduce(
            &mut state,
            Event::InterviewerConnected(connect_operation, Ok(())),
        );
        reduce(&mut state, Event::Command(Action::InterviewChar('W')));
        let effects = reduce(&mut state, Event::Command(Action::InterviewSend));
        let Effect::InterviewerTurn {
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
            Event::InterviewerFinished(
                operation,
                revision,
                InterviewerMode::Hint(1),
                Ok("wrong mode".into()),
            ),
        );
        assert!(matches!(
            wrong_mode.as_slice(),
            [Effect::FinalizeInterviewerTurn {
                accepted: false,
                ..
            }]
        ));
        assert!(state.interviewer.active.is_some());
        assert!(
            !state
                .interviewer
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
            Event::InterviewerFinished(operation, revision, mode, Ok("stale response".into())),
        );
        assert!(state.interviewer.active.is_none());
        assert!(
            !state
                .interviewer
                .messages
                .iter()
                .any(|(_, text)| text == "stale response")
        );
        assert!(state.error.as_deref().unwrap().contains("source changed"));
    }

    #[test]
    fn submission_review_dispatch_and_completion_stay_bound_to_recorded_revision_after_edit() {
        let mut state = solve_state();
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Ready;
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
        let (review_operation, review_revision, review_source) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::InterviewerTurn {
                    operation,
                    revision,
                    mode: InterviewerMode::SubmissionReview,
                    source,
                    ..
                } => Some((*operation, *revision, source.clone())),
                _ => None,
            })
            .expect("submission review");
        assert_eq!(review_revision, revision);
        assert_eq!(review_source, submitted_source);

        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        assert_ne!(review_source, state.solve.as_ref().unwrap().editor.text());
        let completion = reduce(
            &mut state,
            Event::InterviewerFinished(
                review_operation,
                review_revision,
                InterviewerMode::SubmissionReview,
                Ok("recorded result passes".into()),
            ),
        );
        assert!(matches!(
            completion.as_slice(),
            [Effect::FinalizeInterviewerTurn { accepted: true, .. }]
        ));
        assert_eq!(
            state.interviewer.messages.last(),
            Some(&(
                format!("Submission review · recorded revision {review_revision}"),
                "recorded result passes".into()
            ))
        );
        assert!(state.solve.as_ref().unwrap().submitted_source.is_none());
    }

    #[test]
    fn successful_submit_queues_behind_turn_and_connect_then_dispatches_first() {
        let mut turning = solve_state();
        turning.interviewer.disclosure_accepted = true;
        turning.interviewer.status = InterviewerStatus::Thinking;
        turning.interviewer.active = Some((OperationId(90), 0, InterviewerMode::Interviewer));
        let effects = finish_successful_submit(&mut turning, "TURN-BUSY");
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, Effect::InterviewerTurn { .. }))
        );
        assert_eq!(
            turning
                .interviewer
                .pending_submission_review
                .as_ref()
                .map(|review| review.revision),
            Some(0)
        );
        let completion = reduce(
            &mut turning,
            Event::InterviewerFinished(
                OperationId(90),
                0,
                InterviewerMode::Interviewer,
                Ok("question complete".into()),
            ),
        );
        assert!(matches!(
            completion.as_slice(),
            [
                Effect::InterviewerTurn {
                    mode: InterviewerMode::SubmissionReview,
                    revision: 0,
                    ..
                },
                Effect::FinalizeInterviewerTurn {
                    mode: InterviewerMode::Interviewer,
                    accepted: true,
                    ..
                }
            ]
        ));
        assert!(reduce(&mut turning, Event::Command(Action::Hint)).is_empty());

        let mut connecting = solve_state();
        connecting.interviewer.disclosure_accepted = true;
        connecting.interviewer.status = InterviewerStatus::Connecting;
        connecting.interviewer.connecting = Some(OperationId(91));
        finish_successful_submit(&mut connecting, "CONNECTING");
        let connected = reduce(
            &mut connecting,
            Event::InterviewerConnected(OperationId(91), Ok(())),
        );
        assert!(matches!(
            connected.as_slice(),
            [Effect::InterviewerTurn {
                mode: InterviewerMode::SubmissionReview,
                revision: 0,
                ..
            }]
        ));
        assert_eq!(connecting.interviewer.status, InterviewerStatus::Thinking);
    }

    #[test]
    fn newest_queued_submission_replaces_older_and_reset_or_exit_clears_it() {
        let mut state = solve_state();
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Thinking;
        state.interviewer.active = Some((OperationId(90), 0, InterviewerMode::Interviewer));
        finish_successful_submit(&mut state, "FIRST");
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Normal('i'))),
        );
        reduce(
            &mut state,
            Event::Command(Action::Editor(EditorAction::Insert('x'))),
        );
        let second_effects = finish_successful_submit(&mut state, "SECOND");
        let reload_operation = second_effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Load { operation, .. } => Some(*operation),
                _ => None,
            })
            .expect("progress reload");
        let pending = state
            .interviewer
            .pending_submission_review
            .as_ref()
            .expect("newest review queued");
        assert_eq!(pending.revision, 1);
        assert_eq!(pending.source(), "xprint(1)");
        assert!(pending.output().contains("SECOND"));
        assert!(state.status.contains("queued review replaced"));
        reduce(
            &mut state,
            Event::Loaded(reload_operation, Ok(Box::new(AppData::empty()))),
        );
        assert!(state.status.contains("queued review replaced"));

        reduce(&mut state, Event::Command(Action::ResetInterview));
        assert!(state.interviewer.pending_submission_review.is_none());
        state.interviewer.pending_submission_review = Some(RecordedSubmissionReview::new(
            1,
            "private".into(),
            "output".into(),
        ));
        reduce(&mut state, Event::Command(Action::Back));
        reduce(&mut state, Event::RunnerLeftSolve(Ok(())));
        assert!(state.interviewer.pending_submission_review.is_none());
        assert!(state.solve.is_none());
    }

    #[test]
    fn queued_submission_survives_turn_failure_and_dispatches_after_reconnect() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Thinking;
        state.interviewer.active = Some((OperationId(90), 0, InterviewerMode::Interviewer));
        finish_successful_submit(&mut state, "RECORDED");
        let failed = reduce(
            &mut state,
            Event::InterviewerFinished(
                OperationId(90),
                0,
                InterviewerMode::Interviewer,
                Err(crate::interviewer::InterviewerError::protocol(
                    crate::interviewer::Backend::Pi,
                    "turn failed",
                )),
            ),
        );
        assert!(matches!(
            failed.as_slice(),
            [Effect::FinalizeInterviewerTurn {
                accepted: false,
                ..
            }]
        ));
        assert!(state.interviewer.pending_submission_review.is_some());
        let reconnect = reduce(&mut state, Event::Command(Action::InterviewFocus));
        let Effect::ConnectInterviewer { operation } = reconnect[0] else {
            panic!("expected reconnect")
        };
        let review = reduce(&mut state, Event::InterviewerConnected(operation, Ok(())));
        assert!(matches!(
            review.as_slice(),
            [Effect::InterviewerTurn {
                mode: InterviewerMode::SubmissionReview,
                source,
                output,
                ..
            }] if source == "print(1)" && output.contains("RECORDED")
        ));
    }

    #[test]
    fn terminal_interviewer_states_never_queue_or_send_submission_review() {
        for status in [
            InterviewerStatus::Disabled,
            InterviewerStatus::Declined,
            InterviewerStatus::AuthRequired,
        ] {
            let mut state = solve_state();
            state.interviewer.enabled = status != InterviewerStatus::Disabled;
            state.interviewer.disclosure_accepted = true;
            state.interviewer.status = status;
            let effects = finish_successful_submit(&mut state, "PASS");
            assert!(state.interviewer.pending_submission_review.is_none());
            assert!(
                effects
                    .iter()
                    .all(|effect| !matches!(effect, Effect::InterviewerTurn { .. }))
            );
        }
    }

    #[test]
    fn reset_invalidates_delayed_connect_completion() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        let connect = reduce(
            &mut state,
            Event::Command(Action::InterviewDisclosure(true)),
        );
        let Effect::ConnectInterviewer { operation } = connect[0] else {
            panic!("expected connect")
        };
        reduce(&mut state, Event::Command(Action::ResetInterview));
        reduce(&mut state, Event::InterviewerConnected(operation, Ok(())));
        assert_eq!(state.interviewer.status, InterviewerStatus::Offline);
        assert!(!state.interviewer.composer_focused);

        let reconnect = reduce(&mut state, Event::Command(Action::InterviewFocus));
        let Effect::ConnectInterviewer {
            operation: reconnect_operation,
        } = reconnect[0]
        else {
            panic!("expected reconnect")
        };
        assert_ne!(operation, reconnect_operation);
        reduce(
            &mut state,
            Event::InterviewerConnected(
                operation,
                Err(crate::interviewer::InterviewerError::protocol(
                    crate::interviewer::Backend::Pi,
                    "stale",
                )),
            ),
        );
        assert_eq!(state.interviewer.status, InterviewerStatus::Connecting);
        reduce(
            &mut state,
            Event::InterviewerConnected(reconnect_operation, Ok(())),
        );
        assert_eq!(state.interviewer.status, InterviewerStatus::Ready);
    }

    #[test]
    fn interviewer_protocol_error_can_explicitly_reconnect_without_reset() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::ProtocolError;
        let effects = reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert!(matches!(
            effects.as_slice(),
            [Effect::ConnectInterviewer { .. }]
        ));
        assert_eq!(state.interviewer.status, InterviewerStatus::Connecting);
    }

    #[test]
    fn interview_scroll_is_bounded_and_resets_on_append_and_clear() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.scroll = MAX_SCROLL;
        reduce(&mut state, Event::Command(Action::Up));
        assert_eq!(state.interviewer.scroll, MAX_SCROLL);
        reduce(&mut state, Event::Command(Action::Down));
        assert_eq!(state.interviewer.scroll, MAX_SCROLL - 1);
        state
            .interviewer
            .push_message("Interviewer".into(), "new".into());
        assert_eq!(state.interviewer.scroll, 0);
        state.interviewer.scroll = 10;
        state.interviewer.clear_session();
        assert_eq!(state.interviewer.scroll, 0);
    }

    #[test]
    fn cancel_prefers_focused_interviewer_then_runner_and_uses_sole_active_operation() {
        let mut state = solve_state();
        let run = reduce(&mut state, Event::Command(Action::SaveTest));
        let Effect::SaveRun {
            operation: run_operation,
            ..
        } = run[0]
        else {
            panic!("expected runner")
        };
        state.interviewer.active = Some((OperationId(99), 0, InterviewerMode::Interviewer));
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelRun { operation }] if *operation == run_operation
        ));
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelInterviewer { operation }] if *operation == OperationId(99)
        ));
        state.interviewer.active = Some((OperationId(100), 0, InterviewerMode::Interviewer));
        state.solve.as_mut().unwrap().running = None;
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::Cancel)).as_slice(),
            [Effect::CancelInterviewer { operation }] if *operation == OperationId(100)
        ));
    }

    #[test]
    fn hints_are_limited_to_three_per_revision_and_reset_after_edit() {
        let mut state = solve_state();
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Ready;
        for level in 1..=3 {
            let effects = reduce(&mut state, Event::Command(Action::Hint));
            let Effect::InterviewerTurn {
                operation,
                revision,
                mode,
                ..
            } = effects[0]
            else {
                panic!("expected hint")
            };
            assert_eq!(mode, InterviewerMode::Hint(level));
            assert!(matches!(
                reduce(
                    &mut state,
                    Event::InterviewerFinished(
                        operation,
                        revision,
                        mode,
                        Ok(format!("hint-{level}"))
                    )
                )
                .as_slice(),
                [Effect::FinalizeInterviewerTurn { accepted: true, .. }]
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
        state.interviewer.status = InterviewerStatus::Ready;
        let next = reduce(&mut state, Event::Command(Action::Hint));
        assert!(matches!(
            next.as_slice(),
            [Effect::InterviewerTurn {
                mode: InterviewerMode::Hint(1),
                ..
            }]
        ));
    }

    #[test]
    fn disabled_interviewer_never_discloses_or_requests_a_worker_effect() {
        let mut state = solve_state();
        state.disable_interviewer();

        assert!(reduce(&mut state, Event::Command(Action::InterviewFocus)).is_empty());
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Interview);
        assert_eq!(state.interviewer.status, InterviewerStatus::Disabled);
        assert!(!state.interviewer.composer_focused);
        assert!(reduce(&mut state, Event::Command(Action::Hint)).is_empty());
        assert!(
            reduce(&mut state, Event::Command(Action::ResetInterview))
                .iter()
                .all(|effect| !matches!(
                    effect,
                    Effect::ConnectInterviewer { .. } | Effect::InterviewerTurn { .. }
                ))
        );
        assert_eq!(state.interviewer.status, InterviewerStatus::Disabled);
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::SaveTest)).as_slice(),
            [Effect::SaveRun {
                intent: RunIntent::Test,
                ..
            }]
        ));

        let mut submit_state = solve_state();
        submit_state.disable_interviewer();
        assert!(matches!(
            reduce(&mut submit_state, Event::Command(Action::Submit)).as_slice(),
            [Effect::SaveRun {
                intent: RunIntent::Submit,
                ..
            }]
        ));
    }

    #[test]
    fn interviewer_disclosure_decline_keeps_local_solve_available() {
        let mut state = solve_state();
        reduce(&mut state, Event::Command(Action::InterviewFocus));
        assert!(
            reduce(
                &mut state,
                Event::Command(Action::InterviewDisclosure(false))
            )
            .is_empty()
        );
        assert_eq!(state.interviewer.status, InterviewerStatus::Declined);
        let effects = reduce(&mut state, Event::Command(Action::SaveTest));
        assert!(matches!(effects.as_slice(), [Effect::SaveRun { .. }]));
    }

    #[test]
    fn typed_authentication_failures_enter_auth_state_without_disabling_local_solve() {
        let mut state = solve_state();
        state.interviewer.disclosure_accepted = true;
        state.interviewer.status = InterviewerStatus::Connecting;
        let operation = OperationId(91);
        state.interviewer.connecting = Some(operation);
        let error = crate::interviewer::InterviewerError::authentication(
            crate::interviewer::Backend::Pi,
            "fixture authentication required",
        );

        assert!(
            reduce(
                &mut state,
                Event::InterviewerConnected(operation, Err(error)),
            )
            .is_empty()
        );
        assert_eq!(state.interviewer.status, InterviewerStatus::AuthRequired);
        assert!(matches!(
            reduce(&mut state, Event::Command(Action::SaveTest)).as_slice(),
            [Effect::SaveRun {
                intent: RunIntent::Test,
                ..
            }]
        ));
    }

    #[test]
    fn interviewer_output_tail_is_bounded_by_utf8_bytes() {
        let output = format!("discard{}", "界".repeat(16 * 1024));
        let tail = interviewer_output_tail(&output);
        assert!(tail.len() <= 16 * 1024);
        assert!(output.ends_with(&tail));
        assert!(!tail.starts_with("discard"));
        assert!(16 * 1024 - tail.len() < "界".len());
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
