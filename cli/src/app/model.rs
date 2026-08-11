use super::effects::RunIntent;
use crate::codex::prompt::Mode as CodexMode;
use crate::database::{
    Difficulty, EnabledLanguage, MAX_STATEMENT_LENGTH, ProblemImplementation, ProgressSummary,
};
use crate::editor::{EditorDocument, MAX_DOCUMENT_BYTES};
use crate::runner::{CancellationToken, ExecutionPlan};

pub const MAX_ROWS: usize = 10_000;
pub const MAX_RENDERED_MARKDOWN_CHARS: usize = 100_000;
pub const MAX_SCROLL: u16 = u16::MAX;
pub const MAX_RUN_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_COMPOSER_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexStatus {
    Disabled,
    Offline,
    Disclosure,
    Connecting,
    AuthRequired,
    Ready,
    Thinking,
    Feedback,
    Declined,
    Disconnected,
    ProtocolError,
}

impl CodexStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Offline => "offline",
            Self::Disclosure => "disclosure",
            Self::Connecting => "connecting",
            Self::AuthRequired => "auth",
            Self::Ready => "ready",
            Self::Thinking => "busy",
            Self::Feedback => "feedback",
            Self::Declined => "declined",
            Self::Disconnected => "disconnected",
            Self::ProtocolError => "protocol error",
        }
    }
}

#[derive(Clone)]
pub struct CodexUi {
    pub enabled: bool,
    pub status: CodexStatus,
    pub disclosure_accepted: bool,
    pub composer_focused: bool,
    pub composer: String,
    pub messages: Vec<(String, String)>,
    /// Number of wrapped transcript rows to retain below the visible viewport.
    pub scroll: u16,
    pub connecting: Option<OperationId>,
    pub active: Option<(OperationId, u64, CodexMode)>,
    pub hint_revision: Option<u64>,
    pub hint_count: u8,
    pub submission_recorded: bool,
}

impl Default for CodexUi {
    fn default() -> Self {
        Self {
            enabled: true,
            status: CodexStatus::Offline,
            disclosure_accepted: false,
            composer_focused: false,
            composer: String::new(),
            messages: Vec::new(),
            scroll: 0,
            connecting: None,
            active: None,
            hint_revision: None,
            hint_count: 0,
            submission_recorded: false,
        }
    }
}

impl CodexUi {
    pub fn push_message(&mut self, label: String, message: String) {
        self.messages.push((label, message));
        while self.messages.len() > 128
            || self
                .messages
                .iter()
                .map(|(label, message)| label.len() + message.len())
                .sum::<usize>()
                > 256 * 1024
        {
            self.messages.remove(0);
        }
        assert!(self.messages.len() <= 128);
        self.scroll = 0;
    }

    pub fn clear_session(&mut self) {
        self.composer.clear();
        self.messages.clear();
        self.scroll = 0;
        self.connecting = None;
        self.active = None;
        self.hint_revision = None;
        self.hint_count = 0;
        self.submission_recorded = false;
        self.composer_focused = false;
        self.status = if self.enabled {
            CodexStatus::Offline
        } else {
            CodexStatus::Disabled
        };
    }

    pub fn disable(&mut self) {
        self.clear_session();
        self.disclosure_accepted = false;
        self.enabled = false;
        self.status = CodexStatus::Disabled;
    }
}

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
pub struct SubmittedSource {
    pub operation: OperationId,
    pub revision: u64,
    source: String,
}

impl SubmittedSource {
    pub fn new(operation: OperationId, revision: u64, source: String) -> Self {
        assert!(source.len() <= MAX_DOCUMENT_BYTES);
        Self {
            operation,
            revision,
            source,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

impl Drop for SubmittedSource {
    fn drop(&mut self) {
        // This buffer temporarily duplicates source solely to bind an automatic review to the
        // recorded run. Volatile writes keep its explicit cleanup from being optimized away.
        unsafe {
            for byte in self.source.as_mut_vec() {
                std::ptr::write_volatile(byte, 0);
            }
        }
        self.source.clear();
    }
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
    pub submitted_source: Option<SubmittedSource>,
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
    pub codex: CodexUi,
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
            codex: CodexUi::default(),
            quit: false,
        }
    }

    pub fn language_slug(&self) -> Option<&str> {
        self.languages
            .get(self.language_index)
            .map(|item| item.slug.as_str())
    }

    pub fn disable_codex(&mut self) {
        self.codex.disable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ExecutionPlan;
    use std::path::PathBuf;

    #[test]
    fn disabling_codex_clears_session_state_and_is_sticky_across_solve_sessions() {
        let mut state = AppState::new(Vec::new(), 0);
        state.codex.status = CodexStatus::Ready;
        state.codex.disclosure_accepted = true;
        state.codex.composer_focused = true;
        state.codex.composer = "private question".into();
        state
            .codex
            .push_message("Interviewer".into(), "private response".into());

        state.disable_codex();
        assert!(!state.codex.enabled);
        assert_eq!(state.codex.status, CodexStatus::Disabled);
        assert!(!state.codex.disclosure_accepted);
        assert!(!state.codex.composer_focused);
        assert!(state.codex.composer.is_empty());
        assert!(state.codex.messages.is_empty());

        state.codex.clear_session();
        assert_eq!(state.codex.status, CodexStatus::Disabled);
    }

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
            submitted_source: None,
        };
        solve.bounded_output("界".repeat(MAX_RUN_OUTPUT_BYTES));
        assert!(solve.output.starts_with("… output truncated …\n"));
        assert!(solve.output.len() <= MAX_RUN_OUTPUT_BYTES);
        assert!(MAX_RUN_OUTPUT_BYTES - solve.output.len() < "界".len());
    }
}
