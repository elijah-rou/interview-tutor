# Interview TUI

Run `./interview` in an interactive Linux terminal. Optional startup flags are `--db PATH`, `--set ID`, `--language ID`, `--neovim PATH`, and `--interviewer pi|codex|none`; `--no-codex` remains a legacy alias for `--interviewer none`. Without `--set`, the TUI opens the set menu; otherwise it opens that set's problem list.

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

Solve mode has Editor, Problem/Examples, Output/Test, and Interview panes. In Editor Normal, Visual, and command/search modes, Tab and Shift-Tab cycle all four panes, including collapsed accessory rails. In Editor Insert, Replace, and terminal modes, both keys go to Neovim instead; accessory panes always use them for pane cycling. Problem, Output, and Interview collapse independently; Editor cannot collapse. Focused expanded accessories scroll with Up/Down. Interview scrolls by row with Up/Down, by ten rows with PageUp/PageDown, and jumps to the oldest/latest retained message with Home/End; these controls remain available while its composer is focused. It returns to newest when a message is appended or the session is cleared. The visible history retains at most 512 messages and 1 MiB independently of the smaller model-context bound, and its oldest view marks any earlier messages omitted at that display limit. Transcript turns use distinct styled `YOU`, `<backend> · INTERVIEWER`, `HINTER`, and submission-review headings, consistently indented bodies, and blank-row separation. Collapsing Interview closes its composer without clearing its transcript or active operation. Outside the Editor, `i` expands Interview and focuses its composer. In the Editor it retains Vim insert behavior.

Global solve actions are:

- `Space t`: save the current revision atomically and run local tests
- `Space s`: submit the current revision
- Ctrl-C: cancel the operation selected by focus; Interview wins when it is focused and both the selected interviewer and local runner are active, otherwise the local runner is cancelled
- `Space h`: request a hint outside Editor Insert/Command mode and outside the active composer
- `Space r`: clear the Interview session when Interview is focused
- `Space b`: leave solve from Editor Normal mode or any accessory pane
- `Space c`: collapse or expand the focused accessory; Editor reports that it cannot collapse
- `Space q`: quit from Editor Normal mode
- `Space ?`: open Solve help from Editor Normal mode or any accessory; Esc or `?` dismisses it
- Ctrl-Q: quit from every pane and Neovim mode, including the undersized resize screen

F5 and F9 remain compatibility aliases for test and submit. In accessories, the Space leader precedes disclosure and scrolling; the active Interview composer instead receives typed spaces. Unknown leader keys clear the leader and continue normal accessory routing. Solve help is modal, so dismissal keys are handled before ordinary solve input.

Back atomically autosaves a dirty draft without running tests or recording an attempt, then opens the selected set's problem list directly. It first cancels and joins an active local runner. Save failure or an edit newer than the captured save keeps Solve open and dirty for retry. Clean Back opens the problem list without a save. Application quit remains separately guarded when dirty: repeat either `Space q` or Ctrl-Q to discard and quit. Esc never discards solve changes. `:TutorBack` uses the same Back behavior; native lowercase `:back` is not overridden, and native `:q` applies Neovim's own buffer/window rules rather than application navigation.

## Embedded Neovim

Solve requires a compatible trusted Neovim (`>=0.9`, `<2.0`) and starts it with `nvim --clean --embed`. Interview Tutor attaches through Neovim's external-UI MessagePack-RPC protocol and renders the real Neovim grid inside the Editor pane. Normal, Insert, Visual, operator-pending, command/search, registers, macros, counts, text objects, undo/redo, dot-repeat, completion, Lua, shell, job, terminal, split, and tab behavior therefore comes from clean Neovim rather than an emulation. The host reserves Tab and Shift-Tab for pane cycling only outside Insert, Replace, and terminal modes. User initialization and user plugins are not loaded.

Select the executable with `--neovim PATH`, then `INTERVIEW_TUTOR_NEOVIM_EXECUTABLE`; otherwise `nvim` is resolved on `PATH`. The canonical target must be a regular executable owned by the effective user or root and not group/world writable. Missing, incompatible, or failed Neovim is an explicit solve error; there is no manual-editor fallback.

The actual solution path is never given to Neovim. Source is loaded over RPC into a synthetic `acwrite` scratch buffer with swap, persistent undo files, modelines, automatic writes, and clipboard-provider integration disabled. `:TutorTest`, `:TutorSubmit`, `:TutorBack`, `:TutorCollapse`, and `:TutorHelp` expose application actions; Normal-mode `Space t/s/b/c/?`, guarded `Space q`, hints, and F5/F9 are reserved integration mappings. Interview Tutor remains authoritative for exact bytes, revision identity, atomic saving, tests, and submissions.

The host accepts at most 1 MiB and 100,000 logical lines. If a native Neovim operation transiently exceeds that bound, Interview Tutor rejects the candidate and restarts Neovim from the last valid exact source. Application bytes and revision remain unchanged; Neovim-local undo/register/repeat state may reset only on that overflow recovery path. RPC values, nesting, nodes, grids, highlights, queues, stderr, process memory, calls, and teardown are bounded.

## Test, save, and submit semantics

Ctrl-S, F5, `Space t`, `:TutorTest`, and a plain `:write` atomically save if dirty and run the local suite. They never create an attempt row. A clean buffer still runs. If another local run is active, only the newest requested save/test revision is retained; submit is rejected until that run completes. A failed save starts no runner.

F9, `Space s`, and `:TutorSubmit` save, run, then record exactly one attempt after the runner returns an execution result. Pass, fail, timeout, and explicit cancellation outcomes are recorded; preflight/spawn failures that produce no execution result are not. Repeating submit records another attempt. Local runner results remain authoritative with any interviewer backend.

Edits are allowed during any run. Revisions increase monotonically across accepted Neovim source snapshots, while dirty state compares exact bytes against the last saved bytes. Source-consuming Tutor mappings are delivered after preceding Neovim input and bind the run to that accepted snapshot.

Output is bounded and sanitized. `STALE` appears only when displayed output belongs to an older editor revision; edits before the first run are not stale. Save errors, runner errors, interviewer errors, and status such as testing, submitting, cancellation, or stale completion stay visible in the status/error and Output panes.

## Layout

- At 100x30 and larger: expanded Problem/Examples, Editor, Interview, and a full-width Output/Test pane are visible. Collapsed side panes become six-column titled rails, collapsed Output becomes a three-row rail, and Editor receives the freed space. Expanded side panes widen with focus: Interview uses about 45% when focused, and focused Problem is materially wider than its resting width.
- At 80x24: one selected pane appears behind collapse-marked tabs. A collapsed accessory keeps focus on its tab while Editor renders as the content fallback.
- Below 60x20: only a resize panel appears. Ctrl-Q remains available from every focused pane and Neovim mode; dirty source requires a second Ctrl-Q confirmation.

Resizing preserves the editor buffer, cursor state, operations, focused pane, each independent collapse state, and displayed status.

## Local source and process boundary

Source loading and saving are Linux-specific. The application anchors the canonical repository root and target parent with directory descriptors, then uses `openat2` beneath/no-symlink/no-magic-link resolution. Only the catalog-planned regular file is accepted. Root escapes, symlinks, FIFOs, invalid UTF-8, oversized files, and oversized documents are rejected. Saves create an exclusive same-directory temporary file, preserve mode through file descriptors, sync data, rename within the anchored parent, then sync the parent directory.

The local runner starts one direct child in a new process group with no shell. Defaults are a 30-second wall timeout, 250-ms TERM grace, 256-KiB displayed output, 8-KiB pipe reads, and 64 queued events. Timeout, cancellation, and cleanup failures send TERM and then KILL to the group; the direct child is reaped and reader threads have bounded drains and joins. A descendant that deliberately calls `setsid` escapes process-group containment and may continue after its pipes are closed. The PTY fixture records, kills, and reaps that escaped PID; this is a tested boundary, not a containment claim.

## Interviewer interaction

The first action for the selected Pi or Codex backend shows a backend-specific disclosure. Enter/`y` accepts; Esc/`n` declines. After acceptance, `i` opens the composer, Enter sends a nonempty question, and Esc leaves it. `Space h` provides at most three progressively stronger hints per source revision: invariant/question, technique/counterexample, then pseudocode direction. Editing resets that revision's hint allowance. Hints receive no interviewer transcript and use an ephemeral operation.

The interviewer asks one focused question at a time. Automatic submission review starts only after the attempt row is recorded and receives the exact captured source, revision, and bounded test output from that submission. If the interviewer is connecting, recovering, or handling another turn, at most the newest recorded submission review waits; a newer successful submit replaces it. A ready interviewer dispatches that review before another question or hint. Declined, disabled, and authentication-required sessions never send it.

Interviewer and hint responses are accepted only if operation ID, role, captured source revision, and current editor revision still match. A submission-review response instead matches the active review's operation, role, and recorded revision, so editing during review does not relabel it as feedback on current source. The UI labels it `Submission review · recorded revision N`; it cannot change the authoritative local result. The model-context transcript retains at most 128 entries and 256 KiB. The independently bounded visible history retains up to 512 messages and 1 MiB so older dialog remains scrollable; if that larger bound is reached, its oldest view displays an explicit omission marker. The transcript and queued review are memory-only; `Space r`, leaving solve mode, and process exit clear them. No transcript is written by Interview Tutor.

Protocol/authentication/turn failures show an error without disabling local solve. Press `i` to explicitly reconnect for the next distinct operation. Pi is the default and uses a fresh no-session RPC process per application turn. Codex is an explicit compatibility transport and permits at most one replacement app-server process. Rejected or stale Codex turns replace their role-specific remote thread before another turn, while the accepted application transcript is preserved. Failed content is not replayed. Ctrl-C sends the selected transport's bounded abort/interrupt and requires both acknowledgement and terminal settlement. Use `--interviewer none` to disable all probing and spawning. See [interviewer setup, privacy, and troubleshooting](codex-compatibility.md).

## Signals and verification

SIGINT and SIGTERM cancel active work, join workers, restore terminal state and prior signal dispositions, and exit with 130 or 143. For submit, the runner worker checks shared signal state immediately after recording and before publishing completion. A signal observed by that cutoff rewrites that exact attempt to Cancelled; a later signal applies only to runtime teardown.

Run `make test-pty` for the bounded serial acceptance matrix and `make test-race` for the 20-case cancellation matrix plus repeated Rust race tests. Both use only local fake fixtures, including an explicitly configured fake Codex executable. Exact bounds and gate contents are documented in [testing](testing.md).
