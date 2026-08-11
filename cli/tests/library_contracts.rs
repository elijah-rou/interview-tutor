use practice_cli::database::{self, AttemptOutcome, Difficulty, NewProblem, ProgressScope};
use practice_cli::runner;
use rusqlite::params;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
    database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "practice-library-contracts-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::create_dir_all(root.join("problem_sets")).unwrap();
        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(root.join("python/run"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(root.join("python/shipped.py"), "# shipped\n").unwrap();
        fs::write(
            root.join("catalog/problems.json"),
            r#"{
              "schema_version": 2,
              "catalog_revision": 1,
              "problems": [{
                "slug": "shipped",
                "title": "Shipped",
                "difficulty": "Easy",
                "topic": "Arrays",
                "leetcode_id": null,
                "premium": false,
                "leetcode_url": "https://example.com/shipped",
                "neetcode_url": "https://example.com/shipped",
                "statement_markdown": "",
                "test_revision": 2,
                "adapters": [{
                  "language": "python",
                  "solution_path": "python/shipped.py"
                }]
              }]
            }"#,
        )
        .unwrap();
        fs::write(
            root.join("problem_sets/shipped-set.json"),
            r#"{
              "schema_version": 2,
              "id": "shipped-set",
              "name": "Shipped Set",
              "description": "",
              "members": [{"ordinal": 1, "problem_slug": "shipped"}]
            }"#,
        )
        .unwrap();
        let database = root.join("progress.db");
        Self { root, database }
    }

    fn connection(&self) -> rusqlite::Connection {
        database::open_database(&self.database, &self.root).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn add_custom_problem(connection: &rusqlite::Connection, root: &Path) {
    database::create_problem(
        connection,
        &NewProblem {
            slug: "custom",
            title: "Custom",
            difficulty: Difficulty::Medium,
            topic: "Graphs",
            statement_markdown: "",
            leetcode_id: None,
            leetcode_url: "",
            neetcode_url: "",
            premium: false,
        },
    )
    .unwrap();
    fs::write(root.join("python/custom.py"), "# custom\n").unwrap();
    database::add_implementation(connection, "custom", "python", "python/custom.py").unwrap();
}

#[test]
fn execution_plan_retains_validated_solution_paths() {
    let fixture = Fixture::new();
    let connection = fixture.connection();

    let shipped =
        runner::plan_execution(&connection, &fixture.root, "python", "shipped", None).unwrap();
    assert_eq!(
        shipped.solution_path,
        fixture.root.join("python/shipped.py")
    );

    add_custom_problem(&connection, &fixture.root);
    let custom =
        runner::plan_execution(&connection, &fixture.root, "python", "custom", None).unwrap();
    assert_eq!(custom.solution_path, fixture.root.join("python/custom.py"));
}

#[test]
fn set_members_have_positive_typed_ordinals_and_optional_sections() {
    let fixture = Fixture::new();
    let connection = fixture.connection();
    connection
        .execute("UPDATE problem_set_members SET section = 'Core'", [])
        .unwrap();

    let members = database::list_set_members(&connection, "shipped-set").unwrap();
    assert_eq!(members[0].ordinal.get(), 1);
    assert_eq!(members[0].section.as_deref(), Some("Core"));
    assert_eq!(members[0].problem.slug, "shipped");

    connection
        .execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    connection
        .execute("UPDATE problem_set_members SET ordinal = 0", [])
        .unwrap();
    let error = database::list_set_members(&connection, "shipped-set").unwrap_err();
    assert!(error.contains("invalid persisted problem-set ordinal: 0"));
}

#[test]
fn language_and_implementation_queries_return_structured_metadata() {
    let fixture = Fixture::new();
    let connection = fixture.connection();

    let languages = database::list_enabled_languages(&connection).unwrap();
    assert_eq!(languages[0].slug, "python");
    assert_eq!(languages[0].display_name, "Python");
    assert_eq!(languages[0].runner_path, "python/run");

    let problem = database::resolve_problem(&connection, "shipped", None)
        .unwrap()
        .problem;
    let implementations = database::list_enabled_implementations(&connection, problem.id).unwrap();
    assert_eq!(implementations[0].language.slug, "python");
    assert_eq!(implementations[0].language.runner_path, "python/run");
    assert_eq!(implementations[0].solution_path, "python/shipped.py");

    let global = database::list_global_problems(&connection, false).unwrap();
    assert_eq!(global.len(), 1);
    assert_eq!(global[0].problem.slug, "shipped");
    assert_eq!(global[0].implementations, implementations);
}

#[test]
fn resolved_problem_keeps_membership_metadata_separate_from_problem() {
    let fixture = Fixture::new();
    let connection = fixture.connection();
    connection
        .execute("UPDATE problem_set_members SET section = 'Core'", [])
        .unwrap();

    let resolved = database::resolve_problem(&connection, "shipped", Some("shipped-set")).unwrap();
    assert_eq!(resolved.problem.slug, "shipped");
    let membership = resolved.membership.unwrap();
    assert_eq!(membership.ordinal.get(), 1);
    assert_eq!(membership.section.as_deref(), Some("Core"));
}

#[test]
fn progress_summary_honors_scope_language_revision_and_outcome() {
    let fixture = Fixture::new();
    let connection = fixture.connection();
    add_custom_problem(&connection, &fixture.root);
    database::create_problem(
        &connection,
        &NewProblem {
            slug: "second",
            title: "Second",
            difficulty: Difficulty::Medium,
            topic: "Arrays",
            statement_markdown: "",
            leetcode_id: None,
            leetcode_url: "",
            neetcode_url: "",
            premium: false,
        },
    )
    .unwrap();
    database::create_problem_set(&connection, "alternate", "Alternate", "").unwrap();
    database::create_problem_set(&connection, "empty", "Empty", "").unwrap();
    connection
        .execute(
            "INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal) \
             SELECT ps.id, p.id, 2 FROM problem_sets AS ps, problems AS p \
             WHERE ps.slug = 'shipped-set' AND p.slug = 'second'",
            [],
        )
        .unwrap();
    database::add_set_member(&connection, "alternate", "shipped", None, None).unwrap();

    connection
        .execute(
            "INSERT INTO attempts(problem_id, language_id, result, test_revision, duration_ms, run_at) \
             SELECT p.id, l.id, 'pass', 1, 1, '2025-01-01T00:00:00Z' \
             FROM problems AS p, languages AS l \
             WHERE p.slug = 'shipped' AND l.slug = 'python'",
            [],
        )
        .unwrap();
    database::record_attempt(
        &connection,
        "shipped",
        "python",
        AttemptOutcome::Fail,
        1,
        Some(1),
        Some("shipped-set"),
    )
    .unwrap();
    database::record_attempt(
        &connection,
        "shipped",
        "rust",
        AttemptOutcome::Pass,
        1,
        Some(0),
        Some("alternate"),
    )
    .unwrap();
    for problem in ["second", "custom"] {
        database::record_attempt(
            &connection,
            problem,
            "python",
            AttemptOutcome::Pass,
            1,
            Some(0),
            None,
        )
        .unwrap();
    }

    let set_any =
        database::progress_summary(&connection, ProgressScope::ProblemSet("shipped-set"), None)
            .unwrap();
    assert_eq!((set_any.completed, set_any.total), (2, 2));
    assert_eq!(
        set_any.by_difficulty,
        vec![
            database::DifficultyProgress {
                difficulty: Difficulty::Easy,
                completed: 1,
                total: 1,
            },
            database::DifficultyProgress {
                difficulty: Difficulty::Medium,
                completed: 1,
                total: 1,
            },
        ]
    );
    assert_eq!(
        set_any.by_topic,
        vec![database::TopicProgress {
            topic: "Arrays".to_string(),
            completed: 2,
            total: 2,
        }]
    );

    let set_python = database::progress_summary(
        &connection,
        ProgressScope::ProblemSet("shipped-set"),
        Some("python"),
    )
    .unwrap();
    assert_eq!((set_python.completed, set_python.total), (1, 2));
    assert_eq!(set_python.by_difficulty[0].completed, 0);
    assert_eq!(set_python.by_difficulty[1].completed, 1);
    assert_eq!(set_python.by_topic[0].completed, 1);

    let global =
        database::progress_summary(&connection, ProgressScope::Global, Some("python")).unwrap();
    assert_eq!((global.completed, global.total), (2, 3));
    assert_eq!(
        global.by_topic,
        vec![
            database::TopicProgress {
                topic: "Graphs".to_string(),
                completed: 1,
                total: 1,
            },
            database::TopicProgress {
                topic: "Arrays".to_string(),
                completed: 1,
                total: 2,
            },
        ]
    );

    let empty = database::progress_summary(
        &connection,
        ProgressScope::ProblemSet("empty"),
        Some("python"),
    )
    .unwrap();
    assert_eq!((empty.completed, empty.total), (0, 0));
    assert!(empty.by_difficulty.is_empty());
    assert!(empty.by_topic.is_empty());
}

#[test]
fn closed_persisted_enums_reject_unknown_values() {
    assert_eq!(Difficulty::from_str("Hard").unwrap().to_string(), "Hard");
    assert_eq!(
        AttemptOutcome::from_str("cancelled").unwrap().to_string(),
        "cancelled"
    );
    assert!(Difficulty::from_str("Extreme").is_err());
    assert!(AttemptOutcome::from_str("timeout").is_err());
    let sqlite = rusqlite::Connection::open_in_memory().unwrap();
    let parsed: AttemptOutcome = sqlite
        .query_row("SELECT 'pass'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(parsed, AttemptOutcome::Pass);
    let unknown = sqlite
        .query_row("SELECT 'timeout'", [], |row| {
            row.get::<_, AttemptOutcome>(0)
        })
        .unwrap_err();
    assert!(format!("{unknown:?}").contains("unknown persisted attempt outcome"));

    let fixture = Fixture::new();
    let connection = fixture.connection();
    connection
        .execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    connection
        .execute(
            "UPDATE problems SET difficulty = ? WHERE slug = 'shipped'",
            params!["Extreme"],
        )
        .unwrap();
    let error = database::resolve_problem(&connection, "shipped", None).unwrap_err();
    assert!(error.contains("unknown persisted difficulty: Extreme"));
}
