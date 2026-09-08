# Problem sets

A problem set is metadata plus an ordered list of global problem slugs. It never copies titles, difficulty, tests, source paths, or completion. Seven sets ship against the 87-problem global catalog:

| ID | Name | Members |
| --- | --- | ---: |
| `anti-metal` | anti-metal | 10 |
| `blind75` | Blind 75 | 75 |
| `convex` | Convex | 10 |
| `core` | Core | 17 |
| `depot` | Depot | 10 |
| `jane-street` | Jane Street | 15 |
| `runtime-practice` | Runtime Practice (Python only) | 2 |

Runtime Practice contains original drills 3 (bounded asynchronous job runner) and 10 (retry-safe run dashboard) from `resume/cross-company/interview-prep/interview-practice-ten-problems.md`. Set indexes are 1 and 2. Full prompts and local Python contracts are in the catalog; solution directions are deliberately omitted.

No LeetCode round is publicly evidenced for Depot; this is the bounded preparation set. Blind 75 remains an exact 75-member set rather than an alias for the larger catalog. The [anti-metal notes](anti-metal.md), [Core notes](core.md), and [Jane Street notes](jane-street.md) preserve grouping and per-problem rationales outside the v2 JSON because that schema intentionally represents only ordered membership.

Create and compose local sets through `practice sets create/add/move/remove`. Local sets live in the runtime database. Checked-in JSON files are versioned distribution seeds validated against the global catalog.
