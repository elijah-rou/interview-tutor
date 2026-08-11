use super::effects::RunIntent;
use crate::database::{
    Difficulty, EnabledLanguage, MAX_STATEMENT_LENGTH, ProblemImplementation, ProgressSummary,
};
use crate::editor::EditorDocument;
use crate::runner::{CancellationToken, ExecutionPlan};

pub const MAX_ROWS: usize = 10_000;
pub const MAX_RENDERED_MARKDOWN_CHARS: usize = 100_000;
pub const MAX_SCROLL: u16 = u16::MAX;
pub const MAX_RUN_OUTPUT_BYTES: usize = 256 * 1024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscardAction {
    Back,
    Quit,
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
    pub latest_run_revision: Option<u64>,
    pub quit_after_save: Option<(Option<OperationId>, u64)>,
    pub discard_confirmation: Option<DiscardAction>,
    pub refresh_after_submit: bool,
}

impl SolveSession {
    pub fn bounded_output(&mut self, output: String) {
        const MARKER: &str = "… output truncated …\n";
        self.output = if output.len() <= MAX_RUN_OUTPUT_BYTES {
            output
        } else {
            let retained_bytes = MAX_RUN_OUTPUT_BYTES - MARKER.len();
            let mut start = output.len() - retained_bytes;
            while !output.is_char_boundary(start) {
                start += 1;
            }
            format!("{MARKER}{}", &output[start..])
        };
        assert!(self.output.len() <= MAX_RUN_OUTPUT_BYTES);
    }

    pub fn output_scroll_max(&self) -> u16 {
        u16::try_from(self.output.lines().count().saturating_sub(1)).unwrap_or(u16::MAX)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ExecutionPlan;
    use std::path::PathBuf;

    #[test]
    fn run_output_bound_includes_utf8_truncation_marker() {
        let mut solve = SolveSession {
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
            editor: EditorDocument::new(String::new()).unwrap(),
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
        };
        solve.bounded_output("界".repeat(MAX_RUN_OUTPUT_BYTES));
        assert!(solve.output.starts_with("… output truncated …\n"));
        assert!(solve.output.len() <= MAX_RUN_OUTPUT_BYTES);
        assert!(MAX_RUN_OUTPUT_BYTES - solve.output.len() < "界".len());
    }
}
