use super::effects::{Action, EditorAction, Effect, Event, LoadScope, RunIntent};
use super::model::{AppState, Focus, MAX_SCROLL, OperationId, Screen, SolvePane};
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

fn solve_command(state: &mut AppState, action: Action) -> Vec<Effect> {
    let Some(solve) = state.solve.as_mut() else {
        return Vec::new();
    };
    match action {
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
                    state.status = "Test queued for newest revision".into();
                } else {
                    state.error = Some("a run is already active".into());
                }
                return Vec::new();
            }
            let operation = next_operation(state);
            let solve = state.solve.as_mut().expect("solve exists");
            let revision = solve.editor.revision;
            solve.running = Some((operation, revision, intent));
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
                source: solve.editor.text().to_string(),
                revision,
                write_source: solve.editor.dirty(),
                intent,
            }]
        }
        Action::Cancel => {
            if let Some((operation, _, _)) = solve.running {
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
            state.screen = Screen::ProblemDetail;
            state.status = "Ready".into();
            let mut effects = effect.into_iter().collect::<Vec<_>>();
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
                    solve.editor.command_char(character);
                    Ok(())
                }
                EditorAction::ExecuteCommand => match solve.editor.execute_command() {
                    Ok(command) => {
                        return solve_command(
                            state,
                            Action::Editor(EditorAction::Command(command)),
                        );
                    }
                    Err(error) => {
                        solve.editor.error = Some(error.clone());
                        state.error = Some(error);
                        Ok(())
                    }
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
                EditorAction::Left => solve.editor.normal('h'),
                EditorAction::Right => solve.editor.normal('l'),
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
                solve.editor.error = Some(error.clone());
                state.error = Some(error);
            } else {
                state.error = None;
            }
            if solve.editor.revision != revision_before {
                solve.stale = true;
            }
            Vec::new()
        }
        Action::Up => {
            match solve.pane {
                SolvePane::Problem => solve.problem_scroll = solve.problem_scroll.saturating_sub(1),
                SolvePane::Output => solve.output_scroll = solve.output_scroll.saturating_sub(1),
                _ => {}
            }
            Vec::new()
        }
        Action::Down => {
            match solve.pane {
                SolvePane::Problem => solve.problem_scroll = solve.problem_scroll.saturating_add(1),
                SolvePane::Output => solve.output_scroll = solve.output_scroll.saturating_add(1),
                _ => {}
            }
            Vec::new()
        }
        Action::Quit
            if solve.editor.mode != Mode::Insert
                && solve.editor.mode != Mode::Command
                && !solve.editor.dirty() =>
        {
            state.quit = true;
            Vec::new()
        }
        Action::Quit if solve.editor.mode != Mode::Insert && solve.editor.mode != Mode::Command => {
            state.error = Some("unsaved changes: use :wq or leave with Esc".into());
            Vec::new()
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
            Event::RunFinished(operation, revision, intent, saved_source, result) => {
                let Some(solve) = state.solve.as_mut() else {
                    return Vec::new();
                };
                if solve.running != Some((operation, revision, intent)) {
                    return Vec::new();
                }
                solve.running = None;
                solve.cancellation = None;
                if let Some(saved_source) = saved_source.as_deref() {
                    solve.editor.mark_saved(revision, saved_source);
                }
                let quit_matches = solve.quit_after_save == Some((Some(operation), revision));
                let succeeded = result.is_ok();
                match result {
                    Ok(result) => {
                        solve.stale = revision != solve.editor.revision;
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
                    if succeeded && saved_source.is_some() {
                        state.quit = true;
                    }
                }
                if intent == RunIntent::Submit && succeeded {
                    solve.refresh_after_submit = true;
                    return load_effect(state);
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
            Event::Command(Action::Quit) => state.quit = true,
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
        Event::Command(Action::Quit) => state.quit = true,
        Event::Command(Action::SaveTest | Action::Submit | Action::Cancel | Action::Editor(_)) => {}
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
            quit_after_save: None,
            refresh_after_submit: false,
        });
        state
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
