from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "catalog" / "problems.json"
PROBLEM_SET_DIRECTORY = ROOT / "problem_sets"
MAX_STATEMENT_LENGTH = 1_000_000
REGISTRY_TIMEOUT_SECONDS = 10

EXPECTED_INTERVIEW_SETS = {
    "convex": {
        "name": "Convex",
        "members": [
            "longest-substring-without-repeating-characters",
            "merge-intervals",
            "top-k-frequent-elements",
            "time-based-key-value-store",
            "lru-cache",
            "course-schedule",
            "number-of-islands",
            "binary-tree-level-order-traversal",
            "kth-largest-element-in-a-stream",
            "coin-change",
        ],
    },
    "anti-metal": {
        "name": "anti-metal",
        "members": [
            "time-based-key-value-store",
            "lru-cache",
            "course-schedule",
            "merge-intervals",
            "top-k-frequent-elements",
            "longest-substring-without-repeating-characters",
            "number-of-islands",
            "binary-tree-level-order-traversal",
            "kth-largest-element-in-a-stream",
            "coin-change",
        ],
    },
    "depot": {
        "name": "Depot",
        "members": [
            "lru-cache",
            "course-schedule",
            "meeting-rooms-ii",
            "merge-intervals",
            "time-based-key-value-store",
            "top-k-frequent-elements",
            "number-of-islands",
            "sliding-window-maximum",
            "kth-largest-element-in-a-stream",
            "coin-change",
        ],
    },
}

EXPECTED_NEW_PROBLEMS = {
    "kth-largest-element-in-a-stream": {
        "title": "Kth Largest Element in a Stream",
        "difficulty": "Easy",
        "leetcode_id": 703,
        "leetcode_url": "https://leetcode.com/problems/kth-largest-element-in-a-stream/",
    },
    "lru-cache": {
        "title": "LRU Cache",
        "difficulty": "Medium",
        "leetcode_id": 146,
        "leetcode_url": "https://leetcode.com/problems/lru-cache/",
    },
    "time-based-key-value-store": {
        "title": "Time Based Key-Value Store",
        "difficulty": "Medium",
        "leetcode_id": 981,
        "leetcode_url": "https://leetcode.com/problems/time-based-key-value-store/",
    },
    "sliding-window-maximum": {
        "title": "Sliding Window Maximum",
        "difficulty": "Hard",
        "leetcode_id": 239,
        "leetcode_url": "https://leetcode.com/problems/sliding-window-maximum/",
    },
}


class CatalogContentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads(CATALOG_PATH.read_text(encoding="utf-8"))
        cls.problems = cls.catalog["problems"]
        catalog_slugs = [problem["slug"] for problem in cls.problems]
        if len(catalog_slugs) != len(set(catalog_slugs)):
            raise AssertionError("catalog problem slugs must be unique")
        cls.by_slug = {problem["slug"]: problem for problem in cls.problems}
        cls.problem_sets = {
            path.stem: json.loads(path.read_text(encoding="utf-8"))
            for path in PROBLEM_SET_DIRECTORY.glob("*.json")
        }

    def test_shipped_statements_are_complete_unique_and_bounded(self) -> None:
        statements = [problem["statement_markdown"] for problem in self.problems]
        self.assertEqual(len(statements), len(self.problems))
        self.assertTrue(all(statement.strip() for statement in statements))
        self.assertEqual(len(set(statements)), len(statements))
        self.assertTrue(all(len(statement) <= MAX_STATEMENT_LENGTH for statement in statements))

    def test_catalog_has_79_problems_and_expected_new_canonical_metadata(self) -> None:
        self.assertEqual(self.catalog["catalog_revision"], 4)
        self.assertEqual(len(self.problems), 79)
        self.assertEqual([problem["slug"] for problem in self.problems], sorted(self.by_slug))
        for slug, expected in EXPECTED_NEW_PROBLEMS.items():
            with self.subTest(slug=slug):
                problem = self.by_slug[slug]
                self.assertEqual(
                    {field: problem[field] for field in expected},
                    expected,
                )
                self.assertFalse(problem["premium"])
                self.assertEqual(problem["test_revision"], 1)

    def test_all_four_shipped_sets_have_exact_ordered_catalog_members(self) -> None:
        self.assertEqual(set(self.problem_sets), {"anti-metal", "blind75", "convex", "depot"})

        blind75_members = self.problem_sets["blind75"]["members"]
        blind75_slugs = [member["problem_slug"] for member in blind75_members]
        self.assertEqual(len(blind75_members), 75)
        self.assertEqual([member["ordinal"] for member in blind75_members], list(range(1, 76)))
        self.assertEqual(len(blind75_slugs), len(set(blind75_slugs)))
        self.assertLessEqual(set(blind75_slugs), set(self.by_slug))

        for set_id, expected in EXPECTED_INTERVIEW_SETS.items():
            with self.subTest(set_id=set_id):
                problem_set = self.problem_sets[set_id]
                self.assertEqual(problem_set["id"], set_id)
                self.assertEqual(problem_set["name"], expected["name"])
                members = problem_set["members"]
                self.assertEqual([member["ordinal"] for member in members], list(range(1, 11)))
                self.assertEqual(
                    [member["problem_slug"] for member in members],
                    expected["members"],
                )
                self.assertLessEqual(set(expected["members"]), set(self.by_slug))

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
                        "print(json.dumps({'registry': [[p.slug, p.path] for p in PROBLEMS], "
                        "'simple_cases': list(SIMPLE_CASES), "
                        "'custom_cases': list(CUSTOM_TESTS)}))"
                    ),
                ],
                cwd=ROOT / "python",
                check=True,
                capture_output=True,
                text=True,
                timeout=REGISTRY_TIMEOUT_SECONDS,
            ).stdout
        )
        python_registry = python_contract["registry"]
        python_slugs = [entry[0] for entry in python_registry]
        simple_case_slugs = python_contract["simple_cases"]
        custom_case_slugs = python_contract["custom_cases"]

        self.assertEqual(len(python_slugs), len(set(python_slugs)))
        self.assertEqual(python_slugs, sorted(python_slugs))
        self.assertEqual(len(simple_case_slugs), len(set(simple_case_slugs)))
        self.assertEqual(len(custom_case_slugs), len(set(custom_case_slugs)))
        self.assertTrue(set(simple_case_slugs).isdisjoint(custom_case_slugs))

        python_paths = dict(python_registry)
        case_slugs = simple_case_slugs + custom_case_slugs
        self.assertEqual(len(case_slugs), len(python_slugs))

        rust_registry_source = (ROOT / "rust" / "src" / "problems" / "mod.rs").read_text(
            encoding="utf-8"
        )
        rust_entries = re.findall(
            r'Problem::new\(\s*"([^"]+)",\s*([a-z0-9_]+)::run_case,?\s*\)',
            rust_registry_source,
        )
        rust_registry_slugs = [slug for slug, _ in rust_entries]
        rust_registry_modules = [module for _, module in rust_entries]
        self.assertEqual(len(rust_entries), rust_registry_source.count("Problem::new("))
        self.assertEqual(len(rust_registry_slugs), len(set(rust_registry_slugs)))
        self.assertEqual(rust_registry_slugs, sorted(rust_registry_slugs))
        self.assertEqual(len(rust_registry_modules), len(set(rust_registry_modules)))

        rust_list_slugs = subprocess.run(
            [str(ROOT / "rust" / "run"), "--list"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
            timeout=REGISTRY_TIMEOUT_SECONDS,
        ).stdout.splitlines()
        self.assertEqual(len(rust_list_slugs), len(set(rust_list_slugs)))
        self.assertEqual(rust_list_slugs, rust_registry_slugs)

        self.assertEqual(set(self.by_slug), set(python_slugs))
        self.assertEqual(set(self.by_slug), set(case_slugs))
        self.assertEqual(set(self.by_slug), set(rust_registry_slugs))
        rust_paths = {slug: f"rust/src/problems/{module}.rs" for slug, module in rust_entries}
        for slug, problem in self.by_slug.items():
            adapter_languages = [adapter["language"] for adapter in problem["adapters"]]
            self.assertEqual(len(adapter_languages), len(set(adapter_languages)))
            adapters = {
                adapter["language"]: adapter["solution_path"] for adapter in problem["adapters"]
            }
            self.assertEqual(set(adapters), {"python", "rust"})
            self.assertEqual(adapters["python"], f"python/{python_paths[slug]}")
            self.assertEqual(adapters["rust"], rust_paths[slug])
            self.assertTrue((ROOT / adapters["python"]).is_file())
            self.assertTrue((ROOT / adapters["rust"]).is_file())

    def test_statements_have_local_task_and_example_structure(self) -> None:
        for problem in self.problems:
            with self.subTest(slug=problem["slug"]):
                statement = problem["statement_markdown"]
                self.assertEqual(statement.count("## Task"), 1)
                self.assertEqual(statement.count("## Example"), 1)
                sections = re.fullmatch(
                    r"## Task\n\n(.+?)\n\n## Example\n\n(.+)",
                    statement,
                    flags=re.DOTALL,
                )
                self.assertIsNotNone(sections)
                assert sections is not None
                self.assertTrue(sections.group(1).strip())
                self.assertTrue(sections.group(2).strip())
                self.assertNotIn("## Task", sections.group(1))
                self.assertNotIn("## Example", sections.group(2))
                self.assertNotIn("leetcode.com", statement.lower())
                self.assertNotIn("neetcode.io", statement.lower())


if __name__ == "__main__":
    unittest.main()
