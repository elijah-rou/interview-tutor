# Problem sets

A problem set is metadata plus an ordered list of global problem slugs. It never copies titles, difficulty, tests, source paths, or completion. Seven sets ship against the 98-problem global catalog:

| ID | Name | Members |
| --- | --- | ---: |
| `anti-metal` | anti-metal | 10 |
| `blind75` | Blind 75 | 75 |
| `convex` | Convex | 13 |
| `core` | Core | 17 |
| `depot` | Depot | 10 |
| `jane-street` | Jane Street | 15 |
| `runtime-practice` | Runtime Practice (Python and system design) | 10 |

Runtime Practice contains all ten drills from `resume/cross-company/interview-prep/interview-practice-ten-problems.md`, in the original order. Problems 1–6 and 10 have Python starters or a debugging fixture with public tests. Problems 7–9 are system-design prompts: read them with `./practice --set runtime-practice show 7` (or 8/9); they have no executable adapter or automated grade. Full prompts and local Python contracts are in the catalog; solution directions are deliberately omitted. Existing problems 3 and 10 retain their global identity and progress, but now use set indexes 3 and 10.

Convex preserves its original ten algorithm problems and appends three Python concurrency drills: thread-safe LRU cache (#11), bounded blocking queue (#12), and concurrent account transfers (#13). They target the recruiter's explicit concurrency guidance, not reported Convex questions. Prompts include the requested practice points and optional follow-ups; starter solutions remain unimplemented.

No LeetCode round is publicly evidenced for Depot; this is the bounded preparation set. Blind 75 remains an exact 75-member set rather than an alias for the larger catalog. The [anti-metal notes](anti-metal.md), [Core notes](core.md), and [Jane Street notes](jane-street.md) preserve grouping and per-problem rationales outside the v2 JSON because that schema intentionally represents only ordered membership.

Create and compose local sets through `practice sets create/add/move/remove`. Local sets live in the runtime database. Checked-in JSON files are versioned distribution seeds validated against the global catalog.
