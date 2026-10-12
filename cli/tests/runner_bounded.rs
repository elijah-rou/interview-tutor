#![cfg(target_os = "linux")]

use practice_cli::attempt::{
    AttemptReport, CapturedText, CaseRun, MAX_STORED_STREAM_BYTES, NotRunReason, StepRun,
    Termination,
};
use practice_cli::database::{self, AttemptOutcome};
use practice_cli::runner::{
    self, CancellationToken, ExecutionEvent, ExecutionLimits, ExecutionPlan,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "practice-runner-bounded-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::create_dir_all(root.join("problem_sets")).unwrap();
        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("catalog/problems.json"),
            r###"{"schema_version":2,"catalog_revision":1,"problems":[{"slug":"tagged","title":"Tagged","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/tagged","neetcode_url":"https://example.com/tagged","statement_markdown":"## Task\n\nTest.\n\n## Example\n\nInput: x. Output: x.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/solution.py"}]}]}"###,
        )
        .unwrap();
        fs::write(
            root.join("problem_sets/set.json"),
            r#"{"schema_version":2,"id":"set","name":"Set","description":"","members":[{"ordinal":1,"problem_slug":"tagged"}]}"#,
        )
        .unwrap();
        fs::write(root.join("python/solution.py"), "# fixture\n").unwrap();
        let runner = root.join("python/run");
        fs::copy("tests/fixtures/runner_fixture.sh", &runner).unwrap();
        let mut permissions = fs::metadata(&runner).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).unwrap();
        let database = root.join("progress.db");
        Self { root, database }
    }

    fn plan(&self, problem: &str) -> ExecutionPlan {
        ExecutionPlan {
            root: self.root.clone(),
            language: "python".to_string(),
            problem_slug: problem.to_string(),
            set_slug: Some("set".to_string()),
            runner_path: self.root.join("python/run"),
            solution_path: self.root.join("python/solution.py"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn limits() -> ExecutionLimits {
    ExecutionLimits {
        wall_timeout: Duration::from_secs(1),
        term_grace: Duration::from_millis(100),
        discovery_output_bytes: 4096,
        read_chunk_bytes: 1024,
        event_queue_capacity: 8,
    }
}

fn execute(fixture: &Fixture, problem: &str) -> AttemptReport {
    runner::execute(
        &fixture.plan(problem),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        None,
    )
    .unwrap()
}

/// The single case every fixture mode except the multi-case ones declares.
fn only_case(report: &AttemptReport) -> &StepRun {
    let cases = report.output.cases();
    assert_eq!(cases.len(), 1, "{report:?}");
    match &cases[0].run {
        CaseRun::Ran(step) => step,
        CaseRun::NotRun(reason) => panic!("case did not run: {reason:?}"),
    }
}

fn complete(text: &CapturedText) -> &str {
    match text {
        CapturedText::Complete(text) => text,
        CapturedText::Truncated { .. } => panic!("expected complete text, got {text:?}"),
    }
}

fn first_stdout_pid(report: &AttemptReport) -> i32 {
    complete(&only_case(report).stdout)
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn steps(report: &AttemptReport) -> Vec<Termination> {
    std::iter::once(report.output.setup().termination.clone())
        .chain(
            report
                .output
                .cases()
                .iter()
                .filter_map(|case| match &case.run {
                    CaseRun::Ran(step) => Some(step.termination.clone()),
                    CaseRun::NotRun(_) => None,
                }),
        )
        .collect()
}

#[test]
fn captures_each_stream_separately_without_terminal_inheritance() {
    let fixture = Fixture::new();
    let (sender, receiver) = mpsc::sync_channel(8);
    let report = runner::execute(
        &fixture.plan("tagged"),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    drop(sender);
    let events: Vec<_> = receiver.into_iter().collect();

    assert_eq!(report.output.setup().termination, Termination::Exited(0));
    assert_eq!(report.output.cases()[0].name.as_str(), "main");
    let case = only_case(&report);
    assert_eq!(case.termination, Termination::Exited(0));
    assert_eq!(report.outcome(), AttemptOutcome::Pass);
    assert_eq!(complete(&case.stdout), "stdout-tag\n");
    assert_eq!(complete(&case.stderr), "stderr-tag\n");
    assert!(
        events.iter().any(
            |event| matches!(event, ExecutionEvent::Stdout(text) if text.contains("stdout-tag"))
        )
    );
    assert!(
        events.iter().any(
            |event| matches!(event, ExecutionEvent::Stderr(text) if text.contains("stderr-tag"))
        )
    );
}

#[test]
fn bounds_sanitized_output_with_exact_omission_accounting() {
    let fixture = Fixture::new();
    let report = execute(&fixture, "large");
    let CapturedText::Truncated {
        head,
        omitted_bytes,
        tail,
    } = &only_case(&report).stdout
    else {
        panic!("large output must be truncated");
    };
    assert!(head.starts_with("PREFIX:"));
    assert!(tail.ends_with(":TAIL\n"));
    assert_eq!(head.len(), MAX_STORED_STREAM_BYTES / 2);
    assert_eq!(tail.len(), MAX_STORED_STREAM_BYTES / 2);
    let printed = "PREFIX:".len() + 32768 + ":TAIL\n".len();
    assert_eq!(
        omitted_bytes.get() as usize,
        printed - head.len() - tail.len()
    );

    let unsafe_report = execute(&fixture, "unsafe");
    let stdout = complete(&only_case(&unsafe_report).stdout);
    assert!(stdout.contains("redsafe\tline"));
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains('\0'));
}

#[test]
fn truncation_keeps_multibyte_characters_whole() {
    let fixture = Fixture::new();
    let report = execute(&fixture, "multibyte-large");
    let CapturedText::Truncated {
        head,
        omitted_bytes,
        tail,
    } = &only_case(&report).stdout
    else {
        panic!("multibyte output must be truncated");
    };
    assert!(head.chars().all(|character| character == '€'));
    assert!(tail.chars().all(|character| character == '€'));
    assert!(head.len() <= MAX_STORED_STREAM_BYTES / 2);
    assert!(tail.len() <= MAX_STORED_STREAM_BYTES / 2);
    assert_eq!(
        omitted_bytes.get() as usize,
        12_000 - head.len() - tail.len()
    );
}

#[test]
fn derives_outcomes_from_explicit_termination() {
    let fixture = Fixture::new();
    for (problem, termination, outcome) in [
        ("exit-0", Termination::Exited(0), AttemptOutcome::Pass),
        ("exit-2", Termination::Exited(2), AttemptOutcome::Error),
        ("exit-130", Termination::Exited(130), AttemptOutcome::Fail),
    ] {
        let report = execute(&fixture, problem);
        assert_eq!(only_case(&report).termination, termination);
        assert_eq!(report.outcome(), outcome);
        assert_eq!(report.status_code(), termination.status_code());
    }
    let signalled = execute(&fixture, "signal");
    assert_eq!(
        only_case(&signalled).termination,
        Termination::Signalled(15)
    );
    assert_eq!(signalled.outcome(), AttemptOutcome::Error);
}

#[test]
fn every_case_runs_after_a_failure_with_its_own_streams() {
    let fixture = Fixture::new();
    let report = execute(&fixture, "multi");
    let cases = report.output.cases();
    let names: Vec<_> = cases.iter().map(|case| case.name.as_str()).collect();
    assert_eq!(names, ["first", "second", "third"]);
    let ran: Vec<_> = cases
        .iter()
        .map(|case| match &case.run {
            CaseRun::Ran(step) => step,
            CaseRun::NotRun(reason) => panic!("{reason:?}"),
        })
        .collect();
    assert_eq!(complete(&ran[0].stdout), "first-out\n");
    assert_eq!(complete(&ran[0].stderr), "");
    assert_eq!(ran[1].termination, Termination::Exited(1));
    assert_eq!(complete(&ran[1].stderr), "second-err\n");
    assert_eq!(complete(&ran[2].stdout), "third-out\n");
    assert_eq!(report.outcome(), AttemptOutcome::Fail);
    assert_eq!(report.status_code(), 1);
    assert_eq!(report.exit_code(), Some(1));
    assert_eq!(report.output.passed_cases(), 2);
}

#[test]
fn shared_deadline_marks_later_cases_not_run() {
    let fixture = Fixture::new();
    let mut short = limits();
    short.wall_timeout = Duration::from_millis(300);
    let started = Instant::now();
    let report = runner::execute(
        &fixture.plan("multi-timeout"),
        &fixture.database,
        &short,
        &CancellationToken::new(),
        None,
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    let cases = report.output.cases();
    assert!(
        matches!(&cases[0].run, CaseRun::Ran(step) if step.termination == Termination::TimedOut)
    );
    assert_eq!(
        cases[1].run,
        CaseRun::NotRun(NotRunReason::DeadlineExceeded)
    );
    assert_eq!(report.outcome(), AttemptOutcome::Error);
}

#[test]
fn failed_setup_keeps_its_diagnostics_and_runs_no_cases() {
    let fixture = Fixture::new();
    let report = execute(&fixture, "setup-fail");
    assert!(report.output.cases().is_empty());
    let setup = report.output.setup();
    assert_eq!(setup.termination, Termination::Exited(101));
    assert_eq!(complete(&setup.stderr), "compile error\n");
    assert_eq!(report.outcome(), AttemptOutcome::Fail);
    assert_eq!(report.status_code(), 101);
}

#[test]
fn invalid_case_lists_are_protocol_errors() {
    let fixture = Fixture::new();
    for (problem, detail) in [
        ("bad-case-name", "invalid case name"),
        ("duplicate-cases", "duplicate case name"),
        ("no-cases", "expected 1 to 15 case names"),
    ] {
        let error = runner::execute(
            &fixture.plan(problem),
            &fixture.database,
            &limits(),
            &CancellationToken::new(),
            None,
        )
        .unwrap_err();
        assert!(error.contains("case list is invalid"), "{error}");
        assert!(error.contains(detail), "{error}");
    }
}

#[test]
fn defaults_are_bounded_and_saturated_events_never_delay_cleanup() {
    let defaults = ExecutionLimits::default();
    assert_eq!(defaults.wall_timeout, Duration::from_secs(30));
    assert_eq!(defaults.term_grace, Duration::from_millis(250));
    assert_eq!(defaults.discovery_output_bytes, 64 * 1024);
    assert_eq!(defaults.read_chunk_bytes, 8 * 1024);
    assert_eq!(defaults.event_queue_capacity, 64);

    let fixture = Fixture::new();
    let (sender, _receiver) = mpsc::sync_channel(1);
    let started = Instant::now();
    let report = runner::execute(
        &fixture.plan("event-saturation"),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    assert_eq!(only_case(&report).termination, Termination::Exited(0));
    assert!(report.dropped_events > 0);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn sustained_alternating_output_keeps_results_and_events_bounded() {
    let fixture = Fixture::new();
    let mut sustained = limits();
    sustained.wall_timeout = Duration::from_secs(10);
    sustained.event_queue_capacity = 4;
    let (sender, receiver) = mpsc::sync_channel(4);

    let report = runner::execute(
        &fixture.plan("sustained-alternating"),
        &fixture.database,
        &sustained,
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    drop(sender);
    let events: Vec<_> = receiver.into_iter().collect();

    let case = only_case(&report);
    assert_eq!(case.termination, Termination::Exited(0));
    for stream in [&case.stdout, &case.stderr] {
        let CapturedText::Truncated { omitted_bytes, .. } = stream else {
            panic!("sustained output must be truncated");
        };
        assert!(omitted_bytes.get() > 16 * 1024 * 1024);
    }
    assert!(report.dropped_events > 0);
    assert!(events.len() <= 4);
}

#[test]
fn unterminated_hostile_osc_keeps_execution_and_discovery_bounded() {
    let fixture = Fixture::new();
    let mut hostile = limits();
    hostile.wall_timeout = Duration::from_secs(10);
    hostile.discovery_output_bytes = 128;
    hostile.event_queue_capacity = 4;
    let (sender, receiver) = mpsc::sync_channel(4);

    let started = Instant::now();
    let report = runner::execute(
        &fixture.plan("hostile-osc"),
        &fixture.database,
        &hostile,
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    let execution_elapsed = started.elapsed();
    drop(sender);
    let events: Vec<_> = receiver.into_iter().collect();

    let case = only_case(&report);
    assert_eq!(case.termination, Termination::Exited(0));
    assert!(execution_elapsed < Duration::from_secs(10));
    assert_eq!(complete(&case.stdout), "");
    assert!(events.iter().all(|event| match event {
        ExecutionEvent::Stdout(text) | ExecutionEvent::Stderr(text) => !text.contains('x'),
    }));
    assert!(events.len() <= hostile.event_queue_capacity);

    fs::write(
        fixture.root.join("python/run"),
        "#!/bin/sh\nhead -c 32768 /dev/zero | tr '\\000' x\n",
    )
    .unwrap();
    let started = Instant::now();
    let error = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &hostile,
        &CancellationToken::new(),
    )
    .unwrap_err();
    let discovery_elapsed = started.elapsed();

    assert!(discovery_elapsed < Duration::from_secs(10));
    assert!(error.contains("exceeded 128 output bytes"));
}

#[test]
fn cancellation_and_timeout_return_promptly() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    // Cancel after the setup step, which spends one TERM grace reaping its process group.
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        trigger.cancel();
    });
    let started = Instant::now();
    let cancelled = runner::execute(
        &fixture.plan("sleep"),
        &fixture.database,
        &limits(),
        &cancellation,
        None,
    )
    .unwrap();
    thread.join().unwrap();
    assert_eq!(only_case(&cancelled).termination, Termination::Cancelled);
    assert_eq!(cancelled.outcome(), AttemptOutcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(2));

    let mut timeout_limits = limits();
    timeout_limits.wall_timeout = Duration::from_millis(50);
    let started = Instant::now();
    let timed_out = runner::execute(
        &fixture.plan("sleep"),
        &fixture.database,
        &timeout_limits,
        &CancellationToken::new(),
        None,
    )
    .unwrap();
    // The deadline covers setup and cases: setup alone spends one TERM grace, so the case
    // either times out or never starts.
    let deadline_reached = steps(&timed_out).contains(&Termination::TimedOut)
        || timed_out
            .output
            .cases()
            .iter()
            .any(|case| case.run == CaseRun::NotRun(NotRunReason::DeadlineExceeded));
    assert!(deadline_reached, "{timed_out:?}");
    assert_eq!(timed_out.outcome(), AttemptOutcome::Error);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn cancellation_terminates_and_reaps_process_group_descendants() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        trigger.cancel();
    });
    let report = runner::execute(
        &fixture.plan("descendants"),
        &fixture.database,
        &limits(),
        &cancellation,
        None,
    )
    .unwrap();
    thread.join().unwrap();
    assert_eq!(only_case(&report).termination, Termination::Cancelled);
    let pid = first_stdout_pid(&report);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn spawn_errors_do_not_record_and_recording_is_explicit_and_once() {
    let fixture = Fixture::new();
    let connection = database::open_database(&fixture.database, &fixture.root).unwrap();
    let mut plan =
        runner::plan_execution(&connection, &fixture.root, "python", "tagged", Some("set"))
            .unwrap();
    assert_eq!(plan.root, fixture.root);
    assert_eq!(plan.runner_path, fixture.root.join("python/run"));
    assert_eq!(plan.solution_path, fixture.root.join("python/solution.py"));
    assert_eq!(plan.problem_slug, "tagged");
    assert_eq!(plan.language, "python");

    plan.runner_path = fixture.root.join("python/missing");
    let error = runner::execute(
        &plan,
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        None,
    )
    .unwrap_err();
    assert!(error.contains("runner"));

    plan.runner_path = fixture.root.join("python/run");
    let mut permissions = fs::metadata(&plan.runner_path).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&plan.runner_path, permissions).unwrap();
    let error = runner::execute(
        &plan,
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        None,
    )
    .unwrap_err();
    assert!(error.contains("not executable"));
    let mut permissions = fs::metadata(&plan.runner_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plan.runner_path, permissions).unwrap();

    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM attempts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(attempts, 0);

    let report = execute(&fixture, "exit-0");
    let plan = fixture.plan("tagged");
    let attempt_id = runner::record_execution(&connection, &plan, &report).unwrap();
    assert!(attempt_id > 0);
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM attempts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(attempts, 1);
}

#[test]
fn recorded_attempts_round_trip_every_step_and_its_streams() {
    let fixture = Fixture::new();
    let connection = database::open_database(&fixture.database, &fixture.root).unwrap();
    let plan = fixture.plan("tagged");
    let multi = execute(&fixture, "multi");
    let large = execute(&fixture, "large");
    let multi_id = runner::record_execution(&connection, &plan, &multi).unwrap();
    let large_id = runner::record_execution(&connection, &plan, &large).unwrap();

    let record = database::get_attempt(&connection, multi_id).unwrap();
    assert_eq!(record.result, AttemptOutcome::Fail);
    assert_eq!(record.invoked_set.as_deref(), Some("set"));
    assert_eq!(
        record.output,
        practice_cli::attempt::RecordedOutput::Captured(multi.output.clone())
    );
    let record = database::get_attempt(&connection, large_id).unwrap();
    assert_eq!(
        record.output,
        practice_cli::attempt::RecordedOutput::Captured(large.output.clone())
    );
    let exit_code: Option<i32> = connection
        .query_row(
            "SELECT exit_code FROM attempts WHERE id = ?",
            [multi_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exit_code, Some(1));

    let page = database::list_attempts(
        &connection,
        "tagged",
        Some("python"),
        database::AttemptCursor::Newest,
        1,
    )
    .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, large_id);
    assert_eq!(page.rows[0].cases, Some((1, 1)));
    assert!(page.has_older && !page.has_newer);
    let older = database::list_attempts(
        &connection,
        "tagged",
        None,
        database::AttemptCursor::Before(large_id),
        10,
    )
    .unwrap();
    assert_eq!(older.rows.len(), 1);
    assert_eq!(older.rows[0].id, multi_id);
    assert_eq!(older.rows[0].cases, Some((2, 3)));
    assert!(!older.has_older && older.has_newer);
    let newer = database::list_attempts(
        &connection,
        "tagged",
        None,
        database::AttemptCursor::After(multi_id),
        10,
    )
    .unwrap();
    assert_eq!(
        newer.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        [large_id]
    );
    assert!(database::get_attempt(&connection, large_id + 1).is_err());
}

#[test]
fn cancellation_finalization_updates_only_the_recorded_attempt() {
    let fixture = Fixture::new();
    let connection = database::open_database(&fixture.database, &fixture.root).unwrap();
    let plan = runner::plan_execution(&connection, &fixture.root, "python", "tagged", Some("set"))
        .unwrap();
    let report = execute(&fixture, "exit-0");
    let first = runner::record_execution(&connection, &plan, &report).unwrap();
    let second = runner::record_execution(&connection, &plan, &report).unwrap();

    database::finalize_attempt_cancelled(&connection, second, 143).unwrap();
    let attempts: Vec<(i64, String, Option<i32>)> = connection
        .prepare("SELECT id, result, exit_code FROM attempts ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        attempts,
        vec![
            (first, "pass".to_string(), Some(0)),
            (second, "cancelled".to_string(), Some(143)),
        ]
    );
    // Finalization rewrites the outcome only; the steps keep describing what ran.
    let record = database::get_attempt(&connection, second).unwrap();
    assert_eq!(
        record.output,
        practice_cli::attempt::RecordedOutput::Captured(report.output.clone())
    );
    assert!(
        practice_cli::attempt::render_record(&record)
            .contains("cancelled after every step below had finished")
    );
}

#[test]
fn adapter_discovery_is_bounded() {
    let fixture = Fixture::new();
    let adapters = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &limits(),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(adapters, vec!["tagged", "exit-0"]);

    fs::write(fixture.root.join("python/run"), "#!/bin/sh\nsleep 30\n").unwrap();
    let mut timeout_limits = limits();
    timeout_limits.wall_timeout = Duration::from_millis(50);
    let started = Instant::now();
    let error = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &timeout_limits,
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert!(error.contains("TimedOut"));
    assert!(started.elapsed() < Duration::from_secs(2));

    fs::write(
        fixture.root.join("python/run"),
        "#!/bin/sh\nhead -c 32768 /dev/zero | tr '\\000' x\n",
    )
    .unwrap();
    let mut output_limits = limits();
    output_limits.discovery_output_bytes = 128;
    let error = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &output_limits,
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert!(error.contains("exceeded 128 output bytes"));

    fs::write(
        fixture.root.join("python/run"),
        "#!/bin/sh\npython3 -c 'import sys; sys.stdout.buffer.write(b\"\\xff\" * 128)'\n",
    )
    .unwrap();
    let error = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &output_limits,
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert!(error.contains("exceeded 128 output bytes"));

    fs::write(
        fixture.root.join("python/run"),
        "#!/bin/sh\nprintf 'requested\\n' >&2\nprintf 'different\\n'\n",
    )
    .unwrap();
    let adapters = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &limits(),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(adapters, vec!["different"]);
    assert!(!adapters.iter().any(|slug| slug == "requested"));
}

#[test]
fn sanitization_handles_split_sequences_and_keeps_streams_apart() {
    let fixture = Fixture::new();
    let split = execute(&fixture, "split-sequences");
    let stdout = complete(&only_case(&split).stdout);
    assert!(stdout.contains("€redsafe"));
    assert!(stdout.contains("�invalid"));
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains("title"));

    let alternating = execute(&fixture, "alternating");
    let case = only_case(&alternating);
    let expected = |prefix: char| {
        (0..40)
            .map(|index| format!("{prefix}{index}\n"))
            .collect::<String>()
    };
    assert_eq!(complete(&case.stdout), expected('o'));
    assert_eq!(complete(&case.stderr), expected('e'));
}

#[test]
fn escaped_session_pipe_cannot_hang_reader_join() {
    let fixture = Fixture::new();
    let started = Instant::now();
    let report = execute(&fixture, "escaped-descendant");
    let elapsed = started.elapsed();
    assert_eq!(only_case(&report).termination, Termination::Exited(0));
    eprintln!("escaped-session cleanup: {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(500),
        "cleanup took {elapsed:?}"
    );
    let pid = first_stdout_pid(&report);
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

#[test]
fn normal_exit_still_cleans_up_remaining_group_descendants() {
    let fixture = Fixture::new();
    let report = execute(&fixture, "normal-exit-descendant");
    assert_eq!(only_case(&report).termination, Termination::Exited(0));
    let pid = first_stdout_pid(&report);
    let deadline = Instant::now() + Duration::from_millis(500);
    while Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn real_spawn_failure_after_preflight_does_not_record() {
    let fixture = Fixture::new();
    let runner_path = fixture.root.join("python/run");
    fs::write(&runner_path, "#!/definitely/missing/interpreter\n").unwrap();
    let error = runner::execute(
        &fixture.plan("tagged"),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        None,
    )
    .unwrap_err();
    assert!(error.contains("cannot execute"));
    assert!(!fixture.database.exists());
}

#[test]
fn explicit_cancellation_wins_at_timeout_boundary() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut boundary = limits();
    boundary.wall_timeout = Duration::from_millis(10);
    let report = runner::execute(
        &fixture.plan("sleep"),
        &fixture.database,
        &boundary,
        &cancellation,
        None,
    )
    .unwrap();
    assert_eq!(report.output.setup().termination, Termination::Cancelled);
    assert_eq!(report.outcome(), AttemptOutcome::Cancelled);
}
