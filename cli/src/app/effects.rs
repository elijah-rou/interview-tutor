use super::model::{AppData, OperationId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadScope {
    Global,
    ProblemSet(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Load {
        operation: OperationId,
        scope: LoadScope,
        problem_id: Option<i64>,
        language_slug: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Up,
    Down,
    Open,
    Back,
    NextFocus,
    PreviousFocus,
    CycleLanguage,
    Reload,
    Help,
    Quit,
}

#[derive(Debug)]
pub enum Event {
    Command(Action),
    OpenSet(String),
    Loaded(OperationId, Result<Box<AppData>, String>),
}
