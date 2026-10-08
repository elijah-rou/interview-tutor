# Problem sets

A problem set is metadata plus an ordered list of global problem slugs. It never copies titles, difficulty, tests, source paths, or completion. Seven sets ship against the 98-problem global catalog:

| ID | Name | Members |
| --- | --- | ---: |
| `automated-infrastructure` | Automated Infrastructure | 10 |
| `blind75` | Blind 75 | 75 |
| `core` | Core | 17 |
| `distributed-database` | Distributed Database | 13 |
| `quant-software` | Quant Software | 15 |
| `runtime-practice` | Runtime Practice (Python and system design) | 10 |
| `serverless-ci` | Serverless CI | 10 |

Catalog revision 10 renames the four specialized sets to generic names and IDs. Existing databases update automatically on startup, preserving ordered membership, progress, and past attempts' set references. Use the IDs above in commands. If a local set already uses a new ID, the upgrade stops without changing the catalog.

Runtime Practice contains all ten drills from `resume/cross-company/interview-prep/interview-practice-ten-problems.md`, in the original order. Problems 1–6 and 10 have Python starters or a debugging fixture with public tests. Problems 7–9 are system-design prompts: read them with `./practice --set runtime-practice show 7` (or 8/9); they have no executable adapter or automated grade. Full prompts and local Python contracts are in the catalog; solution directions are deliberately omitted. Existing problems 3 and 10 retain their global identity and progress, but now use set indexes 3 and 10.

Distributed Database preserves its original ten algorithm problems and appends three Python concurrency drills: thread-safe LRU cache (#11), bounded blocking queue (#12), and concurrent account transfers (#13). They cover concurrency patterns relevant to distributed databases. Prompts include the requested practice points and optional follow-ups; starter solutions remain unimplemented.

Blind 75 remains an exact 75-member set rather than an alias for the larger catalog. The [Automated Infrastructure notes](automated-infrastructure.md), [Core notes](core.md), and [Quant Software notes](quant-software.md) preserve grouping and per-problem rationales outside the v2 JSON because that schema intentionally represents only ordered membership.

Create and compose local sets through `practice sets create/add/move/remove`. Local sets live in the runtime database. Checked-in JSON files are versioned distribution seeds validated against the global catalog.
