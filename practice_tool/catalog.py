from __future__ import annotations

import json
from pathlib import Path
import re

from practice_tool.models import AdapterSeed, MemberSeed, ProblemSeed, ProblemSetSeed

PROBLEM_SLUG_PATTERN = re.compile(r"^(?=.*[a-z])[a-z0-9][a-z0-9_-]{0,63}$")
RESOURCE_ID_PATTERN = re.compile(r"^[a-z][a-z0-9_-]{0,63}$")
DIFFICULTIES = ("Easy", "Medium", "Hard")


def validate_identifier(value: str, label: str, *, problem_slug: bool = False) -> None:
    pattern = PROBLEM_SLUG_PATTERN if problem_slug else RESOURCE_ID_PATTERN
    if not pattern.fullmatch(value):
        raise ValueError(f"invalid {label}: {value!r}")


def load_seed_catalog(
    root: Path,
) -> tuple[int, tuple[ProblemSeed, ...], tuple[ProblemSetSeed, ...]]:
    raw_catalog = json.loads(
        (root / "catalog" / "problems.json").read_text(encoding="utf-8")
    )
    assert raw_catalog["schema_version"] == 2
    catalog_revision = raw_catalog["catalog_revision"]
    assert catalog_revision > 0
    problems: list[ProblemSeed] = []
    for raw in raw_catalog["problems"]:
        validate_identifier(raw["slug"], "problem slug", problem_slug=True)
        assert raw["difficulty"] in DIFFICULTIES
        assert raw["test_revision"] > 0
        adapters = tuple(
            AdapterSeed(
                language=adapter["language"], solution_path=adapter["solution_path"]
            )
            for adapter in raw["adapters"]
        )
        assert len({adapter.language for adapter in adapters}) == len(adapters)
        for adapter in adapters:
            validate_identifier(adapter.language, "language")
            path = Path(adapter.solution_path)
            assert not path.is_absolute()
            assert ".." not in path.parts
            assert path.parts[0] == adapter.language
            assert (root / path).is_file(), path
        problems.append(
            ProblemSeed(
                slug=raw["slug"],
                title=raw["title"],
                difficulty=raw["difficulty"],
                topic=raw["topic"],
                leetcode_id=raw["leetcode_id"],
                premium=raw["premium"],
                leetcode_url=raw["leetcode_url"],
                neetcode_url=raw["neetcode_url"],
                statement_markdown=raw["statement_markdown"],
                test_revision=raw["test_revision"],
                adapters=adapters,
            )
        )
    problem_slugs = {problem.slug for problem in problems}
    assert len(problem_slugs) == len(problems)

    problem_sets: list[ProblemSetSeed] = []
    for path in sorted((root / "problem_sets").glob("*.json")):
        raw = json.loads(path.read_text(encoding="utf-8"))
        assert raw["schema_version"] == 2
        validate_identifier(raw["id"], "problem-set id")
        assert raw["id"] == path.stem
        members = tuple(
            MemberSeed(ordinal=member["ordinal"], problem_slug=member["problem_slug"])
            for member in raw["members"]
        )
        assert [member.ordinal for member in members] == list(
            range(1, len(members) + 1)
        )
        assert len({member.problem_slug for member in members}) == len(members)
        assert all(member.problem_slug in problem_slugs for member in members)
        problem_sets.append(
            ProblemSetSeed(
                id=raw["id"],
                name=raw["name"],
                description=raw["description"],
                members=members,
            )
        )
    assert len({problem_set.id for problem_set in problem_sets}) == len(problem_sets)
    return catalog_revision, tuple(problems), tuple(problem_sets)
