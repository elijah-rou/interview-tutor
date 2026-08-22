# Problem sets

A problem set is metadata plus an ordered list of global problem slugs. It never copies titles, difficulty, tests, source paths, or completion. Four sets ship against the 79-problem global catalog:

| ID | Name | Members |
| --- | --- | ---: |
| `anti-metal` | anti-metal | 10 |
| `blind75` | Blind 75 | 75 |
| `convex` | Convex | 10 |
| `depot` | Depot | 10 |

No LeetCode round is publicly evidenced for Depot; this is the bounded preparation set. Blind 75 remains an exact 75-member set rather than an alias for the larger catalog. The [anti-metal notes](anti-metal.md) preserve its per-problem selection rationales outside the v2 JSON because that schema intentionally represents only ordered membership.

Create and compose local sets through `practice sets create/add/move/remove`. Local sets live in the runtime database. Checked-in JSON files are versioned distribution seeds validated against the global catalog.
