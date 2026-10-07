# Solve in the terminal UI

A user opens a set in the interactive browser, moves to a problem, reads its statement, and edits the solution in embedded clean Neovim. `Space t` tests without recording, and `Space s` submits and records one attempt. From there the user goes back to the problem list, sees the problem marked done, and quits.

## Sub-features

- `tui-browse` opens a set's problem list and moves the selection with `j`/`k`.
- `tui-detail` opens problem detail with Enter.
- `tui-solve-open` loads the planned source into embedded Neovim with Enter.
- `tui-edit` edits through Neovim and marks the buffer dirty.
- `tui-test` saves atomically and runs the judge with `Space t`, without recording an attempt.
- `tui-submit` saves, runs, and records exactly one attempt with `Space s`, then refreshes progress.
- `tui-back` returns to the problem list with `Space b` and shows the done marker.
- `tui-quit` exits with status `0` on `q` outside solve mode.

## How to get to it (user POV)

- Run `./interview --set blind75 --language python --interviewer none` in an interactive terminal.
- Run `./interview` with no `--set` to start at the set menu (not driven).
- In solve mode, `F5` and `:TutorTest` alias `Space t`, and `F9` and `:TutorSubmit` alias `Space s` (aliases not driven).

## Driving it with verify-tutor

Preconditions:

- Baseline preconditions hold, and `DB=/proof/tui/progress.db` does not exist yet.
- tmux session `verify-tui` exists only inside this run's container, at 120x40 with `TERM=xterm-256color`.
- Drive with `.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID tui`.

- **Start.** Run `tmux new-session -d -s verify-tui -x 120 -y 40 -c /work "./interview --db $DB --set blind75 --language python --interviewer none"`. The screen shows `Problems [active]` and `Best Time to Buy and Sell Stock`.
- **Open detail.** Run `tmux send-keys -t verify-tui j j j j j j j j j j j j j j j Enter`. The screen shows `Two Sum · Easy · Arrays & Hashing`.
- **Open solve.** Run `tmux send-keys -t verify-tui Enter`. The screen shows `Neovim ready`, `No test run yet`, and `raise NotImplementedError`.
- **Make it wrong.** Run `tmux send-keys -t verify-tui ':%s/raise NotImplementedError/return [0, 0]/' Enter`. The screen shows `Normal · DIRTY` and `return [0, 0]`.
- **Test the wrong solution.** Run `tmux send-keys -t verify-tui Space t`. The Output pane shows `Exited(1)` and `[stderr] FAIL two-sum: invalid indices: [0, 0]`, and the status shows `Normal · SAVED`.
- **Fix it.** Send the `:%s` command that replaces `return [0, 0]` with a brute-force `return next([i, j] ...)`, then Enter. The screen shows `return next(`.
- **Test the fix.** Run `tmux send-keys -t verify-tui Space t`. The Output pane shows `Exited(0)` and `[stdout] PASS two-sum`. The `attempts` table is still empty.
- **Submit.** Run `tmux send-keys -t verify-tui Space s`. The header shows `Submit recorded · progress refreshed` and `Progress: 1/75`. `/work/python/problems/easy/two_sum.py` on disk contains `return next([i, j]`.
- **Back.** Run `tmux send-keys -t verify-tui Space b`. The screen shows `Problems [active]` and `✓   Two Sum`.
- **Quit.** Run `tmux send-keys -t verify-tui q`. The pane exits with status `0`.
- **Recorded attempts.** `attempts` holds exactly one row: python pass 0 with set `blind75`. The solution file is restored, and `/work` matches `/src`.

## Gotchas

- Plain `tmux capture-pane` does not show the selection highlight. Navigate by count from row 1, then confirm the detail heading.
- Edit with Neovim ex commands such as `:%s`. Typing multi-line code into Insert mode picks up Neovim's autoindent and nests the indentation.
- `Run complete` stays on screen after the first test. To detect a later run, wait for that run's own output line (`Exited(0)`, `PASS`), not the status word.
- `q` quits only outside solve mode. In solve mode, `Space q` quits from Normal mode and `Ctrl-Q` quits from every pane; a dirty buffer needs the key twice.
- `--interviewer none` is required here. The Pi and Codex interviewers need installed executables, credentials, and network, and the container has none of these. Interview, Tutor, hints, and submission review are therefore reported as skipped, not verified. The repository covers them with the fake Codex fixture in `make test-pty`.
- Below 60x20 only a resize panel renders, and at 80x24 the panes become tabs. The marker text above assumes 120x40.
- Never point `--db` at `.turso/progress.db`. Submits write attempts.
