from __future__ import annotations

import importlib.util
import sys
import traceback
from collections.abc import Callable
from pathlib import Path
from types import ModuleType

from local_judge.registry import BY_SLUG, PROBLEMS, Problem
from local_judge.structures import ListNode, Node, TreeNode
from tests.cases import CUSTOM_TESTS, SIMPLE_CASES, test_simple

ROOT = Path(__file__).resolve().parent.parent


def load_problem(problem: Problem) -> ModuleType:
    path = ROOT / problem.path
    spec = importlib.util.spec_from_file_location(f"local_judge_starter_{problem.slug}", path)
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
        print(problem.slug)


def cases(problem: Problem) -> dict[str, Callable[[ModuleType], None]]:
    """Named cases in execution order. Each existing scenario is one case until it is split."""
    custom = CUSTOM_TESTS.get(problem.slug)
    if custom is not None:
        return {"scenario-1": custom}
    case = SIMPLE_CASES.get(problem.slug)
    if case is None or problem.method_name is None:
        raise RuntimeError(f"test registry is incomplete for {problem.slug}")
    method_name = problem.method_name
    return {"example-1": lambda module: test_simple(module, method_name, case)}


def run_case(problem: Problem, name: str) -> int:
    case = cases(problem).get(name)
    if case is None:
        print(f"unknown case for {problem.slug}: {name!r}", file=sys.stderr)
        return 2
    try:
        case(load_problem(problem))
    except NotImplementedError:
        print(f"FAIL {problem.slug}: starter is not implemented", file=sys.stderr)
        return 1
    except Exception as error:
        traceback.print_exc()
        print(f"FAIL {problem.slug}: {error}", file=sys.stderr)
        return 1
    return 0


def lookup(slug: str) -> Problem | None:
    problem = BY_SLUG.get(slug)
    if problem is None:
        print(f"unknown problem: {slug!r}", file=sys.stderr)
        print("use ./run --list to see valid slugs", file=sys.stderr)
    return problem


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    match arguments:
        case ["--list"]:
            print_problems()
            return 0
        case ["--cases", slug]:
            # Lists names from the registry only; the solution is not imported.
            problem = lookup(slug)
            if problem is None:
                return 2
            for name in cases(problem):
                print(name)
            return 0
        case ["--case", slug, name]:
            problem = lookup(slug)
            if problem is None:
                return 2
            status = run_case(problem, name)
            if status == 0:
                print(f"PASS {slug}")
            return status
        case [slug] if not slug.startswith("-"):
            problem = lookup(slug)
            if problem is None:
                return 2
            status = 0
            for name in cases(problem):
                status = max(status, run_case(problem, name))
            if status == 0:
                print(f"PASS {slug}")
            return status
        case _:
            print(
                "usage: ./run <slug>\n       ./run --list\n"
                "       ./run --cases <slug>\n       ./run --case <slug> <case>",
                file=sys.stderr,
            )
            return 2


assert set(SIMPLE_CASES) | set(CUSTOM_TESTS) == set(BY_SLUG)
assert not (set(SIMPLE_CASES) & set(CUSTOM_TESTS))

if __name__ == "__main__":
    raise SystemExit(main())
