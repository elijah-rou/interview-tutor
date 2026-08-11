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
            r###"{"schema_version":2,"catalog_revision":1,"problems":[{"slug":"descendants","title":"Descendants","difficulty":"Easy","topic":"Testing","leetcode_id":null,"premium":false,"leetcode_url":"https://example.com/descendants","neetcode_url":"https://example.com/descendants","statement_markdown":"## Task\n\nTest cleanup.\n\n## Example\n\nInput: signal. Output: cleanup.","test_revision":1,"adapters":[{"language":"python","solution_path":"python/solution.py"}]}]}"###,
        )
        .unwrap();
        fs::write(
            root.join("problem_sets/set.json"),
            r#"{"schema_version":2,"id":"set","name":"Set","description":"","members":[{"ordinal":1,"problem_slug":"descendants"}]}"#,
        )
        .unwrap();
        fs::write(root.join("python/solution.py"), "# fixture\n").unwrap();
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
