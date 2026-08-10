from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import sqlite3
import subprocess
import time

from practice_tool.database import (
    get_implementation,
    record_attempt,
    resolve_problem,
)


@dataclass(frozen=True, slots=True)
class ExecutionPlan:
    language: str
    problem_slug: str
    set_slug: str | None
    runner_path: Path
    solution_path: Path


@dataclass(frozen=True, slots=True)
class ExecutionResult:
    exit_code: int
    duration_ms: int


def plan_execution(
    connection: sqlite3.Connection,
    root: Path,
    *,
    language: str,
    problem_reference: str,
    set_slug: str | None,
) -> ExecutionPlan:
    problem = resolve_problem(connection, problem_reference, set_slug)
    implementation = get_implementation(connection, problem["id"], language)
    runner = root / implementation["runner_path"]
    solution = root / implementation["solution_path"]
    if not runner.is_file():
        raise ValueError(f"language runner is not installed: {runner}")
    if not solution.is_file():
        raise ValueError(f"solution file is not installed: {solution}")
    return ExecutionPlan(
        language=language,
        problem_slug=problem["slug"],
        set_slug=set_slug,
        runner_path=runner,
        solution_path=solution,
    )


def execute_plan(plan: ExecutionPlan, database_path: Path) -> ExecutionResult:
    environment = os.environ.copy()
    environment["PRACTICE_NO_RECORD"] = "1"
    environment["PRACTICE_DB_PATH"] = str(database_path)
    started_ns = time.monotonic_ns()
    completed = subprocess.run(
        [str(plan.runner_path), "--problem", plan.problem_slug],
        cwd=plan.runner_path.parent,
        env=environment,
        check=False,
    )
    duration_ms = max(0, (time.monotonic_ns() - started_ns) // 1_000_000)
    return ExecutionResult(exit_code=completed.returncode, duration_ms=duration_ms)


def record_execution(
    connection: sqlite3.Connection,
    plan: ExecutionPlan,
    result: ExecutionResult,
) -> None:
    record_attempt(
        connection,
        problem_slug=plan.problem_slug,
        language_slug=plan.language,
        result=(
            "pass"
            if result.exit_code == 0
            else "error"
            if result.exit_code == 2
            else "fail"
        ),
        duration_ms=result.duration_ms,
        exit_code=result.exit_code,
        set_slug=plan.set_slug,
    )
