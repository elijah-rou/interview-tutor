use super::model::{AppData, OperationId};
use crate::editor::EditorCommand;
use crate::runner::{ExecutionPlan, ExecutionResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadScope {
    Global,
    ProblemSet(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunIntent {
    Test,
    Submit,
}

#[derive(Clone, Debug)]
pub enum Effect {
    Load {
        operation: OperationId,
        scope: LoadScope,
        problem_id: Option<i64>,
        language_slug: String,
    },
    OpenSolve {
        operation: OperationId,
        problem_slug: String,
        set_slug: Option<String>,
        language_slug: String,
    },
    SaveRun {
        operation: OperationId,
        plan: ExecutionPlan,
        source: String,
        revision: u64,
        write_source: bool,
        intent: RunIntent,
    },
    CancelRun {
        operation: OperationId,
    },
    LeaveSolve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorAction {
    Normal(char),
    Insert(char),
    Paste(String),
    CommandChar(char),
    ExecuteCommand,
    Escape,
    Enter,
    Backspace,
    CommandBackspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Redo,
    Command(EditorCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
    SaveTest,
    Submit,
    Cancel,
    Editor(EditorAction),
}

#[derive(Debug)]
pub enum Event {
    Command(Action),
    OpenSet(String),
    Loaded(OperationId, Result<Box<AppData>, String>),
    SolveOpened(OperationId, Result<Box<super::model::SolveSession>, String>),
    RunFinished(
        OperationId,
        u64,
        RunIntent,
        Option<String>,
        Result<ExecutionResult, String>,
    ),
}
