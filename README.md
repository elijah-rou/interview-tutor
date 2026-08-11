# Local algorithm practice harness

A language-independent local judge with a Rust control plane, a global problem catalog, reusable ordered problem sets, and shared progress. Blind 75 is the first seeded set, not the application boundary.

Problems have one global identity. A problem can appear in Blind 75, NeetCode 150, NeetCode 250, and custom sets without duplicating solutions or completion state.

## Run problems

Use a global problem slug:

```console
./run python two-sum
./run rust two-sum
```

Or qualify the problem by set and select it by slug or 1-based set index:

```console
./run python blind75 two-sum
./run rust blind75 16
```

The two-argument form is always a global slug. The three-argument form is always `LANGUAGE SET SLUG_OR_INDEX`, so resolution does not guess based on collisions. A zero exit status means the local suite passed. The root runner captures and sanitizes output, enforces a 30 second timeout, terminates the runner process group on interruption, and records exactly one attempt after execution.

The language-local wrappers remain available for direct adapter work:

```console
cd python && ./run two-sum
cd rust && ./run two-sum
```

## Browse sets and progress

```console
./practice sets list
./practice --set blind75 list
./practice --set blind75 show 16
./practice --set blind75 stats
./practice --set blind75 stats --language python
./practice stats --global --language python
```

Completion belongs to the global problem and language at its current test revision. Passing `two-sum` once immediately counts in every set containing it. Set indexes are only selectors; attempts retain the stable problem identity.

## Add problems and compose sets

Problem metadata is independent of membership:

```console
./practice problems add custom-pair-sum \
  --title "Custom Pair Sum" \
  --difficulty Easy \
  --topic "Arrays & Hashing" \
  --statement-file ./custom-pair-sum.md
./practice problems update custom-pair-sum --test-revision 2

./practice sets create favorites --name "Favorites"
./practice sets update favorites --description "Problems to revisit"
./practice sets add favorites two-sum
./practice sets add favorites custom-pair-sum --index 1
./practice sets move favorites two-sum --index 1
./practice sets remove favorites custom-pair-sum
./practice --set favorites list
```

A metadata-only problem is valid and can belong to sets, but cannot run until a language adapter and local test dispatch exist. Register the source path after adding that adapter:

```console
./practice problems adapter custom-pair-sum python python/problems/easy/custom_pair_sum.py
```

`problems` and `sets` provide list, show, update, and guarded delete operations. Delete requires `--yes`; a problem with membership or attempt history cannot be deleted. Checked-in problems and sets are read-only through local CRUD, while custom resources remain editable.

## Catalog and database ownership

```text
catalog/problems.json       global shipped problem metadata and adapter paths
problem_sets/blind75.json   ordered references to global problem slugs
cli/                        standalone Rust control-plane crate
practice                    Rust CLI launcher
run                         terse Rust execution launcher
python/problems/            Python solution starters
rust/src/problems/          Rust solution starters
.turso/progress.db          local runtime database
```

Checked-in catalogs reconcile managed resources once per catalog revision. Managed problems or adapters omitted by a later revision are retired, while custom resources and attempt rows remain intact. Retiring a managed set clears that set's optional `invoked_set_id` context from historical attempts through `ON DELETE SET NULL`. SQLite is the runtime authority for custom CRUD. The normalized schema stores global problems, ordered set membership, languages, implementations, attempts, statement Markdown, and solution paths. The Rust `cli` crate is the control-plane API boundary for a future TUI, which can reuse its catalog, database, selector, and runner modules instead of parsing CLI tables. See `docs/architecture.md` for the ownership, schema, and execution boundaries.

The database is SQLite/libSQL-compatible. The Turso server is optional:

```console
./practice db
turso dev --db-file .turso/progress.db
```

Database precedence is `--db`, then `PRACTICE_DATABASE_URL`, then `PRACTICE_DB_PATH`, then the legacy Blind 75 environment names, then `.turso/progress.db`.

Existing v1 databases migrate automatically to the global v2 schema. Historical attempts and stale removed problems are retained. Parked legacy problems are archived, remain visible with `problems list --all`, and do not inflate active global progress.

## Interface and test policy

Starter APIs follow LeetCode. Premium and Rust APIs without official templates use their conventional NeetCode/LintCode-compatible representation. Tests use public contracts and representative examples; LeetCode's private hidden corpus is not available locally.

LLM hints and the split-pane TUI remain later layers. The schema supports statement Markdown and per-language source paths, and the synchronous bounded runner API is ready for a TUI worker thread. See `docs/architecture.md` for its exact resource limits, termination states, and attempt-outcome mapping.
