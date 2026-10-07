# Judge a solution

A user runs the local judge on a problem in Python or Rust, by global slug or by set and index. A wrong or unimplemented solution exits nonzero with the failing case, and a correct one exits `0` with `PASS`. Each run records exactly one attempt.

## Sub-features

- `judge-python-fail` reports the starter and a wrong Python solution as failures, with the reason.
- `judge-python-pass` passes a correct Python solution.
- `judge-set-selector` resolves `LANGUAGE SET INDEX` to the same global problem and records the invoking set.
- `judge-practice-run` runs the same judge through `./practice run`.
- `judge-rust` fails the Rust starter and passes a correct Rust solution.
- `judge-unknown` rejects an unknown slug without recording an attempt.
- `judge-record` records one attempt per execution with its result and exit code.

## How to get to it (user POV)

- Run `./run LANGUAGE GLOBAL_SLUG`, for example `./run python two-sum`.
- Run `./run LANGUAGE SET SLUG_OR_INDEX`, for example `./run python blind75 16`.
- Run `./practice run LANGUAGE SELECTOR...`.

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/judge/progress.db` does not exist yet.
- `python/problems/easy/two_sum.py` raises `NotImplementedError`, and `rust/src/problems/two_sum.rs` calls `unimplemented!("two-sum")`.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID judge`.

- **Python starter.** Run `./run --db "$DB" python two-sum`. Exit code is `1` and stderr has `[stderr] FAIL two-sum: starter is not implemented`.
- **Wrong Python solution.** Make `twoSum` return `[0, 0]`, then run `./run --db "$DB" python two-sum`. Exit code is `1` and stderr has `[stderr] FAIL two-sum: invalid indices: [0, 0]`.
- **Correct Python solution.** Write a hash-map `twoSum`, then run `./run --db "$DB" python two-sum`. Exit code is `0` and stderr has `[stdout] PASS two-sum`.
- **Set and index.** Run `./run --db "$DB" python blind75 16`. Exit code is `0` and stderr has `[stdout] PASS two-sum`.
- **Through practice.** Run `./practice --db "$DB" run python two-sum`. Exit code is `0` and stderr has `[stdout] PASS two-sum`.
- **Unknown slug.** Run `./run --db "$DB" python no-such-problem`. Exit code is `2` and stderr has `unknown problem: no-such-problem`.
- **Rust starter.** Run `./run --db "$DB" rust two-sum`. Exit code is `101` and stderr has `not implemented: two-sum`.
- **Correct Rust solution.** Replace the starter body with a hash-map solution, then run `./run --db "$DB" rust two-sum`. Exit code is `0` and stderr has `[stdout] PASS two-sum`.
- **Recorded attempts.** Reading `attempts` gives exactly seven rows, in order: python fail 1, python fail 1, python pass 0, python pass 0 with set `blind75`, python pass 0, rust fail 101, rust pass 0. The unknown slug added none.
- **Restore.** Both solution files are restored, and `/work` matches `/src`.

## Gotchas

- The runner relays the child's output to its own stderr, with `[stdout]` and `[stderr]` prefixes. Its stdout is empty, so assert on stderr.
- Failed runs are recorded as attempts too. Only resolution and spawn failures (exit `2`) record nothing.
- Exit codes differ by language: a Python failure is `1`, a Rust panic is `101`, and usage or lookup errors are `2`.
- The starters are intentionally incomplete. A failing starter is expected behavior, not a product gap.
- Only the attempt from `./run python blind75 16` carries an invoking set. The slug forms record `-`.
- The CLI does not build on macOS (`openat2` and `O_PATH`). Judge only inside the container.
- The first Rust run after an edit recompiles the adapter crate, which takes a few seconds and stays within the 180-second step deadline.
