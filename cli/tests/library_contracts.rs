use practice_cli::database::{
    self, AttemptOutcome, Difficulty, NewProblem, ProgressScope,
};
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

    let shipped = runner::plan_execution(
        &connection,
        &fixture.root,
        "python",
        "shipped",
        None,
    )
    .unwrap();
    assert_eq!(shipped.solution_path, fixture.root.join("python/shipped.py"));

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
        .execute(
            "UPDATE problem_set_members SET section = 'Core'",
            [],
        )
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
    assert_eq!(implementations[0].runner_path, "python/run");
    assert_eq!(implementations[0].solution_path, "python/shipped.py");
}

#[test]
fn progress_summary_uses_current_revisions_for_global_and_set_scopes() {
    let fixture = Fixture::new();
    let connection = fixture.connection();
    add_custom_problem(&connection, &fixture.root);
    database::create_problem_set(&connection, "empty", "Empty", "").unwrap();

    database::record_attempt(
        &connection,
        "shipped",
        "python",
        AttemptOutcome::Pass,
        1,
        Some(0),
        None,
    )
    .unwrap();
    connection
        .execute("UPDATE problems SET test_revision = 3 WHERE slug = 'shipped'", [])
        .unwrap();

    let global = database::progress_summary(
        &connection,
        ProgressScope::Global,
        Some("python"),
    )
    .unwrap();
    assert_eq!((global.completed, global.total), (0, 2));
    assert_eq!(global.by_difficulty.len(), 2);
    assert_eq!(global.by_difficulty[0].difficulty, Difficulty::Easy);
    assert_eq!((global.by_difficulty[0].completed, global.by_difficulty[0].total), (0, 1));

    let empty = database::progress_summary(
        &connection,
        ProgressScope::ProblemSet("empty"),
        Some("python"),
    )
    .unwrap();
    assert_eq!((empty.completed, empty.total), (0, 0));
    assert!(empty.by_difficulty.is_empty());
}

#[test]
fn closed_persisted_enums_reject_unknown_values() {
    assert_eq!(Difficulty::from_str("Hard").unwrap().to_string(), "Hard");
    assert_eq!(AttemptOutcome::from_str("cancelled").unwrap().to_string(), "cancelled");
    assert!(Difficulty::from_str("Extreme").is_err());
    assert!(AttemptOutcome::from_str("timeout").is_err());

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
