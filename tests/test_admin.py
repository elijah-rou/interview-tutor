from __future__ import annotations

import json
import os
import sqlite3
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "problem_sets" / "blind75" / "problems.json"


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

    def test_language_runners_list_the_catalog_in_order(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        expected = [problem["slug"] for problem in catalog["problems"]]

        python_result = self.run_command(str(ROOT / "python" / "run"), "--list")
        self.assertEqual(python_result.returncode, 0, python_result.stderr)
        python_slugs = [line.split()[2] for line in python_result.stdout.splitlines()]
        self.assertEqual(python_slugs, expected)

        rust_result = self.run_command(str(ROOT / "rust" / "run"), "--list")
        self.assertEqual(rust_result.returncode, 0, rust_result.stderr)
        rust_slugs = [line.split()[1] for line in rust_result.stdout.splitlines()]
        self.assertEqual(rust_slugs, expected)

    def test_catalog_includes_verified_neetcode_sources(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        external_ids = {problem["leetcode_id"] for problem in catalog["problems"]}
        self.assertIn(39, external_ids)
        self.assertNotIn(377, external_ids)
        self.assertTrue(
            all(
                problem["neetcode_url"].startswith("https://neetcode.io/problems/")
                for problem in catalog["problems"]
            )
        )
        with tempfile.TemporaryDirectory() as directory:
            environment = os.environ.copy()
            environment["BLIND75_DB_PATH"] = str(Path(directory) / "progress.db")
            shown = self.run_command(
                str(ROOT / "practice"), "show", "combination-sum", env=environment
            )
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertIn(
            "https://neetcode.io/problems/combination-target-sum/question", shown.stdout
        )

    def test_catalog_rename_preserves_legacy_database_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "progress.db"
            environment = os.environ.copy()
            environment["BLIND75_DB_PATH"] = str(database)
            initialized = self.run_command(
                str(ROOT / "practice"), "db", env=environment
            )
            self.assertEqual(initialized.returncode, 0, initialized.stderr)
            with sqlite3.connect(database) as connection:
                connection.execute(
                    "UPDATE problems SET slug = 'combination-sum-iv' "
                    "WHERE problem_set_id = 'blind75' AND slug = 'combination-sum'"
                )
                connection.execute(
                    "INSERT INTO attempts(problem_set_id, problem_slug, language, passed, "
                    "test_revision, duration_ms, run_at) "
                    "VALUES ('blind75', 'combination-sum-iv', 'python', 1, 1, 5, "
                    "'2026-08-10T00:00:00+00:00')"
                )
                connection.execute(
                    "CREATE UNIQUE INDEX legacy_problem_ordinal "
                    "ON problems(problem_set_id, ordinal)"
                )
            shown = self.run_command(
                str(ROOT / "practice"), "show", "combination-sum", env=environment
            )
            self.assertEqual(shown.returncode, 0, shown.stderr)
            with sqlite3.connect(database) as connection:
                rows = connection.execute(
                    "SELECT slug, ordinal FROM problems "
                    "WHERE slug IN ('combination-sum', 'combination-sum-iv')"
                ).fetchall()
                retained_attempts = connection.execute(
                    "SELECT COUNT(*) FROM attempts WHERE problem_slug = 'combination-sum-iv'"
                ).fetchone()[0]
            self.assertEqual(retained_attempts, 1)
            ordinals = dict(rows)
            self.assertEqual(ordinals["combination-sum"], 24)
            self.assertGreaterEqual(ordinals["combination-sum-iv"], 1_000_000)

    def test_progress_is_derived_from_a_current_passing_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            environment = os.environ.copy()
            environment["BLIND75_DB_PATH"] = str(Path(directory) / "progress.db")
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
                str(ROOT / "practice"),
                "stats",
                "--language",
                "python",
                env=environment,
            )
            self.assertEqual(stats.returncode, 0, stats.stderr)
            self.assertIn("1/75", stats.stdout)
            self.assertIn("Arrays & Hashing", stats.stdout)


if __name__ == "__main__":
    unittest.main()
