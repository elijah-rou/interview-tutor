from __future__ import annotations

import datetime as dt
from pathlib import Path
import sqlite3

from practice_tool.catalog import load_seed_catalog, validate_identifier
from practice_tool.models import ProblemSeed, ProblemSetSeed

SCHEMA_VERSION = 2
MAX_TITLE_LENGTH = 200
MAX_TOPIC_LENGTH = 100
MAX_DESCRIPTION_LENGTH = 2_000
MAX_STATEMENT_LENGTH = 1_000_000


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).isoformat(timespec="seconds")


def open_database(path: Path, root: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(path, timeout=5.0)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA busy_timeout = 5000")
    connection.execute("PRAGMA journal_mode = WAL")
    connection.execute("PRAGMA foreign_keys = OFF")
    try:
        _migrate_if_needed(connection)
        connection.execute("PRAGMA foreign_keys = ON")
        _ensure_languages(connection)
        revision, problems, problem_sets = load_seed_catalog(root)
        _sync_seed_catalog(connection, revision, problems, problem_sets)
        violations = connection.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise RuntimeError(f"database foreign-key violations: {violations!r}")
        return connection
    except Exception:
        connection.close()
        raise


def _schema_sql() -> str:
    return """
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
    """


def _execute_schema_statements(connection: sqlite3.Connection) -> None:
    for statement in _schema_sql().split(";"):
        if statement.strip():
            connection.execute(statement)


def _migrate_if_needed(connection: sqlite3.Connection) -> None:
    problems_exists = connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'problems'"
    ).fetchone()
    if not problems_exists:
        with connection:
            connection.executescript(_schema_sql())
            connection.execute(f"PRAGMA user_version = {SCHEMA_VERSION}")
        return

    columns = {row["name"] for row in connection.execute("PRAGMA table_info(problems)")}
    if "problem_set_id" not in columns:
        connection.executescript(_schema_sql())
        connection.execute(f"PRAGMA user_version = {SCHEMA_VERSION}")
        return

    _migrate_v1(connection)


def _migrate_v1(connection: sqlite3.Connection) -> None:
    # Foreign keys must remain disabled for the table swap. The whole migration
    # commits once, so an interrupted migration leaves the v1 database intact.
    timestamp = utc_now()
    connection.execute("BEGIN IMMEDIATE")
    try:
        connection.execute("ALTER TABLE attempts RENAME TO legacy_attempts")
        connection.execute("ALTER TABLE problems RENAME TO legacy_problems")
        connection.execute("ALTER TABLE problem_sets RENAME TO legacy_problem_sets")
        legacy_problems = connection.execute(
            "SELECT * FROM legacy_problems ORDER BY problem_set_id, slug"
        ).fetchall()
        metadata_by_slug: dict[str, tuple[object, ...]] = {}
        external_id_owners: dict[int, set[str]] = {}
        for row in legacy_problems:
            metadata = (
                row["title"],
                row["difficulty"],
                row["topic"],
                row["external_id"],
                row["premium"],
                row["test_revision"],
            )
            existing = metadata_by_slug.setdefault(row["slug"], metadata)
            if existing != metadata:
                raise RuntimeError(
                    f"v1 problem metadata conflicts across sets: {row['slug']}"
                )
            if row["external_id"] is not None:
                external_id_owners.setdefault(row["external_id"], set()).add(
                    row["slug"]
                )

        _execute_schema_statements(connection)
        connection.executemany(
            "INSERT INTO languages(slug, display_name, runner_path) VALUES (?, ?, ?)",
            (("python", "Python", "python/run"), ("rust", "Rust", "rust/run")),
        )
        legacy_languages = [
            row["language"]
            for row in connection.execute(
                "SELECT DISTINCT language FROM legacy_attempts ORDER BY language"
            )
            if row["language"] not in ("python", "rust")
        ]
        connection.executemany(
            """
            INSERT INTO languages(slug, display_name, runner_path, enabled)
            VALUES (?, ?, ?, 0)
            """,
            [
                (language, language, f"legacy-language-{index}/run")
                for index, language in enumerate(legacy_languages, 1)
            ],
        )
        conflicting_external_ids = {
            external_id: owners
            for external_id, owners in external_id_owners.items()
            if len(owners) > 1
        }
        if conflicting_external_ids:
            raise RuntimeError("v1 LeetCode ids conflict across global problem slugs")
        active_slugs = {
            row["slug"] for row in legacy_problems if row["ordinal"] < 1_000_000
        }
        problem_values = []
        for slug, metadata in metadata_by_slug.items():
            title, difficulty, topic, external_id, premium, test_revision = metadata
            problem_values.append(
                (
                    slug,
                    title,
                    difficulty,
                    topic,
                    external_id,
                    premium,
                    int(slug not in active_slugs),
                    test_revision,
                    timestamp,
                    timestamp,
                )
            )
        connection.executemany(
            """
            INSERT INTO problems(
                slug, title, difficulty, topic, leetcode_id, premium, managed, archived,
                leetcode_url, neetcode_url, statement_markdown, test_revision,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, '', '', '', ?, ?, ?)
            """,
            problem_values,
        )
        connection.execute(
            """
            INSERT INTO problem_sets(slug, name, description, managed, created_at, updated_at)
            SELECT id, name, '', 0, ?, ? FROM legacy_problem_sets
            """,
            (timestamp, timestamp),
        )
        connection.execute(
            """
            INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal)
            SELECT ps.id, p.id, lp.ordinal
            FROM legacy_problems AS lp
            JOIN problem_sets AS ps ON ps.slug = lp.problem_set_id
            JOIN problems AS p ON p.slug = lp.slug
            WHERE lp.ordinal < 1000000
            """
        )
        legacy_attempt_count = connection.execute(
            "SELECT COUNT(*) AS count FROM legacy_attempts"
        ).fetchone()["count"]
        connection.execute(
            """
            INSERT INTO attempts(
                id, problem_id, language_id, invoked_set_id, result,
                test_revision, duration_ms, exit_code, run_at
            )
            SELECT la.id, p.id, l.id, ps.id,
                   CASE WHEN la.passed = 1 THEN 'pass' ELSE 'fail' END,
                   la.test_revision, la.duration_ms, NULL, la.run_at
            FROM legacy_attempts AS la
            JOIN problems AS p ON p.slug = la.problem_slug
            JOIN languages AS l ON l.slug = la.language
            LEFT JOIN problem_sets AS ps ON ps.slug = la.problem_set_id
            """
        )
        migrated_attempt_count = connection.execute(
            "SELECT COUNT(*) AS count FROM attempts"
        ).fetchone()["count"]
        if migrated_attempt_count != legacy_attempt_count:
            raise RuntimeError(
                "v1 attempt migration count mismatch: "
                f"{migrated_attempt_count} != {legacy_attempt_count}"
            )
        connection.execute("DROP TABLE legacy_attempts")
        connection.execute("DROP TABLE legacy_problems")
        connection.execute("DROP TABLE legacy_problem_sets")
        _execute_schema_statements(connection)
        connection.execute(f"PRAGMA user_version = {SCHEMA_VERSION}")
        connection.commit()
    except Exception:
        connection.rollback()
        raise


def _ensure_languages(connection: sqlite3.Connection) -> None:
    with connection:
        connection.executemany(
            """
            INSERT INTO languages(slug, display_name, runner_path)
            VALUES (?, ?, ?)
            ON CONFLICT(slug) DO UPDATE SET
                display_name = excluded.display_name,
                runner_path = excluded.runner_path
            """,
            (("python", "Python", "python/run"), ("rust", "Rust", "rust/run")),
        )


def _sync_seed_catalog(
    connection: sqlite3.Connection,
    revision: int,
    problems: tuple[ProblemSeed, ...],
    problem_sets: tuple[ProblemSetSeed, ...],
) -> None:
    current_row = connection.execute(
        "SELECT value FROM metadata WHERE key = 'catalog_revision'"
    ).fetchone()
    current_revision = int(current_row["value"]) if current_row else 0
    if current_revision >= revision:
        return
    timestamp = utc_now()
    connection.execute("BEGIN IMMEDIATE")
    try:
        for problem in problems:
            existing_problem = connection.execute(
                "SELECT managed FROM problems WHERE slug = ?", (problem.slug,)
            ).fetchone()
            if (
                current_revision > 0
                and existing_problem is not None
                and existing_problem["managed"] == 0
            ):
                raise RuntimeError(
                    f"shipped catalog conflicts with local problem: {problem.slug}"
                )
            if problem.leetcode_id is not None:
                external_id_conflict = connection.execute(
                    "SELECT slug, managed FROM problems "
                    "WHERE leetcode_id = ? AND slug <> ?",
                    (problem.leetcode_id, problem.slug),
                ).fetchone()
                if (
                    external_id_conflict is not None
                    and external_id_conflict["managed"] == 0
                ):
                    raise RuntimeError(
                        "shipped catalog LeetCode id conflicts with local problem: "
                        f"{external_id_conflict['slug']}"
                    )
                connection.execute(
                    "UPDATE problems SET leetcode_id = NULL, updated_at = ? "
                    "WHERE leetcode_id = ? AND slug <> ?",
                    (timestamp, problem.leetcode_id, problem.slug),
                )
            connection.execute(
                """
                INSERT INTO problems(
                    slug, title, difficulty, topic, leetcode_id, premium, managed, archived,
                    leetcode_url, neetcode_url, statement_markdown, test_revision,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, 1, 0, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(slug) DO UPDATE SET
                    title = excluded.title,
                    difficulty = excluded.difficulty,
                    topic = excluded.topic,
                    leetcode_id = excluded.leetcode_id,
                    premium = excluded.premium,
                    managed = 1,
                    archived = 0,
                    leetcode_url = excluded.leetcode_url,
                    neetcode_url = excluded.neetcode_url,
                    statement_markdown = CASE
                        WHEN problems.statement_markdown = ''
                        THEN excluded.statement_markdown
                        ELSE problems.statement_markdown
                    END,
                    test_revision = excluded.test_revision,
                    updated_at = excluded.updated_at
                """,
                (
                    problem.slug,
                    problem.title,
                    problem.difficulty,
                    problem.topic,
                    problem.leetcode_id,
                    int(problem.premium),
                    problem.leetcode_url,
                    problem.neetcode_url,
                    problem.statement_markdown,
                    problem.test_revision,
                    timestamp,
                    timestamp,
                ),
            )
            problem_id = connection.execute(
                "SELECT id FROM problems WHERE slug = ?", (problem.slug,)
            ).fetchone()["id"]
            for adapter in problem.adapters:
                language = connection.execute(
                    "SELECT id FROM languages WHERE slug = ?", (adapter.language,)
                ).fetchone()
                if language is None:
                    raise RuntimeError(
                        f"catalog references unknown language: {adapter.language}"
                    )
                connection.execute(
                    """
                    INSERT INTO problem_implementations(
                        problem_id, language_id, solution_path, enabled
                    ) VALUES (?, ?, ?, 1)
                    ON CONFLICT(problem_id, language_id) DO UPDATE SET
                        solution_path = excluded.solution_path,
                        enabled = 1
                    """,
                    (problem_id, language["id"], adapter.solution_path),
                )
        for problem_set in problem_sets:
            existing_set = connection.execute(
                "SELECT managed FROM problem_sets WHERE slug = ?", (problem_set.id,)
            ).fetchone()
            if (
                current_revision > 0
                and existing_set is not None
                and existing_set["managed"] == 0
            ):
                raise RuntimeError(
                    f"shipped catalog conflicts with local problem set: {problem_set.id}"
                )
            connection.execute(
                """
                INSERT INTO problem_sets(
                    slug, name, description, managed, created_at, updated_at
                ) VALUES (?, ?, ?, 1, ?, ?)
                ON CONFLICT(slug) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    managed = 1,
                    updated_at = excluded.updated_at
                """,
                (
                    problem_set.id,
                    problem_set.name,
                    problem_set.description,
                    timestamp,
                    timestamp,
                ),
            )
            set_id = connection.execute(
                "SELECT id FROM problem_sets WHERE slug = ?", (problem_set.id,)
            ).fetchone()["id"]
            connection.execute(
                "DELETE FROM problem_set_members WHERE problem_set_id = ?", (set_id,)
            )
            for member in problem_set.members:
                problem_id = connection.execute(
                    "SELECT id FROM problems WHERE slug = ?", (member.problem_slug,)
                ).fetchone()["id"]
                connection.execute(
                    """
                    INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal)
                    VALUES (?, ?, ?)
                    """,
                    (set_id, problem_id, member.ordinal),
                )
        connection.execute(
            """
            INSERT INTO metadata(key, value) VALUES ('catalog_revision', ?)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            """,
            (str(revision),),
        )
        connection.commit()
    except Exception:
        connection.rollback()
        raise


def get_problem_set(connection: sqlite3.Connection, slug: str) -> sqlite3.Row:
    row = connection.execute(
        "SELECT * FROM problem_sets WHERE slug = ?", (slug,)
    ).fetchone()
    if row is None:
        raise ValueError(f"unknown problem set: {slug}")
    return row


def list_set_members(
    connection: sqlite3.Connection, set_slug: str
) -> list[sqlite3.Row]:
    get_problem_set(connection, set_slug)
    return connection.execute(
        """
        SELECT p.*, m.ordinal, m.section
        FROM problem_set_members AS m
        JOIN problem_sets AS ps ON ps.id = m.problem_set_id
        JOIN problems AS p ON p.id = m.problem_id
        WHERE ps.slug = ? ORDER BY m.ordinal
        """,
        (set_slug,),
    ).fetchall()


def resolve_problem(
    connection: sqlite3.Connection, reference: str, set_slug: str | None = None
) -> sqlite3.Row:
    if set_slug is None:
        if reference.isascii() and reference.isdecimal():
            raise ValueError("a numeric problem selector requires a problem set")
        row = connection.execute(
            "SELECT p.*, NULL AS ordinal FROM problems AS p WHERE p.slug = ?",
            (reference,),
        ).fetchone()
        if row is None:
            raise ValueError(f"unknown problem: {reference}")
        return row

    problem_set = get_problem_set(connection, set_slug)
    if reference.isascii() and reference.isdecimal():
        if reference == "0" or (len(reference) > 1 and reference.startswith("0")):
            raise ValueError(f"invalid 1-based problem index: {reference}")
        row = connection.execute(
            """
            SELECT p.*, m.ordinal
            FROM problem_set_members AS m
            JOIN problems AS p ON p.id = m.problem_id
            WHERE m.problem_set_id = ? AND m.ordinal = ?
            """,
            (problem_set["id"], int(reference)),
        ).fetchone()
        if row is None:
            raise ValueError(f"problem index out of range for {set_slug}: {reference}")
        return row

    row = connection.execute(
        """
        SELECT p.*, m.ordinal
        FROM problem_set_members AS m
        JOIN problems AS p ON p.id = m.problem_id
        WHERE m.problem_set_id = ? AND p.slug = ?
        """,
        (problem_set["id"], reference),
    ).fetchone()
    if row is None:
        global_problem = connection.execute(
            "SELECT 1 FROM problems WHERE slug = ?", (reference,)
        ).fetchone()
        if global_problem:
            raise ValueError(f"problem is not in {set_slug}: {reference}")
        raise ValueError(f"unknown problem: {reference}")
    return row


def get_implementation(
    connection: sqlite3.Connection, problem_id: int, language_slug: str
) -> sqlite3.Row:
    language = connection.execute(
        "SELECT * FROM languages WHERE slug = ? AND enabled = 1", (language_slug,)
    ).fetchone()
    if language is None:
        raise ValueError(f"unknown or disabled language: {language_slug}")
    implementation = connection.execute(
        """
        SELECT i.*, l.slug AS language, l.runner_path
        FROM problem_implementations AS i
        JOIN languages AS l ON l.id = i.language_id
        WHERE i.problem_id = ? AND i.language_id = ? AND i.enabled = 1
        """,
        (problem_id, language["id"]),
    ).fetchone()
    if implementation is None:
        problem_slug = connection.execute(
            "SELECT slug FROM problems WHERE id = ?", (problem_id,)
        ).fetchone()["slug"]
        raise ValueError(
            f"no active {language_slug} adapter for problem: {problem_slug}"
        )
    return implementation


def completed_problem_ids(
    connection: sqlite3.Connection, language_slug: str | None = None
) -> set[int]:
    parameters: list[object] = []
    language_clause = ""
    if language_slug:
        language_clause = " AND l.slug = ?"
        parameters.append(language_slug)
    rows = connection.execute(
        """
        SELECT DISTINCT a.problem_id
        FROM attempts AS a
        JOIN problems AS p ON p.id = a.problem_id
        JOIN languages AS l ON l.id = a.language_id
        WHERE a.result = 'pass' AND a.test_revision = p.test_revision
        """
        + language_clause,
        parameters,
    )
    return {row["problem_id"] for row in rows}


def record_attempt(
    connection: sqlite3.Connection,
    problem_slug: str,
    language_slug: str,
    result: str,
    duration_ms: int,
    exit_code: int | None,
    set_slug: str | None,
) -> None:
    if duration_ms < 0:
        raise ValueError("duration must not be negative")
    problem = resolve_problem(connection, problem_slug)
    language = connection.execute(
        "SELECT id FROM languages WHERE slug = ?", (language_slug,)
    ).fetchone()
    if language is None:
        raise ValueError(f"unknown language: {language_slug}")
    if set_slug:
        resolve_problem(connection, problem_slug, set_slug)
        set_id = get_problem_set(connection, set_slug)["id"]
    else:
        set_id = None
    with connection:
        connection.execute(
            """
            INSERT INTO attempts(
                problem_id, language_id, invoked_set_id, result, test_revision,
                duration_ms, exit_code, run_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                problem["id"],
                language["id"],
                set_id,
                result,
                problem["test_revision"],
                duration_ms,
                exit_code,
                utc_now(),
            ),
        )


def create_problem(
    connection: sqlite3.Connection,
    *,
    slug: str,
    title: str,
    difficulty: str,
    topic: str,
    statement_markdown: str,
    leetcode_id: int | None,
    leetcode_url: str,
    neetcode_url: str,
    premium: bool,
) -> None:
    validate_identifier(slug, "problem slug", problem_slug=True)
    if difficulty not in ("Easy", "Medium", "Hard"):
        raise ValueError(f"invalid difficulty: {difficulty}")
    if not title.strip():
        raise ValueError("problem title must not be empty")
    if len(title) > MAX_TITLE_LENGTH:
        raise ValueError(f"problem title exceeds {MAX_TITLE_LENGTH} characters")
    if not topic.strip():
        raise ValueError("problem topic must not be empty")
    if len(topic) > MAX_TOPIC_LENGTH:
        raise ValueError(f"problem topic exceeds {MAX_TOPIC_LENGTH} characters")
    if len(statement_markdown) > MAX_STATEMENT_LENGTH:
        raise ValueError(f"problem statement exceeds {MAX_STATEMENT_LENGTH} characters")
    if leetcode_id is not None and leetcode_id <= 0:
        raise ValueError("LeetCode id must be positive")
    for label, url in (("LeetCode", leetcode_url), ("NeetCode", neetcode_url)):
        if url and not url.startswith(("https://", "http://")):
            raise ValueError(f"{label} URL must use http or https")
    timestamp = utc_now()
    try:
        with connection:
            connection.execute(
                """
                INSERT INTO problems(
                    slug, title, difficulty, topic, leetcode_id, premium, managed, archived,
                    leetcode_url, neetcode_url, statement_markdown, test_revision,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, 1, ?, ?)
                """,
                (
                    slug,
                    title.strip(),
                    difficulty,
                    topic.strip(),
                    leetcode_id,
                    int(premium),
                    leetcode_url,
                    neetcode_url,
                    statement_markdown,
                    timestamp,
                    timestamp,
                ),
            )
    except sqlite3.IntegrityError as error:
        raise ValueError(f"problem already exists or conflicts: {slug}") from error


def create_problem_set(
    connection: sqlite3.Connection, *, slug: str, name: str, description: str
) -> None:
    validate_identifier(slug, "problem-set id")
    if not name.strip():
        raise ValueError("problem-set name must not be empty")
    if len(name) > MAX_TITLE_LENGTH:
        raise ValueError(f"problem-set name exceeds {MAX_TITLE_LENGTH} characters")
    if len(description) > MAX_DESCRIPTION_LENGTH:
        raise ValueError(
            f"problem-set description exceeds {MAX_DESCRIPTION_LENGTH} characters"
        )
    timestamp = utc_now()
    try:
        with connection:
            connection.execute(
                """
                INSERT INTO problem_sets(
                    slug, name, description, managed, created_at, updated_at
                ) VALUES (?, ?, ?, 0, ?, ?)
                """,
                (slug, name.strip(), description, timestamp, timestamp),
            )
    except sqlite3.IntegrityError as error:
        raise ValueError(f"problem set already exists: {slug}") from error


def _get_mutable_problem_set(
    connection: sqlite3.Connection, set_slug: str
) -> sqlite3.Row:
    problem_set = get_problem_set(connection, set_slug)
    if problem_set["managed"]:
        raise ValueError(f"shipped problem set is read-only: {set_slug}")
    return problem_set


def _validate_member_order(connection: sqlite3.Connection, problem_set_id: int) -> int:
    ordinals = [
        row["ordinal"]
        for row in connection.execute(
            "SELECT ordinal FROM problem_set_members "
            "WHERE problem_set_id = ? ORDER BY ordinal",
            (problem_set_id,),
        )
    ]
    if ordinals != list(range(1, len(ordinals) + 1)):
        raise RuntimeError("problem-set membership order is not contiguous")
    return len(ordinals)


def add_set_member(
    connection: sqlite3.Connection,
    *,
    set_slug: str,
    problem_slug: str,
    index: int | None,
    section: str | None,
) -> None:
    problem_set = _get_mutable_problem_set(connection, set_slug)
    problem = resolve_problem(connection, problem_slug)
    count = _validate_member_order(connection, problem_set["id"])
    if count >= 999_999:
        raise ValueError("problem set reached its maximum size")
    target = count + 1 if index is None else index
    if target < 1 or target > count + 1:
        raise ValueError(f"index must be between 1 and {count + 1}")
    connection.execute("BEGIN IMMEDIATE")
    try:
        connection.execute(
            "UPDATE problem_set_members SET ordinal = ordinal + 1000000 "
            "WHERE problem_set_id = ? AND ordinal >= ?",
            (problem_set["id"], target),
        )
        connection.execute(
            "UPDATE problem_set_members SET ordinal = ordinal - 999999 "
            "WHERE problem_set_id = ? AND ordinal >= ?",
            (problem_set["id"], target + 1000000),
        )
        connection.execute(
            """
            INSERT INTO problem_set_members(
                problem_set_id, problem_id, ordinal, section
            ) VALUES (?, ?, ?, ?)
            """,
            (problem_set["id"], problem["id"], target, section),
        )
        connection.execute(
            "UPDATE problem_sets SET updated_at = ? WHERE id = ?",
            (utc_now(), problem_set["id"]),
        )
        connection.commit()
    except sqlite3.IntegrityError as error:
        connection.rollback()
        raise ValueError(f"problem is already in set: {problem_slug}") from error
    except Exception:
        connection.rollback()
        raise


def remove_set_member(
    connection: sqlite3.Connection, *, set_slug: str, problem_slug: str
) -> None:
    problem_set = _get_mutable_problem_set(connection, set_slug)
    _validate_member_order(connection, problem_set["id"])
    row = connection.execute(
        """
        SELECT m.ordinal
        FROM problem_set_members AS m
        JOIN problems AS p ON p.id = m.problem_id
        WHERE m.problem_set_id = ? AND p.slug = ?
        """,
        (problem_set["id"], problem_slug),
    ).fetchone()
    if row is None:
        raise ValueError(f"problem is not in {set_slug}: {problem_slug}")
    connection.execute("BEGIN IMMEDIATE")
    try:
        connection.execute(
            "DELETE FROM problem_set_members WHERE problem_set_id = ? AND ordinal = ?",
            (problem_set["id"], row["ordinal"]),
        )
        connection.execute(
            "UPDATE problem_set_members SET ordinal = ordinal + 1000000 "
            "WHERE problem_set_id = ? AND ordinal > ?",
            (problem_set["id"], row["ordinal"]),
        )
        connection.execute(
            "UPDATE problem_set_members SET ordinal = ordinal - 1000001 "
            "WHERE problem_set_id = ? AND ordinal > 1000000",
            (problem_set["id"],),
        )
        connection.execute(
            "UPDATE problem_sets SET updated_at = ? WHERE id = ?",
            (utc_now(), problem_set["id"]),
        )
        connection.commit()
    except Exception:
        connection.rollback()
        raise


def add_implementation(
    connection: sqlite3.Connection,
    *,
    problem_slug: str,
    language_slug: str,
    solution_path: str,
) -> None:
    problem = resolve_problem(connection, problem_slug)
    language = connection.execute(
        "SELECT id FROM languages WHERE slug = ?", (language_slug,)
    ).fetchone()
    if language is None:
        raise ValueError(f"unknown language: {language_slug}")
    path = Path(solution_path)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError("solution path must be relative to the project root")
    if not path.parts or path.parts[0] != language_slug:
        raise ValueError(f"solution path must be inside {language_slug}/")
    with connection:
        connection.execute(
            """
            INSERT INTO problem_implementations(
                problem_id, language_id, solution_path, enabled
            ) VALUES (?, ?, ?, 1)
            ON CONFLICT(problem_id, language_id) DO UPDATE SET
                solution_path = excluded.solution_path,
                enabled = 1
            """,
            (problem["id"], language["id"], solution_path),
        )


def move_set_member(
    connection: sqlite3.Connection,
    *,
    set_slug: str,
    problem_slug: str,
    index: int,
) -> None:
    problem_set = _get_mutable_problem_set(connection, set_slug)
    _validate_member_order(connection, problem_set["id"])
    rows = connection.execute(
        """
        SELECT m.problem_id, p.slug
        FROM problem_set_members AS m
        JOIN problems AS p ON p.id = m.problem_id
        WHERE m.problem_set_id = ? ORDER BY m.ordinal
        """,
        (problem_set["id"],),
    ).fetchall()
    if index < 1 or index > len(rows):
        raise ValueError(f"index must be between 1 and {len(rows)}")
    problem_ids = [row["problem_id"] for row in rows]
    selected = next(
        (row for row in rows if row["slug"] == problem_slug),
        None,
    )
    if selected is None:
        raise ValueError(f"problem is not in {set_slug}: {problem_slug}")
    problem_ids.remove(selected["problem_id"])
    problem_ids.insert(index - 1, selected["problem_id"])
    connection.execute("BEGIN IMMEDIATE")
    try:
        connection.execute(
            "UPDATE problem_set_members SET ordinal = ordinal + 1000000 "
            "WHERE problem_set_id = ?",
            (problem_set["id"],),
        )
        for ordinal, problem_id in enumerate(problem_ids, 1):
            connection.execute(
                "UPDATE problem_set_members SET ordinal = ? "
                "WHERE problem_set_id = ? AND problem_id = ?",
                (ordinal, problem_set["id"], problem_id),
            )
        connection.execute(
            "UPDATE problem_sets SET updated_at = ? WHERE id = ?",
            (utc_now(), problem_set["id"]),
        )
        connection.commit()
    except Exception:
        connection.rollback()
        raise


def update_problem(
    connection: sqlite3.Connection,
    *,
    slug: str,
    title: str | None,
    difficulty: str | None,
    topic: str | None,
    statement_markdown: str | None,
    test_revision: int | None,
    leetcode_id: int | None,
    premium: bool | None,
    leetcode_url: str | None,
    neetcode_url: str | None,
) -> None:
    problem = resolve_problem(connection, slug)
    if problem["managed"]:
        raise ValueError(f"shipped problem is read-only: {slug}")
    new_title = problem["title"] if title is None else title
    new_difficulty = problem["difficulty"] if difficulty is None else difficulty
    new_topic = problem["topic"] if topic is None else topic
    new_statement = (
        problem["statement_markdown"]
        if statement_markdown is None
        else statement_markdown
    )
    new_revision = problem["test_revision"] if test_revision is None else test_revision
    new_leetcode_id = problem["leetcode_id"] if leetcode_id is None else leetcode_id
    if new_leetcode_id == 0:
        new_leetcode_id = None
    if new_leetcode_id is not None and new_leetcode_id < 0:
        raise ValueError("LeetCode id must be positive")
    new_premium = problem["premium"] if premium is None else int(premium)
    new_leetcode_url = problem["leetcode_url"] if leetcode_url is None else leetcode_url
    new_neetcode_url = problem["neetcode_url"] if neetcode_url is None else neetcode_url
    if new_revision < problem["test_revision"]:
        raise ValueError("test revision cannot decrease")
    if new_revision <= 0:
        raise ValueError("test revision must be positive")
    if new_difficulty not in ("Easy", "Medium", "Hard"):
        raise ValueError(f"invalid difficulty: {new_difficulty}")
    if not new_title.strip() or len(new_title) > MAX_TITLE_LENGTH:
        raise ValueError("problem title is empty or too long")
    if not new_topic.strip() or len(new_topic) > MAX_TOPIC_LENGTH:
        raise ValueError("problem topic is empty or too long")
    if len(new_statement) > MAX_STATEMENT_LENGTH:
        raise ValueError(f"problem statement exceeds {MAX_STATEMENT_LENGTH} characters")
    for label, url in (("LeetCode", new_leetcode_url), ("NeetCode", new_neetcode_url)):
        if url and not url.startswith(("https://", "http://")):
            raise ValueError(f"{label} URL must use http or https")
    with connection:
        try:
            connection.execute(
                """
                UPDATE problems SET
                    title = ?, difficulty = ?, topic = ?, statement_markdown = ?,
                    test_revision = ?, leetcode_id = ?, premium = ?,
                    leetcode_url = ?, neetcode_url = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    new_title.strip(),
                    new_difficulty,
                    new_topic.strip(),
                    new_statement,
                    new_revision,
                    new_leetcode_id,
                    new_premium,
                    new_leetcode_url,
                    new_neetcode_url,
                    utc_now(),
                    problem["id"],
                ),
            )
        except sqlite3.IntegrityError as error:
            raise ValueError(
                "LeetCode id is already assigned to another problem"
            ) from error


def delete_problem(connection: sqlite3.Connection, *, slug: str) -> None:
    problem = resolve_problem(connection, slug)
    if problem["managed"]:
        raise ValueError(f"shipped problem is read-only: {slug}")
    memberships = connection.execute(
        "SELECT COUNT(*) AS count FROM problem_set_members WHERE problem_id = ?",
        (problem["id"],),
    ).fetchone()["count"]
    attempts = connection.execute(
        "SELECT COUNT(*) AS count FROM attempts WHERE problem_id = ?",
        (problem["id"],),
    ).fetchone()["count"]
    if memberships or attempts:
        raise ValueError(
            f"cannot delete {slug}: {memberships} memberships and {attempts} attempts remain"
        )
    with connection:
        connection.execute(
            "DELETE FROM problem_implementations WHERE problem_id = ?", (problem["id"],)
        )
        connection.execute("DELETE FROM problems WHERE id = ?", (problem["id"],))


def update_problem_set(
    connection: sqlite3.Connection,
    *,
    slug: str,
    name: str | None,
    description: str | None,
) -> None:
    problem_set = _get_mutable_problem_set(connection, slug)
    new_name = problem_set["name"] if name is None else name
    new_description = problem_set["description"] if description is None else description
    if not new_name.strip() or len(new_name) > MAX_TITLE_LENGTH:
        raise ValueError("problem-set name is empty or too long")
    if len(new_description) > MAX_DESCRIPTION_LENGTH:
        raise ValueError(
            f"problem-set description exceeds {MAX_DESCRIPTION_LENGTH} characters"
        )
    with connection:
        connection.execute(
            "UPDATE problem_sets SET name = ?, description = ?, updated_at = ? WHERE id = ?",
            (new_name.strip(), new_description, utc_now(), problem_set["id"]),
        )


def delete_problem_set(connection: sqlite3.Connection, *, slug: str) -> None:
    problem_set = _get_mutable_problem_set(connection, slug)
    with connection:
        connection.execute(
            "DELETE FROM problem_sets WHERE id = ?", (problem_set["id"],)
        )
