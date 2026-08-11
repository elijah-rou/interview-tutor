from __future__ import annotations

import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "catalog" / "problems.json"
BLIND75 = ROOT / "problem_sets" / "blind75.json"


class ProjectCliTests(unittest.TestCase):
    @staticmethod
    def fixture_catalog(revision: int = 1) -> dict[str, object]:
        return {
            "schema_version": 2,
            "catalog_revision": revision,
            "problems": [
                {
                    "slug": "managed-one",
                    "title": "Managed One",
                    "difficulty": "Easy",
                    "topic": "Arrays",
                    "leetcode_id": 900001,
                    "premium": False,
                    "leetcode_url": "https://leetcode.com/problems/managed-one/",
                    "neetcode_url": "https://neetcode.io/problems/managed-one",
                    "statement_markdown": "Managed statement.",
                    "test_revision": 1,
                    "adapters": [
                        {
                            "language": "python",
                            "solution_path": "python/managed_one.py",
                        },
                        {
                            "language": "rust",
                            "solution_path": "rust/managed_one.rs",
                        },
                    ],
                },
                {
                    "slug": "managed-retired",
                    "title": "Managed Retired",
                    "difficulty": "Medium",
                    "topic": "Graphs",
                    "leetcode_id": 900002,
                    "premium": False,
                    "leetcode_url": "https://leetcode.com/problems/managed-retired/",
                    "neetcode_url": "https://neetcode.io/problems/managed-retired",
                    "statement_markdown": "Retired statement.",
                    "test_revision": 1,
                    "adapters": [
                        {
                            "language": "python",
                            "solution_path": "python/managed_retired.py",
                        }
                    ],
                },
            ],
        }

    @staticmethod
    def fixture_sets() -> dict[str, dict[str, object]]:
        return {
            "managed-set": {
                "schema_version": 2,
                "id": "managed-set",
                "name": "Managed Set",
                "description": "",
                "members": [
                    {"ordinal": 1, "problem_slug": "managed-one"},
                    {"ordinal": 2, "problem_slug": "managed-retired"},
                ],
            },
            "managed-retired-set": {
                "schema_version": 2,
                "id": "managed-retired-set",
                "name": "Managed Retired Set",
                "description": "",
                "members": [{"ordinal": 1, "problem_slug": "managed-retired"}],
            },
        }

    def write_fixture_root(
        self,
        root: Path,
        catalog: dict[str, object],
        problem_sets: dict[str, dict[str, object]],
    ) -> None:
        (root / "catalog").mkdir(parents=True, exist_ok=True)
        (root / "problem_sets").mkdir(parents=True, exist_ok=True)
        (root / "python").mkdir(parents=True, exist_ok=True)
        (root / "rust").mkdir(parents=True, exist_ok=True)
        (root / "catalog" / "problems.json").write_text(
            json.dumps(catalog), encoding="utf-8"
        )
        for old_set in (root / "problem_sets").glob("*.json"):
            old_set.unlink()
        for set_id, problem_set in problem_sets.items():
            (root / "problem_sets" / f"{set_id}.json").write_text(
                json.dumps(problem_set), encoding="utf-8"
            )
        for relative_path in (
            "python/managed_one.py",
            "rust/managed_one.rs",
            "python/managed_retired.py",
            "python/custom.py",
            "python/custom_updated.py",
        ):
            path = root / relative_path
            path.write_text("# fixture\n", encoding="utf-8")
        runner = root / "python" / "run"
        runner.write_text(
            "#!/bin/sh\nprintf '%s\\n' managed-one managed-retired custom-problem\n",
            encoding="utf-8",
        )
        runner.chmod(0o755)

    def run_fixture_command(
        self, root: Path, database: Path, *arguments: str
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment["PRACTICE_ROOT"] = str(root)
        environment["PRACTICE_DB_PATH"] = str(database)
        return self.run_command(
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "--manifest-path",
            str(ROOT / "cli" / "Cargo.toml"),
            "--bin",
            "practice",
            "--",
            *arguments,
            env=environment,
        )

    def run_command(
        self, *arguments: str, env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [*arguments],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
            timeout=30,
        )

    def test_out_of_range_database_versions_are_rejected_without_mutation(self) -> None:
        for unsupported_version in (-1, 3):
            with self.subTest(user_version=unsupported_version):
                with tempfile.TemporaryDirectory() as directory:
                    database = Path(directory) / "progress.db"
                    environment = os.environ.copy()
                    environment["PRACTICE_DB_PATH"] = str(database)
                    initialized = self.run_command(
                        str(ROOT / "practice"), "sets", "list", env=environment
                    )
                    self.assertEqual(initialized.returncode, 0, initialized.stderr)
                    with sqlite3.connect(database) as connection:
                        connection.execute(
                            f"PRAGMA user_version = {unsupported_version}"
                        )
                        connection.execute(
                            "INSERT INTO metadata(key, value) "
                            "VALUES ('version_marker', 'untouched')"
                        )
                        before = connection.execute(
                            "SELECT type, name, sql FROM sqlite_master ORDER BY type, name"
                        ).fetchall()
                    result = self.run_command(
                        str(ROOT / "practice"), "sets", "list", env=environment
                    )
                    self.assertEqual(result.returncode, 2)
                    self.assertIn("unsupported database schema version", result.stderr)
                    with sqlite3.connect(database) as connection:
                        version = connection.execute("PRAGMA user_version").fetchone()[
                            0
                        ]
                        marker = connection.execute(
                            "SELECT value FROM metadata WHERE key = 'version_marker'"
                        ).fetchone()
                        after = connection.execute(
                            "SELECT type, name, sql FROM sqlite_master ORDER BY type, name"
                        ).fetchall()
                    self.assertEqual(version, unsupported_version)
                    self.assertEqual(marker, ("untouched",))
                    self.assertEqual(after, before)

    def test_catalog_rejects_unknown_fields_and_blank_persisted_names(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "root"
            database = Path(directory) / "progress.db"
            catalog = self.fixture_catalog()
            catalog["unexpected"] = True
            self.write_fixture_root(root, catalog, self.fixture_sets())
            unknown = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(unknown.returncode, 2)
            self.assertIn("unknown field", unknown.stderr)
            self.assertFalse(database.exists())

            catalog = self.fixture_catalog()
            catalog["problems"][0]["adapters"][0]["unexpected"] = True  # type: ignore[index]
            self.write_fixture_root(root, catalog, self.fixture_sets())
            nested_unknown = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(nested_unknown.returncode, 2)
            self.assertIn("unknown field", nested_unknown.stderr)
            self.assertFalse(database.exists())

            for field in ("title", "topic"):
                catalog = self.fixture_catalog()
                catalog["problems"][0][field] = "   "  # type: ignore[index]
                self.write_fixture_root(root, catalog, self.fixture_sets())
                blank = self.run_fixture_command(root, database, "sets", "list")
                self.assertEqual(blank.returncode, 2)
                self.assertIn("must not be blank", blank.stderr)
                self.assertFalse(database.exists())

            problem_sets = self.fixture_sets()
            problem_sets["managed-set"]["name"] = "\t"
            self.write_fixture_root(root, self.fixture_catalog(), problem_sets)
            blank_set = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(blank_set.returncode, 2)
            self.assertIn("problem-set name must not be blank", blank_set.stderr)
            self.assertFalse(database.exists())

            self.write_fixture_root(root, self.fixture_catalog(), self.fixture_sets())
            initialized = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(initialized.returncode, 0, initialized.stderr)
            invalid_upgrade = self.fixture_catalog(revision=2)
            invalid_upgrade["unexpected"] = True
            self.write_fixture_root(root, invalid_upgrade, self.fixture_sets())
            rejected_upgrade = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(rejected_upgrade.returncode, 2)
            with sqlite3.connect(database) as connection:
                revision = connection.execute(
                    "SELECT value FROM metadata WHERE key = 'catalog_revision'"
                ).fetchone()
                managed_count = connection.execute(
                    "SELECT COUNT(*) FROM problems WHERE managed = 1"
                ).fetchone()
            self.assertEqual(revision, ("1",))
            self.assertEqual(managed_count, (2,))

    def test_catalog_rejects_invalid_urls_and_duplicate_positive_leetcode_ids(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "root"
            database = Path(directory) / "progress.db"
            catalog = self.fixture_catalog()
            catalog["problems"][0]["leetcode_url"] = "file:///tmp/problem"  # type: ignore[index]
            self.write_fixture_root(root, catalog, self.fixture_sets())
            invalid_url = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(invalid_url.returncode, 2)
            self.assertIn("URL must use http or https", invalid_url.stderr)
            self.assertFalse(database.exists())

            for invalid_value in (
                "https://example.com/a path",
                "https://example.com/path?query=bad value",
                "https://example.com/path\\segment",
                "https://example.com/path?query=bad\u0007value",
            ):
                with self.subTest(url=repr(invalid_value)):
                    catalog = self.fixture_catalog()
                    catalog["problems"][0]["neetcode_url"] = invalid_value  # type: ignore[index]
                    self.write_fixture_root(root, catalog, self.fixture_sets())
                    invalid_complete_url = self.run_fixture_command(
                        root, database, "sets", "list"
                    )
                    self.assertEqual(invalid_complete_url.returncode, 2)
                    self.assertIn("invalid NeetCode URL", invalid_complete_url.stderr)
                    self.assertFalse(database.exists())

            catalog = self.fixture_catalog()
            catalog["problems"][1]["leetcode_id"] = 900001  # type: ignore[index]
            self.write_fixture_root(root, catalog, self.fixture_sets())
            duplicate_id = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(duplicate_id.returncode, 2)
            self.assertIn("duplicate LeetCode id", duplicate_id.stderr)
            self.assertFalse(database.exists())

    def test_catalog_revision_reconciles_only_managed_resources(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "root"
            database = Path(directory) / "progress.db"
            self.write_fixture_root(root, self.fixture_catalog(), self.fixture_sets())
            initialized = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(initialized.returncode, 0, initialized.stderr)
            for arguments in (
                (
                    "problems",
                    "add",
                    "custom-problem",
                    "--title",
                    "Custom Problem",
                    "--difficulty",
                    "Easy",
                    "--topic",
                    "Custom",
                ),
                (
                    "problems",
                    "adapter",
                    "custom-problem",
                    "python",
                    "python/custom.py",
                ),
                (
                    "problems",
                    "adapter",
                    "custom-problem",
                    "python",
                    "python/custom_updated.py",
                ),
                ("sets", "create", "custom-set", "--name", "Custom Set"),
                ("sets", "add", "custom-set", "custom-problem"),
                (
                    "_record",
                    "python",
                    "custom-problem",
                    "pass",
                    "1",
                    "--problem-set",
                    "custom-set",
                ),
                (
                    "_record",
                    "python",
                    "managed-retired",
                    "fail",
                    "2",
                    "--problem-set",
                    "managed-retired-set",
                ),
            ):
                result = self.run_fixture_command(root, database, *arguments)
                self.assertEqual(result.returncode, 0, result.stderr)

            upgraded_catalog = self.fixture_catalog(revision=2)
            upgraded_catalog["problems"] = [upgraded_catalog["problems"][0]]  # type: ignore[index]
            upgraded_catalog["problems"][0]["statement_markdown"] = (  # type: ignore[index]
                "Managed statement revision two."
            )
            upgraded_catalog["problems"][0]["test_revision"] = 2  # type: ignore[index]
            upgraded_catalog["problems"][0]["adapters"] = [  # type: ignore[index]
                {
                    "language": "python",
                    "solution_path": "python/managed_one.py",
                }
            ]
            upgraded_sets = {
                "managed-set": {
                    "schema_version": 2,
                    "id": "managed-set",
                    "name": "Managed Set Revised",
                    "description": "",
                    "members": [{"ordinal": 1, "problem_slug": "managed-one"}],
                }
            }
            self.write_fixture_root(root, upgraded_catalog, upgraded_sets)
            upgraded = self.run_fixture_command(root, database, "sets", "list")
            self.assertEqual(upgraded.returncode, 0, upgraded.stderr)

            with sqlite3.connect(database) as connection:
                managed_problems = connection.execute(
                    "SELECT slug, archived FROM problems WHERE managed = 1 ORDER BY slug"
                ).fetchall()
                managed_revision = connection.execute(
                    "SELECT statement_markdown, test_revision FROM problems "
                    "WHERE slug = 'managed-one'"
                ).fetchone()
                adapters = connection.execute(
                    """
                    SELECT p.slug, l.slug, i.enabled
                    FROM problem_implementations AS i
                    JOIN problems AS p ON p.id = i.problem_id
                    JOIN languages AS l ON l.id = i.language_id
                    WHERE p.managed = 1 ORDER BY p.slug, l.slug
                    """
                ).fetchall()
                sets = connection.execute(
                    "SELECT slug, managed FROM problem_sets ORDER BY slug"
                ).fetchall()
                members = connection.execute(
                    """
                    SELECT ps.slug, p.slug
                    FROM problem_set_members AS m
                    JOIN problem_sets AS ps ON ps.id = m.problem_set_id
                    JOIN problems AS p ON p.id = m.problem_id
                    ORDER BY ps.slug, m.ordinal
                    """
                ).fetchall()
                attempts = connection.execute(
                    """
                    SELECT p.slug, ps.slug
                    FROM attempts AS a
                    JOIN problems AS p ON p.id = a.problem_id
                    LEFT JOIN problem_sets AS ps ON ps.id = a.invoked_set_id
                    ORDER BY a.id
                    """
                ).fetchall()
                custom_adapter = connection.execute(
                    """
                    SELECT i.solution_path, i.enabled
                    FROM problem_implementations AS i
                    JOIN problems AS p ON p.id = i.problem_id
                    JOIN languages AS l ON l.id = i.language_id
                    WHERE p.slug = 'custom-problem' AND l.slug = 'python'
                    """
                ).fetchone()
                revision = connection.execute(
                    "SELECT value FROM metadata WHERE key = 'catalog_revision'"
                ).fetchone()
            self.assertEqual(
                managed_problems,
                [("managed-one", 0), ("managed-retired", 1)],
            )
            self.assertEqual(managed_revision, ("Managed statement revision two.", 2))
            self.assertEqual(
                adapters,
                [
                    ("managed-one", "python", 1),
                    ("managed-one", "rust", 0),
                    ("managed-retired", "python", 0),
                ],
            )
            self.assertEqual(sets, [("custom-set", 0), ("managed-set", 1)])
            self.assertEqual(
                members,
                [("custom-set", "custom-problem"), ("managed-set", "managed-one")],
            )
            self.assertEqual(
                attempts,
                [("custom-problem", "custom-set"), ("managed-retired", None)],
            )
            self.assertEqual(custom_adapter, ("python/custom_updated.py", 1))
            self.assertEqual(revision, ("2",))

    def test_language_registries_cover_the_seeded_global_catalog(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        expected = {problem["slug"] for problem in catalog["problems"]}
        self.assertEqual(len(expected), 75)

        python_result = self.run_command(str(ROOT / "python" / "run"), "--list")
        self.assertEqual(python_result.returncode, 0, python_result.stderr)
        python_slugs = set(python_result.stdout.splitlines())
        self.assertEqual(python_slugs, expected)

        rust_result = self.run_command(str(ROOT / "rust" / "run"), "--list")
        self.assertEqual(rust_result.returncode, 0, rust_result.stderr)
        rust_slugs = set(rust_result.stdout.splitlines())
        self.assertEqual(rust_slugs, expected)

    def test_blind75_is_only_an_ordered_set_of_global_problem_references(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        problem_set = json.loads(BLIND75.read_text(encoding="utf-8"))
        global_slugs = {problem["slug"] for problem in catalog["problems"]}
        member_slugs = [member["problem_slug"] for member in problem_set["members"]]
        self.assertEqual(len(member_slugs), 75)
        self.assertEqual(len(set(member_slugs)), 75)
        self.assertEqual(set(member_slugs), global_slugs)
        self.assertEqual(
            [member["ordinal"] for member in problem_set["members"]], list(range(1, 76))
        )
        self.assertTrue(
            all(
                problem["neetcode_url"].startswith("https://neetcode.io/problems/")
                for problem in catalog["problems"]
            )
        )

    def test_v1_database_migrates_without_losing_progress_or_stale_history(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "progress.db"
            with sqlite3.connect(database) as connection:
                connection.executescript(
                    """
                    PRAGMA foreign_keys = ON;
                    CREATE TABLE problem_sets (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT;
                    CREATE TABLE problems (
                        problem_set_id TEXT NOT NULL REFERENCES problem_sets(id),
                        slug TEXT NOT NULL,
                        ordinal INTEGER NOT NULL CHECK (ordinal > 0),
                        title TEXT NOT NULL,
                        difficulty TEXT NOT NULL,
                        topic TEXT NOT NULL,
                        external_id INTEGER,
                        premium INTEGER NOT NULL,
                        test_revision INTEGER NOT NULL,
                        PRIMARY KEY(problem_set_id, slug),
                        UNIQUE(problem_set_id, ordinal)
                    ) STRICT;
                    CREATE TABLE attempts (
                        id INTEGER PRIMARY KEY,
                        problem_set_id TEXT NOT NULL,
                        problem_slug TEXT NOT NULL,
                        language TEXT NOT NULL,
                        passed INTEGER NOT NULL,
                        test_revision INTEGER NOT NULL,
                        duration_ms INTEGER NOT NULL,
                        run_at TEXT NOT NULL,
                        FOREIGN KEY(problem_set_id, problem_slug)
                            REFERENCES problems(problem_set_id, slug)
                    ) STRICT;
                    INSERT INTO problem_sets VALUES ('blind75', 'Blind 75');
                    INSERT INTO problems VALUES (
                        'blind75', 'two-sum', 16, 'Two Sum', 'Easy',
                        'Arrays & Hashing', 1, 0, 1
                    );
                    INSERT INTO problems VALUES (
                        'blind75', 'combination-sum-iv', 1000024, 'Combination Sum IV',
                        'Medium', 'Dynamic Programming', 377, 0, 1
                    );
                    INSERT INTO attempts VALUES (
                        1, 'blind75', 'two-sum', 'python', 1, 1, 7,
                        '2026-08-10T00:00:00+00:00'
                    );
                    INSERT INTO attempts VALUES (
                        2, 'blind75', 'combination-sum-iv', 'python', 1, 1, 9,
                        '2026-08-10T00:01:00+00:00'
                    );
                    INSERT INTO attempts VALUES (
                        3, 'blind75', 'two-sum', 'go', 0, 1, 11,
                        '2026-08-10T00:02:00+00:00'
                    );
                    """
                )
            environment = os.environ.copy()
            environment["PRACTICE_DB_PATH"] = str(database)
            stats = self.run_command(
                str(ROOT / "practice"), "stats", "--language", "python", env=environment
            )
            self.assertEqual(stats.returncode, 0, stats.stderr)
            self.assertIn("1/75", stats.stdout)
            global_stats = self.run_command(
                str(ROOT / "practice"),
                "stats",
                "--global",
                "--language",
                "python",
                env=environment,
            )
            archived = self.run_command(
                str(ROOT / "practice"),
                "problems",
                "list",
                "--all",
                env=environment,
            )
            self.assertIn("1/75", global_stats.stdout)
            self.assertIn("combination-sum-iv", archived.stdout)
            self.assertIn("archived", archived.stdout)
            with sqlite3.connect(database) as connection:
                version = connection.execute("PRAGMA user_version").fetchone()[0]
                attempt_count = connection.execute(
                    "SELECT COUNT(*) FROM attempts"
                ).fetchone()[0]
                stale = connection.execute(
                    "SELECT id, leetcode_id FROM problems WHERE slug = 'combination-sum-iv'"
                ).fetchone()
                active_members = connection.execute(
                    "SELECT COUNT(*) FROM problem_set_members"
                ).fetchone()[0]
                go_language = connection.execute(
                    "SELECT enabled FROM languages WHERE slug = 'go'"
                ).fetchone()
            self.assertEqual(version, 2)
            self.assertEqual(attempt_count, 3)
            self.assertIsNotNone(stale)
            self.assertEqual(stale[1], 377)
            self.assertEqual(active_members, 75)
            self.assertEqual(go_language, (0,))

    def test_v1_migration_rejects_conflicting_global_problem_metadata_atomically(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "progress.db"
            with sqlite3.connect(database) as connection:
                connection.executescript(
                    """
                    CREATE TABLE problem_sets (id TEXT PRIMARY KEY, name TEXT NOT NULL);
                    CREATE TABLE problems (
                        problem_set_id TEXT NOT NULL,
                        slug TEXT NOT NULL,
                        ordinal INTEGER NOT NULL,
                        title TEXT NOT NULL,
                        difficulty TEXT NOT NULL,
                        topic TEXT NOT NULL,
                        external_id INTEGER,
                        premium INTEGER NOT NULL,
                        test_revision INTEGER NOT NULL,
                        PRIMARY KEY(problem_set_id, slug)
                    );
                    CREATE TABLE attempts (
                        id INTEGER PRIMARY KEY,
                        problem_set_id TEXT NOT NULL,
                        problem_slug TEXT NOT NULL,
                        language TEXT NOT NULL,
                        passed INTEGER NOT NULL,
                        test_revision INTEGER NOT NULL,
                        duration_ms INTEGER NOT NULL,
                        run_at TEXT NOT NULL
                    );
                    INSERT INTO problem_sets VALUES ('first', 'First');
                    INSERT INTO problem_sets VALUES ('second', 'Second');
                    INSERT INTO problems VALUES (
                        'first', 'shared', 1, 'First Title', 'Easy', 'Arrays',
                        NULL, 0, 1
                    );
                    INSERT INTO problems VALUES (
                        'second', 'shared', 1, 'Different Title', 'Easy', 'Arrays',
                        NULL, 0, 1
                    );
                    """
                )
            environment = os.environ.copy()
            environment["PRACTICE_DB_PATH"] = str(database)
            result = self.run_command(
                str(ROOT / "practice"), "sets", "list", env=environment
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("metadata conflicts", result.stderr)
            with sqlite3.connect(database) as connection:
                tables = {
                    row[0]
                    for row in connection.execute(
                        "SELECT name FROM sqlite_master WHERE type = 'table'"
                    )
                }
                problem_count = connection.execute(
                    "SELECT COUNT(*) FROM problems"
                ).fetchone()[0]
            self.assertIn("attempts", tables)
            self.assertNotIn("legacy_attempts", tables)
            self.assertEqual(problem_count, 2)

            with sqlite3.connect(database) as connection:
                connection.execute(
                    "UPDATE problems SET title = 'First Title' WHERE problem_set_id = 'second'"
                )
                connection.execute(
                    "INSERT INTO problems VALUES "
                    "('first', 'alpha', 2, 'Alpha', 'Easy', 'Arrays', 42, 0, 1)"
                )
                connection.execute(
                    "INSERT INTO problems VALUES "
                    "('second', 'beta', 2, 'Beta', 'Easy', 'Arrays', 42, 0, 1)"
                )
            external_id_result = self.run_command(
                str(ROOT / "practice"), "sets", "list", env=environment
            )
            self.assertEqual(external_id_result.returncode, 2)
            self.assertIn("LeetCode ids conflict", external_id_result.stderr)

            with sqlite3.connect(database) as connection:
                connection.execute(
                    "DELETE FROM problems WHERE slug IN ('alpha', 'beta')"
                )
                connection.execute(
                    "UPDATE problems SET external_id = 1 WHERE slug = 'shared'"
                )
            catalog_conflict = self.run_command(
                str(ROOT / "practice"), "sets", "list", env=environment
            )
            self.assertEqual(catalog_conflict.returncode, 2)
            self.assertIn(
                "shipped catalog LeetCode id conflicts", catalog_conflict.stderr
            )
            with sqlite3.connect(database) as connection:
                version = connection.execute("PRAGMA user_version").fetchone()[0]
                tables = {
                    row[0]
                    for row in connection.execute(
                        "SELECT name FROM sqlite_master WHERE type = 'table'"
                    )
                }
            self.assertEqual(version, 0)
            self.assertIn("problems", tables)
            self.assertNotIn("legacy_problems", tables)

    def test_progress_is_derived_from_a_current_passing_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            environment = os.environ.copy()
            environment["PRACTICE_DB_PATH"] = str(Path(directory) / "progress.db")
            record = self.run_command(
                str(ROOT / "practice"),
                "_record",
                "python",
                "two-sum",
                "pass",
                "7",
                env=environment,
            )
            self.assertEqual(record.returncode, 0, record.stderr)
            stats = self.run_command(
                str(ROOT / "practice"), "stats", "--language", "python", env=environment
            )
            self.assertEqual(stats.returncode, 0, stats.stderr)
            self.assertIn("1/75", stats.stdout)
            self.assertIn("Arrays & Hashing", stats.stdout)


if __name__ == "__main__":
    unittest.main()
