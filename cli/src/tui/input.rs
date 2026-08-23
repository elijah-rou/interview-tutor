use crate::app::{Action, AppState, EditorAction, Screen};
use crate::editor::Mode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn routes_to_neovim(key: KeyEvent, state: &AppState) -> bool {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    let Some(solve) = state.solve.as_ref() else {
        return false;
    };
    if state.screen != Screen::Solve
        || solve.pane != crate::app::model::SolvePane::Editor
        || solve.editor_status == crate::app::model::EditorRuntimeStatus::Failed
        || state.show_help
    {
        return false;
    }
    if matches!(key.code, KeyCode::Char('c' | 'q')) && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return false;
    }
    if matches!(key.code, KeyCode::F(5 | 9))
        || key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return false;
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        // Neovim reports Insert, Replace, and terminal modes as insert-like so
        // their Tab mappings remain authoritative.
        return solve.editor.mode == Mode::Insert;
    }
    true
}

pub fn action_for_key(key: KeyEvent, state: &mut AppState) -> Option<Action> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if state.screen != Screen::Solve {
        return browser_key(key);
    }
    let solve = state.solve.as_ref()?;
    let mode = solve.editor.mode;
    let pane = solve.pane;
    let editor_failed = solve.editor_status == crate::app::model::EditorRuntimeStatus::Failed;

    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.leader_pending = false;
        return Some(Action::Quit);
    }
    if state.show_help {
        state.leader_pending = false;
        return match key.code {
            KeyCode::Esc => Some(Action::Back),
            KeyCode::Char('?') => Some(Action::Help),
            _ => None,
        };
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.leader_pending = false;
        return Some(Action::Cancel);
    }
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL)
        || key.code == KeyCode::F(5)
    {
        state.leader_pending = false;
        return Some(Action::SaveTest);
    }
    if key.code == KeyCode::F(9) {
        state.leader_pending = false;
        return Some(Action::Submit);
    }
    let tab_targets_neovim =
        pane == crate::app::model::SolvePane::Editor && mode == Mode::Insert && !editor_failed;
    if !tab_targets_neovim {
        if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT)
            || key.code == KeyCode::BackTab
        {
            state.leader_pending = false;
            return Some(Action::PreviousFocus);
        }
        if key.code == KeyCode::Tab {
            state.leader_pending = false;
            return Some(Action::NextFocus);
        }
    }

    if pane == crate::app::model::SolvePane::Interview && state.interviewer.composer_focused {
        state.leader_pending = false;
        return match key.code {
            KeyCode::Esc => Some(Action::InterviewEscape),
            KeyCode::Enter => Some(Action::InterviewSend),
            KeyCode::Backspace => Some(Action::InterviewBackspace),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(Action::InterviewChar(character))
            }
            _ => None,
        };
    }

    let host_leader_eligible =
        pane != crate::app::model::SolvePane::Editor || editor_failed && mode == Mode::Normal;
    if host_leader_eligible && state.leader_pending {
        state.leader_pending = false;
        let leader_action = match key.code {
            KeyCode::Char('t') => Some(Action::SaveTest),
            KeyCode::Char('s') => Some(Action::Submit),
            KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char('c') => Some(Action::ToggleCollapse),
            KeyCode::Char('h') => Some(Action::Hint),
            KeyCode::Char('?') => Some(Action::Help),
            KeyCode::Char('r') if pane == crate::app::model::SolvePane::Interview => {
                Some(Action::ResetInterview)
            }
            KeyCode::Char('q') if pane == crate::app::model::SolvePane::Editor => {
                Some(Action::Quit)
            }
            _ => None,
        };
        if leader_action.is_some() {
            return leader_action;
        }
    }
    if host_leader_eligible && key.code == KeyCode::Char(' ') {
        state.leader_pending = true;
        return None;
    }

    if pane == crate::app::model::SolvePane::Interview
        && state.interviewer.status == crate::app::model::InterviewerStatus::Disclosure
    {
        return match key.code {
            KeyCode::Enter | KeyCode::Char('y') => Some(Action::InterviewDisclosure(true)),
            KeyCode::Esc | KeyCode::Char('n') => Some(Action::InterviewDisclosure(false)),
            _ => None,
        };
    }

    if pane != crate::app::model::SolvePane::Editor {
        if key.code == KeyCode::Char('i') {
            return Some(Action::InterviewFocus);
        }
        return match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            _ => None,
        };
    }
    if editor_failed {
        return None;
    }

    match mode {
        Mode::Insert => match key.code {
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            KeyCode::Left => Some(Action::Editor(EditorAction::Left)),
            KeyCode::Right => Some(Action::Editor(EditorAction::Right)),
            KeyCode::Up => Some(Action::Editor(EditorAction::Up)),
            KeyCode::Down => Some(Action::Editor(EditorAction::Down)),
            KeyCode::Backspace => Some(Action::Editor(EditorAction::Backspace)),
            KeyCode::Delete => Some(Action::Editor(EditorAction::Delete)),
            KeyCode::Enter => Some(Action::Editor(EditorAction::Enter)),
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(Action::Editor(EditorAction::Insert(c)))
            }
            _ => None,
        },
        Mode::Command => match key.code {
            KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
            KeyCode::Enter => Some(Action::Editor(EditorAction::ExecuteCommand)),
            KeyCode::Backspace => Some(Action::Editor(EditorAction::CommandBackspace)),
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(Action::Editor(EditorAction::CommandChar(c)))
            }
            _ => None,
        },
        Mode::Normal | Mode::Visual => {
            if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Some(Action::Editor(EditorAction::Redo));
            }
            match key.code {
                KeyCode::Esc => Some(Action::Editor(EditorAction::Escape)),
                KeyCode::Up => Some(Action::Editor(EditorAction::Up)),
                KeyCode::Down => Some(Action::Editor(EditorAction::Down)),
                KeyCode::Left => Some(Action::Editor(EditorAction::Left)),
                KeyCode::Right => Some(Action::Editor(EditorAction::Right)),
                KeyCode::Char(c) => Some(Action::Editor(EditorAction::Normal(c))),
                _ => None,
            }
        }
    }
}
fn browser_key(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => Some(Action::Quit),
        (KeyCode::Char('?'), _) => Some(Action::Help),
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => Some(Action::Down),
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => Some(Action::Up),
        (KeyCode::Enter, _) => Some(Action::Open),
        (KeyCode::Esc | KeyCode::Backspace, _) => Some(Action::Back),
        (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => Some(Action::PreviousFocus),
        (KeyCode::Tab, _) => Some(Action::NextFocus),
        (KeyCode::Char('l'), _) => Some(Action::CycleLanguage),
        (KeyCode::Char('r'), _) => Some(Action::Reload),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::{SolvePane, SolveSession};
    use crate::database::EnabledLanguage;
    use crate::editor::EditorDocument;
    use crate::runner::ExecutionPlan;
    use std::path::PathBuf;

    fn solve_state() -> AppState {
        let mut state = AppState::new(Vec::new(), 0);
        state.screen = Screen::Solve;
        state.solve = Some(SolveSession {
            generation: crate::neovim::SessionGeneration(1),
            problem_id: 1,
            problem_slug: "p".into(),
            problem_title: "P".into(),
            statement: String::new(),
            language: "python".into(),
            plan: ExecutionPlan {
                root: PathBuf::from("/tmp"),
                language: "python".into(),
                problem_slug: "p".into(),
                set_slug: None,
                runner_path: PathBuf::from("/tmp/run"),
                solution_path: PathBuf::from("/tmp/p.py"),
            },
            editor: EditorDocument::new("x".into()).unwrap(),
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
    fn focused_editor_routes_full_neovim_surface_except_outer_app_keys() {
        let mut state = solve_state();
        for key in [
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        ] {
            assert!(routes_to_neovim(key, &state));
        }
        for key in [
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        ] {
            assert!(!routes_to_neovim(key, &state));
        }
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert!(!routes_to_neovim(
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
            &state
        ));
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        state.solve.as_mut().unwrap().editor_status =
            crate::app::model::EditorRuntimeStatus::Failed;
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
    }

    #[test]
    fn insert_mode_tab_reaches_neovim_while_other_modes_cycle_panes() {
        let mut state = solve_state();
        assert_eq!(state.solve.as_ref().unwrap().pane, SolvePane::Editor);
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                &mut state
            ),
            Some(Action::PreviousFocus)
        );
        for mode in ["i", "R", "t"] {
            state
                .solve
                .as_mut()
                .unwrap()
                .editor
                .update_neovim_mode(mode);
            for key in [
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            ] {
                assert!(routes_to_neovim(key, &state), "mode {mode}");
                assert_eq!(action_for_key(key, &mut state), None, "mode {mode}");
            }
        }
        state.solve.as_mut().unwrap().editor.update_neovim_mode("c");
        assert!(!routes_to_neovim(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &state
        ));
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
        state.solve.as_mut().unwrap().editor.update_neovim_mode("i");
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        assert!(!routes_to_neovim(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &state
        ));
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
    }

    #[test]
    fn solve_run_leader_and_pane_keys_are_mapped() {
        let mut state = solve_state();
        for (key, expected) in [
            (
                KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
                Action::SaveTest,
            ),
            (
                KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
                Action::Submit,
            ),
            (
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                Action::SaveTest,
            ),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Action::Cancel,
            ),
            (
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                Action::NextFocus,
            ),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                Action::PreviousFocus,
            ),
        ] {
            assert_eq!(action_for_key(key, &mut state), Some(expected));
        }
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Editor(EditorAction::Normal(' ')))
        );
        state.solve.as_mut().unwrap().pane = SolvePane::Output;
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::SaveTest)
        );
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state),
            Some(Action::Down)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Down)
        );
    }

    #[test]
    fn tab_and_backtab_precede_interview_disclosure_and_composer() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.status = crate::app::model::InterviewerStatus::Disclosure;
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                &mut state
            ),
            Some(Action::PreviousFocus)
        );

        state.interviewer.status = crate::app::model::InterviewerStatus::Ready;
        state.interviewer.composer_focused = true;
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut state),
            Some(Action::NextFocus)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                &mut state
            ),
            Some(Action::PreviousFocus)
        );
    }

    #[test]
    fn host_run_aliases_never_enter_neovim_modes() {
        let mut state = solve_state();
        for mode in ["n", "i", "v", "c", "no", "/"] {
            state
                .solve
                .as_mut()
                .unwrap()
                .editor
                .update_neovim_mode(mode);
            for (key, expected) in [
                (
                    KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
                    Action::SaveTest,
                ),
                (
                    KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
                    Action::Submit,
                ),
                (
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                    Action::SaveTest,
                ),
            ] {
                assert!(
                    !routes_to_neovim(key, &state),
                    "alias routed in mode {mode}"
                );
                assert_eq!(action_for_key(key, &mut state), Some(expected));
            }
        }
    }

    #[test]
    fn solve_i_hint_and_cancel_routing_is_contextual() {
        let mut state = solve_state();
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Editor(EditorAction::Normal('i')))
        );

        for pane in [SolvePane::Problem, SolvePane::Output, SolvePane::Interview] {
            state.solve.as_mut().unwrap().pane = pane;
            assert_eq!(
                action_for_key(
                    KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                    &mut state
                ),
                Some(Action::InterviewFocus)
            );
            state.interviewer.composer_focused = false;
            assert_eq!(
                action_for_key(
                    KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                    &mut state
                ),
                None
            );
            assert_eq!(
                action_for_key(
                    KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                    &mut state
                ),
                Some(Action::Hint)
            );
        }

        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        state.solve.as_mut().unwrap().editor.normal('i').unwrap();
        assert_eq!(state.solve.as_ref().unwrap().editor.mode, Mode::Insert);
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Editor(EditorAction::Insert(' ')))
        );
    }

    #[test]
    fn global_solve_quit_reaches_every_pane_and_editor_mode() {
        let mut state = solve_state();
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
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
                assert!(!routes_to_neovim(quit, &state), "pane {pane:?} mode {mode}");
                assert_eq!(
                    action_for_key(quit, &mut state),
                    Some(Action::Quit),
                    "pane {pane:?} mode {mode}"
                );
            }
        }
    }

    #[test]
    fn solve_help_uses_accessory_leader_and_modal_keys_precede_solve_routing() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Problem;
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Help)
        );

        state.show_help = true;
        state.solve.as_mut().unwrap().pane = SolvePane::Editor;
        assert!(!routes_to_neovim(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &state
        ));
        assert_eq!(
            action_for_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut state),
            Some(Action::Back)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut state
            ),
            None,
            "ordinary solve commands must not pass through the help modal"
        );
    }

    #[test]
    fn accessory_leader_routes_all_milestone_actions_and_unknown_keys_continue() {
        let mut state = solve_state();
        for pane in [SolvePane::Problem, SolvePane::Output, SolvePane::Interview] {
            state.solve.as_mut().unwrap().pane = pane;
            state.interviewer.composer_focused = false;
            state.interviewer.status = crate::app::model::InterviewerStatus::Ready;
            for (key, expected) in [
                ('t', Action::SaveTest),
                ('s', Action::Submit),
                ('b', Action::Back),
                ('c', Action::ToggleCollapse),
            ] {
                assert_eq!(
                    action_for_key(
                        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                        &mut state
                    ),
                    None
                );
                assert_eq!(
                    action_for_key(
                        KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                        &mut state
                    ),
                    Some(expected.clone()),
                    "pane {pane:?} key {key}"
                );
            }

            assert_eq!(
                action_for_key(
                    KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                    &mut state
                ),
                None
            );
            assert_eq!(
                action_for_key(
                    KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                    &mut state
                ),
                Some(Action::InterviewFocus),
                "unknown leader key must continue normal accessory routing"
            );
        }
    }

    #[test]
    fn leader_precedence_respects_composer_and_disclosure() {
        let mut state = solve_state();
        state.solve.as_mut().unwrap().pane = SolvePane::Interview;
        state.interviewer.status = crate::app::model::InterviewerStatus::Ready;
        state.interviewer.composer_focused = true;
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::InterviewChar(' '))
        );
        assert!(!state.leader_pending);

        state.interviewer.composer_focused = false;
        state.interviewer.status = crate::app::model::InterviewerStatus::Disclosure;
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::ToggleCollapse)
        );
        assert_eq!(
            state.interviewer.status,
            crate::app::model::InterviewerStatus::Disclosure
        );

        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut state
            ),
            None
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::InterviewDisclosure(true)),
            "unknown leader key must continue into disclosure routing"
        );
    }

    #[test]
    fn documented_keys_are_mapped() {
        let mut state = AppState::new(
            vec![EnabledLanguage {
                slug: "python".into(),
                display_name: "Python".into(),
                runner_path: "python/run".into(),
            }],
            0,
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &mut state
            ),
            Some(Action::Quit)
        );
        assert_eq!(
            action_for_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                &mut state
            ),
            Some(Action::PreviousFocus)
        );
    }
}
