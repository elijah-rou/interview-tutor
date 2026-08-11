use super::effects::RunIntent;
use crate::database::{
    Difficulty, EnabledLanguage, MAX_STATEMENT_LENGTH, ProblemImplementation, ProgressSummary,
};
use crate::editor::EditorDocument;
use crate::runner::{CancellationToken, ExecutionPlan};

pub const MAX_ROWS: usize = 10_000;
pub const MAX_RENDERED_MARKDOWN_CHARS: usize = 100_000;
pub const MAX_SCROLL: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    SetMenu,
    ProblemList,
    ProblemDetail,
    Solve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolvePane {
    Editor,
    Problem,
    Output,
    Interview,
}

#[derive(Clone, Debug)]
pub struct SolveSession {
    pub problem_id: i64,
    pub problem_slug: String,
    pub problem_title: String,
    pub statement: String,
    pub language: String,
    pub plan: ExecutionPlan,
    pub editor: EditorDocument,
    pub pane: SolvePane,
    pub output: String,
    pub output_scroll: u16,
    pub problem_scroll: u16,
    pub running: Option<(OperationId, u64, RunIntent)>,
    pub cancellation: Option<CancellationToken>,
    pub pending_save: Option<(u64, String)>,
    pub stale: bool,
    pub quit_after_save: Option<(Option<OperationId>, u64)>,
    pub refresh_after_submit: bool,
}

impl SolveSession {
    pub fn bounded_output(&mut self, output: String) {
        const MAX_OUTPUT: usize = 256 * 1024;
        self.output = if output.len() <= MAX_OUTPUT {
            output
        } else {
            let mut start = output.len() - MAX_OUTPUT;
            while !output.is_char_boundary(start) {
                start += 1;
            }
            format!("… output truncated …\n{}", &output[start..])
        };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Main,
    Progress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetRow {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub member_count: usize,
    pub completed_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProblemRow {
    pub id: i64,
    pub ordinal: Option<i64>,
    pub slug: String,
    pub title: String,
    pub difficulty: Difficulty,
    pub topic: String,
    pub completed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProblemDetail {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub difficulty: Difficulty,
    pub topic: String,
    pub statement_markdown: String,
    pub implementations: Vec<ProblemImplementation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppData {
    pub sets: Vec<SetRow>,
    pub problems: Vec<ProblemRow>,
    pub detail: Option<ProblemDetail>,
    pub progress: ProgressSummary,
}

impl AppData {
    pub fn empty() -> Self {
        Self {
            sets: Vec::new(),
            problems: Vec::new(),
            detail: None,
            progress: ProgressSummary {
                completed: 0,
                total: 0,
                by_difficulty: Vec::new(),
                by_topic: Vec::new(),
            },
        }
    }

    pub fn assert_bounded(&self) {
        assert!(self.sets.len() <= MAX_ROWS);
        assert!(self.problems.len() <= MAX_ROWS);
        assert!(self.progress.by_topic.len() <= MAX_ROWS);
        assert!(self.progress.by_difficulty.len() <= 3);
        if let Some(detail) = &self.detail {
            assert!(detail.statement_markdown.chars().count() <= MAX_STATEMENT_LENGTH);
            assert!(detail.implementations.len() <= MAX_ROWS);
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub screen: Screen,
    pub focus: Focus,
    pub languages: Vec<EnabledLanguage>,
    pub language_index: usize,
    pub selected_set_id: Option<String>,
    pub selected_problem_id: Option<i64>,
    pub set_index: usize,
    pub problem_index: usize,
    pub detail_scroll: u16,
    pub progress_scroll: u16,
    pub data: AppData,
    pub solve: Option<SolveSession>,
    pub active_operation: Option<OperationId>,
    pub next_operation: u64,
    pub status: String,
    pub error: Option<String>,
    pub show_help: bool,
    pub leader_pending: bool,
    pub quit: bool,
}

impl AppState {
    pub fn new(languages: Vec<EnabledLanguage>, language_index: usize) -> Self {
        assert!(languages.len() <= MAX_ROWS);
        assert!(languages.is_empty() || language_index < languages.len());
        Self {
            screen: Screen::SetMenu,
            focus: Focus::Main,
            languages,
            language_index,
            selected_set_id: None,
            selected_problem_id: None,
            set_index: 0,
            problem_index: 0,
            detail_scroll: 0,
            progress_scroll: 0,
            data: AppData::empty(),
            solve: None,
            active_operation: None,
            next_operation: 1,
            status: "Ready".to_string(),
            error: None,
            show_help: false,
            leader_pending: false,
            quit: false,
        }
    }

    pub fn language_slug(&self) -> Option<&str> {
        self.languages
            .get(self.language_index)
            .map(|item| item.slug.as_str())
    }
}
