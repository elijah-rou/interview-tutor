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
        )

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
