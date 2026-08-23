# Interview UX overhaul plan

## Approved product decisions

- Embed a required, clean Neovim process as an external UI inside the editor pane. Do not use Modalkit, a hand-built Vim subset, or an external full-screen editor.
- `Space-t` tests, `Space-s` submits, `Space-b` leaves solve, and `Space-c` toggles the focused accessory pane. Keep F5/F9 as compatibility aliases.
- Leaving solve autosaves a dirty draft atomically without testing or recording an attempt, then returns directly to the selected set's question list.
- Pi is the default interviewer. Codex remains explicitly selectable. No silent fallback between backends.
- Problem, Output, and Interview collapse independently to visible focusable rails. Editor cannot collapse.

## Invariants

- Preserve exact source bytes, including final-newline state. The host commits only snapshots within the existing 1 MiB/100,000-line bounds. If Neovim transiently exceeds a bound, reject the candidate and restart it from the last valid host snapshot; application bytes/revision stay unchanged, while Neovim-local undo/register/repeat state may reset.
- Revisions remain monotonic and change exactly once per accepted content-changing input. Dirty state is byte equality against the last saved source.
- Test and submit retain their current stale-result, atomic-save, cancellation, attempt-recording, and recorded-submission-review contracts.
- Interview prompts, transcript, queues, JSONL records, stderr, processes, retries, and timeouts remain explicitly bounded.
- Disclosure happens before interviewer probe/spawn. Backend failures never disable local editing, testing, submission, or navigation.
- Preserve existing custom/user progress and shipped set behavior. No database schema change is required.

## Milestone 1: required embedded Neovim

1. Add a private bounded MessagePack-RPC implementation under `cli/src/neovim/` using pinned `rmp = "=0.8.14"`. Do not use `nvim-rs` or `neovim-lib`; their hidden child ownership and unbounded channels/pending state conflict with deterministic cleanup and resource limits.
2. Require a canonical trusted `nvim` executable selected by `--neovim`, then `INTERVIEW_TUTOR_NEOVIM_EXECUTABLE`, then `PATH`. Validate ownership/mode/identity around the version/API probe. Require a compatible stable API with `ext_linegrid`; missing, incompatible, or crashed Neovim is a visible solve error, never a silent builtin fallback.
3. Spawn one clean process per Solve session as `nvim --clean --embed`, without `--headless`. Use a fresh process group, piped RPC/stderr, bounded startup/call/shutdown deadlines, stderr ring, queues, grids, decoded values, memory/address-space, file descriptors, and kill/reap cleanup. Neovim owns no outer terminal descriptor or source path.
4. Keep `EditorDocument` as the host authority for exact current/saved UTF-8 bytes, monotonic revision, dirty state, and bounds. Runtime owns the Neovim process and published UI snapshot. Create one synthetic `acwrite` scratch buffer with swap, persistent undo, modelines, automatic writes, and clipboard provider disabled; load source through RPC and track `endofline` separately.
5. Characterize exact round trips for empty, no-final-LF, one/multiple final LF, CRLF bytes, NUL, combining characters, CJK, and emoji. Subscribe to buffer updates, coalesce multipart events by changedtick, and publish only complete exact source candidates. Each accepted byte change increments host revision once; movements/mode/redraw changes do not.
6. On an oversized candidate, freeze input, reject the host update, kill/reap Neovim, and restart it from the last valid source/revision. Bound transient memory. Explicitly accept loss of Neovim-local undo/register/repeat state only on this rejected-overflow recovery path.
7. Attach one root UI grid with `rgb=true`, `ext_linegrid=true`, and Neovim-drawn command line/messages/popup/status. Parse redraw updates in order and publish only at `flush`. Handle grid resize/line/scroll/clear/cursor, default colors, highlight definitions, mode information, busy state, and capability changes; ignore only documented harmless/future metadata.
8. Render the published cell grid directly inside the Ratatui Editor block, including RGB/style attributes, wide continuation cells, cursor shape/visibility, native command/search UI, messages, completion, Visual selection, and native syntax highlighting. Ratatui remains sole owner of raw mode, alternate screen, outer cursor, mouse capture, and bracketed paste.
9. Serialize key, paste, mouse, resize, and barrier requests through the editor worker. Convert Crossterm keys to `nvim_input`, paste through `nvim_paste`, mouse through `nvim_input_mouse`, and resize through `nvim_ui_try_resize`. Source-consuming app actions wait for a barrier after all prior editor input so test/submit/back/interviewer always capture the newest accepted source.
10. Install buffer-local Normal mappings for `Space-t/s/b/c`, F5/F9 aliases, and `:TutorTest`, `:TutorSubmit`, `:TutorBack`, and `:TutorCollapse`. Callbacks use nonblocking `rpcnotify` with buffer identity/changedtick; alternate write targets never expose or write the real source path. All other clean Neovim mappings, registers, macros, search, Ex, Lua, shell, job, and terminal capabilities remain available as trusted local behavior.
11. Preserve app action precedence outside the editor: cancellation, F aliases, Tab focus, composer/disclosure, accessory leader, then Neovim. Inside Editor, Space reaches Neovim and its mode-specific buffer mappings decide eligibility; application chords do not trigger in Insert/Visual/operator/search/command modes.
12. Add a pinned compatible Neovim installation to CI and require real-Neovim behavior tests. Use a fake Neovim only for malformed protocol, flood, hang, crash, identity, and cleanup cases.

### Milestone 1 acceptance

- Real-Neovim PTY tests cover uppercase commands, counts, composed operators/text objects, Visual modes, named/unnamed registers, macros, `/` and `n/N`, undo/redo, dot-repeat, native command/search/message UI, Unicode, paste, mouse, resize, Tutor actions, and F aliases.
- RPC/grid tests cover every handled MessagePack marker/event, fragmentation, depth/size/node/grid/highlight/queue bounds, request correlation, flush-only publication, overlapping scroll, wide cells, unknown-event compatibility, EOF, timeout, crash, and deterministic process-group cleanup.
- Exact source/LF tests prove accepted snapshots preserve bytes and revisions. Overflow tests prove host bytes/saved bytes/revision remain unchanged and Neovim restarts from that exact snapshot.
- Immediate edit→test/submit/back barriers capture the newest bytes. Existing stale/save/submit behavior remains green.

## Milestone 2: backend-neutral interviewer with Pi default

1. Introduce `interviewer/` domain modules for mode, strict prompt/response envelopes, transcript, hint accounting, typed errors, and backend-neutral session behavior. Retain Codex transport under `codex/`; add Pi transport under `pi/`.
2. Generalize Codex-named app state/effects/events/worker/UI to Interviewer equivalents. Use one race-safe worker and one selected transport, not parallel duplicated workers.
3. Add `--interviewer <pi|codex|none>` with precedence: CLI, legacy `--no-codex`→none, `INTERVIEW_TUTOR_INTERVIEWER`, default `pi`. Reject conflicts/invalid/empty values. Add `INTERVIEW_TUTOR_PI_EXECUTABLE`; retain the Codex variable.
4. Pin compatibility to Pi 0.84.2 initially. Canonicalize and verify executable identity before and after the bounded version probe; require a regular effective-user/root-owned file that is not group/world writable.
5. Launch Pi in a mode-0700 empty cwd and fresh process group:
   `pi --mode rpc --no-session --no-tools --no-extensions --no-skills --no-prompt-templates --no-themes --no-context-files --no-approve --offline`.
   Clear the environment, then copy a documented Pi/provider-auth allowlist; force update checks and telemetry off. Never pass secrets on argv.
6. Implement strict LF-only JSONL framing, correlated response IDs, 2 MiB record/queue/aggregate bounds, `prompt` acceptance, completion at `agent_settled`, `get_last_assistant_text`, one correction turn, and abort acknowledgement plus settled ordering. Reject tool/bash/extension-UI events, unexpected IDs, stale text, non-stop completions, floods, EOF, and malformed records.
7. Use a fresh no-session Pi process per application turn (retained only for one correction). The application transcript remains authoritative; reducer finalization commits only accepted operation/revision/mode responses.
8. Make header, disclosure, auth help, errors, footer, docs, and fake fixtures backend-aware. Pi disclosure names the selected model provider and exact disabled resources; Codex disclosure remains accurate.

### Milestone 2 acceptance

- Selector precedence, legacy disable, no fallback, exact argv/environment, executable identity/version, auth failure, cancellation, cleanup, strict protocol, retry/correction, stale finalization, and bounded flood cases pass.
- Default-Pi and explicit-Codex PTY flows pass; none probes/spawns neither executable.
- Existing Codex compatibility tests remain green.

## Milestone 3: navigation, leader actions, and collapsible panes

1. Add independent Problem/Output/Interview expanded state. Tab always visits all panes. Collapsing Interview closes its composer but preserves transcript and active operation. Interview focus expands Interview. Updates never auto-expand panes.
2. Implement leader precedence: Ctrl-C; F aliases; Tab; composer; eligible leader; disclosure; accessory navigation; Vim. Leader is eligible only in editor Normal mode or accessory panes. Unknown second keys clear leader and continue normal routing.
3. Add `Effect::SaveDraft` and `Event::DraftSaved`. Dirty Back must first stop/join any runner (`LeaveSolve`), then atomically save the newest source without execution/attempt recording. On matching success, reset interviewer, reload the scoped list, and enter `ProblemList`. On failure/newer edits, remain in Solve with the buffer dirty and a precise retry message.
4. Clean Back returns directly to `ProblemList`. `Space-b` works from every pane. `:back` maps to the same behavior. Dirty process quit remains separately guarded.
5. Full layout starts at 100×30. Collapsed side panes render narrow titled rails; collapsed Output renders a three-row rail. Expanded panes redistribute freed area to Editor. Compact layouts keep collapse-marked tabs and render Editor as fallback when a collapsed accessory is focused. Resize preserves all state.
6. Replace F-key-centric footer/help text with width-bounded Space leader guidance while retaining F5/F9 aliases in detailed help.

### Milestone 3 acceptance

- RED reducer/input/render tests cover every chord/context, autosave success/failure/stale/cancellation ordering, independent collapse combinations, compact/full/undersized resize, and focus restoration.
- PTY covers broad Vim editing, `Space-t`, `Space-s`, autosave Back to list, collapsed rails, resize preservation, default Pi, explicit Codex, and disabled interviewer.

## Verification and review

1. Capture baseline `make check`, `make test-pty`, `make test-race`, and release build before mutation.
2. Implement serially with one writer in this worktree; require each milestone's focused RED-GREEN evidence before moving on.
3. Run fresh-context reviews for editor correctness/bounds, interviewer security/protocol, and TUI user-flow/PTY coverage.
4. Apply accepted findings with one fix worker, review again if changes are material, then run:
   - `make check`
   - `make test-harness`
   - `make test-pty`
   - `make test-race`
   - `cargo check --manifest-path cli/Cargo.toml --bins --release --locked --offline`
5. Manually exercise full and compact TUI flows using fake Pi/Codex fixtures. Do not use live provider credentials in automated tests.

## Explicit exclusions

- User Neovim configuration or user plugins. `--clean` builtin behavior remains available, including native Ex, Lua, shell, job, terminal, register, macro, split, and tab capabilities within the embedded grid.
- A manual editor fallback when Neovim is missing or incompatible.
- Silent Pi↔Codex fallback.
- Persisting interviewer transcripts.
- Collapsing the Editor pane.
