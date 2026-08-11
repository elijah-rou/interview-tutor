use super::effects::{Action, Effect, Event};
use super::model::{AppState, Focus, OperationId, Screen};

fn load_effect(state: &mut AppState) -> Vec<Effect> {
    let Some(language_slug) = state.language_slug().map(str::to_string) else {
        state.error = Some("no enabled languages".to_string());
        return Vec::new();
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
        set_slug: state.selected_set_id.clone(),
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
    match event {
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
            Screen::ProblemDetail => {}
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
            Screen::ProblemDetail => {}
        },
        Event::Command(Action::Open) => match state.screen {
            Screen::SetMenu => {
                if let Some(row) = state.data.sets.get(state.set_index) {
                    state.selected_set_id = Some(row.slug.clone());
                    state.selected_problem_id = None;
                    state.problem_index = 0;
                    state.screen = Screen::ProblemList;
                    return load_effect(state);
                }
            }
            Screen::ProblemList => {
                if let Some(row) = state.data.problems.get(state.problem_index) {
                    state.selected_problem_id = Some(row.id);
                    state.screen = Screen::ProblemDetail;
                    return load_effect(state);
                }
            }
            Screen::ProblemDetail => {}
        },
        Event::OpenSet(slug) => {
            state.selected_set_id = Some(slug);
            state.selected_problem_id = None;
            state.screen = Screen::ProblemList;
            return load_effect(state);
        }
        Event::Command(Action::Back) => {
            state.show_help = false;
            match state.screen {
                Screen::ProblemDetail => state.screen = Screen::ProblemList,
                Screen::ProblemList => {
                    state.screen = Screen::SetMenu;
                    state.selected_problem_id = None;
                }
                Screen::SetMenu => {}
            }
        }
        Event::Command(Action::NextFocus) => state.focus = Focus::Progress,
        Event::Command(Action::PreviousFocus) => state.focus = Focus::Main,
        Event::Command(Action::CycleLanguage) => {
            if !state.languages.is_empty() {
                state.language_index = (state.language_index + 1) % state.languages.len();
                return load_effect(state);
            }
        }
        Event::Command(Action::Reload) => return load_effect(state),
        Event::Command(Action::Help) => state.show_help = !state.show_help,
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

    #[test]
    fn menus_clamp_without_wrapping_and_empty_data_is_safe() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Up));
        assert_eq!(state.set_index, 0);
        for _ in 0..4 {
            reduce(&mut state, Event::Command(Action::Down));
        }
        assert_eq!(state.set_index, 1);
        state.data = AppData::empty();
        reduce(&mut state, Event::Command(Action::Down));
        assert_eq!(state.set_index, 0);
    }

    #[test]
    fn reload_preserves_ids_and_ignores_stale_completion() {
        let mut state = state();
        state.set_index = 1;
        state.selected_set_id = Some("b".into());
        let first = reduce(&mut state, Event::Command(Action::Reload));
        let second = reduce(&mut state, Event::Command(Action::Reload));
        let Effect::Load { operation: old, .. } = first[0].clone();
        reduce(
            &mut state,
            Event::Loaded(old, Ok(Box::new(AppData::empty()))),
        );
        assert_eq!(state.selected_set_id.as_deref(), Some("b"));
        let Effect::Load { operation, .. } = second[0].clone();
        let mut data = AppData::empty();
        data.sets = state.data.sets.iter().cloned().rev().collect();
        reduce(&mut state, Event::Loaded(operation, Ok(Box::new(data))));
        assert_eq!(state.set_index, 0);
        assert_eq!(state.selected_set_id.as_deref(), Some("b"));
    }

    #[test]
    fn open_back_language_and_quit_are_explicit() {
        let mut state = state();
        reduce(&mut state, Event::Command(Action::Open));
        assert_eq!(state.screen, Screen::ProblemList);
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
        assert_eq!(state.screen, Screen::ProblemDetail);
        reduce(&mut state, Event::Command(Action::Back));
        assert_eq!(state.screen, Screen::ProblemList);
        reduce(&mut state, Event::Command(Action::CycleLanguage));
        assert!(state.active_operation.is_some());
        reduce(&mut state, Event::Command(Action::Quit));
        assert!(state.quit);
    }
}
