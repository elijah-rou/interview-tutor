# Track progress

A user sees how much of a set, or of the whole catalog, they have completed in a given language. A passed problem shows as done in set lists and counts once in global stats.

## Sub-features

- `progress-set` reports completed and total counts for one set and language, by difficulty and topic.
- `progress-global` deduplicates all global problems for a language.
- `progress-language` keeps completion separate per language.
- `progress-list-marker` marks a completed problem in the set list's language column.
- `progress-scope` leaves sets that do not contain the problem unchanged.

## How to get to it (user POV)

- Run `./practice --set ID stats --language LANGUAGE`.
- Run `./practice stats --global --language LANGUAGE`.
- Run `./practice --set ID list` and read the Python and Rust columns.
- In the TUI, read `Progress: N/M` in the header and the Progress pane (driven in [Solve in the terminal UI](./interview-tui.md)).

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/progress/progress.db` does not exist yet.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID progress`.

- **Empty progress.** Run `./practice --db "$DB" --set blind75 stats --language python`. Stdout has `Blind 75 progress (python): 0/75 (0.0%)`.
- **Solve one problem.** Write a correct `twoSum`, run `./run --db "$DB" python two-sum` (exit `0`), then restore the starter.
- **Set stats.** Run `./practice --db "$DB" --set blind75 stats --language python`. Stdout has `Blind 75 progress (python): 1/75 (1.3%)`.
- **Global stats.** Run `./practice --db "$DB" stats --global --language python`. Stdout has `All Problems progress (python): 1/98 (1.0%)`.
- **Other language.** Run `./practice --db "$DB" --set blind75 stats --language rust`. Stdout has `Blind 75 progress (rust): 0/75 (0.0%)`.
- **Done marker.** Run `./practice --db "$DB" --set blind75 list`. The row starting `16` has `yes` in the Python column for `two-sum`.
- **Unrelated set.** Run `./practice --db "$DB" --set distributed-database stats --language python`. Stdout has `0/13`.
- **Recorded attempts.** `attempts` holds exactly one row: python pass 0.

## Gotchas

- `stats` defaults to `--language any`. Pass the language you mean.
- Completion belongs to a problem and language at that problem's current test revision. Changing a problem's tests resets its completion.
- `two-sum` belongs only to `blind75`, so this drive cannot show a shared problem counting in two sets. Proving that needs a problem in several shipped sets (check `problem_sets/*.json`) or a custom set from [Administer the catalog](./admin.md). That case is not driven yet.
- Percentages are rounded to one decimal place. Assert the exact string the CLI prints.
