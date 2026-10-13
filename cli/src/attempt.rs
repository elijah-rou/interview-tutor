//! Per-case attempt evidence: what one execution ran, what each step printed, and how the
//! attempt outcome folds from its steps. Pure types and rendering; no process or SQL access.

use crate::database::AttemptOutcome;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::num::NonZeroU64;

/// Most cases one problem may declare through `RUNNER --cases SLUG`.
pub const MAX_CASES: usize = 15;
/// Stored sanitized bytes per stream per step. Truncated text keeps half as head, half as tail.
pub const MAX_STORED_STREAM_BYTES: usize = 8 * 1024;
pub const MAX_CASE_NAME_BYTES: usize = 64;
/// Upper bound on `render_record` output. Indentation can multiply short lines, so this is
/// derived from the worst case (every stored byte a newline) rather than the stored size.
pub const MAX_RENDERED_ATTEMPT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Termination {
    Exited(i32),
    Cancelled,
    TimedOut,
    Signalled(i32),
}

impl Termination {
    pub fn outcome(&self) -> AttemptOutcome {
        match self {
            Self::Exited(0) => AttemptOutcome::Pass,
            Self::Exited(2) => AttemptOutcome::Error,
            Self::Exited(_) => AttemptOutcome::Fail,
            Self::Cancelled => AttemptOutcome::Cancelled,
            Self::TimedOut | Self::Signalled(_) => AttemptOutcome::Error,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Exited(code) => Some(*code),
            Self::Cancelled | Self::TimedOut | Self::Signalled(_) => None,
        }
    }

    pub fn status_code(&self) -> i32 {
        match self {
            Self::Exited(code) => *code,
            Self::Cancelled => 130,
            Self::TimedOut | Self::Signalled(_) => 2,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Exited(code) => format!("exit {code}"),
            Self::Cancelled => "cancelled".to_string(),
            Self::TimedOut => "timed out".to_string(),
            Self::Signalled(signal) => format!("signal {signal}"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CaseName(String);

impl CaseName {
    pub fn parse(value: &str) -> Result<Self, String> {
        let valid = (1..=MAX_CASE_NAME_BYTES).contains(&value.len())
            && value
                .bytes()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err(format!(
                "invalid case name {value:?}: expected 1 to {MAX_CASE_NAME_BYTES} bytes of [a-z0-9-] starting with [a-z0-9]"
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Sanitized UTF-8 retained from one stream. Truncation is a variant, never a marker inside
/// the text, so output that happens to look like an omission marker cannot be mistaken for one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapturedText {
    Complete(String),
    Truncated {
        head: String,
        omitted_bytes: NonZeroU64,
        tail: String,
    },
}

impl CapturedText {
    pub fn complete(text: String) -> Result<Self, String> {
        if text.len() > MAX_STORED_STREAM_BYTES {
            return Err(format!(
                "captured text exceeds {MAX_STORED_STREAM_BYTES} bytes"
            ));
        }
        Ok(Self::Complete(text))
    }

    pub fn truncated(head: String, omitted_bytes: u64, tail: String) -> Result<Self, String> {
        let half = MAX_STORED_STREAM_BYTES / 2;
        if head.len() > half || tail.len() > half {
            return Err(format!(
                "truncated captured text halves must not exceed {half} bytes"
            ));
        }
        let omitted_bytes = NonZeroU64::new(omitted_bytes)
            .ok_or_else(|| "truncated captured text must omit at least one byte".to_string())?;
        Ok(Self::Truncated {
            head,
            omitted_bytes,
            tail,
        })
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Complete(text) if text.is_empty())
    }

    /// Retained text with a single omission line between head and tail when truncated.
    pub fn render(&self) -> String {
        match self {
            Self::Complete(text) => text.clone(),
            Self::Truncated {
                head,
                omitted_bytes,
                tail,
            } => {
                let separator = if head.is_empty() || head.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                format!("{head}{separator}[... {omitted_bytes} bytes omitted ...]\n{tail}")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepRun {
    pub termination: Termination,
    pub duration_ms: i64,
    pub stdout: CapturedText,
    pub stderr: CapturedText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotRunReason {
    Cancelled,
    DeadlineExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaseRun {
    Ran(StepRun),
    NotRun(NotRunReason),
}

impl CaseRun {
    pub fn outcome(&self) -> AttemptOutcome {
        match self {
            Self::Ran(step) => step.termination.outcome(),
            Self::NotRun(NotRunReason::Cancelled) => AttemptOutcome::Cancelled,
            Self::NotRun(NotRunReason::DeadlineExceeded) => AttemptOutcome::Error,
        }
    }

    fn status_code(&self) -> i32 {
        match self {
            Self::Ran(step) => step.termination.status_code(),
            Self::NotRun(NotRunReason::Cancelled) => 130,
            Self::NotRun(NotRunReason::DeadlineExceeded) => 2,
        }
    }

    fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Ran(step) => step.termination.exit_code(),
            Self::NotRun(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseEntry {
    pub name: CaseName,
    pub run: CaseRun,
}

/// Everything one execution ran: the setup step that listed cases, then each case in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestOutput {
    setup: StepRun,
    cases: Vec<CaseEntry>,
}

impl TestOutput {
    pub fn new(setup: StepRun, cases: Vec<CaseEntry>) -> Result<Self, String> {
        if cases.len() > MAX_CASES {
            return Err(format!("attempt has more than {MAX_CASES} cases"));
        }
        let setup_passed = setup.termination.outcome() == AttemptOutcome::Pass;
        if setup_passed && cases.is_empty() {
            return Err("a passing setup step must list at least one case".to_string());
        }
        if !setup_passed && !cases.is_empty() {
            return Err("cases cannot run after a failed setup step".to_string());
        }
        let mut names = HashSet::with_capacity(cases.len());
        if !cases.iter().all(|case| names.insert(&case.name)) {
            return Err("attempt case names must be unique".to_string());
        }
        let first_not_run = cases
            .iter()
            .position(|case| matches!(case.run, CaseRun::NotRun(_)))
            .unwrap_or(cases.len());
        if cases[first_not_run..]
            .iter()
            .any(|case| matches!(case.run, CaseRun::Ran(_)))
        {
            return Err("cases that did not run must follow every case that ran".to_string());
        }
        Ok(Self { setup, cases })
    }

    pub fn setup(&self) -> &StepRun {
        &self.setup
    }

    pub fn cases(&self) -> &[CaseEntry] {
        &self.cases
    }

    pub fn passed_cases(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.run.outcome() == AttemptOutcome::Pass)
            .count()
    }

    pub fn outcome(&self) -> AttemptOutcome {
        self.deciding_step()
            .map_or(AttemptOutcome::Pass, |(outcome, _, _)| outcome)
    }

    /// The first step whose outcome is the folded outcome, with its status and exit codes.
    fn deciding_step(&self) -> Option<(AttemptOutcome, i32, Option<i32>)> {
        let steps = std::iter::once((
            self.setup.termination.outcome(),
            self.setup.termination.status_code(),
            self.setup.termination.exit_code(),
        ))
        .chain(self.cases.iter().map(|case| {
            (
                case.run.outcome(),
                case.run.status_code(),
                case.run.exit_code(),
            )
        }));
        let mut decided: Option<(AttemptOutcome, i32, Option<i32>)> = None;
        for step in steps {
            if step.0 == AttemptOutcome::Pass {
                continue;
            }
            if decided.is_none_or(|current| severity(step.0) > severity(current.0)) {
                decided = Some(step);
            }
        }
        decided
    }
}

fn severity(outcome: AttemptOutcome) -> u8 {
    match outcome {
        AttemptOutcome::Pass => 0,
        AttemptOutcome::Fail => 1,
        AttemptOutcome::Error => 2,
        AttemptOutcome::Cancelled => 3,
    }
}

/// The in-memory result of one execution, before and after it is recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptReport {
    pub output: TestOutput,
    pub duration_ms: i64,
    /// Progress events the caller's channel could not accept. Never persisted.
    pub dropped_events: usize,
    cancelled: bool,
}

impl AttemptReport {
    pub fn new(output: TestOutput, duration_ms: i64, dropped_events: usize) -> Self {
        assert!(duration_ms >= 0);
        Self {
            output,
            duration_ms,
            dropped_events,
            cancelled: false,
        }
    }

    /// Records that cancellation was observed after the last step finished.
    pub fn mark_cancelled(&mut self) {
        self.cancelled = true;
    }

    pub fn outcome(&self) -> AttemptOutcome {
        if self.cancelled {
            AttemptOutcome::Cancelled
        } else {
            self.output.outcome()
        }
    }

    pub fn status_code(&self) -> i32 {
        if self.cancelled {
            return 130;
        }
        self.output
            .deciding_step()
            .map_or(0, |(_, status_code, _)| status_code)
    }

    pub fn exit_code(&self) -> Option<i32> {
        if self.cancelled {
            return None;
        }
        self.output
            .deciding_step()
            .map_or(Some(0), |(_, _, exit_code)| exit_code)
    }

    #[cfg(test)]
    pub(crate) fn test_report(termination: Termination, stdout: &str) -> Self {
        let setup = StepRun {
            termination: Termination::Exited(0),
            duration_ms: 0,
            stdout: CapturedText::complete("example-1\n".to_string()).unwrap(),
            stderr: CapturedText::complete(String::new()).unwrap(),
        };
        let case = CaseEntry {
            name: CaseName::parse("example-1").unwrap(),
            run: CaseRun::Ran(StepRun {
                termination,
                duration_ms: 1,
                stdout: CapturedText::complete(stdout.to_string()).unwrap(),
                stderr: CapturedText::complete(String::new()).unwrap(),
            }),
        };
        Self::new(TestOutput::new(setup, vec![case]).unwrap(), 1, 0)
    }
}

/// Output stored with a recorded attempt. Attempts recorded before schema v3 have none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordedOutput {
    PredatesCapture,
    Captured(TestOutput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptRecord {
    pub id: i64,
    pub run_at: String,
    pub problem_slug: String,
    pub language: String,
    pub invoked_set: Option<String>,
    pub result: AttemptOutcome,
    pub duration_ms: i64,
    pub test_revision: i64,
    pub current_revision: bool,
    pub output: RecordedOutput,
}

/// One history row: attempt metadata plus its case tally, without stream text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptSummary {
    pub id: i64,
    pub run_at: String,
    pub language: String,
    pub result: AttemptOutcome,
    pub duration_ms: i64,
    pub test_revision: i64,
    pub current_revision: bool,
    /// Passed and total cases; `None` when the attempt predates output capture.
    pub cases: Option<(usize, usize)>,
}

/// Rendering of a fresh execution, shown by `practice run` and the TUI Output pane.
pub fn render_report(report: &AttemptReport) -> String {
    let mut text = render_steps(&report.output);
    let cases = report.output.cases().len();
    let _ = writeln!(
        text,
        "{} · {}/{} cases passed · {} ms",
        report.outcome(),
        report.output.passed_cases(),
        cases,
        report.duration_ms
    );
    text
}

/// Rendering of a stored attempt, shown by `practice attempts show` and the TUI Attempt screen.
pub fn render_record(record: &AttemptRecord) -> String {
    let set = record
        .invoked_set
        .as_deref()
        .map(|set| format!(" · set {set}"))
        .unwrap_or_default();
    let revision = if record.current_revision {
        "current"
    } else {
        "outdated"
    };
    let mut text = format!(
        "Attempt {} · {} · {} · {} · {} · {} ms · revision {} ({revision}){set}\n",
        record.id,
        record.problem_slug,
        record.language,
        record.result,
        record.run_at,
        record.duration_ms,
        record.test_revision,
    );
    match &record.output {
        RecordedOutput::PredatesCapture => {
            text.push_str("output: not recorded (attempt predates output capture)\n");
        }
        RecordedOutput::Captured(output) => {
            if record.result == AttemptOutcome::Cancelled
                && output.outcome() != AttemptOutcome::Cancelled
            {
                text.push_str("cancelled after every step below had finished\n");
            }
            text.push_str(&render_steps(output));
        }
    }
    text
}

fn render_steps(output: &TestOutput) -> String {
    let mut text = String::new();
    let setup = output.setup();
    if setup.termination.outcome() != AttemptOutcome::Pass || !setup.stderr.is_empty() {
        let _ = writeln!(
            text,
            "setup · {} · {} · {} ms",
            setup.termination.outcome(),
            setup.termination.describe(),
            setup.duration_ms
        );
        // A passing setup's stdout is only the case list.
        if setup.termination.outcome() != AttemptOutcome::Pass {
            render_stream(&mut text, "stdout", &setup.stdout);
        }
        render_stream(&mut text, "stderr", &setup.stderr);
    }
    let total = output.cases().len();
    for (index, case) in output.cases().iter().enumerate() {
        let position = index + 1;
        match &case.run {
            CaseRun::Ran(step) => {
                let _ = writeln!(
                    text,
                    "case {position}/{total} {} · {} · {} · {} ms",
                    case.name.as_str(),
                    step.termination.outcome(),
                    step.termination.describe(),
                    step.duration_ms
                );
                render_stream(&mut text, "stdout", &step.stdout);
                render_stream(&mut text, "stderr", &step.stderr);
            }
            CaseRun::NotRun(reason) => {
                let reason = match reason {
                    NotRunReason::Cancelled => "cancelled",
                    NotRunReason::DeadlineExceeded => "deadline exceeded",
                };
                let _ = writeln!(
                    text,
                    "case {position}/{total} {} · not run ({reason})",
                    case.name.as_str()
                );
            }
        }
    }
    text
}

fn render_stream(text: &mut String, label: &str, captured: &CapturedText) {
    if captured.is_empty() {
        return;
    }
    let _ = writeln!(text, "  {label}");
    for line in captured.render().lines() {
        let _ = writeln!(text, "    {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> CapturedText {
        CapturedText::complete(value.to_string()).unwrap()
    }

    fn step(termination: Termination) -> StepRun {
        StepRun {
            termination,
            duration_ms: 3,
            stdout: text(""),
            stderr: text(""),
        }
    }

    fn case(name: &str, run: CaseRun) -> CaseEntry {
        CaseEntry {
            name: CaseName::parse(name).unwrap(),
            run,
        }
    }

    fn ran(termination: Termination) -> CaseRun {
        CaseRun::Ran(step(termination))
    }

    #[test]
    fn case_names_are_bounded_lowercase_identifiers() {
        assert!(CaseName::parse("example-1").is_ok());
        assert!(CaseName::parse("1st").is_ok());
        for invalid in ["", "-lead", "Upper", "under_score", "a b", &"x".repeat(65)] {
            assert!(CaseName::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(CaseName::parse(&"x".repeat(64)).is_ok());
    }

    #[test]
    fn captured_text_enforces_stored_bounds_and_renders_one_marker() {
        assert!(CapturedText::complete("x".repeat(MAX_STORED_STREAM_BYTES)).is_ok());
        assert!(CapturedText::complete("x".repeat(MAX_STORED_STREAM_BYTES + 1)).is_err());
        assert!(CapturedText::truncated("a".into(), 0, "b".into()).is_err());
        assert!(
            CapturedText::truncated("a".repeat(MAX_STORED_STREAM_BYTES / 2 + 1), 1, "b".into())
                .is_err()
        );
        let truncated = CapturedText::truncated("head".into(), 42, "tail\n".into()).unwrap();
        assert_eq!(
            truncated.render(),
            "head\n[... 42 bytes omitted ...]\ntail\n"
        );
        let lookalike = text("[... 42 bytes omitted ...]\n");
        assert_ne!(lookalike, truncated);
    }

    #[test]
    fn test_output_rejects_inconsistent_case_lists() {
        let pass = || step(Termination::Exited(0));
        assert!(TestOutput::new(pass(), Vec::new()).is_err());
        assert!(
            TestOutput::new(
                step(Termination::Exited(1)),
                vec![case("a", ran(Termination::Exited(0)))]
            )
            .is_err()
        );
        assert!(
            TestOutput::new(
                pass(),
                vec![
                    case("a", ran(Termination::Exited(0))),
                    case("a", ran(Termination::Exited(0)))
                ]
            )
            .is_err()
        );
        assert!(
            TestOutput::new(
                pass(),
                vec![
                    case("a", CaseRun::NotRun(NotRunReason::Cancelled)),
                    case("b", ran(Termination::Exited(0)))
                ]
            )
            .is_err()
        );
        let too_many = (0..=MAX_CASES)
            .map(|index| case(&format!("c{index}"), ran(Termination::Exited(0))))
            .collect();
        assert!(TestOutput::new(pass(), too_many).is_err());
        assert!(TestOutput::new(step(Termination::Exited(101)), Vec::new()).is_ok());
    }

    #[test]
    fn outcome_folds_by_severity_and_codes_follow_the_deciding_step() {
        let output = TestOutput::new(
            step(Termination::Exited(0)),
            vec![
                case("a", ran(Termination::Exited(1))),
                case("b", ran(Termination::TimedOut)),
                case("c", CaseRun::NotRun(NotRunReason::DeadlineExceeded)),
            ],
        )
        .unwrap();
        let report = AttemptReport::new(output, 10, 0);
        assert_eq!(report.outcome(), AttemptOutcome::Error);
        assert_eq!(report.status_code(), 2);
        assert_eq!(report.exit_code(), None);

        let failing = TestOutput::new(
            step(Termination::Exited(0)),
            vec![
                case("a", ran(Termination::Exited(0))),
                case("b", ran(Termination::Exited(101))),
                case("c", ran(Termination::Exited(1))),
            ],
        )
        .unwrap();
        let report = AttemptReport::new(failing, 10, 0);
        assert_eq!(report.outcome(), AttemptOutcome::Fail);
        assert_eq!(report.status_code(), 101);
        assert_eq!(report.exit_code(), Some(101));
        assert_eq!(report.output.passed_cases(), 1);

        let cancelled = TestOutput::new(
            step(Termination::Exited(0)),
            vec![
                case("a", ran(Termination::Exited(1))),
                case("b", CaseRun::NotRun(NotRunReason::Cancelled)),
            ],
        )
        .unwrap();
        assert_eq!(cancelled.outcome(), AttemptOutcome::Cancelled);

        let setup_failed = TestOutput::new(step(Termination::Exited(101)), Vec::new()).unwrap();
        let report = AttemptReport::new(setup_failed, 5, 0);
        assert_eq!(report.outcome(), AttemptOutcome::Fail);
        assert_eq!(report.status_code(), 101);
    }

    #[test]
    fn passing_report_exits_zero_until_cancelled_after_completion() {
        let mut report = AttemptReport::test_report(Termination::Exited(0), "ok\n");
        assert_eq!(report.outcome(), AttemptOutcome::Pass);
        assert_eq!((report.status_code(), report.exit_code()), (0, Some(0)));
        report.mark_cancelled();
        assert_eq!(report.outcome(), AttemptOutcome::Cancelled);
        assert_eq!((report.status_code(), report.exit_code()), (130, None));
    }

    #[test]
    fn renders_each_case_with_separate_streams() {
        let output = TestOutput::new(
            step(Termination::Exited(0)),
            vec![
                case(
                    "example-1",
                    CaseRun::Ran(StepRun {
                        termination: Termination::Exited(0),
                        duration_ms: 4,
                        stdout: text("debug\nPASS\n"),
                        stderr: text(""),
                    }),
                ),
                case(
                    "example-2",
                    CaseRun::Ran(StepRun {
                        termination: Termination::Exited(1),
                        duration_ms: 5,
                        stdout: text(""),
                        stderr: CapturedText::truncated("Trace".into(), 9, "Error\n".into())
                            .unwrap(),
                    }),
                ),
            ],
        )
        .unwrap();
        let rendered = render_report(&AttemptReport::new(output, 12, 0));
        assert_eq!(
            rendered,
            "case 1/2 example-1 · pass · exit 0 · 4 ms\n  stdout\n    debug\n    PASS\n\
             case 2/2 example-2 · fail · exit 1 · 5 ms\n  stderr\n    Trace\n    \
             [... 9 bytes omitted ...]\n    Error\nfail · 1/2 cases passed · 12 ms\n"
        );
    }

    #[test]
    fn record_rendering_explains_missing_and_post_completion_cancelled_output() {
        let mut record = AttemptRecord {
            id: 17,
            run_at: "2026-09-01T10:00:00Z".into(),
            problem_slug: "two-sum".into(),
            language: "python".into(),
            invoked_set: None,
            result: AttemptOutcome::Pass,
            duration_ms: 90,
            test_revision: 1,
            current_revision: true,
            output: RecordedOutput::PredatesCapture,
        };
        assert_eq!(
            render_record(&record),
            "Attempt 17 · two-sum · python · pass · 2026-09-01T10:00:00Z · 90 ms · revision 1 (current)\n\
             output: not recorded (attempt predates output capture)\n"
        );
        record.result = AttemptOutcome::Cancelled;
        record.output = RecordedOutput::Captured(
            AttemptReport::test_report(Termination::Exited(0), "")
                .output
                .clone(),
        );
        let rendered = render_record(&record);
        assert!(rendered.contains("cancelled after every step below had finished\n"));
        assert!(rendered.contains("case 1/1 example-1 · pass · exit 0 · 1 ms\n"));
    }

    #[test]
    fn worst_case_attempt_rendering_fits_its_bound() {
        let full = || CapturedText::complete("\n".repeat(MAX_STORED_STREAM_BYTES)).unwrap();
        let truncated = || {
            let half = "\n".repeat(MAX_STORED_STREAM_BYTES / 2);
            CapturedText::truncated(half.clone(), u64::MAX, half).unwrap()
        };
        let setup = StepRun {
            termination: Termination::Exited(0),
            duration_ms: i64::MAX,
            stdout: full(),
            stderr: truncated(),
        };
        let cases = (0..MAX_CASES)
            .map(|index| CaseEntry {
                name: CaseName::parse(&format!("{index:x<64}")).unwrap(),
                run: CaseRun::Ran(StepRun {
                    termination: Termination::Signalled(i32::MIN),
                    duration_ms: i64::MAX,
                    stdout: truncated(),
                    stderr: full(),
                }),
            })
            .collect();
        let record = AttemptRecord {
            id: i64::MAX,
            run_at: "2026-10-12T00:00:00Z".into(),
            problem_slug: "x".repeat(200),
            language: "x".repeat(200),
            invoked_set: Some("x".repeat(200)),
            result: AttemptOutcome::Cancelled,
            duration_ms: i64::MAX,
            test_revision: i64::MAX,
            current_revision: false,
            output: RecordedOutput::Captured(TestOutput::new(setup, cases).unwrap()),
        };
        let rendered = render_record(&record);
        assert!(rendered.len() > 1024 * 1024, "{}", rendered.len());
        assert!(
            rendered.len() <= MAX_RENDERED_ATTEMPT_BYTES,
            "{}",
            rendered.len()
        );
    }

    #[test]
    fn failed_setup_shows_its_streams() {
        let setup = StepRun {
            termination: Termination::Exited(101),
            duration_ms: 7,
            stdout: text(""),
            stderr: text("error[E0308]: mismatched types\n"),
        };
        let report = AttemptReport::new(TestOutput::new(setup, Vec::new()).unwrap(), 7, 0);
        assert_eq!(
            render_report(&report),
            "setup · fail · exit 101 · 7 ms\n  stderr\n    error[E0308]: mismatched types\n\
             fail · 0/0 cases passed · 7 ms\n"
        );
    }
}
