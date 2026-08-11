use super::model::{AppData, MAX_ROWS, ProblemDetail, ProblemRow, SetRow};
use crate::database::{
    self, MAX_DESCRIPTION_LENGTH, MAX_STATEMENT_LENGTH, MAX_TITLE_LENGTH, MAX_TOPIC_LENGTH,
    ProgressScope, RowLimit,
};
use crate::runner::{self, ExecutionPlan};
use rusqlite::Connection;
use std::path::Path;

fn validate_chars(label: &str, value: &str, maximum: usize) -> Result<(), String> {
    let length = value.chars().count();
    if length > maximum {
        return Err(format!("{label} exceeds {maximum} characters: {length}"));
    }
    Ok(())
}

fn validate_problem(problem: &database::Problem) -> Result<(), String> {
    validate_chars("problem slug", &problem.slug, MAX_TITLE_LENGTH)?;
    validate_chars("problem title", &problem.title, MAX_TITLE_LENGTH)?;
    validate_chars("problem topic", &problem.topic, MAX_TOPIC_LENGTH)?;
    validate_chars(
        "problem statement",
        &problem.statement_markdown,
        MAX_STATEMENT_LENGTH,
    )?;
    validate_chars(
        "problem LeetCode URL",
        &problem.leetcode_url,
        MAX_DESCRIPTION_LENGTH,
    )?;
    validate_chars(
        "problem NeetCode URL",
        &problem.neetcode_url,
        MAX_DESCRIPTION_LENGTH,
    )
}

pub struct Repository {
    connection: Connection,
}

impl Repository {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn prepare_execution(
        &self,
        root: &Path,
        problem_slug: &str,
        set_slug: Option<&str>,
        language_slug: &str,
    ) -> Result<ExecutionPlan, String> {
        runner::plan_execution(
            &self.connection,
            root,
            language_slug,
            problem_slug,
            set_slug,
        )
    }

    pub fn load(
        &self,
        set_slug: Option<&str>,
        problem_id: Option<i64>,
        language_slug: &str,
    ) -> Result<AppData, String> {
        let limit = RowLimit::new(MAX_ROWS)?;
        database::get_enabled_language(&self.connection, language_slug)?;
        let raw_sets = database::list_problem_sets_bounded(&self.connection, limit)?;
        let mut sets = Vec::with_capacity(raw_sets.len());
        for (set, member_count) in raw_sets {
            validate_chars("problem-set slug", &set.slug, MAX_TITLE_LENGTH)?;
            validate_chars("problem-set name", &set.name, MAX_TITLE_LENGTH)?;
            validate_chars(
                "problem-set description",
                &set.description,
                MAX_DESCRIPTION_LENGTH,
            )?;
            let progress = database::progress_summary_bounded(
                &self.connection,
                ProgressScope::ProblemSet(&set.slug),
                Some(language_slug),
                limit,
            )?;
            let member_count = usize::try_from(member_count)
                .map_err(|_| "negative problem-set member count".to_string())?;
            if member_count > MAX_ROWS {
                return Err(format!("problem-set member count exceeds {MAX_ROWS}"));
            }
            sets.push(SetRow {
                slug: set.slug,
                name: set.name,
                description: set.description,
                member_count,
                completed_count: progress.completed,
            });
        }

        let progress = database::progress_summary_bounded(
            &self.connection,
            set_slug.map_or(ProgressScope::Global, ProgressScope::ProblemSet),
            Some(language_slug),
            limit,
        )?;
        let completed =
            database::completed_problem_ids_bounded(&self.connection, Some(language_slug), limit)?;
        let mut problems = Vec::new();
        if let Some(slug) = set_slug {
            for member in database::list_set_members_bounded(&self.connection, slug, limit)? {
                validate_problem(&member.problem)?;
                problems.push(ProblemRow {
                    id: member.problem.id,
                    ordinal: Some(member.ordinal.get()),
                    slug: member.problem.slug,
                    title: member.problem.title,
                    difficulty: member.problem.difficulty,
                    topic: member.problem.topic,
                    completed: completed.contains(&member.problem.id),
                });
            }
        } else {
            for problem in database::list_active_global_problems_bounded(&self.connection, limit)? {
                validate_problem(&problem)?;
                problems.push(ProblemRow {
                    id: problem.id,
                    ordinal: None,
                    slug: problem.slug,
                    title: problem.title,
                    difficulty: problem.difficulty,
                    topic: problem.topic,
                    completed: completed.contains(&problem.id),
                });
            }
        }
        for item in &progress.by_topic {
            validate_chars("progress topic", &item.topic, MAX_TOPIC_LENGTH)?;
        }

        let detail = if let Some(id) = problem_id {
            let row = problems
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("selected problem no longer exists: {id}"))?;
            let resolved = database::resolve_problem(&self.connection, &row.slug, set_slug)?;
            validate_chars(
                "problem statement",
                &resolved.problem.statement_markdown,
                MAX_STATEMENT_LENGTH,
            )?;
            let implementations =
                database::list_enabled_implementations_bounded(&self.connection, id, limit)?;
            for implementation in &implementations {
                validate_chars(
                    "implementation path",
                    &implementation.solution_path,
                    MAX_DESCRIPTION_LENGTH,
                )?;
                validate_chars(
                    "language slug",
                    &implementation.language.slug,
                    MAX_TITLE_LENGTH,
                )?;
                validate_chars(
                    "language display name",
                    &implementation.language.display_name,
                    MAX_TITLE_LENGTH,
                )?;
                validate_chars(
                    "language runner path",
                    &implementation.language.runner_path,
                    MAX_DESCRIPTION_LENGTH,
                )?;
            }
            Some(ProblemDetail {
                id: resolved.problem.id,
                slug: resolved.problem.slug,
                title: resolved.problem.title,
                difficulty: resolved.problem.difficulty,
                topic: resolved.problem.topic,
                statement_markdown: resolved.problem.statement_markdown,
                implementations,
            })
        } else {
            None
        };
        let data = AppData {
            sets,
            problems,
            detail,
            progress,
        };
        data.assert_bounded();
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{self, AttemptOutcome};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDatabase(PathBuf);
    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn typed_repository_orders_rows_and_counts_only_current_passes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "interview-repository-{}-{nonce}.db",
            std::process::id()
        ));
        let cleanup = TestDatabase(path.clone());
        let connection = database::open_database(&path, &root).unwrap();
        database::create_problem_set(&connection, "empty", "Empty", "No members").unwrap();
        let sets = database::list_problem_sets(&connection).unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].0.slug, "blind75");
        assert_eq!(sets[1].0.slug, "empty");
        let set_slug = sets
            .iter()
            .find(|(_, count)| *count >= 3)
            .map(|(set, _)| set.slug.clone())
            .unwrap();
        let members = database::list_set_members(&connection, &set_slug).unwrap();
        database::record_attempt(
            &connection,
            &members[0].problem.slug,
            "python",
            AttemptOutcome::Pass,
            1,
            Some(0),
            Some(&set_slug),
        )
        .unwrap();
        database::record_attempt(
            &connection,
            &members[1].problem.slug,
            "python",
            AttemptOutcome::Pass,
            1,
            Some(0),
            Some(&set_slug),
        )
        .unwrap();
        connection
            .execute(
                "UPDATE problems SET test_revision = test_revision + 1 WHERE id = ?",
                [members[1].problem.id],
            )
            .unwrap();
        database::record_attempt(
            &connection,
            &members[2].problem.slug,
            "python",
            AttemptOutcome::Fail,
            1,
            Some(1),
            Some(&set_slug),
        )
        .unwrap();

        let repository = Repository::new(connection);
        let list = repository.load(Some(&set_slug), None, "python").unwrap();
        assert_eq!(list.progress.completed, 1);
        assert_eq!(list.progress.total, members.len());
        assert_eq!(list.problems[0].ordinal, Some(1));
        assert!(list.problems[0].completed);
        assert!(!list.problems[1].completed);
        assert!(!list.problems[2].completed);
        let detail = repository
            .load(Some(&set_slug), Some(members[0].problem.id), "rust")
            .unwrap();
        let detail = detail.detail.unwrap();
        assert_eq!(detail.id, members[0].problem.id);
        assert!(!detail.statement_markdown.is_empty());
        assert!(
            detail
                .implementations
                .iter()
                .any(|item| item.language.slug == "python")
        );

        repository
            .connection
            .execute(
                "UPDATE problems SET statement_markdown = ? WHERE id = ?",
                rusqlite::params!["界".repeat(MAX_STATEMENT_LENGTH + 1), members[0].problem.id],
            )
            .unwrap();
        let error = repository
            .load(Some(&set_slug), Some(members[0].problem.id), "python")
            .unwrap_err();
        assert!(error.contains("problem statement exceeds"));
        assert!(cleanup.0.exists());
    }

    #[test]
    fn bounded_language_query_rejects_invalid_limits_and_overflow() {
        assert!(RowLimit::new(0).is_err());
        assert!(RowLimit::new(database::MAX_DATABASE_QUERY_ROWS + 1).is_err());
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE languages (
                id INTEGER PRIMARY KEY, slug TEXT, display_name TEXT,
                runner_path TEXT, enabled INTEGER
             );
             INSERT INTO languages VALUES (1, 'a', 'A', 'a/run', 1);
             INSERT INTO languages VALUES (2, 'b', 'B', 'b/run', 1);",
            )
            .unwrap();
        let error =
            database::list_enabled_languages_bounded(&connection, RowLimit::new(1).unwrap())
                .unwrap_err();
        assert!(error.contains("row limit exceeded"));
    }
}
