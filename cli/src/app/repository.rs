use super::model::{AppData, MAX_ROWS, ProblemDetail, ProblemRow, SetRow};
use crate::database::{self, ProgressScope};
use rusqlite::Connection;

pub struct Repository {
    connection: Connection,
}

impl Repository {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn load(
        &self,
        set_slug: Option<&str>,
        problem_id: Option<i64>,
        language_slug: &str,
    ) -> Result<AppData, String> {
        database::get_enabled_language(&self.connection, language_slug)?;
        let raw_sets = database::list_problem_sets(&self.connection)?;
        if raw_sets.len() > MAX_ROWS {
            return Err(format!(
                "problem-set row limit exceeded: {}",
                raw_sets.len()
            ));
        }
        let mut sets = Vec::with_capacity(raw_sets.len());
        for (set, member_count) in raw_sets {
            let progress = database::progress_summary(
                &self.connection,
                ProgressScope::ProblemSet(&set.slug),
                Some(language_slug),
            )?;
            sets.push(SetRow {
                slug: set.slug,
                name: set.name,
                description: set.description,
                member_count: usize::try_from(member_count)
                    .map_err(|_| "negative problem-set member count".to_string())?,
                completed_count: progress.completed,
            });
        }

        let progress = database::progress_summary(
            &self.connection,
            set_slug.map_or(ProgressScope::Global, ProgressScope::ProblemSet),
            Some(language_slug),
        )?;
        let completed = database::completed_problem_ids(&self.connection, Some(language_slug))?;
        let mut problems = Vec::new();
        if let Some(slug) = set_slug {
            for member in database::list_set_members(&self.connection, slug)? {
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
            for problem in database::list_active_global_problems(&self.connection)? {
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
        if problems.len() > MAX_ROWS {
            return Err(format!("problem row limit exceeded: {}", problems.len()));
        }

        let detail = if let Some(id) = problem_id {
            let row = problems
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("selected problem no longer exists: {id}"))?;
            let resolved = database::resolve_problem(&self.connection, &row.slug, set_slug)?;
            Some(ProblemDetail {
                id: resolved.problem.id,
                slug: resolved.problem.slug,
                title: resolved.problem.title,
                difficulty: resolved.problem.difficulty,
                topic: resolved.problem.topic,
                statement_markdown: resolved.problem.statement_markdown,
                implementations: database::list_enabled_implementations(&self.connection, id)?,
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
        assert!(cleanup.0.exists());
    }
}
