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

#[derive(Clone)]
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
    StartNeovim {
        generation: crate::neovim::SessionGeneration,
        source: String,
        synthetic_name: String,
        language: String,
    },
    SaveRun {
        operation: OperationId,
        plan: ExecutionPlan,
        source: String,
        revision: u64,
        write_source: bool,
        intent: RunIntent,
    },
    SaveDraft {
        operation: OperationId,
        plan: ExecutionPlan,
        source: String,
        revision: u64,
    },
    CancelRun {
        operation: OperationId,
    },
    ConnectInterviewer {
        operation: OperationId,
    },
    InterviewerTurn {
        operation: OperationId,
        revision: u64,
        mode: crate::interviewer::Mode,
        statement: String,
        source: String,
        output: String,
        question: String,
        solved: bool,
    },
    FinalizeInterviewerTurn {
        operation: OperationId,
        revision: u64,
        mode: crate::interviewer::Mode,
        accepted: bool,
    },
    CancelInterviewer {
        operation: OperationId,
    },
    ResetInterviewer,
    LeaveSolve,
    StopNeovim {
        generation: crate::neovim::SessionGeneration,
    },
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
    InterviewFocus,
    InterviewChar(char),
    InterviewBackspace,
    InterviewSend,
    InterviewEscape,
    InterviewDisclosure(bool),
    Hint,
    ResetInterview,
    ToggleCollapse,
    EditorCollapse,
    Editor(EditorAction),
}

pub enum Event {
    Command(Action),
    OpenSet(String),
    Loaded(OperationId, Result<Box<AppData>, String>),
    SolveOpened(OperationId, Result<Box<super::model::SolveSession>, String>),
    NeovimStarted(Result<(), String>),
    NeovimDocument(crate::neovim::DocumentUpdate),
    NeovimView(std::sync::Arc<crate::neovim::grid::GridSnapshot>),
    NeovimWarning(String),
    NeovimFailed(String),
    RunFinished(
        OperationId,
        u64,
        RunIntent,
        Option<String>,
        Result<ExecutionResult, String>,
    ),
    DraftSaved(OperationId, u64, String, Result<(), String>),
    InterviewerConnected(
        OperationId,
        Result<(), crate::interviewer::InterviewerError>,
    ),
    InterviewerFinished(
        OperationId,
        u64,
        crate::interviewer::Mode,
        Result<String, crate::interviewer::InterviewerError>,
    ),
    InterviewerDisconnected(String),
}
