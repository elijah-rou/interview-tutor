from __future__ import annotations

import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class GeneralizedCliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.database = Path(self.temporary_directory.name) / "progress.db"
        self.environment = os.environ.copy()
        self.environment["PRACTICE_DB_PATH"] = str(self.database)

    def run_command(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [*arguments],
            cwd=ROOT,
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
            timeout=30,
        )

    def test_pure_numeric_problem_slug_is_rejected_as_index_ambiguous(self) -> None:
        result = self.run_command(
            str(ROOT / "practice"),
            "problems",
            "add",
            "123",
            "--title",
            "Numeric",
            "--difficulty",
            "Easy",
            "--topic",
            "Custom",
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("invalid problem slug", result.stderr)

    def test_problem_and_problem_set_can_be_created_and_composed(self) -> None:
        added = self.run_command(
            str(ROOT / "practice"),
            "problems",
            "add",
            "custom-pair-sum",
            "--title",
            "Custom Pair Sum",
            "--difficulty",
            "Easy",
            "--topic",
            "Arrays & Hashing",
            "--statement",
            "Return two matching indexes.",
        )
        self.assertEqual(added.returncode, 0, added.stderr)
        created = self.run_command(
            str(ROOT / "practice"),
            "sets",
            "create",
            "custom-set",
            "--name",
            "Custom Set",
        )
        self.assertEqual(created.returncode, 0, created.stderr)
        included = self.run_command(
            str(ROOT / "practice"),
            "sets",
            "add",
            "custom-set",
            "custom-pair-sum",
        )
        self.assertEqual(included.returncode, 0, included.stderr)
        listed = self.run_command(str(ROOT / "practice"), "--set", "custom-set", "list")
        self.assertEqual(listed.returncode, 0, listed.stderr)
        self.assertIn("custom-pair-sum", listed.stdout)

    def test_numeric_selector_requires_a_valid_explicit_problem_set_index(self) -> None:
        global_numeric = self.run_command(str(ROOT / "run"), "python", "16")
        zero = self.run_command(str(ROOT / "run"), "python", "blind75", "0")
        leading_zero = self.run_command(str(ROOT / "run"), "python", "blind75", "016")
        out_of_range = self.run_command(str(ROOT / "run"), "python", "blind75", "76")
        for result in (global_numeric, zero, leading_zero, out_of_range):
            self.assertEqual(result.returncode, 2)
        with sqlite3.connect(self.database) as connection:
            attempts = connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
        self.assertEqual(attempts, 0)

    def test_set_index_and_slug_resolve_to_the_same_global_problem(self) -> None:
        by_index = self.run_command(
            str(ROOT / "practice"), "--set", "blind75", "show", "16"
        )
        by_slug = self.run_command(
            str(ROOT / "practice"), "--set", "blind75", "show", "two-sum"
        )
        self.assertEqual(by_index.returncode, 0, by_index.stderr)
        self.assertEqual(by_slug.returncode, 0, by_slug.stderr)
        self.assertIn("Two Sum", by_index.stdout)
        self.assertEqual(by_index.stdout, by_slug.stdout)

    def test_custom_problem_and_set_metadata_support_update_and_safe_delete(
        self,
    ) -> None:
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"),
                "problems",
                "add",
                "scratch",
                "--title",
                "Scratch",
                "--difficulty",
                "Easy",
                "--topic",
                "Custom",
            ).returncode,
            0,
        )
        updated = self.run_command(
            str(ROOT / "practice"),
            "problems",
            "update",
            "scratch",
            "--title",
            "Scratch Updated",
            "--statement",
            "A local statement.",
            "--test-revision",
            "2",
        )
        self.assertEqual(updated.returncode, 0, updated.stderr)
        shown = self.run_command(str(ROOT / "practice"), "problems", "show", "scratch")
        self.assertIn("Scratch Updated", shown.stdout)
        self.assertIn("A local statement.", shown.stdout)
        deleted = self.run_command(
            str(ROOT / "practice"), "problems", "delete", "scratch", "--yes"
        )
        self.assertEqual(deleted.returncode, 0, deleted.stderr)

        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"),
                "sets",
                "create",
                "scratch-set",
                "--name",
                "Scratch",
            ).returncode,
            0,
        )
        renamed = self.run_command(
            str(ROOT / "practice"),
            "sets",
            "update",
            "scratch-set",
            "--name",
            "Scratch Updated",
        )
        self.assertEqual(renamed.returncode, 0, renamed.stderr)
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"), "sets", "add", "scratch-set", "two-sum"
            ).returncode,
            0,
        )
        attempted = self.run_command(
            str(ROOT / "run"), "python", "scratch-set", "two-sum"
        )
        self.assertEqual(attempted.returncode, 1)
        removed = self.run_command(
            str(ROOT / "practice"), "sets", "delete", "scratch-set", "--yes"
        )
        self.assertEqual(removed.returncode, 0, removed.stderr)
        with sqlite3.connect(self.database) as connection:
            preserved_attempt = connection.execute(
                "SELECT invoked_set_id FROM attempts ORDER BY id DESC LIMIT 1"
            ).fetchone()
        self.assertEqual(preserved_attempt, (None,))

    def test_empty_environment_values_do_not_mask_configuration(self) -> None:
        fallback_database = Path(self.temporary_directory.name) / "fallback.db"
        self.environment["PRACTICE_DATABASE_URL"] = ""
        self.environment["PRACTICE_DB_PATH"] = str(fallback_database)
        database_result = self.run_command(str(ROOT / "practice"), "db")
        self.assertEqual(database_result.returncode, 0, database_result.stderr)
        self.assertEqual(database_result.stdout.splitlines()[0], str(fallback_database))
        self.assertTrue(fallback_database.is_file())

        self.environment["PRACTICE_ROOT"] = ""
        root_result = self.run_command(
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "--manifest-path",
            str(ROOT / "cli" / "Cargo.toml"),
            "--bin",
            "practice",
            "--",
            "db",
        )
        self.assertEqual(root_result.returncode, 0, root_result.stderr)
        self.assertEqual(root_result.stdout.splitlines()[0], str(fallback_database))

    def test_explicit_empty_database_flag_is_rejected(self) -> None:
        result = self.run_command(str(ROOT / "practice"), "--db", "", "db")
        self.assertEqual(result.returncode, 2)
        self.assertIn("database path must not be empty", result.stderr)

    def test_shipped_resources_are_read_only_through_local_crud(self) -> None:
        moved = self.run_command(
            str(ROOT / "practice"),
            "sets",
            "move",
            "blind75",
            "two-sum",
            "--index",
            "1",
        )
        updated = self.run_command(
            str(ROOT / "practice"),
            "problems",
            "update",
            "two-sum",
            "--title",
            "Changed",
        )
        self.assertEqual(moved.returncode, 2)
        self.assertEqual(updated.returncode, 2)
        discovery_marker = Path(self.temporary_directory.name) / "discovery-called"
        fixture_runner = Path(self.temporary_directory.name) / "runner"
        fixture_runner.write_text(
            f"#!/bin/sh\ntouch {discovery_marker}\nprintf '%s\\n' two-sum\n",
            encoding="utf-8",
        )
        fixture_runner.chmod(0o755)
        with sqlite3.connect(self.database) as connection:
            connection.execute(
                "UPDATE languages SET runner_path = ? WHERE slug = 'python'",
                (str(fixture_runner),),
            )
        adapter = self.run_command(
            str(ROOT / "practice"),
            "problems",
            "adapter",
            "two-sum",
            "python",
            "missing-solution.py",
        )
        self.assertEqual(adapter.returncode, 2)
        self.assertIn("read-only", moved.stderr)
        self.assertIn("read-only", updated.stderr)
        self.assertIn("read-only", adapter.stderr)
        self.assertFalse(discovery_marker.exists())
        with sqlite3.connect(self.database) as connection:
            solution_path = connection.execute(
                """
                SELECT i.solution_path
                FROM problem_implementations AS i
                JOIN problems AS p ON p.id = i.problem_id
                JOIN languages AS l ON l.id = i.language_id
                WHERE p.slug = 'two-sum' AND l.slug = 'python'
                """
            ).fetchone()
        self.assertEqual(solution_path, ("python/problems/easy/two_sum.py",))

    def test_empty_custom_set_reports_zero_progress(self) -> None:
        created = self.run_command(
            str(ROOT / "practice"), "sets", "create", "empty", "--name", "Empty"
        )
        self.assertEqual(created.returncode, 0, created.stderr)
        stats = self.run_command(str(ROOT / "practice"), "--set", "empty", "stats")
        self.assertEqual(stats.returncode, 0, stats.stderr)
        self.assertIn("0/0 (0.0%)", stats.stdout)

    def test_set_membership_insert_move_and_remove_keep_contiguous_order(self) -> None:
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"), "sets", "create", "ordered", "--name", "Ordered"
            ).returncode,
            0,
        )
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"), "sets", "add", "ordered", "two-sum"
            ).returncode,
            0,
        )
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"),
                "sets",
                "add",
                "ordered",
                "climbing-stairs",
                "--index",
                "1",
            ).returncode,
            0,
        )
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"),
                "sets",
                "move",
                "ordered",
                "two-sum",
                "--index",
                "1",
            ).returncode,
            0,
        )
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"), "sets", "remove", "ordered", "climbing-stairs"
            ).returncode,
            0,
        )
        with sqlite3.connect(self.database) as connection:
            rows = connection.execute(
                """
                SELECT p.slug, m.ordinal
                FROM problem_set_members AS m
                JOIN problems AS p ON p.id = m.problem_id
                JOIN problem_sets AS ps ON ps.id = m.problem_set_id
                WHERE ps.slug = 'ordered' ORDER BY m.ordinal
                """
            ).fetchall()
        self.assertEqual(rows, [("two-sum", 1)])

    def test_progress_for_a_problem_is_shared_across_problem_sets(self) -> None:
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"),
                "sets",
                "create",
                "favorites",
                "--name",
                "Favorites",
            ).returncode,
            0,
        )
        self.assertEqual(
            self.run_command(
                str(ROOT / "practice"), "sets", "add", "favorites", "two-sum"
            ).returncode,
            0,
        )
        recorded = self.run_command(
            str(ROOT / "practice"),
            "_record",
            "python",
            "two-sum",
            "pass",
            "5",
            "--problem-set",
            "favorites",
        )
        self.assertEqual(recorded.returncode, 0, recorded.stderr)
        blind_stats = self.run_command(
            str(ROOT / "practice"), "--set", "blind75", "stats", "--language", "python"
        )
        favorite_stats = self.run_command(
            str(ROOT / "practice"),
            "--set",
            "favorites",
            "stats",
            "--language",
            "python",
        )
        global_stats = self.run_command(
            str(ROOT / "practice"), "stats", "--global", "--language", "python"
        )
        self.assertIn("1/75", blind_stats.stdout)
        self.assertIn("1/1", favorite_stats.stdout)
        self.assertIn("1/75", global_stats.stdout)

    def test_root_rust_run_executes_the_registered_case_before_recording(self) -> None:
        result = self.run_command(str(ROOT / "run"), "rust", "two-sum")
        self.assertNotEqual(result.returncode, 0)
        with sqlite3.connect(self.database) as connection:
            attempt = connection.execute(
                """
                SELECT a.result, a.exit_code
                FROM attempts AS a
                JOIN problems AS p ON p.id = a.problem_id
                WHERE p.slug = 'two-sum'
                ORDER BY a.id DESC LIMIT 1
                """
            ).fetchone()
        self.assertIsNotNone(attempt)
        self.assertEqual(attempt[0], "fail")
        self.assertNotEqual(attempt[1], 0)

    def test_root_run_resolves_problem_set_index_before_dispatch(self) -> None:
        result = self.run_command(str(ROOT / "run"), "python", "blind75", "16")
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL two-sum", result.stderr)
        with sqlite3.connect(self.database) as connection:
            attempts = connection.execute(
                """
                SELECT p.slug, ps.slug
                FROM attempts AS a
                JOIN problems AS p ON p.id = a.problem_id
                LEFT JOIN problem_sets AS ps ON ps.id = a.invoked_set_id
                ORDER BY a.id
                """
            ).fetchall()
        self.assertEqual(attempts, [("two-sum", "blind75")])

    def test_language_all_propagates_adapter_discovery_failure(self) -> None:
        binary_directory = Path(self.temporary_directory.name) / "bin"
        binary_directory.mkdir()
        for command, status in (("python3", 7), ("cargo", 8)):
            executable = binary_directory / command
            executable.write_text(f"#!/bin/sh\nexit {status}\n", encoding="utf-8")
            executable.chmod(0o755)
        self.environment["PATH"] = f"{binary_directory}:{self.environment['PATH']}"
        python_result = self.run_command(str(ROOT / "python" / "run"), "all")
        rust_result = self.run_command(str(ROOT / "rust" / "run"), "all")
        self.assertEqual(python_result.returncode, 7)
        self.assertEqual(rust_result.returncode, 8)

    def test_language_local_runner_remains_a_recording_compatibility_entrypoint(
        self,
    ) -> None:
        result = self.run_command(str(ROOT / "python" / "run"), "two-sum")
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL two-sum", result.stderr)
        with sqlite3.connect(self.database) as connection:
            attempt_count = connection.execute(
                "SELECT COUNT(*) FROM attempts"
            ).fetchone()[0]
        self.assertEqual(attempt_count, 1)

    def test_cli_is_a_self_hosted_rust_executable(self) -> None:
        version = self.run_command(str(ROOT / "practice"), "--version")
        self.assertEqual(version.returncode, 0, version.stderr)
        self.assertEqual(version.stdout.strip(), "practice 0.1.0")

        binary_directory = Path(self.temporary_directory.name) / "no-python"
        binary_directory.mkdir()
        python = binary_directory / "python3"
        python.write_text("#!/bin/sh\nexit 93\n", encoding="utf-8")
        python.chmod(0o755)
        self.environment["PATH"] = f"{binary_directory}:{self.environment['PATH']}"
        result = self.run_command(str(ROOT / "practice"), "sets", "list")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("blind75", result.stdout)

    def test_root_run_accepts_a_global_problem_slug_without_a_set(self) -> None:
        result = self.run_command(str(ROOT / "run"), "python", "two-sum")
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL two-sum", result.stderr)
        with sqlite3.connect(self.database) as connection:
            invoked_set = connection.execute(
                "SELECT invoked_set_id FROM attempts"
            ).fetchone()[0]
        self.assertIsNone(invoked_set)

    def test_problem_without_language_adapter_fails_before_dispatch(self) -> None:
        self.run_command(
            str(ROOT / "practice"),
            "problems",
            "add",
            "metadata-only",
            "--title",
            "Metadata Only",
            "--difficulty",
            "Easy",
            "--topic",
            "Custom",
        )
        result = self.run_command(str(ROOT / "run"), "python", "metadata-only")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no active python adapter", result.stderr)


if __name__ == "__main__":
    unittest.main()
