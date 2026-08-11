from __future__ import annotations

import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "catalog" / "problems.json"
BLIND75_PATH = ROOT / "problem_sets" / "blind75.json"
MAX_STATEMENT_LENGTH = 1_000_000


class CatalogContentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads(CATALOG_PATH.read_text(encoding="utf-8"))
        cls.problems = cls.catalog["problems"]
        cls.by_slug = {problem["slug"]: problem for problem in cls.problems}
        cls.blind75 = json.loads(BLIND75_PATH.read_text(encoding="utf-8"))

    def test_shipped_statements_are_complete_unique_and_bounded(self) -> None:
        statements = [problem["statement_markdown"] for problem in self.problems]
        self.assertEqual(len(statements), 75)
        self.assertTrue(all(statement.strip() for statement in statements))
        self.assertEqual(len(set(statements)), 75)
        self.assertTrue(
            all(len(statement) <= MAX_STATEMENT_LENGTH for statement in statements)
        )

    def test_blind75_exactly_covers_catalog_in_contiguous_order(self) -> None:
        members = self.blind75["members"]
        self.assertEqual([member["ordinal"] for member in members], list(range(1, 76)))
        self.assertEqual(
            {member["problem_slug"] for member in members}, set(self.by_slug)
        )

    def test_every_problem_has_exactly_the_registered_python_and_rust_adapters(
        self,
    ) -> None:
        python_contract = json.loads(
            subprocess.run(
                [
                    "python3",
                    "-c",
                    (
                        "import json; from local_judge.registry import PROBLEMS; "
                        "from tests.cases import CUSTOM_TESTS, SIMPLE_CASES; "
                        "print(json.dumps({'paths': {p.slug: p.path for p in PROBLEMS}, "
                        "'cases': sorted(set(SIMPLE_CASES) | set(CUSTOM_TESTS))}))"
                    ),
                ],
                cwd=ROOT / "python",
                check=True,
                capture_output=True,
                text=True,
            ).stdout
        )
        python_paths = python_contract["paths"]
        rust_slugs = set(
            subprocess.run(
                [str(ROOT / "rust" / "run"), "--list"],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.splitlines()
        )
        case_slugs = set(python_contract["cases"])

        self.assertEqual(set(self.by_slug), set(python_paths))
        self.assertEqual(set(self.by_slug), case_slugs)
        self.assertEqual(set(self.by_slug), rust_slugs)
        for slug, problem in self.by_slug.items():
            adapters = {
                adapter["language"]: adapter["solution_path"]
                for adapter in problem["adapters"]
            }
            self.assertEqual(set(adapters), {"python", "rust"})
            self.assertEqual(adapters["python"], f"python/{python_paths[slug]}")
            self.assertTrue((ROOT / adapters["python"]).is_file())
            self.assertTrue((ROOT / adapters["rust"]).is_file())

    def test_statements_have_local_task_and_example_structure(self) -> None:
        for problem in self.problems:
            with self.subTest(slug=problem["slug"]):
                statement = problem["statement_markdown"]
                self.assertIn("## Task", statement)
                self.assertIn("## Example", statement)
                self.assertNotIn("leetcode.com", statement.lower())
                self.assertNotIn("neetcode.io", statement.lower())


if __name__ == "__main__":
    unittest.main()
