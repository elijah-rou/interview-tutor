# Browse the catalog

A user lists the shipped problem sets, opens a set's ordered problem list, reads a problem's statement by slug or by set index, inspects global problem and set records, and gets a clear error for an unknown set or problem.

## Sub-features

- `catalog-sets` lists every shipped set with its size.
- `catalog-set-list` lists one set's problems in order, with Python and Rust done columns.
- `catalog-show` shows a problem's metadata and statement by slug or by 1-based set index.
- `catalog-global` lists and shows global problems and sets, independent of set membership.
- `catalog-db` creates the database on first use and reports its path.
- `catalog-unknown` rejects an unknown set or problem with exit code 2.

## How to get to it (user POV)

- Run `./practice sets list` in a terminal.
- Run `./practice --set ID list`, `./practice --set ID show SLUG`, or `./practice --set ID show INDEX`.
- Run `./practice problems list`, `./practice problems show SLUG`, or `./practice sets show ID`.
- Run `./practice db`.

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/catalog/progress.db` does not exist yet.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID catalog`.

- **List sets.** Run `./practice --db "$DB" sets list`. Exit code is `0` and stdout lists `automated-infrastructure`, `blind75`, `distributed-database`, `core`, `serverless-ci`, `quant-software`, and `runtime-practice`.
- **List a set.** Run `./practice --db "$DB" --set blind75 list`. Exit code is `0` and stdout includes `best-time-to-buy-and-sell-stock` and `two-sum`.
- **Show by slug.** Run `./practice --db "$DB" --set blind75 show two-sum`. Stdout includes `Two Sum` and `Problem set: blind75 #16`.
- **Show by index.** Run `./practice --db "$DB" --set blind75 show 16`. Stdout includes `Slug: two-sum`.
- **List another set.** Run `./practice --db "$DB" --set distributed-database list`. Exit code is `0`.
- **Global problem views.** Run `./practice --db "$DB" problems show two-sum` and `./practice --db "$DB" problems list`. Both exit `0` and include `two-sum`.
- **Global set view.** Run `./practice --db "$DB" sets show runtime-practice`. Stdout includes `runtime-practice`.
- **Database path.** Run `./practice --db "$DB" db`. Stdout includes `/proof/catalog/progress.db`.
- **Unknown set.** Run `./practice --db "$DB" --set no-such-set list`. Exit code is `2` and stderr is `error: unknown problem set: no-such-set`.
- **Unknown problem.** Run `./practice --db "$DB" --set blind75 show no-such-problem`. Exit code is `2` and stderr is `error: unknown problem: no-such-problem`.
- **No side effects.** The feature's `attempts` table is empty, and `/work` still matches `/src`.

## Gotchas

- `--set` defaults to `blind75` for `list`, `show`, and `stats`. Always pass it explicitly.
- Do not pipe `./practice` into `head` or another reader that closes early. The CLI panics with `failed printing to stdout: Broken pipe`. Redirect to a file instead. This is a product gap.
- Every usage or lookup error exits `2`, not `1`.
- Any `./practice` command creates and seeds the database if it is missing, so the first command is never read-only with respect to the file system.
- Set lists show slugs; the TUI shows titles. Assert on the one each surface prints.
