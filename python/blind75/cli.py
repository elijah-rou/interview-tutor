from __future__ import annotations

import importlib.util
import sys
import traceback
from pathlib import Path
from types import ModuleType

from blind75.manifest import BY_SLUG, PROBLEMS, Problem
from blind75.structures import ListNode, Node, TreeNode
from tests.cases import CUSTOM_TESTS, SIMPLE_CASES, test_simple

ROOT = Path(__file__).resolve().parent.parent


def load_problem(problem: Problem) -> ModuleType:
    path = ROOT / problem.path
    spec = importlib.util.spec_from_file_location(f"blind75_starter_{problem.slug}", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    # LeetCode supplies these definitions. The local runner supplies the shared equivalents.
    module.ListNode = ListNode
    module.TreeNode = TreeNode
    module.Node = Node
    spec.loader.exec_module(module)
    return module


def print_problems() -> None:
    for problem in PROBLEMS:
        print(f"{problem.order:02d}  {problem.difficulty:<6}  {problem.slug}")


def run(problem: Problem) -> None:
    module = load_problem(problem)
    custom = CUSTOM_TESTS.get(problem.slug)
    if custom is not None:
        custom(module)
        return
    case = SIMPLE_CASES.get(problem.slug)
    if case is None or problem.method_name is None:
        raise RuntimeError(f"test registry is incomplete for {problem.slug}")
    test_simple(module, problem.method_name, case)


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    if arguments == ["--list"]:
        print_problems()
        return 0
    if len(arguments) != 1 or arguments[0].startswith("-"):
        print("usage: ./run <slug>\n       ./run --list", file=sys.stderr)
        return 2
    slug = arguments[0]
    problem = BY_SLUG.get(slug)
    if problem is None:
        print(f"unknown problem: {slug!r}", file=sys.stderr)
        print("use ./run --list to see valid slugs", file=sys.stderr)
        return 2
    try:
        run(problem)
    except NotImplementedError:
        print(f"FAIL {slug}: starter is not implemented", file=sys.stderr)
        return 1
    except Exception as error:
        print(f"FAIL {slug}: {error}", file=sys.stderr)
        if "--traceback" in arguments:
            traceback.print_exc()
        return 1
    print(f"PASS {slug}")
    return 0


assert set(SIMPLE_CASES) | set(CUSTOM_TESTS) == set(BY_SLUG)
assert not (set(SIMPLE_CASES) & set(CUSTOM_TESTS))

if __name__ == "__main__":
    raise SystemExit(main())
