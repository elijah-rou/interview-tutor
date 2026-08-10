from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class AdapterSeed:
    language: str
    solution_path: str


@dataclass(frozen=True, slots=True)
class ProblemSeed:
    slug: str
    title: str
    difficulty: str
    topic: str
    leetcode_id: int | None
    premium: bool
    leetcode_url: str
    neetcode_url: str
    statement_markdown: str
    test_revision: int
    adapters: tuple[AdapterSeed, ...]


@dataclass(frozen=True, slots=True)
class MemberSeed:
    ordinal: int
    problem_slug: str


@dataclass(frozen=True, slots=True)
class ProblemSetSeed:
    id: str
    name: str
    description: str
    members: tuple[MemberSeed, ...]
