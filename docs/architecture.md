# Architecture

Interview Tutor separates a small control plane from bounded local data and process boundaries. The Rust `cli` crate is the authority shared by the noninteractive CLI and TUI; language runners never own catalog or progress state.

## Components and ownership

1. **Distribution catalog:** `catalog/problems.json` defines global problem metadata, local statement briefs, test revision, and installed language source paths. `problem_sets/*.json` defines only metadata plus ordered global slugs. A catalog revision triggers one managed reconciliation.
2. **SQLite repository:** `.turso/progress.db` by default stores global/custom problems, set membership, languages, implementations, attempts, and derived progress inputs. It is authoritative for custom CRUD. Shipped rows are managed/read-only through local CRUD; a later catalog revision may retire omitted managed rows but never deletes custom resources or attempt history.
3. **Execution planning:** `database::resolve_problem` and `runner::plan_execution` turn stable language/problem/set selectors into an explicit project root, runner, source path, and invoked-set context. Neither the TUI nor the root launchers infer an adapter from display text.
4. **Source/editor:** `source` performs Linux-anchored load/save of exactly the planned regular file. `editor` owns bounded Unicode-grapheme text, modes, revision/dirty state, and highlighting.
5. **Local runner:** `runner` executes the selected language adapter without a shell, sanitizes/bounds output, controls the process group, and returns one authoritative result. Recording is a separate explicit database operation.
6. **Application/TUI:** `app` is the state/reducer/effect boundary. `tui` owns terminal entry/restoration, input/rendering, the repository connection on the main thread, and bounded runner/Codex workers.
7. **Codex boundary:** `codex` validates one trusted configured executable and exact app-server version, constructs the disclosed payload, enforces protocol/resource bounds, and retains only a memory transcript. It cannot alter runner results or progress directly.

## Catalog and database model

SQLite schema v2 uses durable integer keys internally:

- `problems`: global metadata, local statement Markdown, test revision, managed/archive state
- `problem_sets`: named collections
- `problem_set_members`: many-to-many membership and contiguous 1-based ordinal
- `languages`: installed runner commands
- `problem_implementations`: per-problem source paths and adapter availability
- `attempts`: problem/language outcome and optional invoked-set context

Completion is derived from a passing attempt whose revision matches the problem's current test revision. It is not stored per set. Removing/reordering membership cannot remove learning history. Retiring a managed set clears only its optional attempt context through `ON DELETE SET NULL`.

Database open validates the catalog before use, enables a 5-second SQLite busy timeout and WAL journaling, performs schema migration/seed reconciliation transactionally, re-enables foreign keys, and verifies them. A TUI submit opens a separate repository connection only after execution returns. Each accepted submit records one attempt atomically; signal finalization may update that exact attempt to Cancelled before completion is published.

## Control plane and data plane

Control messages are small, typed values: selectors, `ExecutionPlan`, operation ID, source revision, run intent, cancellation token, and reducer effects/events. They determine which immutable snapshot an operation owns. Stale operation IDs are ignored; a stale source revision may be displayed but cannot describe the current buffer or enter a Codex transcript.

Data values are explicitly bounded at ingress: statement/source text, source snapshots, stdout/stderr chunks, rendered output, composer text, protocol lines, assistant responses, transcript entries, and temporary submitted-source copies. The TUI never parses CLI tables. The local runner never writes progress. Codex never executes the language adapter and cannot mark a problem complete.

## Threads and channels

The main thread owns `AppState`, the primary SQLite connection, Crossterm/Ratatui, input polling, rendering, and reducer application. It never blocks on a local test or model turn.

- One runner worker has a command channel of 2 and event channel of 64. It serializes source save, synchronous execute, and optional attempt record/finalization. App state retains at most the newest queued save/test; submit is rejected while a run is active.
- During execution, the local runner creates at most two pipe reader threads. Their bounded channel uses the configured event capacity (64 by default). The coordinator continuously drains into sanitized bounded retention. Optional caller progress uses nonblocking `try_send`; full/disconnected consumers increment a drop count and cannot delay cleanup. All reader threads are joined.
- One Codex worker has a command channel of 2 and event channel of 64. It owns one app-server process/session and serializes connect/turn/reset. The process reader channel holds 64 protocol messages, and pending request IDs are capped at 16. Interviewer/reviewer and hinter use two separate ephemeral app-server conversation threads; these are protocol resources, not additional application authority.

Worker events carry operation/revision/role identity. A Codex response is held pending until the reducer explicitly accepts the matching current turn; cancellation, edit, reset, or stale identity discards it before transcript commit. Both workers are cancelled and joined before terminal restoration.

## Resource bounds

Catalog/TUI queries are capped at 10,000 rows. Statements accept at most 1,000,000 characters and rendered Markdown at most 100,000 characters. The editor accepts at most 1 MiB, 100,000 lines, 32 undo snapshots, and a 256-byte command. TUI local output is capped at 256 KiB and the composer at 16 KiB.

The local executor defaults to a 30-second wall timeout, 250-ms TERM grace, 256-KiB display cap, 8-KiB reads, and 64 reader/progress events. Configurable limits are validated: wall time 10 ms to 1 hour, TERM grace at most 10 seconds, display output 64 bytes to 16 MiB, reads 256 bytes to 64 KiB, and event capacity at most 1024.

Persistent per-stream VTE parsers decode arbitrary UTF-8 chunk boundaries and remove CSI, OSC, and dangerous controls while retaining newlines/tabs. Sanitized prefix/tail retention emits one deterministic omission marker. The final UTF-8 display never exceeds its cap; raw accounting reports exact omitted bytes. Normal execution retains at most 1.5 times the display cap in sanitized text. Discovery can retain separate stdout/stderr and has a 4.5-times aggregate maximum; it parses stdout only and rejects truncation.

Codex bounds are a 10-second version probe/startup, 120-second turn, 2-second interrupt/shutdown acknowledgement, 1-second kill/reap, 250-ms reader drain, 64-KiB version output, 1-MiB stderr ring, 2-MiB protocol line, 64-KiB assistant response, 16-KiB question, 128 transcript entries, and 256-KiB transcript. TUI prompt output includes only the most recent 16 KiB. Hints stop at three per source revision.

## Local execution and process cleanup

The executor starts one direct child in a new Linux process group and captures stdout/stderr. `Termination` distinguishes numeric exit, cancellation, timeout, and external signal. Exit 0 maps to pass, exit 2 to error, every other numeric exit to fail, explicit cancellation to cancelled, and timeout/signal to error.

Timeout, cancellation, post-spawn failure, or residual descendants trigger group TERM, bounded grace, then KILL. The direct child is reaped before pipe shutdown. Nonblocking readers observe shutdown and have a 100-ms final drain, then are joined. A descendant that deliberately calls `setsid` escapes group signaling and may continue, but pipe closure bounds reader impact. Tests explicitly track and reap such fixture descendants rather than claiming containment.

Adapter `--list` discovery uses the same process implementation with a 5-second CLI timeout and 64-KiB output. It parses only sanitized stdout, ignores stderr as slug input while including it in errors, and rejects partial/truncated stdout before database mutation.

## Source and atomic write boundary

`source` canonicalizes and opens the project root as a directory descriptor. Relative catalog paths must contain only normal components. Linux `openat2` applies beneath, no-symlink, and no-magic-link resolution to both load and save. The target must remain a regular file; root escapes, symlinks, FIFOs, invalid UTF-8, and bound violations fail closed.

A save opens the anchored parent descriptor, verifies the existing target, creates an exclusive mode-0600 same-directory temporary, writes exact bytes, preserves target mode through `fchmod`, flushes and `fsync`s the temporary, renames within that same anchored parent, then `fsync`s the directory. Failure before rename removes the temporary. This gives atomic replacement and ordered durability without reopening an attacker-substitutable path.

## Signals and terminal lifecycle

CLI execution registers scoped SIGINT/SIGTERM handlers against its cancellation token. After all runner threads join and an attempt is recorded, the remaining main thread blocks both signals, checks cancellation plus pending signals, restores handlers, consumes pending signals, and restores the mask. A signal observed at this cutoff rewrites that attempt to Cancelled with exit 130/143. A later signal cannot rewrite a published run.

The TUI shares signal state with the runner worker. Immediately after a submit record and before `RunFinished`, the worker performs the same completion cutoff. Runtime SIGINT/SIGTERM cancels active work, resets Codex, joins workers, restores alternate screen/raw mode/cursor and prior signal dispositions/mask, then exits 130/143. Panic and startup-error paths use the same terminal guard.

## Codex privacy boundary

Only after disclosure consent does the Codex worker start the configured process. Its application payload has five fields: statement, source, bounded latest output, bounded in-memory transcript, and question. Hint turns omit transcript. Submission review uses the exact captured submitted revision after recording and wipes that temporary copy when dropped.

Each process gets an empty mode-0700 temporary cwd and a cleared environment containing only `HOME`, `CODEX_HOME`, `PATH`, locale, proxy, and certificate variables. Threads request and verify ephemeral/null-path storage, exact cwd, read-only sandbox, no sandbox network, disabled web search, and never-approve. The wrapper rejects command/file/permission/user-input/MCP requests and reads no API key or token.

This is payload minimization, not total isolation. The trusted configured executable can use read-only tools, config, MCP, and other locally readable paths allowed by its own sandbox/configuration because `HOME`/`CODEX_HOME` remain available. A dedicated minimal profile/home is the stronger operational boundary. Transcript state is cleared on reset/exit. Temporary-cwd cleanup during normal reset/exit is best effort because it runs through `Drop`; explicit shutdown paths surface cleanup failures.

## Extension path

Adding another set imports any missing global problems/adapters and then adds ordered slug references. Shared problems keep one source and completion identity. Adding another language installs a set-agnostic runner plus catalog source mappings; it does not add set-specific execution logic.
