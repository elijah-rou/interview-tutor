use super::effects::{Action, Effect, Event, LoadScope};
use super::model::{AppState, Focus, MAX_SCROLL, OperationId, Screen};

fn load_effect(state: &mut AppState) -> Vec<Effect> {
    let Some(language_slug) = state.language_slug().map(str::to_string) else {
        state.error = Some("no enabled languages".to_string());
        return Vec::new();
    };
    let scope = match state.screen {
        Screen::SetMenu => LoadScope::Global,
        Screen::ProblemList | Screen::ProblemDetail => match &state.selected_set_id {
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
        problem_id: (state.screen == Screen::ProblemDetail)
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

pub fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    if state.show_help {
        if matches!(event, Event::Command(Action::Back | Action::Help)) {
            state.show_help = false;
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
            Screen::ProblemDetail => {}
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
        let Effect::Load { scope, .. } = effects.into_iter().next().unwrap();
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
    fn help_is_modal() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Help));
        reduce(&mut state, Event::Command(Action::Down));
        reduce(&mut state, Event::Command(Action::Quit));
        assert_eq!(state.set_index, 0);
        assert!(!state.quit);
        reduce(&mut state, Event::Command(Action::Back));
        assert!(!state.show_help);
    }

    #[test]
    fn stale_completion_is_ignored() {
        let mut state = state();
        let first = reduce(&mut state, Event::Command(Action::Reload));
        let second = reduce(&mut state, Event::Command(Action::Reload));
        let Effect::Load { operation: old, .. } = first[0].clone();
        reduce(
            &mut state,
            Event::Loaded(old, Ok(Box::new(AppData::empty()))),
        );
        assert_eq!(state.selected_set_id.as_deref(), Some("a"));
        let Effect::Load { operation, .. } = second[0].clone();
        reduce(
            &mut state,
            Event::Loaded(operation, Ok(Box::new(AppData::empty()))),
        );
        assert!(state.active_operation.is_none());
    }
}
