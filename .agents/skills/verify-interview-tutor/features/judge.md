# Judge a solution

A user runs the local judge on a problem in Python or Rust, by global slug or by set and index. A wrong or unimplemented solution exits nonzero with the failing case, and a correct one exits `0` with `PASS`. Each run records exactly one attempt, with every case's result, stdout, and stderr, which `./practice attempts list` and `./practice attempts show` read back.

## Sub-features

- `judge-python-fail` reports the starter and a wrong Python solution as failures, with the reason.
- `judge-python-pass` passes a correct Python solution.
- `judge-set-selector` resolves `LANGUAGE SET INDEX` to the same global problem and records the invoking set.
- `judge-practice-run` runs the same judge through `./practice run`.
- `judge-rust` fails the Rust starter and passes a correct Rust solution.
- `judge-unknown` rejects an unknown slug without recording an attempt.
- `judge-record` records one attempt per execution with its result and exit code.
- `judge-launcher` records through `python/run SLUG`, which delegates to `./practice run`.
- `judge-history` lists a problem's attempts and shows one attempt's per-case output, whole or one stream.

## How to get to it (user POV)

- Run `./run LANGUAGE GLOBAL_SLUG`, for example `./run python two-sum`.
- Run `./run LANGUAGE SET SLUG_OR_INDEX`, for example `./run python blind75 16`.
- Run `./practice run LANGUAGE SELECTOR...`.
- Run `python/run SLUG` or `rust/run SLUG`.
- Run `./practice attempts list PROBLEM --language LANGUAGE` and `./practice attempts show ID [--case NAME --stream stdout|stderr]`.

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/judge/progress.db` does not exist yet.
- `python/problems/easy/two_sum.py` raises `NotImplementedError`, and `rust/src/problems/two_sum.rs` calls `unimplemented!("two-sum")`.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID judge`.

- **Python starter.** Run `./run --db "$DB" python two-sum`. Exit code is `1` and stderr has `case 1/1 scenario-1 · fail · exit 1` and `FAIL two-sum: starter is not implemented`.
- **Wrong Python solution.** Make `twoSum` return `[0, 0]`, then run `./run --db "$DB" python two-sum`. Exit code is `1` and stderr has `FAIL two-sum: invalid indices: [0, 0]` and `Traceback (most recent call last)`.
- **Correct Python solution.** Write a hash-map `twoSum`, then run `./run --db "$DB" python two-sum`. Exit code is `0` and stderr has `case 1/1 scenario-1 · pass · exit 0` and `PASS two-sum`.
- **Set and index.** Run `./run --db "$DB" python blind75 16`. Exit code is `0` and stderr has `pass · 1/1 cases passed`.
- **Through practice.** Run `./practice --db "$DB" run python two-sum`. Exit code is `0` and stderr has `PASS two-sum`.
- **Language launcher.** Run `env PRACTICE_DB_PATH="$DB" ./python/run two-sum`. Exit code is `0` and stderr has `case 1/1 scenario-1 · pass · exit 0`.
- **Unknown slug.** Run `./run --db "$DB" python no-such-problem`. Exit code is `2` and stderr has `unknown problem: no-such-problem`.
- **Rust starter.** Run `./run --db "$DB" rust two-sum`. Exit code is `101` and stderr has `not implemented: two-sum`.
- **Correct Rust solution.** Replace the starter body with a hash-map solution, then run `./run --db "$DB" rust two-sum`. Exit code is `0` and stderr has `PASS two-sum`.
- **Recorded attempts.** Reading `attempts` gives exactly eight rows, in order: python fail 1, python fail 1, python pass 0, python pass 0 with set `blind75`, python pass 0, python pass 0, rust fail 101, rust pass 0. The unknown slug added none.
- **Recorded steps.** Reading `attempt_steps` gives two rows per attempt: the setup step (ordinal 0, exit 0) and case `scenario-1` with that attempt's exit code.
- **List attempts.** Run `./practice --db "$DB" attempts list two-sum --language python`. Attempt 2 shows `fail` with cases `0/1`, and no Rust attempt is listed.
- **Show an attempt.** Run `./practice --db "$DB" attempts show 2`. Stdout has `Attempt 2 · two-sum · python · fail`, `case 1/1 scenario-1 · fail · exit 1`, and `FAIL two-sum: invalid indices: [0, 0]`.
- **Show one stream.** Run `./practice --db "$DB" attempts show 2 --case scenario-1 --stream stderr`. The first stdout line is exactly `FAIL two-sum: invalid indices: [0, 0]`.
- **Show a Rust attempt.** Run `./practice --db "$DB" attempts show 7`. Stdout has `case 1/1 scenario-1 · fail · exit 101` and `not implemented: two-sum`.
- **Restore.** Both solution files are restored, and `/work` matches `/src`.

## Gotchas

- `./practice run` prints each case's result, stdout, and stderr to its own stderr after the run. Its stdout is empty, so assert on stderr. `./practice attempts` prints to stdout.
- Each problem currently declares one case: `scenario-1` for custom Python tests and every Rust problem, `example-1` for simple Python cases. `two-sum` uses `scenario-1` in both languages.
- Failed runs are recorded as attempts too. Only resolution and spawn failures (exit `2`) record nothing.
- Exit codes differ by language: a Python failure is `1`, a Rust panic is `101`, and usage or lookup errors are `2`.
- The starters are intentionally incomplete. A failing starter is expected behavior, not a product gap.
- Only the attempt from `./run python blind75 16` carries an invoking set. The slug forms record `-`.
- The CLI does not build on macOS (`openat2` and `O_PATH`). Judge only inside the container.
- The first Rust run after an edit recompiles the adapter crate, which takes a few seconds and stays within the 180-second step deadline.
