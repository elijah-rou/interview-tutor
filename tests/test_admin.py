from __future__ import annotations

import json
import os
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
