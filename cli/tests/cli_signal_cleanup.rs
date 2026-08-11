#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    descendant_pid_file: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "practice-cli-signal-cleanup-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::create_dir_all(root.join("problem_sets")).unwrap();
        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("catalog/problems.json"),
            r###"{"schema_version":2,"catalog_revision":1,"problems":[{"slug":"descendants","title":"Descendants","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/descendants","neetcode_url":"https://example.com/descendants","statement_markdown":"## Task\n\nTest cleanup.\n\n## Example\n\nInput: signal. Output: cleanup.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/solution.py"}]},{"slug":"signal-exit-race","title":"Signal Exit Race","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/signal-exit-race","neetcode_url":"https://example.com/signal-exit-race","statement_markdown":"## Task\n\nTest status priority.\n\n## Example\n\nInput: signal. Output: cancellation.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/signal_exit_race.py"}]},{"slug":"signal-final-drain","title":"Signal Final Drain","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/signal-final-drain","neetcode_url":"https://example.com/signal-final-drain","statement_markdown":"## Task\n\nTest final cancellation.\n\n## Example\n\nInput: signal. Output: cancellation.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/signal_final_drain.py"}]},{"slug":"batch-exit-130","title":"Batch Exit 130","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/batch-exit-130","neetcode_url":"https://example.com/batch-exit-130","statement_markdown":"## Task\n\nTest normal 130.\n\n## Example\n\nInput: run. Output: fail.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/batch_exit_130.py"}]},{"slug":"batch-hang","title":"Batch Hang","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/batch-hang","neetcode_url":"https://example.com/batch-hang","statement_markdown":"## Task\n\nTest batch cancellation.\n\n## Example\n\nInput: signal. Output: cancellation.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/batch_hang.py"}]},{"slug":"batch-later","title":"Batch Later","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/batch-later","neetcode_url":"https://example.com/batch-later","statement_markdown":"## Task\n\nMust not run.\n\n## Example\n\nInput: none. Output: none.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/batch_later.py"}]}]}"###,
        )
        .unwrap();
        fs::write(
            root.join("problem_sets/set.json"),
            r#"{"schema_version":2,"id":"set","name":"Set","description":"","members":[{"ordinal":1,"problem_slug":"batch-exit-130"},{"ordinal":2,"problem_slug":"batch-hang"},{"ordinal":3,"problem_slug":"batch-later"}]}"#,
        )
        .unwrap();
        fs::write(root.join("python/solution.py"), "# fixture\n").unwrap();
        fs::write(root.join("python/signal_exit_race.py"), "# fixture\n").unwrap();
        fs::write(root.join("python/signal_final_drain.py"), "# fixture\n").unwrap();
        fs::write(root.join("python/batch_exit_130.py"), "# fixture\n").unwrap();
        fs::write(root.join("python/batch_hang.py"), "# fixture\n").unwrap();
        fs::write(root.join("python/batch_later.py"), "# fixture\n").unwrap();
        let runner = root.join("python/run");
        fs::copy("tests/fixtures/runner_fixture.sh", &runner).unwrap();
        let mut permissions = fs::metadata(&runner).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).unwrap();
        Self {
            database: root.join("progress.db"),
            descendant_pid_file: root.join("descendant.pid"),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn wait_for_descendant(path: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            return text.trim().parse().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "runner did not publish descendant pid"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn child_exit_observation_does_not_override_concurrent_cancellation() {
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_practice"))
        .args([
            "--db",
            fixture.database.to_str().unwrap(),
            "run",
            "python",
            "signal-exit-race",
        ])
        .env("PRACTICE_ROOT", &fixture.root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(130),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    let attempts: Vec<String> = connection
        .prepare("SELECT result FROM attempts")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(attempts, vec!["cancelled"]);
}

#[test]
fn signal_during_sqlite_recording_updates_the_exact_attempt_and_matching_status() {
    for (signal, expected_status) in [(libc::SIGINT, 130), (libc::SIGTERM, 143)] {
        let fixture = Fixture::new();
        let initialized = Command::new(env!("CARGO_BIN_EXE_practice"))
            .args([
                "--db",
                fixture.database.to_str().unwrap(),
                "problems",
                "list",
            ])
            .env("PRACTICE_ROOT", &fixture.root)
            .output()
            .unwrap();
        assert!(initialized.status.success());

        let release_file = fixture.root.join("release-recording");
        let child = Command::new(env!("CARGO_BIN_EXE_practice"))
            .args([
                "--db",
                fixture.database.to_str().unwrap(),
                "run",
                "python",
                "signal-final-drain",
            ])
            .env("PRACTICE_ROOT", &fixture.root)
            .env("PRACTICE_DESCENDANT_PID_FILE", &fixture.descendant_pid_file)
            .env("PRACTICE_RECORD_RELEASE_FILE", &release_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_descendant(&fixture.descendant_pid_file);

        let lock = rusqlite::Connection::open(&fixture.database).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        fs::write(&release_file, "release\n").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        lock.execute_batch("COMMIT").unwrap();
        let output = child.wait_with_output().unwrap();

        assert_eq!(
            output.status.code(),
            Some(expected_status),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let connection = rusqlite::Connection::open(&fixture.database).unwrap();
        let attempts: Vec<(String, Option<i32>)> = connection
            .prepare("SELECT result, exit_code FROM attempts ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            attempts,
            vec![("cancelled".to_string(), Some(expected_status))]
        );
    }
}

#[test]
fn batch_normal_130_remains_interruptible_and_stops_after_explicit_signal() {
    for (signal, expected_status) in [(libc::SIGINT, 130), (libc::SIGTERM, 143)] {
        let fixture = Fixture::new();
        let later_attempt_file = fixture.root.join("later-attempt");
        let child = Command::new(env!("CARGO_BIN_EXE_practice"))
            .args([
                "--db",
                fixture.database.to_str().unwrap(),
                "--set",
                "set",
                "test",
                "python",
                "all",
            ])
            .env("PRACTICE_ROOT", &fixture.root)
            .env("PRACTICE_DESCENDANT_PID_FILE", &fixture.descendant_pid_file)
            .env("PRACTICE_LATER_ATTEMPT_FILE", &later_attempt_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let descendant = wait_for_descendant(&fixture.descendant_pid_file);
        let started = Instant::now();
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        let output = child.wait_with_output().unwrap();
        let elapsed = started.elapsed();

        assert_eq!(
            output.status.code(),
            Some(expected_status),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(elapsed < Duration::from_secs(2), "cleanup took {elapsed:?}");
        let deadline = Instant::now() + Duration::from_millis(500);
        while Path::new(&format!("/proc/{descendant}")).exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{descendant}")).exists());
        assert!(!later_attempt_file.exists());

        let connection = rusqlite::Connection::open(&fixture.database).unwrap();
        let attempts: Vec<(String, String, Option<i32>)> = connection
            .prepare(
                "SELECT p.slug, a.result, a.exit_code FROM attempts AS a JOIN problems AS p ON p.id = a.problem_id ORDER BY a.id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            attempts,
            vec![
                ("batch-exit-130".to_string(), "fail".to_string(), Some(130),),
                (
                    "batch-hang".to_string(),
                    "cancelled".to_string(),
                    Some(expected_status),
                ),
            ]
        );
    }
}

#[test]
fn cli_signals_cancel_once_cleanup_group_and_use_conventional_status() {
    for (signal, expected_status) in [(libc::SIGINT, 130), (libc::SIGTERM, 143)] {
        let fixture = Fixture::new();
        let child = Command::new(env!("CARGO_BIN_EXE_practice"))
            .args([
                "--db",
                fixture.database.to_str().unwrap(),
                "run",
                "python",
                "descendants",
            ])
            .env("PRACTICE_ROOT", &fixture.root)
            .env("PRACTICE_DESCENDANT_PID_FILE", &fixture.descendant_pid_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let descendant = wait_for_descendant(&fixture.descendant_pid_file);
        let started = Instant::now();
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        let output = child.wait_with_output().unwrap();
        let elapsed = started.elapsed();

        assert_eq!(output.status.code(), Some(expected_status));
        eprintln!("signal {signal} cleanup: {elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "cleanup took {elapsed:?}");
        let deadline = Instant::now() + Duration::from_millis(500);
        while Path::new(&format!("/proc/{descendant}")).exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{descendant}")).exists());

        let connection = rusqlite::Connection::open(&fixture.database).unwrap();
        let attempts: Vec<String> = connection
            .prepare("SELECT result FROM attempts")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(attempts, vec!["cancelled"]);
    }
}
