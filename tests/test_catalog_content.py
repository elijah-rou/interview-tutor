from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "catalog" / "problems.json"
BLIND75_PATH = ROOT / "problem_sets" / "blind75.json"
MAX_STATEMENT_LENGTH = 1_000_000
REGISTRY_TIMEOUT_SECONDS = 10


class CatalogContentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads(CATALOG_PATH.read_text(encoding="utf-8"))
        cls.problems = cls.catalog["problems"]
        catalog_slugs = [problem["slug"] for problem in cls.problems]
        if len(catalog_slugs) != len(set(catalog_slugs)):
            raise AssertionError("catalog problem slugs must be unique")
        cls.by_slug = {problem["slug"]: problem for problem in cls.problems}
        cls.blind75 = json.loads(BLIND75_PATH.read_text(encoding="utf-8"))

    def test_shipped_statements_are_complete_unique_and_bounded(self) -> None:
        statements = [problem["statement_markdown"] for problem in self.problems]
        self.assertEqual(len(statements), len(self.problems))
        self.assertTrue(all(statement.strip() for statement in statements))
        self.assertEqual(len(set(statements)), len(statements))
        self.assertTrue(
            all(len(statement) <= MAX_STATEMENT_LENGTH for statement in statements)
        )

    def test_blind75_has_75_unique_contiguous_catalog_members(self) -> None:
        members = self.blind75["members"]
        member_slugs = [member["problem_slug"] for member in members]

        self.assertEqual(len(members), 75)
        self.assertEqual([member["ordinal"] for member in members], list(range(1, 76)))
        self.assertEqual(len(member_slugs), len(set(member_slugs)))
        self.assertLessEqual(set(member_slugs), set(self.by_slug))

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
        self.assertEqual(len(simple_case_slugs), len(set(simple_case_slugs)))
        self.assertEqual(len(custom_case_slugs), len(set(custom_case_slugs)))
        self.assertTrue(set(simple_case_slugs).isdisjoint(custom_case_slugs))

        python_paths = dict(python_registry)
        case_slugs = simple_case_slugs + custom_case_slugs
        self.assertEqual(len(case_slugs), len(python_slugs))

        rust_registry_source = (
            ROOT / "rust" / "src" / "problems" / "mod.rs"
        ).read_text(encoding="utf-8")
        rust_entries = re.findall(
            r'Problem::new\(\s*"([^"]+)",\s*([a-z0-9_]+)::run_case,?\s*\)',
            rust_registry_source,
        )
        rust_registry_slugs = [slug for slug, _ in rust_entries]
        rust_registry_modules = [module for _, module in rust_entries]
        self.assertEqual(len(rust_entries), rust_registry_source.count("Problem::new("))
        self.assertEqual(len(rust_registry_slugs), len(set(rust_registry_slugs)))
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
        rust_paths = {
            slug: f"rust/src/problems/{module}.rs" for slug, module in rust_entries
        }
        for slug, problem in self.by_slug.items():
            adapter_languages = [adapter["language"] for adapter in problem["adapters"]]
            self.assertEqual(len(adapter_languages), len(set(adapter_languages)))
            adapters = {
                adapter["language"]: adapter["solution_path"]
                for adapter in problem["adapters"]
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
