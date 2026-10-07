# Administer the catalog

A user adds a custom problem, groups shipped and custom problems in a custom ordered set, reorders and removes members, and deletes custom resources with an explicit `--yes`. Shipped problems and sets stay read-only.

## Sub-features

- `admin-problem-crud` adds, shows, updates, and deletes a custom metadata-only problem.
- `admin-set-crud` creates a custom set, adds members at an index, moves and removes members, and deletes the set.
- `admin-shipped-readonly` refuses to update or delete shipped problems and sets.
- `admin-guarded-delete` requires `--yes` to delete.
- `admin-adapter-refusal` refuses an adapter whose file is missing or that the language runner does not expose.
- `admin-run-metadata-only` refuses to run a problem that has no language adapter.

## How to get to it (user POV)

- Run `./practice problems add|show|update|delete|adapter ...`.
- Run `./practice sets create|add|move|remove|delete ...`.
- Run `./practice --set CUSTOM_ID list` to see a custom set.

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/admin/progress.db` does not exist yet.
- No problem `verify-pair-sum` or set `verify-favorites` exists.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID admin`.

- **Add a problem.** Run `./practice --db "$DB" problems add verify-pair-sum --title 'Verify Pair Sum' --difficulty Easy --topic 'Arrays & Hashing' --statement 'Return two indices whose values sum to target.'`. Exit code is `0`. Then `./practice --db "$DB" problems show verify-pair-sum` shows `Verify Pair Sum`.
- **Update it.** Run `./practice --db "$DB" problems update verify-pair-sum --title 'Verify Pair Sum Two'`. Then `problems show verify-pair-sum` shows `Verify Pair Sum Two`.
- **Create a set.** Run `./practice --db "$DB" sets create verify-favorites --name 'Verify Favorites'`, then `sets add verify-favorites two-sum` and `sets add verify-favorites verify-pair-sum --index 1`. Then `./practice --db "$DB" --set verify-favorites list` shows `1 ... verify-pair-sum` and `2 ... two-sum`.
- **Reorder.** Run `./practice --db "$DB" sets move verify-favorites two-sum --index 1`. The set list now shows `1 ... two-sum` and `2 ... verify-pair-sum`, and `sets list` includes `verify-favorites`.
- **Adapter refusals.** Run `./practice --db "$DB" problems adapter verify-pair-sum python python/problems/easy/verify_pair_sum.py`. Exit code is `2` with `solution file does not exist`. With the existing file `python/problems/easy/contains_duplicate.py`, exit code is `2` with `python runner does not expose problem adapter: verify-pair-sum`.
- **Shipped stays read-only.** Run `problems delete two-sum --yes` and `problems update two-sum --title 'Renamed'`. Both exit `2` with `shipped problem is read-only: two-sum`. Then `sets delete blind75 --yes` exits `2` with `shipped problem set is read-only: blind75`.
- **Guarded delete.** Run `./practice --db "$DB" sets delete verify-favorites`. Exit code is `2` with `required arguments were not provided`.
- **Metadata-only run.** Run `./run --db "$DB" python verify-pair-sum`. Exit code is `2` with `no active python adapter for problem: verify-pair-sum`.
- **Remove and delete.** Run `sets remove verify-favorites verify-pair-sum`, `sets delete verify-favorites --yes`, and `problems delete verify-pair-sum --yes`, each exiting `0`. Then `problems show verify-pair-sum` exits `2` with `unknown problem: verify-pair-sum`, and `sets list` no longer shows `verify-favorites`.
- **Shipped intact.** Run `./practice --db "$DB" --set blind75 show two-sum`. It still shows `Two Sum`. No attempts were recorded, and `/work` matches `/src`.

## Gotchas

- `sets move` takes the position as `--index N`, not as a positional argument.
- Leaving out `--yes` is a usage error (exit `2`) from argument parsing, not a confirmation prompt.
- A successful `problems adapter` needs the language runner to expose the slug, which means adding a registry entry and test dispatch in `python/local_judge/` or `rust/src/problems/`. That is a code change, not a CLI action, so the success path is not driven.
- Custom resources live only in the feature's database. The checked-in catalog files never change.
- `problems update` and `sets update` accept other fields too. Only `--title` is driven.
