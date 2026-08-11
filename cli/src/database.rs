use crate::catalog::{
    ProblemSeed, ProblemSetSeed, SeedCatalog, validate_http_url, validate_identifier,
};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};
use std::str::FromStr;
use std::time::Duration;

pub const MAX_TITLE_LENGTH: usize = 200;
pub const MAX_TOPIC_LENGTH: usize = 100;
pub const MAX_DESCRIPTION_LENGTH: usize = 2_000;
pub const MAX_STATEMENT_LENGTH: usize = 1_000_000;
pub const MAX_DATABASE_QUERY_ROWS: usize = 100_000;
const SCHEMA_VERSION: i64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowLimit(usize);

impl RowLimit {
    pub fn new(maximum: usize) -> Result<Self, String> {
        if maximum == 0 || maximum > MAX_DATABASE_QUERY_ROWS {
            return Err(format!(
                "database row limit must be between 1 and {MAX_DATABASE_QUERY_ROWS}"
            ));
        }
        Ok(Self(maximum))
    }

    fn sql_limit(self) -> i64 {
        i64::try_from(self.0 + 1).expect("validated database row limit fits i64")
    }

    fn check<T>(self, label: &str, rows: Vec<T>) -> Result<Vec<T>, String> {
        if rows.len() > self.0 {
            return Err(format!("{label} row limit exceeded: maximum {}", self.0));
        }
        Ok(rows)
    }
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS problems (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    difficulty TEXT NOT NULL CHECK (difficulty IN ('Easy', 'Medium', 'Hard')),
    topic TEXT NOT NULL,
    leetcode_id INTEGER,
    premium INTEGER NOT NULL CHECK (premium IN (0, 1)),
    managed INTEGER NOT NULL DEFAULT 0 CHECK (managed IN (0, 1)),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    leetcode_url TEXT NOT NULL,
    neetcode_url TEXT NOT NULL,
    statement_markdown TEXT NOT NULL DEFAULT '',
    test_revision INTEGER NOT NULL CHECK (test_revision > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS problems_leetcode_id
    ON problems(leetcode_id) WHERE leetcode_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS problem_sets (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    managed INTEGER NOT NULL DEFAULT 0 CHECK (managed IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS problem_set_members (
    problem_set_id INTEGER NOT NULL REFERENCES problem_sets(id) ON DELETE CASCADE,
    problem_id INTEGER NOT NULL REFERENCES problems(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    section TEXT,
    PRIMARY KEY (problem_set_id, problem_id),
    UNIQUE (problem_set_id, ordinal)
) STRICT;
CREATE TABLE IF NOT EXISTS languages (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    runner_path TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;
CREATE TABLE IF NOT EXISTS problem_implementations (
    problem_id INTEGER NOT NULL REFERENCES problems(id) ON DELETE RESTRICT,
    language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE RESTRICT,
    solution_path TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    PRIMARY KEY (problem_id, language_id),
    UNIQUE (language_id, solution_path)
) STRICT;
CREATE TABLE IF NOT EXISTS attempts (
    id INTEGER PRIMARY KEY,
    problem_id INTEGER NOT NULL REFERENCES problems(id) ON DELETE RESTRICT,
    language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE RESTRICT,
    invoked_set_id INTEGER REFERENCES problem_sets(id) ON DELETE SET NULL,
    result TEXT NOT NULL CHECK (result IN ('pass', 'fail', 'error', 'cancelled')),
    test_revision INTEGER NOT NULL CHECK (test_revision > 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    exit_code INTEGER,
    run_at TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS attempts_problem_language_revision
    ON attempts(problem_id, language_id, test_revision, result, run_at);
"#;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl FromStr for Difficulty {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "Easy" => Ok(Self::Easy),
            "Medium" => Ok(Self::Medium),
            "Hard" => Ok(Self::Hard),
            _ => Err(format!("invalid difficulty: {value}")),
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
        })
    }
}

impl FromSql for Difficulty {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let value = value.as_str()?;
        Self::from_str(value).map_err(|_| {
            FromSqlError::Other(format!("unknown persisted difficulty: {value}").into())
        })
    }
}

impl ToSql for Difficulty {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    Pass,
    Fail,
    Error,
    Cancelled,
}

impl FromStr for AttemptOutcome {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pass" => Ok(Self::Pass),
            "fail" => Ok(Self::Fail),
            "error" => Ok(Self::Error),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("invalid attempt outcome: {value}")),
        }
    }
}

impl fmt::Display for AttemptOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        })
    }
}

impl FromSql for AttemptOutcome {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let value = value.as_str()?;
        Self::from_str(value).map_err(|_| {
            FromSqlError::Other(format!("unknown persisted attempt outcome: {value}").into())
        })
    }
}

impl ToSql for AttemptOutcome {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PositiveOrdinal(i64);

impl PositiveOrdinal {
    pub fn new(value: i64) -> Result<Self, String> {
        if value > 0 {
            Ok(Self(value))
        } else {
            Err(format!("invalid persisted problem-set ordinal: {value}"))
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

impl FromSql for PositiveOrdinal {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let value = value.as_i64()?;
        Self::new(value).map_err(|error| FromSqlError::Other(error.into()))
    }
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub difficulty: Difficulty,
    pub topic: String,
    pub leetcode_id: Option<i64>,
    pub premium: bool,
    pub managed: bool,
    pub archived: bool,
    pub leetcode_url: String,
    pub neetcode_url: String,
    pub statement_markdown: String,
    pub test_revision: i64,
}

#[derive(Clone, Debug)]
pub struct SetMember {
    pub problem: Problem,
    pub ordinal: PositiveOrdinal,
    pub section: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProblemMembership {
    pub ordinal: PositiveOrdinal,
    pub section: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedProblem {
    pub problem: Problem,
    pub membership: Option<ProblemMembership>,
}

#[derive(Clone, Debug)]
pub struct ProblemSet {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub managed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnabledLanguage {
    pub slug: String,
    pub display_name: String,
    pub runner_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProblemImplementation {
    pub language: EnabledLanguage,
    pub solution_path: String,
}

#[derive(Clone, Debug)]
pub struct ProblemListRow {
    pub problem: Problem,
    pub implementations: Vec<ProblemImplementation>,
}

#[derive(Clone, Copy, Debug)]
pub enum ProgressScope<'a> {
    Global,
    ProblemSet(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DifficultyProgress {
    pub difficulty: Difficulty,
    pub completed: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicProgress {
    pub topic: String,
    pub completed: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressSummary {
    pub completed: usize,
    pub total: usize,
    pub by_difficulty: Vec<DifficultyProgress>,
    pub by_topic: Vec<TopicProgress>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyMetadata {
    title: String,
    difficulty: String,
    topic: String,
    external_id: Option<i64>,
    premium: i64,
    test_revision: i64,
}

#[derive(Clone, Debug)]
struct LegacyProblem {
    _problem_set_id: String,
    slug: String,
    ordinal: i64,
    title: String,
    difficulty: String,
    topic: String,
    external_id: Option<i64>,
    premium: i64,
    test_revision: i64,
}

fn sql_error(error: rusqlite::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

fn constraint_error(error: rusqlite::Error, message: String) -> String {
    match &error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            message
        }
        _ => sql_error(error),
    }
}

fn timestamp(connection: &Connection) -> Result<String, String> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(sql_error)
}

fn transaction<T>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(sql_error)?;
    match operation() {
        Ok(value) => {
            if let Err(error) = connection.execute_batch("COMMIT") {
                let _ = connection.execute_batch("ROLLBACK");
                Err(sql_error(error))
            } else {
                Ok(value)
            }
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub fn open_database(path: &Path, root: &Path) -> Result<Connection, String> {
    let catalog = crate::catalog::load_seed_catalog(root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let schema_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql_error)?;
    if !(0..=SCHEMA_VERSION).contains(&schema_version) {
        return Err(format!(
            "unsupported database schema version: {schema_version} (supported 0 through {SCHEMA_VERSION})"
        ));
    }
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(sql_error)?;
    connection
        .query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
        .map_err(sql_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(sql_error)?;
    migrate_if_needed(&connection, &catalog, schema_version)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sql_error)?;
    ensure_languages(&connection)?;
    sync_seed_catalog(&connection, &catalog)?;
    let violation: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    if violation.is_some() {
        return Err("database foreign-key violations detected".to_string());
    }
    Ok(connection)
}

fn migrate_if_needed(
    connection: &Connection,
    catalog: &SeedCatalog,
    schema_version: i64,
) -> Result<(), String> {
    if !(0..=SCHEMA_VERSION).contains(&schema_version) {
        return Err(format!(
            "unsupported database schema version: {schema_version} (supported 0 through {SCHEMA_VERSION})"
        ));
    }
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'problems')",
            [],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if !exists {
        if schema_version != 0 {
            return Err(format!(
                "database schema version {schema_version} is missing the problems table"
            ));
        }
        return transaction(connection, || {
            connection.execute_batch(SCHEMA_SQL).map_err(sql_error)?;
            connection
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(sql_error)
        });
    }
    if schema_version == SCHEMA_VERSION {
        return transaction(connection, || {
            connection.execute_batch(SCHEMA_SQL).map_err(sql_error)
        });
    }
    let mut statement = connection
        .prepare("PRAGMA table_info(problems)")
        .map_err(sql_error)?;
    let columns: HashSet<String> = statement
        .query_map([], |row| row.get(1))
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)?;
    let legacy_shape = columns.contains("problem_set_id");
    if schema_version == 1 && !legacy_shape {
        return Err("database schema version 1 does not have the v1 table shape".to_string());
    }
    if legacy_shape {
        return migrate_v1(connection, catalog);
    }
    if schema_version != 0 {
        return Err(format!(
            "database schema version {schema_version} does not have a supported table shape"
        ));
    }
    transaction(connection, || {
        connection.execute_batch(SCHEMA_SQL).map_err(sql_error)?;
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql_error)
    })
}

fn migrate_v1(connection: &Connection, catalog: &SeedCatalog) -> Result<(), String> {
    transaction(connection, || {
        let now = timestamp(connection)?;
        connection
            .execute_batch(
                "ALTER TABLE attempts RENAME TO legacy_attempts;
                 ALTER TABLE problems RENAME TO legacy_problems;
                 ALTER TABLE problem_sets RENAME TO legacy_problem_sets;",
            )
            .map_err(sql_error)?;
        let legacy_problems = {
            let mut statement = connection
                .prepare(
                    "SELECT problem_set_id, slug, ordinal, title, difficulty, topic, \
                     external_id, premium, test_revision \
                     FROM legacy_problems ORDER BY problem_set_id, slug",
                )
                .map_err(sql_error)?;
            statement
                .query_map([], |row| {
                    Ok(LegacyProblem {
                        _problem_set_id: row.get(0)?,
                        slug: row.get(1)?,
                        ordinal: row.get(2)?,
                        title: row.get(3)?,
                        difficulty: row.get(4)?,
                        topic: row.get(5)?,
                        external_id: row.get(6)?,
                        premium: row.get(7)?,
                        test_revision: row.get(8)?,
                    })
                })
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        let mut metadata_by_slug: BTreeMap<String, LegacyMetadata> = BTreeMap::new();
        let mut external_owners: HashMap<i64, BTreeSet<String>> = HashMap::new();
        for problem in &legacy_problems {
            let metadata = LegacyMetadata {
                title: problem.title.clone(),
                difficulty: problem.difficulty.clone(),
                topic: problem.topic.clone(),
                external_id: problem.external_id,
                premium: problem.premium,
                test_revision: problem.test_revision,
            };
            if let Some(existing) = metadata_by_slug.get(&problem.slug) {
                if existing != &metadata {
                    return Err(format!(
                        "v1 problem metadata conflicts across sets: {}",
                        problem.slug
                    ));
                }
            } else {
                metadata_by_slug.insert(problem.slug.clone(), metadata);
            }
            if let Some(external_id) = problem.external_id {
                external_owners
                    .entry(external_id)
                    .or_default()
                    .insert(problem.slug.clone());
            }
        }
        if external_owners.values().any(|owners| owners.len() > 1) {
            return Err("v1 LeetCode ids conflict across global problem slugs".to_string());
        }
        for shipped_problem in &catalog.problems {
            let Some(leetcode_id) = shipped_problem.leetcode_id else {
                continue;
            };
            if external_owners.get(&leetcode_id).is_some_and(|owners| {
                owners
                    .iter()
                    .any(|slug| slug != shipped_problem.slug.as_str())
            }) {
                return Err(format!(
                    "shipped catalog LeetCode id conflicts with v1 problem: {}",
                    leetcode_id
                ));
            }
        }
        let active_slugs: HashSet<&str> = legacy_problems
            .iter()
            .filter(|problem| problem.ordinal < 1_000_000)
            .map(|problem| problem.slug.as_str())
            .collect();

        connection.execute_batch(SCHEMA_SQL).map_err(sql_error)?;
        for (slug, name, runner) in [
            ("python", "Python", "python/run"),
            ("rust", "Rust", "rust/run"),
        ] {
            connection
                .execute(
                    "INSERT INTO languages(slug, display_name, runner_path) VALUES (?, ?, ?)",
                    params![slug, name, runner],
                )
                .map_err(sql_error)?;
        }
        let legacy_languages = {
            let mut statement = connection
                .prepare("SELECT DISTINCT language FROM legacy_attempts ORDER BY language")
                .map_err(sql_error)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        let mut parked_language_index = 0_u32;
        for language in legacy_languages {
            if language == "python" || language == "rust" {
                continue;
            }
            parked_language_index += 1;
            connection
                .execute(
                    "INSERT INTO languages(slug, display_name, runner_path, enabled) \
                     VALUES (?, ?, ?, 0)",
                    params![
                        language,
                        language,
                        format!("legacy-language-{parked_language_index}/run")
                    ],
                )
                .map_err(sql_error)?;
        }
        for (slug, metadata) in metadata_by_slug {
            connection
                .execute(
                    "INSERT INTO problems( \
                        slug, title, difficulty, topic, leetcode_id, premium, managed, archived, \
                        leetcode_url, neetcode_url, statement_markdown, test_revision, \
                        created_at, updated_at \
                     ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, '', '', '', ?, ?, ?)",
                    params![
                        slug,
                        metadata.title,
                        metadata.difficulty,
                        metadata.topic,
                        metadata.external_id,
                        metadata.premium,
                        i64::from(!active_slugs.contains(slug.as_str())),
                        metadata.test_revision,
                        now,
                        now
                    ],
                )
                .map_err(sql_error)?;
        }
        connection
            .execute(
                "INSERT INTO problem_sets(slug, name, description, managed, created_at, updated_at) \
                 SELECT id, name, '', 0, ?, ? FROM legacy_problem_sets",
                params![now, now],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal) \
                 SELECT ps.id, p.id, lp.ordinal \
                 FROM legacy_problems AS lp \
                 JOIN problem_sets AS ps ON ps.slug = lp.problem_set_id \
                 JOIN problems AS p ON p.slug = lp.slug \
                 WHERE lp.ordinal < 1000000",
                [],
            )
            .map_err(sql_error)?;
        let legacy_attempt_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM legacy_attempts", [], |row| row.get(0))
            .map_err(sql_error)?;
        connection
            .execute(
                "INSERT INTO attempts( \
                    id, problem_id, language_id, invoked_set_id, result, \
                    test_revision, duration_ms, exit_code, run_at \
                 ) \
                 SELECT la.id, p.id, l.id, ps.id, \
                        CASE WHEN la.passed = 1 THEN 'pass' ELSE 'fail' END, \
                        la.test_revision, la.duration_ms, NULL, la.run_at \
                 FROM legacy_attempts AS la \
                 JOIN problems AS p ON p.slug = la.problem_slug \
                 JOIN languages AS l ON l.slug = la.language \
                 LEFT JOIN problem_sets AS ps ON ps.slug = la.problem_set_id",
                [],
            )
            .map_err(sql_error)?;
        let migrated_attempt_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM attempts", [], |row| row.get(0))
            .map_err(sql_error)?;
        if legacy_attempt_count != migrated_attempt_count {
            return Err(format!(
                "v1 attempt migration count mismatch: {migrated_attempt_count} != {legacy_attempt_count}"
            ));
        }
        connection
            .execute_batch(
                "DROP TABLE legacy_attempts;
                 DROP TABLE legacy_problems;
                 DROP TABLE legacy_problem_sets;",
            )
            .map_err(sql_error)?;
        connection.execute_batch(SCHEMA_SQL).map_err(sql_error)?;
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql_error)
    })
}

fn ensure_languages(connection: &Connection) -> Result<(), String> {
    for (slug, name, runner) in [
        ("python", "Python", "python/run"),
        ("rust", "Rust", "rust/run"),
    ] {
        connection
            .execute(
                "INSERT INTO languages(slug, display_name, runner_path) VALUES (?, ?, ?) \
                 ON CONFLICT(slug) DO UPDATE SET \
                    display_name = excluded.display_name, runner_path = excluded.runner_path",
                params![slug, name, runner],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}

fn sync_seed_catalog(connection: &Connection, catalog: &SeedCatalog) -> Result<(), String> {
    let current_revision: u32 = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'catalog_revision'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_error)?
        .map_or(Ok(0), |value| {
            value
                .parse()
                .map_err(|error| format!("invalid stored catalog revision: {error}"))
        })?;
    if current_revision >= catalog.revision {
        return Ok(());
    }
    transaction(connection, || {
        let now = timestamp(connection)?;
        connection
            .execute(
                "UPDATE problems SET archived = 1, updated_at = ? WHERE managed = 1",
                params![now],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "UPDATE problem_implementations SET enabled = 0 \
                 WHERE problem_id IN (SELECT id FROM problems WHERE managed = 1)",
                [],
            )
            .map_err(sql_error)?;

        let current_set_ids: HashSet<&str> = catalog
            .problem_sets
            .iter()
            .map(|problem_set| problem_set.id.as_str())
            .collect();
        let retired_set_ids = {
            let mut statement = connection
                .prepare("SELECT id, slug FROM problem_sets WHERE managed = 1 ORDER BY id")
                .map_err(sql_error)?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sql_error)?
                .filter_map(|row| match row {
                    Ok((id, slug)) if !current_set_ids.contains(slug.as_str()) => Some(Ok(id)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        for set_id in retired_set_ids {
            connection
                .execute("DELETE FROM problem_sets WHERE id = ?", params![set_id])
                .map_err(sql_error)?;
        }

        for problem in &catalog.problems {
            sync_problem(connection, problem, current_revision, &now)?;
        }
        for problem_set in &catalog.problem_sets {
            sync_problem_set(connection, problem_set, current_revision, &now)?;
        }
        connection
            .execute(
                "INSERT INTO metadata(key, value) VALUES ('catalog_revision', ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![catalog.revision.to_string()],
            )
            .map_err(sql_error)?;
        Ok(())
    })
}

fn sync_problem(
    connection: &Connection,
    problem: &ProblemSeed,
    current_revision: u32,
    now: &str,
) -> Result<(), String> {
    let existing_managed: Option<bool> = connection
        .query_row(
            "SELECT managed FROM problems WHERE slug = ?",
            params![problem.slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    if current_revision > 0 && existing_managed == Some(false) {
        return Err(format!(
            "shipped catalog conflicts with local problem: {}",
            problem.slug
        ));
    }
    if let Some(leetcode_id) = problem.leetcode_id {
        let conflict: Option<(String, bool)> = connection
            .query_row(
                "SELECT slug, managed FROM problems WHERE leetcode_id = ? AND slug <> ?",
                params![leetcode_id, problem.slug],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some((slug, false)) = conflict {
            return Err(format!(
                "shipped catalog LeetCode id conflicts with local problem: {slug}"
            ));
        }
        connection
            .execute(
                "UPDATE problems SET leetcode_id = NULL, updated_at = ? \
                 WHERE leetcode_id = ? AND slug <> ?",
                params![now, leetcode_id, problem.slug],
            )
            .map_err(sql_error)?;
    }
    connection
        .execute(
            "INSERT INTO problems( \
                slug, title, difficulty, topic, leetcode_id, premium, managed, archived, \
                leetcode_url, neetcode_url, statement_markdown, test_revision, \
                created_at, updated_at \
             ) VALUES (?, ?, ?, ?, ?, ?, 1, 0, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(slug) DO UPDATE SET \
                title = excluded.title, difficulty = excluded.difficulty, topic = excluded.topic, \
                leetcode_id = excluded.leetcode_id, premium = excluded.premium, managed = 1, \
                archived = 0, leetcode_url = excluded.leetcode_url, \
                neetcode_url = excluded.neetcode_url, \
                statement_markdown = excluded.statement_markdown, \
                test_revision = excluded.test_revision, updated_at = excluded.updated_at",
            params![
                problem.slug,
                problem.title,
                problem.difficulty,
                problem.topic,
                problem.leetcode_id,
                problem.premium,
                problem.leetcode_url,
                problem.neetcode_url,
                problem.statement_markdown,
                problem.test_revision,
                now,
                now
            ],
        )
        .map_err(sql_error)?;
    let problem_id: i64 = connection
        .query_row(
            "SELECT id FROM problems WHERE slug = ?",
            params![problem.slug],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    for adapter in &problem.adapters {
        let language_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM languages WHERE slug = ?",
                params![adapter.language],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let Some(language_id) = language_id else {
            return Err(format!(
                "catalog references unknown language: {}",
                adapter.language
            ));
        };
        connection
            .execute(
                "INSERT INTO problem_implementations( \
                    problem_id, language_id, solution_path, enabled \
                 ) VALUES (?, ?, ?, 1) \
                 ON CONFLICT(problem_id, language_id) DO UPDATE SET \
                    solution_path = excluded.solution_path, enabled = 1",
                params![problem_id, language_id, adapter.solution_path],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}

fn sync_problem_set(
    connection: &Connection,
    problem_set: &ProblemSetSeed,
    current_revision: u32,
    now: &str,
) -> Result<(), String> {
    let existing_managed: Option<bool> = connection
        .query_row(
            "SELECT managed FROM problem_sets WHERE slug = ?",
            params![problem_set.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    if current_revision > 0 && existing_managed == Some(false) {
        return Err(format!(
            "shipped catalog conflicts with local problem set: {}",
            problem_set.id
        ));
    }
    connection
        .execute(
            "INSERT INTO problem_sets( \
                slug, name, description, managed, created_at, updated_at \
             ) VALUES (?, ?, ?, 1, ?, ?) \
             ON CONFLICT(slug) DO UPDATE SET \
                name = excluded.name, description = excluded.description, \
                managed = 1, updated_at = excluded.updated_at",
            params![
                problem_set.id,
                problem_set.name,
                problem_set.description,
                now,
                now
            ],
        )
        .map_err(sql_error)?;
    let set_id: i64 = connection
        .query_row(
            "SELECT id FROM problem_sets WHERE slug = ?",
            params![problem_set.id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    connection
        .execute(
            "DELETE FROM problem_set_members WHERE problem_set_id = ?",
            params![set_id],
        )
        .map_err(sql_error)?;
    for member in &problem_set.members {
        let problem_id: i64 = connection
            .query_row(
                "SELECT id FROM problems WHERE slug = ?",
                params![member.problem_slug],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal) \
                 VALUES (?, ?, ?)",
                params![set_id, problem_id, member.ordinal],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}

fn problem_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Problem> {
    Ok(Problem {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        difficulty: row.get(3)?,
        topic: row.get(4)?,
        leetcode_id: row.get(5)?,
        premium: row.get(6)?,
        managed: row.get(7)?,
        archived: row.get(8)?,
        leetcode_url: row.get(9)?,
        neetcode_url: row.get(10)?,
        statement_markdown: row.get(11)?,
        test_revision: row.get(12)?,
    })
}

fn set_member_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SetMember> {
    Ok(SetMember {
        problem: problem_from_row(row)?,
        ordinal: row.get(13)?,
        section: row.get(14)?,
    })
}

const PROBLEM_COLUMNS: &str = "p.id, p.slug, p.title, p.difficulty, p.topic, p.leetcode_id, p.premium, \
     p.managed, p.archived, p.leetcode_url, p.neetcode_url, p.statement_markdown, \
     p.test_revision";

pub fn get_problem_set(connection: &Connection, slug: &str) -> Result<ProblemSet, String> {
    connection
        .query_row(
            "SELECT id, slug, name, description, managed FROM problem_sets WHERE slug = ?",
            params![slug],
            |row| {
                Ok(ProblemSet {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    managed: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(sql_error)?
        .ok_or_else(|| format!("unknown problem set: {slug}"))
}

pub fn list_problem_sets(connection: &Connection) -> Result<Vec<(ProblemSet, i64)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT ps.id, ps.slug, ps.name, ps.description, ps.managed, COUNT(m.problem_id) \
             FROM problem_sets AS ps \
             LEFT JOIN problem_set_members AS m ON m.problem_set_id = ps.id \
             GROUP BY ps.id ORDER BY ps.slug",
        )
        .map_err(sql_error)?;
    statement
        .query_map([], |row| {
            Ok((
                ProblemSet {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    managed: row.get(4)?,
                },
                row.get(5)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)
}

pub fn list_problem_sets_bounded(
    connection: &Connection,
    limit: RowLimit,
) -> Result<Vec<(ProblemSet, i64)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT ps.id, ps.slug, ps.name, ps.description, ps.managed, COUNT(m.problem_id) \
             FROM problem_sets AS ps \
             LEFT JOIN problem_set_members AS m ON m.problem_set_id = ps.id \
             GROUP BY ps.id ORDER BY ps.slug LIMIT ?",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![limit.sql_limit()], |row| {
            Ok((
                ProblemSet {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    managed: row.get(4)?,
                },
                row.get(5)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    limit.check("problem-set", rows)
}

pub fn list_set_members(connection: &Connection, set_slug: &str) -> Result<Vec<SetMember>, String> {
    get_problem_set(connection, set_slug)?;
    let sql = format!(
        "SELECT {PROBLEM_COLUMNS}, m.ordinal, m.section \
         FROM problem_set_members AS m \
         JOIN problem_sets AS ps ON ps.id = m.problem_set_id \
         JOIN problems AS p ON p.id = m.problem_id \
         WHERE ps.slug = ? ORDER BY m.ordinal"
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    statement
        .query_map(params![set_slug], set_member_from_row)
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)
}

pub fn list_set_members_bounded(
    connection: &Connection,
    set_slug: &str,
    limit: RowLimit,
) -> Result<Vec<SetMember>, String> {
    get_problem_set(connection, set_slug)?;
    let sql = format!(
        "SELECT {PROBLEM_COLUMNS}, m.ordinal, m.section \
         FROM problem_set_members AS m \
         JOIN problem_sets AS ps ON ps.id = m.problem_set_id \
         JOIN problems AS p ON p.id = m.problem_id \
         WHERE ps.slug = ? ORDER BY m.ordinal LIMIT ?"
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    let rows = statement
        .query_map(params![set_slug, limit.sql_limit()], set_member_from_row)
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    limit.check("problem-set member", rows)
}

fn is_ascii_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn resolve_problem(
    connection: &Connection,
    reference: &str,
    set_slug: Option<&str>,
) -> Result<ResolvedProblem, String> {
    let Some(set_slug) = set_slug else {
        if is_ascii_decimal(reference) {
            return Err("a numeric problem selector requires a problem set".to_string());
        }
        let sql = format!("SELECT {PROBLEM_COLUMNS} FROM problems AS p WHERE p.slug = ?");
        let problem = connection
            .query_row(&sql, params![reference], problem_from_row)
            .optional()
            .map_err(sql_error)?
            .ok_or_else(|| format!("unknown problem: {reference}"))?;
        return Ok(ResolvedProblem {
            problem,
            membership: None,
        });
    };
    let problem_set = get_problem_set(connection, set_slug)?;
    let (selector, selector_value): (&str, rusqlite::types::Value) = if is_ascii_decimal(reference)
    {
        if reference == "0" || (reference.len() > 1 && reference.starts_with('0')) {
            return Err(format!("invalid 1-based problem index: {reference}"));
        }
        let ordinal: i64 = reference
            .parse()
            .map_err(|_| format!("invalid 1-based problem index: {reference}"))?;
        ("m.ordinal", ordinal.into())
    } else {
        ("p.slug", reference.to_string().into())
    };
    let sql = format!(
        "SELECT {PROBLEM_COLUMNS}, m.ordinal, m.section \
         FROM problem_set_members AS m \
         JOIN problems AS p ON p.id = m.problem_id \
         WHERE m.problem_set_id = ? AND {selector} = ?"
    );
    if let Some(member) = connection
        .query_row(
            &sql,
            params![problem_set.id, selector_value],
            set_member_from_row,
        )
        .optional()
        .map_err(sql_error)?
    {
        return Ok(ResolvedProblem {
            problem: member.problem,
            membership: Some(ProblemMembership {
                ordinal: member.ordinal,
                section: member.section,
            }),
        });
    }
    if is_ascii_decimal(reference) {
        return Err(format!(
            "problem index out of range for {set_slug}: {reference}"
        ));
    }
    let global_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM problems WHERE slug = ?)",
            params![reference],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if global_exists {
        Err(format!("problem is not in {set_slug}: {reference}"))
    } else {
        Err(format!("unknown problem: {reference}"))
    }
}

pub fn get_implementation(
    connection: &Connection,
    problem_id: i64,
    language_slug: &str,
) -> Result<ProblemImplementation, String> {
    let language_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM languages WHERE slug = ? AND enabled = 1",
            params![language_slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    let Some(language_id) = language_id else {
        return Err(format!("unknown or disabled language: {language_slug}"));
    };
    let implementation = connection
        .query_row(
            "SELECT l.slug, l.display_name, l.runner_path, i.solution_path \
             FROM problem_implementations AS i \
             JOIN languages AS l ON l.id = i.language_id \
             WHERE i.problem_id = ? AND i.language_id = ? \
               AND i.enabled = 1 AND l.enabled = 1",
            params![problem_id, language_id],
            |row| {
                Ok(ProblemImplementation {
                    language: EnabledLanguage {
                        slug: row.get(0)?,
                        display_name: row.get(1)?,
                        runner_path: row.get(2)?,
                    },
                    solution_path: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(sql_error)?;
    implementation.ok_or_else(|| {
        let slug = connection
            .query_row(
                "SELECT slug FROM problems WHERE id = ?",
                params![problem_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "<unknown>".to_string());
        format!("no active {language_slug} adapter for problem: {slug}")
    })
}

pub fn completed_problem_ids(
    connection: &Connection,
    language_slug: Option<&str>,
) -> Result<HashSet<i64>, String> {
    let (sql, language): (&str, Option<&str>) = if language_slug.is_some() {
        (
            "SELECT DISTINCT a.problem_id \
             FROM attempts AS a \
             JOIN problems AS p ON p.id = a.problem_id \
             JOIN languages AS l ON l.id = a.language_id \
             WHERE a.result = 'pass' AND a.test_revision = p.test_revision AND l.slug = ?",
            language_slug,
        )
    } else {
        (
            "SELECT DISTINCT a.problem_id \
             FROM attempts AS a \
             JOIN problems AS p ON p.id = a.problem_id \
             JOIN languages AS l ON l.id = a.language_id \
             WHERE a.result = 'pass' AND a.test_revision = p.test_revision",
            None,
        )
    };
    let mut statement = connection.prepare(sql).map_err(sql_error)?;
    let rows = if let Some(language) = language {
        statement
            .query_map(params![language], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(sql_error)?
    } else {
        statement
            .query_map([], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(sql_error)?
    };
    Ok(rows.into_iter().collect())
}

pub fn completed_problem_ids_bounded(
    connection: &Connection,
    language_slug: Option<&str>,
    limit: RowLimit,
) -> Result<HashSet<i64>, String> {
    let (sql, language) = if language_slug.is_some() {
        (
            "SELECT DISTINCT a.problem_id \
             FROM attempts AS a \
             JOIN problems AS p ON p.id = a.problem_id \
             JOIN languages AS l ON l.id = a.language_id \
             WHERE a.result = 'pass' AND a.test_revision = p.test_revision AND l.slug = ?1 \
             LIMIT ?2",
            language_slug,
        )
    } else {
        (
            "SELECT DISTINCT a.problem_id \
             FROM attempts AS a \
             JOIN problems AS p ON p.id = a.problem_id \
             JOIN languages AS l ON l.id = a.language_id \
             WHERE a.result = 'pass' AND a.test_revision = p.test_revision LIMIT ?1",
            None,
        )
    };
    let mut statement = connection.prepare(sql).map_err(sql_error)?;
    let rows = if let Some(language) = language {
        statement
            .query_map(params![language, limit.sql_limit()], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(sql_error)?
    } else {
        statement
            .query_map(params![limit.sql_limit()], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(sql_error)?
    };
    Ok(limit
        .check("completed-problem", rows)?
        .into_iter()
        .collect())
}

pub fn language_is_enabled(connection: &Connection, slug: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM languages WHERE slug = ? AND enabled = 1)",
            params![slug],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

pub fn get_enabled_language(
    connection: &Connection,
    slug: &str,
) -> Result<EnabledLanguage, String> {
    connection
        .query_row(
            "SELECT slug, display_name, runner_path FROM languages \
             WHERE slug = ? AND enabled = 1",
            params![slug],
            |row| {
                Ok(EnabledLanguage {
                    slug: row.get(0)?,
                    display_name: row.get(1)?,
                    runner_path: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(sql_error)?
        .ok_or_else(|| format!("unknown or disabled language: {slug}"))
}

pub fn list_enabled_languages(connection: &Connection) -> Result<Vec<EnabledLanguage>, String> {
    let mut statement = connection
        .prepare(
            "SELECT slug, display_name, runner_path FROM languages \
             WHERE enabled = 1 ORDER BY slug",
        )
        .map_err(sql_error)?;
    statement
        .query_map([], |row| {
            Ok(EnabledLanguage {
                slug: row.get(0)?,
                display_name: row.get(1)?,
                runner_path: row.get(2)?,
            })
        })
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)
}

pub fn list_enabled_languages_bounded(
    connection: &Connection,
    limit: RowLimit,
) -> Result<Vec<EnabledLanguage>, String> {
    let mut statement = connection
        .prepare(
            "SELECT slug, display_name, runner_path FROM languages \
             WHERE enabled = 1 ORDER BY slug LIMIT ?",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![limit.sql_limit()], |row| {
            Ok(EnabledLanguage {
                slug: row.get(0)?,
                display_name: row.get(1)?,
                runner_path: row.get(2)?,
            })
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let rows = limit.check("enabled-language", rows)?;
    for language in &rows {
        if language.slug.chars().count() > MAX_TITLE_LENGTH
            || language.display_name.chars().count() > MAX_TITLE_LENGTH
            || language.runner_path.chars().count() > MAX_DESCRIPTION_LENGTH
        {
            return Err("enabled-language row contains oversized text".to_string());
        }
    }
    Ok(rows)
}

pub fn list_enabled_implementations(
    connection: &Connection,
    problem_id: i64,
) -> Result<Vec<ProblemImplementation>, String> {
    let mut statement = connection
        .prepare(
            "SELECT l.slug, l.display_name, l.runner_path, i.solution_path \
             FROM problem_implementations AS i \
             JOIN languages AS l ON l.id = i.language_id \
             WHERE i.problem_id = ? AND i.enabled = 1 AND l.enabled = 1 \
             ORDER BY l.slug",
        )
        .map_err(sql_error)?;
    statement
        .query_map(params![problem_id], |row| {
            Ok(ProblemImplementation {
                language: EnabledLanguage {
                    slug: row.get(0)?,
                    display_name: row.get(1)?,
                    runner_path: row.get(2)?,
                },
                solution_path: row.get(3)?,
            })
        })
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)
}

pub fn list_enabled_implementations_bounded(
    connection: &Connection,
    problem_id: i64,
    limit: RowLimit,
) -> Result<Vec<ProblemImplementation>, String> {
    let mut statement = connection
        .prepare(
            "SELECT l.slug, l.display_name, l.runner_path, i.solution_path \
             FROM problem_implementations AS i \
             JOIN languages AS l ON l.id = i.language_id \
             WHERE i.problem_id = ? AND i.enabled = 1 AND l.enabled = 1 \
             ORDER BY l.slug LIMIT ?",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![problem_id, limit.sql_limit()], |row| {
            Ok(ProblemImplementation {
                language: EnabledLanguage {
                    slug: row.get(0)?,
                    display_name: row.get(1)?,
                    runner_path: row.get(2)?,
                },
                solution_path: row.get(3)?,
            })
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    limit.check("implementation", rows)
}

pub fn list_global_problems(
    connection: &Connection,
    include_archived: bool,
) -> Result<Vec<ProblemListRow>, String> {
    let archived_clause = if include_archived {
        ""
    } else {
        "WHERE p.archived = 0"
    };
    let problem_sql = format!(
        "SELECT {PROBLEM_COLUMNS} FROM problems AS p \
         {archived_clause} ORDER BY p.slug"
    );
    let problems = {
        let mut statement = connection.prepare(&problem_sql).map_err(sql_error)?;
        statement
            .query_map([], problem_from_row)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?
    };

    let implementation_sql = format!(
        "SELECT p.id, l.slug, l.display_name, l.runner_path, i.solution_path \
         FROM problems AS p \
         JOIN problem_implementations AS i ON i.problem_id = p.id \
         JOIN languages AS l ON l.id = i.language_id \
         {archived_clause} \
         AND i.enabled = 1 AND l.enabled = 1 \
         ORDER BY p.id, l.slug"
    );
    let mut implementations_by_problem: HashMap<i64, Vec<ProblemImplementation>> = HashMap::new();
    {
        let mut statement = connection.prepare(&implementation_sql).map_err(sql_error)?;
        let implementations = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    ProblemImplementation {
                        language: EnabledLanguage {
                            slug: row.get(1)?,
                            display_name: row.get(2)?,
                            runner_path: row.get(3)?,
                        },
                        solution_path: row.get(4)?,
                    },
                ))
            })
            .map_err(sql_error)?;
        for implementation in implementations {
            let (problem_id, implementation) = implementation.map_err(sql_error)?;
            implementations_by_problem
                .entry(problem_id)
                .or_default()
                .push(implementation);
        }
    }

    Ok(problems
        .into_iter()
        .map(|problem| {
            let implementations = implementations_by_problem
                .remove(&problem.id)
                .unwrap_or_default();
            ProblemListRow {
                problem,
                implementations,
            }
        })
        .collect())
}

pub fn list_active_global_problems(connection: &Connection) -> Result<Vec<Problem>, String> {
    let sql = format!(
        "SELECT {PROBLEM_COLUMNS} FROM problems AS p \
         WHERE p.archived = 0 ORDER BY p.slug"
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    statement
        .query_map([], problem_from_row)
        .map_err(sql_error)?
        .collect::<Result<_, _>>()
        .map_err(sql_error)
}

pub fn list_active_global_problems_bounded(
    connection: &Connection,
    limit: RowLimit,
) -> Result<Vec<Problem>, String> {
    let sql = format!(
        "SELECT {PROBLEM_COLUMNS} FROM problems AS p \
         WHERE p.archived = 0 ORDER BY p.slug LIMIT ?"
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    let rows = statement
        .query_map(params![limit.sql_limit()], problem_from_row)
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    limit.check("problem", rows)
}

pub fn progress_summary(
    connection: &Connection,
    scope: ProgressScope<'_>,
    language_slug: Option<&str>,
) -> Result<ProgressSummary, String> {
    progress_summary_bounded(
        connection,
        scope,
        language_slug,
        RowLimit::new(MAX_DATABASE_QUERY_ROWS).expect("database maximum is a valid row limit"),
    )
}

pub fn progress_summary_bounded(
    connection: &Connection,
    scope: ProgressScope<'_>,
    language_slug: Option<&str>,
    limit: RowLimit,
) -> Result<ProgressSummary, String> {
    if let Some(language_slug) = language_slug
        && !language_is_enabled(connection, language_slug)?
    {
        return Err(format!("unknown or disabled language: {language_slug}"));
    }

    let (sql, set_id) = match scope {
        ProgressScope::Global => (
            "SELECT p.id, p.difficulty, p.topic, \
                EXISTS(SELECT 1 FROM attempts AS a \
                       JOIN languages AS l ON l.id = a.language_id \
                       WHERE a.problem_id = p.id AND a.result = 'pass' \
                         AND a.test_revision = p.test_revision \
                         AND (?1 IS NULL OR l.slug = ?1)) \
             FROM problems AS p WHERE p.archived = 0 AND ?2 IS NULL ORDER BY p.slug LIMIT ?3",
            None,
        ),
        ProgressScope::ProblemSet(set_slug) => (
            "SELECT p.id, p.difficulty, p.topic, \
                EXISTS(SELECT 1 FROM attempts AS a \
                       JOIN languages AS l ON l.id = a.language_id \
                       WHERE a.problem_id = p.id AND a.result = 'pass' \
                         AND a.test_revision = p.test_revision \
                         AND (?1 IS NULL OR l.slug = ?1)) \
             FROM problem_set_members AS m \
             JOIN problems AS p ON p.id = m.problem_id \
             WHERE m.problem_set_id = ?2 ORDER BY m.ordinal LIMIT ?3",
            Some(get_problem_set(connection, set_slug)?.id),
        ),
    };

    let mut statement = connection.prepare(sql).map_err(sql_error)?;
    let rows = statement
        .query_map(params![language_slug, set_id, limit.sql_limit()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Difficulty>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .map_err(sql_error)?;
    let mut total = 0_usize;
    let mut completed = 0_usize;
    let mut difficulties: BTreeMap<Difficulty, (usize, usize)> = BTreeMap::new();
    let mut topics: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut topic_order = Vec::new();
    for row in rows {
        let (_problem_id, difficulty, topic, is_completed) = row.map_err(sql_error)?;
        if topic.chars().count() > MAX_TOPIC_LENGTH {
            return Err(format!(
                "progress topic exceeds {MAX_TOPIC_LENGTH} characters"
            ));
        }
        let completed_increment = usize::from(is_completed);
        total += 1;
        completed += completed_increment;
        let difficulty_progress = difficulties.entry(difficulty).or_default();
        difficulty_progress.0 += completed_increment;
        difficulty_progress.1 += 1;
        if !topics.contains_key(&topic) {
            topic_order.push(topic.clone());
        }
        let topic_progress = topics.entry(topic).or_default();
        topic_progress.0 += completed_increment;
        topic_progress.1 += 1;
    }

    if total > limit.0 {
        return Err(format!(
            "progress-input row limit exceeded: maximum {}",
            limit.0
        ));
    }

    Ok(ProgressSummary {
        completed,
        total,
        by_difficulty: difficulties
            .into_iter()
            .map(|(difficulty, (completed, total))| DifficultyProgress {
                difficulty,
                completed,
                total,
            })
            .collect(),
        by_topic: topic_order
            .into_iter()
            .map(|topic| {
                let (completed, total) = topics
                    .remove(&topic)
                    .expect("topic order and aggregate map stay synchronized");
                TopicProgress {
                    topic,
                    completed,
                    total,
                }
            })
            .collect(),
    })
}

pub fn record_attempt(
    connection: &Connection,
    problem_slug: &str,
    language_slug: &str,
    result: AttemptOutcome,
    duration_ms: i64,
    exit_code: Option<i32>,
    set_slug: Option<&str>,
) -> Result<i64, String> {
    if duration_ms < 0 {
        return Err("duration must not be negative".to_string());
    }
    let problem = resolve_problem(connection, problem_slug, None)?.problem;
    let language_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM languages WHERE slug = ?",
            params![language_slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    let Some(language_id) = language_id else {
        return Err(format!("unknown language: {language_slug}"));
    };
    let set_id = if let Some(set_slug) = set_slug {
        resolve_problem(connection, problem_slug, Some(set_slug))?;
        Some(get_problem_set(connection, set_slug)?.id)
    } else {
        None
    };
    connection
        .execute(
            "INSERT INTO attempts( \
                problem_id, language_id, invoked_set_id, result, test_revision, \
                duration_ms, exit_code, run_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            params![
                problem.id,
                language_id,
                set_id,
                result,
                problem.test_revision,
                duration_ms,
                exit_code
            ],
        )
        .map_err(sql_error)?;
    let attempt_id = connection.last_insert_rowid();
    assert!(attempt_id > 0);
    Ok(attempt_id)
}

pub fn finalize_attempt_cancelled(
    connection: &Connection,
    attempt_id: i64,
    signal_exit_code: i32,
) -> Result<(), String> {
    if attempt_id <= 0 {
        return Err("attempt id must be positive".to_string());
    }
    if !matches!(signal_exit_code, 130 | 143) {
        return Err("cancelled signal exit code must be 130 or 143".to_string());
    }
    let transaction = connection.unchecked_transaction().map_err(sql_error)?;
    let updated = transaction
        .execute(
            "UPDATE attempts SET result = ?, exit_code = ? WHERE id = ?",
            params![AttemptOutcome::Cancelled, signal_exit_code, attempt_id],
        )
        .map_err(sql_error)?;
    if updated != 1 {
        return Err(format!(
            "attempt finalization updated {updated} rows; expected 1"
        ));
    }
    transaction.commit().map_err(sql_error)
}

pub struct NewProblem<'a> {
    pub slug: &'a str,
    pub title: &'a str,
    pub difficulty: Difficulty,
    pub topic: &'a str,
    pub statement_markdown: &'a str,
    pub leetcode_id: Option<i64>,
    pub leetcode_url: &'a str,
    pub neetcode_url: &'a str,
    pub premium: bool,
}

fn validate_url(label: &str, url: &str) -> Result<(), String> {
    if url.is_empty() {
        Ok(())
    } else {
        validate_http_url(label, url)
    }
}

pub fn create_problem(connection: &Connection, new: &NewProblem<'_>) -> Result<(), String> {
    validate_identifier(new.slug, "problem slug", true)?;
    let title = new.title.trim();
    let topic = new.topic.trim();
    if title.is_empty() {
        return Err("problem title must not be empty".to_string());
    }
    if title.chars().count() > MAX_TITLE_LENGTH {
        return Err(format!(
            "problem title exceeds {MAX_TITLE_LENGTH} characters"
        ));
    }
    if topic.is_empty() {
        return Err("problem topic must not be empty".to_string());
    }
    if topic.chars().count() > MAX_TOPIC_LENGTH {
        return Err(format!(
            "problem topic exceeds {MAX_TOPIC_LENGTH} characters"
        ));
    }
    if new.statement_markdown.chars().count() > MAX_STATEMENT_LENGTH {
        return Err(format!(
            "problem statement exceeds {MAX_STATEMENT_LENGTH} characters"
        ));
    }
    if new.leetcode_id.is_some_and(|id| id <= 0) {
        return Err("LeetCode id must be positive".to_string());
    }
    validate_url("LeetCode", new.leetcode_url)?;
    validate_url("NeetCode", new.neetcode_url)?;
    let now = timestamp(connection)?;
    connection
        .execute(
            "INSERT INTO problems( \
                slug, title, difficulty, topic, leetcode_id, premium, managed, archived, \
                leetcode_url, neetcode_url, statement_markdown, test_revision, \
                created_at, updated_at \
             ) VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, 1, ?, ?)",
            params![
                new.slug,
                title,
                new.difficulty,
                topic,
                new.leetcode_id,
                new.premium,
                new.leetcode_url,
                new.neetcode_url,
                new.statement_markdown,
                now,
                now
            ],
        )
        .map_err(|error| {
            constraint_error(
                error,
                format!("problem already exists or conflicts: {}", new.slug),
            )
        })?;
    Ok(())
}

pub fn create_problem_set(
    connection: &Connection,
    slug: &str,
    name: &str,
    description: &str,
) -> Result<(), String> {
    validate_identifier(slug, "problem-set id", false)?;
    let name = name.trim();
    if name.is_empty() {
        return Err("problem-set name must not be empty".to_string());
    }
    if name.chars().count() > MAX_TITLE_LENGTH {
        return Err(format!(
            "problem-set name exceeds {MAX_TITLE_LENGTH} characters"
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION_LENGTH {
        return Err(format!(
            "problem-set description exceeds {MAX_DESCRIPTION_LENGTH} characters"
        ));
    }
    let now = timestamp(connection)?;
    connection
        .execute(
            "INSERT INTO problem_sets( \
                slug, name, description, managed, created_at, updated_at \
             ) VALUES (?, ?, ?, 0, ?, ?)",
            params![slug, name, description, now, now],
        )
        .map_err(|error| constraint_error(error, format!("problem set already exists: {slug}")))?;
    Ok(())
}

fn mutable_problem_set(connection: &Connection, slug: &str) -> Result<ProblemSet, String> {
    let problem_set = get_problem_set(connection, slug)?;
    if problem_set.managed {
        Err(format!("shipped problem set is read-only: {slug}"))
    } else {
        Ok(problem_set)
    }
}

fn validate_member_order(connection: &Connection, set_id: i64) -> Result<usize, String> {
    let mut statement = connection
        .prepare(
            "SELECT ordinal FROM problem_set_members \
             WHERE problem_set_id = ? ORDER BY ordinal",
        )
        .map_err(sql_error)?;
    let ordinals = statement
        .query_map(params![set_id], |row| row.get::<_, i64>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    if ordinals
        .iter()
        .enumerate()
        .any(|(index, ordinal)| *ordinal != (index + 1) as i64)
    {
        Err("problem-set membership order is not contiguous".to_string())
    } else {
        Ok(ordinals.len())
    }
}

pub fn add_set_member(
    connection: &Connection,
    set_slug: &str,
    problem_slug: &str,
    index: Option<i64>,
    section: Option<&str>,
) -> Result<(), String> {
    transaction(connection, || {
        let problem_set = mutable_problem_set(connection, set_slug)?;
        let problem = resolve_problem(connection, problem_slug, None)?.problem;
        let count = validate_member_order(connection, problem_set.id)?;
        if count >= 999_999 {
            return Err("problem set reached its maximum size".to_string());
        }
        let target = index.unwrap_or(count as i64 + 1);
        if target < 1 || target > count as i64 + 1 {
            return Err(format!("index must be between 1 and {}", count + 1));
        }
        connection
            .execute(
                "UPDATE problem_set_members SET ordinal = ordinal + 1000000 \
                 WHERE problem_set_id = ? AND ordinal >= ?",
                params![problem_set.id, target],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "UPDATE problem_set_members SET ordinal = ordinal - 999999 \
                 WHERE problem_set_id = ? AND ordinal >= ?",
                params![problem_set.id, target + 1_000_000],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal, section) \
                 VALUES (?, ?, ?, ?)",
                params![problem_set.id, problem.id, target, section],
            )
            .map_err(|error| {
                constraint_error(error, format!("problem is already in set: {problem_slug}"))
            })?;
        connection
            .execute(
                "UPDATE problem_sets SET updated_at = \
                    strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
                params![problem_set.id],
            )
            .map_err(sql_error)?;
        Ok(())
    })
}

pub fn remove_set_member(
    connection: &Connection,
    set_slug: &str,
    problem_slug: &str,
) -> Result<(), String> {
    transaction(connection, || {
        let problem_set = mutable_problem_set(connection, set_slug)?;
        validate_member_order(connection, problem_set.id)?;
        let ordinal: Option<i64> = connection
            .query_row(
                "SELECT m.ordinal FROM problem_set_members AS m \
                 JOIN problems AS p ON p.id = m.problem_id \
                 WHERE m.problem_set_id = ? AND p.slug = ?",
                params![problem_set.id, problem_slug],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let Some(ordinal) = ordinal else {
            return Err(format!("problem is not in {set_slug}: {problem_slug}"));
        };
        connection
            .execute(
                "DELETE FROM problem_set_members WHERE problem_set_id = ? AND ordinal = ?",
                params![problem_set.id, ordinal],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "UPDATE problem_set_members SET ordinal = ordinal + 1000000 \
                 WHERE problem_set_id = ? AND ordinal > ?",
                params![problem_set.id, ordinal],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "UPDATE problem_set_members SET ordinal = ordinal - 1000001 \
                 WHERE problem_set_id = ? AND ordinal > 1000000",
                params![problem_set.id],
            )
            .map_err(sql_error)?;
        connection
            .execute(
                "UPDATE problem_sets SET updated_at = \
                    strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
                params![problem_set.id],
            )
            .map_err(sql_error)?;
        Ok(())
    })
}

pub fn move_set_member(
    connection: &Connection,
    set_slug: &str,
    problem_slug: &str,
    index: i64,
) -> Result<(), String> {
    transaction(connection, || {
        let problem_set = mutable_problem_set(connection, set_slug)?;
        validate_member_order(connection, problem_set.id)?;
        let rows = {
            let mut statement = connection
                .prepare(
                    "SELECT m.problem_id, p.slug FROM problem_set_members AS m \
                     JOIN problems AS p ON p.id = m.problem_id \
                     WHERE m.problem_set_id = ? ORDER BY m.ordinal",
                )
                .map_err(sql_error)?;
            statement
                .query_map(params![problem_set.id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        if index < 1 || index > rows.len() as i64 {
            return Err(format!("index must be between 1 and {}", rows.len()));
        }
        let Some(selected_index) = rows.iter().position(|(_, slug)| slug == problem_slug) else {
            return Err(format!("problem is not in {set_slug}: {problem_slug}"));
        };
        let mut problem_ids: Vec<i64> = rows.into_iter().map(|(id, _)| id).collect();
        let selected = problem_ids.remove(selected_index);
        problem_ids.insert(index as usize - 1, selected);
        connection
            .execute(
                "UPDATE problem_set_members SET ordinal = ordinal + 1000000 \
                 WHERE problem_set_id = ?",
                params![problem_set.id],
            )
            .map_err(sql_error)?;
        for (ordinal, problem_id) in problem_ids.iter().enumerate() {
            connection
                .execute(
                    "UPDATE problem_set_members SET ordinal = ? \
                     WHERE problem_set_id = ? AND problem_id = ?",
                    params![ordinal as i64 + 1, problem_set.id, problem_id],
                )
                .map_err(sql_error)?;
        }
        connection
            .execute(
                "UPDATE problem_sets SET updated_at = \
                    strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
                params![problem_set.id],
            )
            .map_err(sql_error)?;
        Ok(())
    })
}

pub fn add_implementation(
    connection: &Connection,
    problem_slug: &str,
    language_slug: &str,
    solution_path: &str,
) -> Result<(), String> {
    let problem = resolve_problem(connection, problem_slug, None)?.problem;
    if problem.managed {
        return Err(format!("shipped problem is read-only: {problem_slug}"));
    }
    let language_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM languages WHERE slug = ?",
            params![language_slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    let Some(language_id) = language_id else {
        return Err(format!("unknown language: {language_slug}"));
    };
    let path = Path::new(solution_path);
    if path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err("solution path must be relative to the project root".to_string());
    }
    let first = path.components().next().and_then(|part| match part {
        Component::Normal(value) => value.to_str(),
        _ => None,
    });
    if first != Some(language_slug) {
        return Err(format!("solution path must be inside {language_slug}/"));
    }
    connection
        .execute(
            "INSERT INTO problem_implementations( \
                problem_id, language_id, solution_path, enabled \
             ) VALUES (?, ?, ?, 1) \
             ON CONFLICT(problem_id, language_id) DO UPDATE SET \
                solution_path = excluded.solution_path, enabled = 1",
            params![problem.id, language_id, solution_path],
        )
        .map_err(sql_error)?;
    Ok(())
}

pub struct ProblemUpdate<'a> {
    pub title: Option<&'a str>,
    pub difficulty: Option<Difficulty>,
    pub topic: Option<&'a str>,
    pub statement_markdown: Option<&'a str>,
    pub test_revision: Option<i64>,
    pub leetcode_id: Option<Option<i64>>,
    pub premium: Option<bool>,
    pub leetcode_url: Option<&'a str>,
    pub neetcode_url: Option<&'a str>,
}

pub fn update_problem(
    connection: &Connection,
    slug: &str,
    update: &ProblemUpdate<'_>,
) -> Result<(), String> {
    let problem = resolve_problem(connection, slug, None)?.problem;
    if problem.managed {
        return Err(format!("shipped problem is read-only: {slug}"));
    }
    let title = update.title.unwrap_or(&problem.title).trim();
    let difficulty = update.difficulty.unwrap_or(problem.difficulty);
    let topic = update.topic.unwrap_or(&problem.topic).trim();
    let statement = update
        .statement_markdown
        .unwrap_or(&problem.statement_markdown);
    let revision = update.test_revision.unwrap_or(problem.test_revision);
    let leetcode_id = update.leetcode_id.unwrap_or(problem.leetcode_id);
    let premium = update.premium.unwrap_or(problem.premium);
    let leetcode_url = update.leetcode_url.unwrap_or(&problem.leetcode_url);
    let neetcode_url = update.neetcode_url.unwrap_or(&problem.neetcode_url);
    if revision < problem.test_revision {
        return Err("test revision cannot decrease".to_string());
    }
    if revision <= 0 {
        return Err("test revision must be positive".to_string());
    }
    if leetcode_id.is_some_and(|id| id <= 0) {
        return Err("LeetCode id must be positive".to_string());
    }
    if title.is_empty() || title.chars().count() > MAX_TITLE_LENGTH {
        return Err("problem title is empty or too long".to_string());
    }
    if topic.is_empty() || topic.chars().count() > MAX_TOPIC_LENGTH {
        return Err("problem topic is empty or too long".to_string());
    }
    if statement.chars().count() > MAX_STATEMENT_LENGTH {
        return Err(format!(
            "problem statement exceeds {MAX_STATEMENT_LENGTH} characters"
        ));
    }
    validate_url("LeetCode", leetcode_url)?;
    validate_url("NeetCode", neetcode_url)?;
    connection
        .execute(
            "UPDATE problems SET \
                title = ?, difficulty = ?, topic = ?, statement_markdown = ?, \
                test_revision = ?, leetcode_id = ?, premium = ?, \
                leetcode_url = ?, neetcode_url = ?, \
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE id = ?",
            params![
                title,
                difficulty,
                topic,
                statement,
                revision,
                leetcode_id,
                premium,
                leetcode_url,
                neetcode_url,
                problem.id
            ],
        )
        .map_err(|error| {
            constraint_error(
                error,
                "LeetCode id is already assigned to another problem".to_string(),
            )
        })?;
    Ok(())
}

pub fn delete_problem(connection: &Connection, slug: &str) -> Result<(), String> {
    let problem = resolve_problem(connection, slug, None)?.problem;
    if problem.managed {
        return Err(format!("shipped problem is read-only: {slug}"));
    }
    let memberships: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM problem_set_members WHERE problem_id = ?",
            params![problem.id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let attempts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM attempts WHERE problem_id = ?",
            params![problem.id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if memberships != 0 || attempts != 0 {
        return Err(format!(
            "cannot delete {slug}: {memberships} memberships and {attempts} attempts remain"
        ));
    }
    transaction(connection, || {
        connection
            .execute(
                "DELETE FROM problem_implementations WHERE problem_id = ?",
                params![problem.id],
            )
            .map_err(sql_error)?;
        connection
            .execute("DELETE FROM problems WHERE id = ?", params![problem.id])
            .map_err(sql_error)?;
        Ok(())
    })
}

pub fn update_problem_set(
    connection: &Connection,
    slug: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<(), String> {
    let problem_set = mutable_problem_set(connection, slug)?;
    let name = name.unwrap_or(&problem_set.name).trim();
    let description = description.unwrap_or(&problem_set.description);
    if name.is_empty() || name.chars().count() > MAX_TITLE_LENGTH {
        return Err("problem-set name is empty or too long".to_string());
    }
    if description.chars().count() > MAX_DESCRIPTION_LENGTH {
        return Err(format!(
            "problem-set description exceeds {MAX_DESCRIPTION_LENGTH} characters"
        ));
    }
    connection
        .execute(
            "UPDATE problem_sets SET name = ?, description = ?, \
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
            params![name, description, problem_set.id],
        )
        .map_err(sql_error)?;
    Ok(())
}

pub fn delete_problem_set(connection: &Connection, slug: &str) -> Result<(), String> {
    let problem_set = mutable_problem_set(connection, slug)?;
    connection
        .execute(
            "DELETE FROM problem_sets WHERE id = ?",
            params![problem_set.id],
        )
        .map_err(sql_error)?;
    Ok(())
}
