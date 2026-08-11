# Interview TUI

Run `./interview` in an interactive Linux terminal. Optional startup flags are `--db PATH`, `--set ID`, `--language ID`, and `--no-codex`. Without `--set`, the TUI opens the set menu; otherwise it opens that set's problem list.

## Browser keys

The set list, problem list, and detail screens use:

- `j`/`k` or Down/Up: move the current selection or scroll detail
- Enter: open the selected set/problem; from problem detail, load the selected language source into solve mode
- Esc or Backspace: go back
- `l`: cycle the enabled language
- `r`: reload catalog/progress data
- Tab/Shift-Tab: cycle the browser's main and progress focus
- `?`: open help; Esc closes it
- `q`: quit outside solve mode

The status line reports the selected language and current operation. Errors remain visible rather than silently changing screens.

## Solve panes and keys

Solve mode has Editor, Problem/Examples, Output/Test, and Interview panes. Tab and Shift-Tab cycle them. Focused Problem, Output, and Interview panes scroll with Up/Down; Interview scrolls from newest toward older bounded transcript content and returns to newest when a message is appended or the session is cleared. Outside the Editor, `i` focuses the Interview composer. In the Editor it retains Vim insert behavior.

Global solve actions are:

- Ctrl-S or F5: save the current revision atomically and run local tests
- F9: submit the current revision
- Ctrl-C: cancel the operation selected by focus; Interview wins when it is focused and both Codex and the local runner are active, otherwise the local runner is cancelled
- `Space h`: request a hint outside Editor Insert/Command mode and outside the active composer
- `Space r`: clear the Interview session when Interview is focused
- `Space b`: go back from Editor Normal mode
- `Space q`: quit from Editor Normal mode

Back and quit are guarded when the buffer is dirty. The first identical action warns; the second discards. An edit or different action clears the confirmation. Esc never discards solve changes. `:q` rejects a dirty buffer.

## Native editor subset

No Vim or Neovim process is launched. The built-in editor supports this exact subset:

- Normal mode: arrow keys, `h j k l`, `w b`, `0 $`, `gg G`, `i a o O`, `x`, `dd`, `u`, Ctrl-R, and `:`
- Insert mode: Unicode text and paste, arrows, Backspace, Delete, Enter, and Esc
- Command mode: `:w`, `:wq`, `:q`, and `:submit`; Esc cancels command entry

Visual mode, search, macros, registers, counts, plugins, Vimscript, and every other command are unsupported and produce an error where applicable. Cursor motion, insertion, deletion, and display use Unicode grapheme clusters, including combining marks and emoji ZWJ sequences. Normal mode addresses a grapheme; Insert mode addresses an insertion point. The document is bounded to 1 MiB, 100,000 lines, 32 undo snapshots, and a 256-byte command.

## Test, save, and submit semantics

Ctrl-S, F5, and `:w` atomically save if dirty and run the local suite. They never create an attempt row. A clean buffer still runs. If another local run is active, only the newest requested save/test revision is retained; submit is rejected until that run completes. A failed save starts no runner.

F9 and `:submit` save, run, then record exactly one attempt after the runner returns an execution result. Pass, fail, timeout, and explicit cancellation outcomes are recorded; preflight/spawn failures that produce no execution result are not. Repeating submit records another attempt. Local runner results remain authoritative even when Codex is enabled.

`:wq` starts save/test and exits only after that operation returns and the saved/tested bytes still equal the current buffer. It does not require a passing test. If the buffer changes while the run is active, the newer dirty revision stays open. Edits are allowed during any run. Revisions increase monotonically across edits, undo, and redo, while dirty state compares bytes against the last saved bytes.

Output is bounded and sanitized. `STALE` appears only when displayed output belongs to an older editor revision; edits before the first run are not stale. Save errors, runner errors, Codex errors, and status such as testing, submitting, cancellation, or stale completion stay visible in the status/error and Output panes.

## Layout

- At 100x30 and larger: Problem/Examples, Editor, Interview, and a full-width Output/Test pane are visible.
- At 80x24: one selected pane appears behind tabs.
- Below 60x20: only a resize panel appears. It still reports the guarded `Space q` behavior and any active error.

Resizing preserves the editor buffer, cursor state, operations, and displayed status.

## Local source and process boundary

Source loading and saving are Linux-specific. The application anchors the canonical repository root and target parent with directory descriptors, then uses `openat2` beneath/no-symlink/no-magic-link resolution. Only the catalog-planned regular file is accepted. Root escapes, symlinks, FIFOs, invalid UTF-8, oversized files, and oversized documents are rejected. Saves create an exclusive same-directory temporary file, preserve mode through file descriptors, sync data, rename within the anchored parent, then sync the parent directory.

The local runner starts one direct child in a new process group with no shell. Defaults are a 30-second wall timeout, 250-ms TERM grace, 256-KiB displayed output, 8-KiB pipe reads, and 64 queued events. Timeout, cancellation, and cleanup failures send TERM and then KILL to the group; the direct child is reaped and reader threads have bounded drains and joins. A descendant that deliberately calls `setsid` escapes process-group containment and may continue after its pipes are closed. The PTY fixture records, kills, and reaps that escaped PID; this is a tested boundary, not a containment claim.

## Codex interaction

The first Codex action per launch shows a disclosure. Enter/`y` accepts; Esc/`n` declines. After acceptance, `i` opens the composer, Enter sends a nonempty question, and Esc leaves it. `Space h` provides at most three progressively stronger hints per source revision: invariant/question, technique/counterexample, then pseudocode direction. Editing resets that revision's hint allowance. Hints use a separate ephemeral Codex thread and receive no interviewer transcript.

The interviewer asks one focused question at a time. Automatic submission review starts only after the attempt row is recorded and receives the exact captured source, revision, and bounded test output from that submission. If Codex is connecting, recovering, or handling another turn, at most the newest recorded submission review waits; a newer successful submit replaces it. Ready Codex dispatches that review before another question or hint. Declined, disabled, and authentication-required sessions never send it.

Interviewer and hint responses are accepted only if operation ID, role, captured source revision, and current editor revision still match. A submission-review response instead matches the active review's operation, role, and recorded revision, so editing during review does not relabel it as feedback on current source. The UI labels it `Submission review · recorded revision N`; it cannot change the authoritative local result. The transcript and queued review are bounded and memory-only; `Space r`, leaving solve mode, and process exit clear them. No transcript is written by Interview Tutor.

Protocol/authentication/turn failures show an error without disabling local solve. Press `i` to explicitly reconnect for the next distinct operation. Failed content is not replayed, and a session gets at most one replacement process. Ctrl-C requests a bounded app-server turn interrupt; a missing acknowledgement invalidates and kills the process. Use `--no-codex` to disable all probing and spawning. See [Codex setup, privacy, and troubleshooting](codex-compatibility.md).

## Signals and verification

SIGINT and SIGTERM cancel active work, join workers, restore terminal state and prior signal dispositions, and exit with 130 or 143. For submit, the runner worker checks shared signal state immediately after recording and before publishing completion. A signal observed by that cutoff rewrites that exact attempt to Cancelled; a later signal applies only to runtime teardown.

Run `make test-pty` for the 32-case serial acceptance matrix and `make test-race` for the 20-case cancellation matrix plus repeated Rust race tests. Both use only local fake fixtures, including an explicitly configured fake Codex executable. Exact bounds and gate contents are documented in [testing](testing.md).
