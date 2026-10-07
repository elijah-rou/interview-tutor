# interview-tutor verification map

This directory is the maintained source for verifying what users of interview-tutor can do. Read this index before driving, then use the matching feature file as the recipe. Commands assume the repository root as the working directory on the host and `/work` inside the container.

## Baseline preconditions

- A run was started with `.agents/skills/verify-interview-tutor/scripts/verify-tutor launch` and printed `READY` with a `RUN_ID`. Never drive a container that this run did not launch.
- `.agents/skills/verify-interview-tutor/scripts/verify-tutor doctor RUN_ID` ends with `DOCTOR PASS`.
- Inside the container, `/work` is a copy of the checkout. Its `python/problems/easy/two_sum.py` and `rust/src/problems/two_sum.rs` are the unimplemented starters, identical to `/src`.
- Each feature uses its own fresh database, `DB=/proof/<feature>/progress.db`, passed as `--db "$DB"`. The repository's `.turso/progress.db` is never used.
- The shipped catalog has 98 problems in seven sets: `anti-metal`, `blind75` (75), `convex` (13), `core`, `depot`, `jane-street`, and `runtime-practice`. `two-sum` is `blind75` #16 and belongs to no other set.
- Evidence goes to `.agents/skills/verify-interview-tutor/evidence/RUN_ID/<feature>/`, which cleanup never touches.

## Driving conventions

- `verify-tutor drive RUN_ID <feature>` runs every command in that feature file in order, inside the container, from `/work`. The feature's labeled bullets are its checks. To run a single command by hand, use `docker exec -w /work CONTAINER_ID <command>` with a scratch `--db` under `/tmp`.
- Treat commands literally: keep slugs, quoted titles, and flags exactly as written.
- Match TUI screens by visible text (titles, status words, pane headings), never by coordinates or highlight colour. Wait for that text with a deadline; never wait with a fixed sleep.
- Restore every solution file a drive rewrites. Delete each custom problem and set a drive creates before it ends.
- Each feature is driven once per run. To repeat a feature, launch a new run.

## Proof and skip reporting

- CLI proof is the command, stdout, stderr, and exit code for every step (`NN-step.cmd`, `.out`, `.err`, `.exit`).
- A judge result is proved by the exit code, the relayed `[stdout]` or `[stderr]` line, and the `attempts` rows read back from the feature's database.
- A catalog mutation is proved by a second, read-only command that shows the stored value.
- TUI proof is a screen capture after each action (`NN-label.screen.txt`), the solution file as saved by the TUI, the attempt rows, and the program's exit status.
- `summary.txt` in each feature's artifacts records the feature ID, the entry points driven, and every passing check.
- An entry point that cannot be reached is reported with the command attempted and the unmet precondition.
- A skipped entry point is never reported as verified through a different one. The current skips are listed in each feature's Gotchas.

## Features

- [Browse the catalog](./catalog.md): set listing, set problem lists, problem detail by slug and by index, global problem and set views, and unknown selectors.
- [Judge a solution](./judge.md): `./run` and `./practice run` against Python and Rust, with starter, wrong, and correct solutions, and recorded attempts.
- [Track progress](./progress.md): set and global stats per language, and the done marker in set lists.
- [Administer the catalog](./admin.md): custom problems and sets (create, update, order, guarded delete), read-only shipped resources, and adapter refusals.
- [Solve in the terminal UI](./interview-tui.md): browse to a problem, edit in embedded Neovim, test, submit, go back, and quit.
