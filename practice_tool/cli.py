from __future__ import annotations

import argparse
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

from practice_tool.catalog import DIFFICULTIES
from practice_tool.database import (
    MAX_STATEMENT_LENGTH,
    add_implementation,
    add_set_member,
    completed_problem_ids,
    create_problem,
    create_problem_set,
    delete_problem,
    delete_problem_set,
    open_database,
    get_problem_set,
    list_set_members,
    move_set_member,
    record_attempt,
    remove_set_member,
    resolve_problem,
    update_problem,
    update_problem_set,
)
from practice_tool.runner import execute_plan, plan_execution, record_execution

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DATABASE_PATH = ROOT / ".turso" / "progress.db"


def resolve_database_path(cli_path: str | None) -> Path:
    configured = (
        cli_path
        or os.environ.get("PRACTICE_DATABASE_URL")
        or os.environ.get("PRACTICE_DB_PATH")
        or os.environ.get("BLIND75_DATABASE_URL")
        or os.environ.get("BLIND75_DB_PATH")
    )
    if not configured:
        return DEFAULT_DATABASE_PATH
    if configured.startswith("file:"):
        configured = configured.removeprefix("file:")
    path = Path(configured).expanduser()
    return path if path.is_absolute() else (ROOT / path).resolve()


def print_table(headers: list[str], rows: list[list[object]]) -> None:
    text_rows = [[str(cell) for cell in row] for row in rows]
    widths = [len(header) for header in headers]
    for row in text_rows:
        if len(row) != len(headers):
            raise RuntimeError("table row does not match its header")
        widths = [max(width, len(cell)) for width, cell in zip(widths, row)]
    print("  ".join(header.ljust(width) for header, width in zip(headers, widths)))
    print("  ".join("-" * width for width in widths))
    for row in text_rows:
        print("  ".join(cell.ljust(width) for cell, width in zip(row, widths)))


def command_list(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    members = list_set_members(connection, args.problem_set)
    languages = connection.execute(
        "SELECT slug FROM languages WHERE enabled = 1 ORDER BY slug"
    ).fetchall()
    completed = {
        language["slug"]: completed_problem_ids(connection, language["slug"])
        for language in languages
    }
    rows: list[list[object]] = []
    for problem in members:
        if args.difficulty and problem["difficulty"].lower() != args.difficulty:
            continue
        if args.topic and args.topic.lower() not in problem["topic"].lower():
            continue
        rows.append(
            [
                problem["ordinal"],
                problem["difficulty"],
                problem["topic"],
                *[
                    "yes" if problem["id"] in completed[language["slug"]] else "-"
                    for language in languages
                ],
                problem["slug"],
            ]
        )
    print_table(
        [
            "#",
            "Difficulty",
            "Topic",
            *[row["slug"].title() for row in languages],
            "Problem",
        ],
        rows,
    )
    return 0


def print_problem(problem: sqlite3.Row, set_slug: str | None) -> None:
    print(problem["title"])
    print(f"Slug: {problem['slug']}")
    if set_slug is not None:
        print(f"Problem set: {set_slug} #{problem['ordinal']}")
    print(f"Difficulty: {problem['difficulty']}")
    print(f"Topic: {problem['topic']}")
    if problem["archived"]:
        print("State: archived")
    if problem["leetcode_url"]:
        print(f"LeetCode: {problem['leetcode_url']}")
    if problem["neetcode_url"]:
        print(f"NeetCode: {problem['neetcode_url']}")
    if problem["statement_markdown"]:
        print("\n" + problem["statement_markdown"])


def command_show(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    problem = resolve_problem(connection, args.problem, args.problem_set)
    print_problem(problem, args.problem_set)
    return 0


def command_stats(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    if args.global_stats:
        members = connection.execute(
            "SELECT p.*, NULL AS ordinal, NULL AS section "
            "FROM problems AS p WHERE p.archived = 0 ORDER BY p.slug"
        ).fetchall()
        set_name = "All Problems"
    else:
        members = list_set_members(connection, args.problem_set)
        set_name = get_problem_set(connection, args.problem_set)["name"]
    language = None if args.language == "any" else args.language
    if language is not None:
        installed = connection.execute(
            "SELECT 1 FROM languages WHERE slug = ? AND enabled = 1", (language,)
        ).fetchone()
        if installed is None:
            raise ValueError(f"unknown or disabled language: {language}")
    completed = completed_problem_ids(connection, language)
    done = sum(problem["id"] in completed for problem in members)
    total = len(members)
    percent = done / total if total else 0.0
    label = "any language" if language is None else language
    print(f"{set_name} progress ({label}): {done}/{total} ({percent:.1%})")

    for field, heading in (("difficulty", "By difficulty"), ("topic", "By topic")):
        groups: dict[str, list[sqlite3.Row]] = {}
        for problem in members:
            groups.setdefault(problem[field], []).append(problem)
        names = list(groups)
        if field == "difficulty":
            names = [name for name in DIFFICULTIES if name in groups]
        rows = []
        for name in names:
            group = groups[name]
            group_done = sum(problem["id"] in completed for problem in group)
            rows.append(
                [name, group_done, len(group), f"{group_done / len(group):.1%}"]
            )
        if rows:
            print(f"\n{heading}")
            print_table(
                [heading.removeprefix("By ").title(), "Done", "Total", "Progress"],
                rows,
            )
    return 0


def run_one(
    args: argparse.Namespace,
    connection: sqlite3.Connection,
    *,
    language: str,
    problem_reference: str,
    set_slug: str | None,
) -> int:
    plan = plan_execution(
        connection,
        ROOT,
        language=language,
        problem_reference=problem_reference,
        set_slug=set_slug,
    )
    result = execute_plan(plan, args.database_path)
    record_execution(connection, plan, result)
    return result.exit_code


def command_run(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    if len(args.selectors) == 1:
        set_slug = None
        reference = args.selectors[0]
    elif len(args.selectors) == 2:
        set_slug, reference = args.selectors
    else:
        raise ValueError(
            "run expects LANGUAGE PROBLEM or LANGUAGE SET PROBLEM_OR_INDEX"
        )
    return run_one(
        args,
        connection,
        language=args.language,
        problem_reference=reference,
        set_slug=set_slug,
    )


def command_test(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    # Compatibility for the v1 command. New callers should use root ./run.
    if args.problem == "all":
        status = 0
        for problem in list_set_members(connection, args.problem_set):
            result = run_one(
                args,
                connection,
                language=args.language,
                problem_reference=problem["slug"],
                set_slug=args.problem_set,
            )
            if result != 0:
                status = result
        return status
    return run_one(
        args,
        connection,
        language=args.language,
        problem_reference=args.problem,
        set_slug=args.problem_set,
    )


def command_db(args: argparse.Namespace, _: sqlite3.Connection) -> int:
    print(args.database_path)
    print(f"Turso local server: turso dev --db-file {args.database_path}")
    return 0


def command_record(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    record_attempt(
        connection,
        problem_slug=args.problem,
        language_slug=args.language,
        result=args.result,
        duration_ms=args.duration_ms,
        exit_code=args.exit_code,
        set_slug=args.invoked_set,
    )
    return 0


def command_problems_list(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    archived_filter = "" if args.include_archived else "WHERE p.archived = 0"
    rows = connection.execute(
        f"""
        SELECT p.slug, p.difficulty, p.topic, p.archived,
               GROUP_CONCAT(l.slug, ', ') AS languages
        FROM problems AS p
        LEFT JOIN problem_implementations AS i
          ON i.problem_id = p.id AND i.enabled = 1
        LEFT JOIN languages AS l ON l.id = i.language_id AND l.enabled = 1
        {archived_filter}
        GROUP BY p.id ORDER BY p.slug
        """
    ).fetchall()
    print_table(
        ["Difficulty", "Topic", "Languages", "State", "Problem"],
        [
            [
                row["difficulty"],
                row["topic"],
                row["languages"] or "-",
                "archived" if row["archived"] else "active",
                row["slug"],
            ]
            for row in rows
        ],
    )
    return 0


def command_problems_show(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    problem = resolve_problem(connection, args.problem)
    print_problem(problem, None)
    adapters = connection.execute(
        """
        SELECT l.slug, i.solution_path
        FROM problem_implementations AS i
        JOIN languages AS l ON l.id = i.language_id
        WHERE i.problem_id = ? AND i.enabled = 1 ORDER BY l.slug
        """,
        (problem["id"],),
    ).fetchall()
    if adapters:
        print("\nAdapters")
        for adapter in adapters:
            print(f"{adapter['slug']}: {adapter['solution_path']}")
    return 0


def command_problems_add(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    statement = args.statement
    if args.statement_file:
        with Path(args.statement_file).open(encoding="utf-8") as statement_file:
            statement = statement_file.read(MAX_STATEMENT_LENGTH + 1)
    create_problem(
        connection,
        slug=args.problem,
        title=args.title,
        difficulty=args.difficulty,
        topic=args.topic,
        statement_markdown=statement,
        leetcode_id=args.leetcode_id,
        leetcode_url=args.leetcode_url,
        neetcode_url=args.neetcode_url,
        premium=args.premium,
    )
    print(f"Added problem: {args.problem}")
    return 0


def command_problems_update(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    statement = args.statement
    if args.statement_file:
        with Path(args.statement_file).open(encoding="utf-8") as statement_file:
            statement = statement_file.read(MAX_STATEMENT_LENGTH + 1)
    update_problem(
        connection,
        slug=args.problem,
        title=args.title,
        difficulty=args.difficulty,
        topic=args.topic,
        statement_markdown=statement,
        test_revision=args.test_revision,
        leetcode_id=args.leetcode_id,
        premium=args.premium,
        leetcode_url=args.leetcode_url,
        neetcode_url=args.neetcode_url,
    )
    print(f"Updated problem: {args.problem}")
    return 0


def command_problems_delete(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    delete_problem(connection, slug=args.problem)
    print(f"Deleted problem: {args.problem}")
    return 0


def command_problems_adapter(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    solution = ROOT / args.solution_path
    if not solution.is_file():
        raise ValueError(f"solution file does not exist: {solution}")
    language = connection.execute(
        "SELECT runner_path FROM languages WHERE slug = ? AND enabled = 1",
        (args.language,),
    ).fetchone()
    if language is None:
        raise ValueError(f"unknown or disabled language: {args.language}")
    runner = ROOT / language["runner_path"]
    described = subprocess.run(
        [str(runner), "--list"],
        cwd=runner.parent,
        text=True,
        capture_output=True,
        check=False,
    )
    if described.returncode != 0:
        raise ValueError(f"language runner discovery failed: {args.language}")
    if args.problem not in described.stdout.splitlines():
        raise ValueError(
            f"{args.language} runner does not expose problem adapter: {args.problem}"
        )
    add_implementation(
        connection,
        problem_slug=args.problem,
        language_slug=args.language,
        solution_path=args.solution_path,
    )
    print(f"Registered {args.language} adapter for {args.problem}")
    return 0


def command_sets_list(_: argparse.Namespace, connection: sqlite3.Connection) -> int:
    rows = connection.execute(
        """
        SELECT ps.slug, ps.name, COUNT(m.problem_id) AS problems
        FROM problem_sets AS ps
        LEFT JOIN problem_set_members AS m ON m.problem_set_id = ps.id
        GROUP BY ps.id ORDER BY ps.slug
        """
    ).fetchall()
    print_table(
        ["ID", "Name", "Problems"],
        [[row["slug"], row["name"], row["problems"]] for row in rows],
    )
    return 0


def command_sets_show(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    problem_set = get_problem_set(connection, args.problem_set_id)
    print(problem_set["name"])
    print(f"ID: {problem_set['slug']}")
    if problem_set["description"]:
        print(problem_set["description"])
    members = list_set_members(connection, args.problem_set_id)
    if members:
        print()
        print_table(
            ["#", "Difficulty", "Topic", "Problem"],
            [
                [row["ordinal"], row["difficulty"], row["topic"], row["slug"]]
                for row in members
            ],
        )
    return 0


def command_sets_update(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    update_problem_set(
        connection,
        slug=args.problem_set_id,
        name=args.name,
        description=args.description,
    )
    print(f"Updated problem set: {args.problem_set_id}")
    return 0


def command_sets_delete(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    delete_problem_set(connection, slug=args.problem_set_id)
    print(f"Deleted problem set: {args.problem_set_id}")
    return 0


def command_sets_create(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    create_problem_set(
        connection,
        slug=args.problem_set_id,
        name=args.name,
        description=args.description,
    )
    print(f"Created problem set: {args.problem_set_id}")
    return 0


def command_sets_add(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    add_set_member(
        connection,
        set_slug=args.problem_set_id,
        problem_slug=args.problem,
        index=args.index,
        section=args.section,
    )
    print(f"Added {args.problem} to {args.problem_set_id}")
    return 0


def command_sets_move(args: argparse.Namespace, connection: sqlite3.Connection) -> int:
    move_set_member(
        connection,
        set_slug=args.problem_set_id,
        problem_slug=args.problem,
        index=args.index,
    )
    print(f"Moved {args.problem} to #{args.index} in {args.problem_set_id}")
    return 0


def command_sets_remove(
    args: argparse.Namespace, connection: sqlite3.Connection
) -> int:
    remove_set_member(
        connection, set_slug=args.problem_set_id, problem_slug=args.problem
    )
    print(f"Removed {args.problem} from {args.problem_set_id}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="practice", description="Local algorithm practice catalog and progress CLI"
    )
    parser.add_argument("--db", help="database file or file: URL")
    parser.add_argument(
        "--set",
        dest="problem_set",
        default="blind75",
        metavar="ID",
        help="problem set for list/show/stats (default: blind75)",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    list_parser = subparsers.add_parser("list", help="list one problem set")
    list_parser.add_argument("--difficulty", choices=("easy", "medium", "hard"))
    list_parser.add_argument("--topic", help="case-insensitive topic substring")
    list_parser.set_defaults(handler=command_list)

    show_parser = subparsers.add_parser(
        "show", help="show a set problem by slug or index"
    )
    show_parser.add_argument("problem")
    show_parser.set_defaults(handler=command_show)

    stats_parser = subparsers.add_parser("stats", help="show set or global progress")
    stats_parser.add_argument("--language", default="any")
    stats_parser.add_argument(
        "--global",
        dest="global_stats",
        action="store_true",
        help="deduplicate all global problems",
    )
    stats_parser.set_defaults(handler=command_stats)

    run_parser = subparsers.add_parser("run", help="resolve and execute a problem")
    run_parser.add_argument("language")
    run_parser.add_argument("selectors", nargs="+")
    run_parser.set_defaults(handler=command_run)

    test_parser = subparsers.add_parser("test", help="compatibility execution command")
    test_parser.add_argument("language")
    test_parser.add_argument("problem")
    test_parser.set_defaults(handler=command_test)

    db_parser = subparsers.add_parser("db", help="initialize and show the database")
    db_parser.set_defaults(handler=command_db)

    problems_parser = subparsers.add_parser("problems", help="manage global problems")
    problems_subparsers = problems_parser.add_subparsers(
        dest="problems_command", required=True
    )
    problems_list = problems_subparsers.add_parser("list")
    problems_list.add_argument("--all", dest="include_archived", action="store_true")
    problems_list.set_defaults(handler=command_problems_list)
    problems_show = problems_subparsers.add_parser("show")
    problems_show.add_argument("problem")
    problems_show.set_defaults(handler=command_problems_show)
    problems_add = problems_subparsers.add_parser("add")
    problems_add.add_argument("problem")
    problems_add.add_argument("--title", required=True)
    problems_add.add_argument("--difficulty", choices=DIFFICULTIES, required=True)
    problems_add.add_argument("--topic", required=True)
    statement_group = problems_add.add_mutually_exclusive_group()
    statement_group.add_argument("--statement", default="")
    statement_group.add_argument("--statement-file")
    problems_add.add_argument("--leetcode-id", type=int)
    problems_add.add_argument("--leetcode-url", default="")
    problems_add.add_argument("--neetcode-url", default="")
    problems_add.add_argument("--premium", action="store_true")
    problems_add.set_defaults(handler=command_problems_add)
    problems_update = problems_subparsers.add_parser("update")
    problems_update.add_argument("problem")
    problems_update.add_argument("--title")
    problems_update.add_argument("--difficulty", choices=DIFFICULTIES)
    problems_update.add_argument("--topic")
    update_statement = problems_update.add_mutually_exclusive_group()
    update_statement.add_argument("--statement")
    update_statement.add_argument("--statement-file")
    problems_update.add_argument("--test-revision", type=int)
    update_leetcode_id = problems_update.add_mutually_exclusive_group()
    update_leetcode_id.add_argument("--leetcode-id", type=int)
    update_leetcode_id.add_argument(
        "--clear-leetcode-id", dest="leetcode_id", action="store_const", const=0
    )
    update_premium = problems_update.add_mutually_exclusive_group()
    update_premium.add_argument("--premium", dest="premium", action="store_true")
    update_premium.add_argument("--not-premium", dest="premium", action="store_false")
    problems_update.set_defaults(premium=None)
    problems_update.add_argument("--leetcode-url")
    problems_update.add_argument("--neetcode-url")
    problems_update.set_defaults(handler=command_problems_update)
    problems_delete = problems_subparsers.add_parser("delete")
    problems_delete.add_argument("problem")
    problems_delete.add_argument("--yes", action="store_true", required=True)
    problems_delete.set_defaults(handler=command_problems_delete)
    problems_adapter = problems_subparsers.add_parser("adapter")
    problems_adapter.add_argument("problem")
    problems_adapter.add_argument("language")
    problems_adapter.add_argument("solution_path")
    problems_adapter.set_defaults(handler=command_problems_adapter)

    sets_parser = subparsers.add_parser("sets", help="manage ordered problem sets")
    sets_subparsers = sets_parser.add_subparsers(dest="sets_command", required=True)
    sets_list = sets_subparsers.add_parser("list")
    sets_list.set_defaults(handler=command_sets_list)
    sets_show = sets_subparsers.add_parser("show")
    sets_show.add_argument("problem_set_id")
    sets_show.set_defaults(handler=command_sets_show)
    sets_create = sets_subparsers.add_parser("create")
    sets_create.add_argument("problem_set_id")
    sets_create.add_argument("--name", required=True)
    sets_create.add_argument("--description", default="")
    sets_create.set_defaults(handler=command_sets_create)
    sets_update = sets_subparsers.add_parser("update")
    sets_update.add_argument("problem_set_id")
    sets_update.add_argument("--name")
    sets_update.add_argument("--description")
    sets_update.set_defaults(handler=command_sets_update)
    sets_delete = sets_subparsers.add_parser("delete")
    sets_delete.add_argument("problem_set_id")
    sets_delete.add_argument("--yes", action="store_true", required=True)
    sets_delete.set_defaults(handler=command_sets_delete)
    sets_add = sets_subparsers.add_parser("add")
    sets_add.add_argument("problem_set_id")
    sets_add.add_argument("problem")
    sets_add.add_argument("--index", type=int)
    sets_add.add_argument("--section")
    sets_add.set_defaults(handler=command_sets_add)
    sets_move = sets_subparsers.add_parser("move")
    sets_move.add_argument("problem_set_id")
    sets_move.add_argument("problem")
    sets_move.add_argument("--index", type=int, required=True)
    sets_move.set_defaults(handler=command_sets_move)
    sets_remove = sets_subparsers.add_parser("remove")
    sets_remove.add_argument("problem_set_id")
    sets_remove.add_argument("problem")
    sets_remove.set_defaults(handler=command_sets_remove)

    return parser


def build_record_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="practice _record")
    parser.add_argument("language")
    parser.add_argument("problem")
    parser.add_argument("result", choices=("pass", "fail", "error", "cancelled"))
    parser.add_argument("duration_ms", type=int)
    parser.add_argument("--problem-set", dest="invoked_set")
    parser.add_argument("--exit-code", type=int)
    parser.set_defaults(handler=command_record, db=None)
    return parser


def main(argv: list[str] | None = None) -> int:
    raw_arguments = sys.argv[1:] if argv is None else argv
    if raw_arguments and raw_arguments[0] == "_record":
        args = build_record_parser().parse_args(raw_arguments[1:])
    else:
        args = build_parser().parse_args(raw_arguments)
    args.database_path = resolve_database_path(args.db)
    try:
        connection = open_database(args.database_path, ROOT)
        try:
            return args.handler(args, connection)
        finally:
            connection.close()
    except (
        ValueError,
        RuntimeError,
        sqlite3.Error,
        OSError,
        AssertionError,
        KeyError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
