#![cfg(target_os = "linux")]

use practice_cli::database::{self, AttemptOutcome};
use practice_cli::runner::{
    self, CancellationToken, ExecutionEvent, ExecutionLimits, ExecutionPlan, Termination,
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
        display_output_bytes: 4096,
        read_chunk_bytes: 1024,
        event_queue_capacity: 8,
    }
}

fn execute(fixture: &Fixture, problem: &str) -> runner::ExecutionResult {
    runner::execute(
        &fixture.plan(problem),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        None,
    )
    .unwrap()
}

#[test]
fn captures_tagged_output_without_terminal_inheritance() {
    let fixture = Fixture::new();
    let (sender, receiver) = mpsc::sync_channel(8);
    let result = runner::execute(
        &fixture.plan("tagged"),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    drop(sender);
    let events: Vec<_> = receiver.into_iter().collect();

    assert_eq!(result.termination, Termination::Exited(0));
    assert_eq!(result.outcome(), AttemptOutcome::Pass);
    assert!(result.display_output.contains("[stdout] stdout-tag"));
    assert!(result.display_output.contains("[stderr] stderr-tag"));
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
fn bounds_sanitized_output_with_deterministic_omission_marker() {
    let fixture = Fixture::new();
    let mut small = limits();
    small.display_output_bytes = 128;
    let result = runner::execute(
        &fixture.plan("large"),
        &fixture.database,
        &small,
        &CancellationToken::new(),
        None,
    )
    .unwrap();

    assert!(result.display_output.starts_with("[stdout] PREFIX:"));
    assert!(result.display_output.ends_with(":TAIL\n"));
    assert!(result.omitted_bytes > 32_000);
    assert!(
        result
            .display_output
            .contains(&format!("[... {} bytes omitted ...]", result.omitted_bytes))
    );
    assert!(result.display_output.len() <= small.display_output_bytes);

    let unsafe_result = execute(&fixture, "unsafe");
    assert!(unsafe_result.display_output.contains("redsafe\tline"));
    assert!(!unsafe_result.display_output.contains('\u{1b}'));
    assert!(!unsafe_result.display_output.contains('\0'));
}

#[test]
fn derives_outcomes_from_explicit_termination() {
    let fixture = Fixture::new();
    for (problem, termination, outcome) in [
        ("exit-0", Termination::Exited(0), AttemptOutcome::Pass),
        ("exit-2", Termination::Exited(2), AttemptOutcome::Error),
        ("exit-130", Termination::Exited(130), AttemptOutcome::Fail),
    ] {
        let result = execute(&fixture, problem);
        assert_eq!(result.termination, termination);
        assert_eq!(result.outcome(), outcome);
    }
    let signalled = execute(&fixture, "signal");
    assert_eq!(signalled.termination, Termination::Signalled(15));
    assert_eq!(signalled.outcome(), AttemptOutcome::Error);
}

#[test]
fn defaults_are_bounded_and_saturated_events_never_delay_cleanup() {
    let defaults = ExecutionLimits::default();
    assert_eq!(defaults.wall_timeout, Duration::from_secs(30));
    assert_eq!(defaults.term_grace, Duration::from_millis(250));
    assert_eq!(defaults.display_output_bytes, 256 * 1024);
    assert_eq!(defaults.read_chunk_bytes, 8 * 1024);
    assert_eq!(defaults.event_queue_capacity, 64);

    let fixture = Fixture::new();
    let (sender, _receiver) = mpsc::sync_channel(1);
    let started = Instant::now();
    let result = runner::execute(
        &fixture.plan("event-saturation"),
        &fixture.database,
        &limits(),
        &CancellationToken::new(),
        Some(&sender),
    )
    .unwrap();
    assert_eq!(result.termination, Termination::Exited(0));
    assert!(result.dropped_events > 0);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn cancellation_and_timeout_return_promptly() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
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
    assert_eq!(cancelled.termination, Termination::Cancelled);
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
    assert_eq!(timed_out.termination, Termination::TimedOut);
    assert_eq!(timed_out.outcome(), AttemptOutcome::Error);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn cancellation_terminates_and_reaps_process_group_descendants() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        trigger.cancel();
    });
    let result = runner::execute(
        &fixture.plan("descendants"),
        &fixture.database,
        &limits(),
        &cancellation,
        None,
    )
    .unwrap();
    thread.join().unwrap();
    assert_eq!(result.termination, Termination::Cancelled);
    let pid = result
        .display_output
        .lines()
        .find_map(|line| line.strip_prefix("[stdout] "))
        .unwrap()
        .parse::<i32>()
        .unwrap();
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

    let result = execute(&fixture, "exit-0");
    let plan = fixture.plan("tagged");
    runner::record_execution(&connection, &plan, &result).unwrap();
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM attempts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(attempts, 1);
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
    output_limits.display_output_bytes = 128;
    let error = runner::discover_adapters(
        &fixture.root.join("python/run"),
        &output_limits,
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert!(error.contains("exceeded 128 output bytes"));
}

#[test]
fn final_output_cap_handles_invalid_utf8_split_sequences_and_stream_tags() {
    let fixture = Fixture::new();
    let split = execute(&fixture, "split-sequences");
    assert!(split.display_output.contains("€redsafe"));
    assert!(split.display_output.contains("�invalid"));
    assert!(!split.display_output.contains('\u{1b}'));
    assert!(!split.display_output.contains("title"));

    let mut capped = limits();
    capped.display_output_bytes = 96;
    let alternating = runner::execute(
        &fixture.plan("alternating"),
        &fixture.database,
        &capped,
        &CancellationToken::new(),
        None,
    )
    .unwrap();
    assert!(alternating.display_output.len() <= capped.display_output_bytes);
    assert!(alternating.omitted_bytes > 0);
    assert!(alternating.display_output.contains("bytes omitted"));
}

#[test]
fn escaped_session_pipe_cannot_hang_reader_join() {
    let fixture = Fixture::new();
    let started = Instant::now();
    let result = execute(&fixture, "escaped-descendant");
    let elapsed = started.elapsed();
    assert_eq!(result.termination, Termination::Exited(0));
    assert!(
        elapsed < Duration::from_millis(500),
        "cleanup took {elapsed:?}"
    );
    let pid = result
        .display_output
        .lines()
        .find_map(|line| line.strip_prefix("[stdout] "))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

#[test]
fn normal_exit_still_cleans_up_remaining_group_descendants() {
    let fixture = Fixture::new();
    let result = execute(&fixture, "normal-exit-descendant");
    assert_eq!(result.termination, Termination::Exited(0));
    let pid = result
        .display_output
        .lines()
        .find_map(|line| line.strip_prefix("[stdout] "))
        .unwrap()
        .parse::<i32>()
        .unwrap();
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
    let result = runner::execute(
        &fixture.plan("sleep"),
        &fixture.database,
        &boundary,
        &cancellation,
        None,
    )
    .unwrap();
    assert_eq!(result.termination, Termination::Cancelled);
}
