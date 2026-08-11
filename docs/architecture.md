# Architecture

## Ownership

The checked-in distribution has two independent layers:

1. `catalog/problems.json` defines global problems and installed language adapter source paths.
2. `problem_sets/*.json` defines ordered references to global problem slugs.

A catalog revision seeds or upgrades the local database once. Each upgrade reconciles the exact managed catalog and retires managed resources that were omitted, without changing custom resources or deleting attempt rows. Retiring a managed set clears its optional invoked-set context from historical attempts through `ON DELETE SET NULL`. The database is the runtime authority for user-created problems and sets; opening the CLI does not continuously overwrite local CRUD. Shipped problems and sets are marked managed and read-only through local CRUD. Custom resources are fully editable.

## Runtime schema

SQLite v2 uses durable integer keys internally:

- `problems`: global metadata, statement Markdown, test revision, and archival state.
- `problem_sets`: named collections.
- `problem_set_members`: many-to-many membership plus a contiguous 1-based ordinal.
- `languages`: installed runner commands.
- `problem_implementations`: per-problem source paths and adapter availability.
- `attempts`: global problem/language results with optional invoked-set context.

Completion is derived from a passing attempt whose revision matches the global problem's current test revision. It is never stored per set. Removing or reordering a membership cannot delete learning history; deleting a retired set only clears the attempt's optional invoked-set context.

## Execution

`practice_cli::database::resolve_problem` resolves either a global slug or an exact set slug/index. `practice_cli::runner::plan_execution` preserves the project root, runner path, solution path, problem, language, and optional invoked set. `execute` invokes the set-agnostic language protocol without a shell, while `record_execution` remains an explicit operation that writes exactly one central attempt. Spawn and preflight failures do not record. Language-local wrappers suppress their compatibility recorder during central execution.

The Linux executor starts one direct child in a new process group and captures both pipes. Defaults are a 30 second wall timeout, 250 millisecond TERM grace, 256 KiB of final rendered display output, 8 KiB pipe reads, and 64 queued events. Limits are validated and bounded. Persistent per-stream VTE parsers decode arbitrary UTF-8 chunk boundaries and discard CSI, OSC, and dangerous controls while preserving newlines and tabs. Parser output is drained immediately into bounded, already-sanitized prefix/tail retention, so truncated raw terminal sequences are never reparsed from ground state. The final UTF-8 result, including stream tags and one deterministic omission marker, never exceeds the display cap. A separate bounded raw-byte accounting path reports exact bytes beyond the retention cap in `omitted_bytes`. Normal execution retains at most 1.5 times the display cap of sanitized text. Discovery adds at most 1.5 times the cap for each separately retained stdout and stderr stream, for a 4.5-times aggregate sanitized-text maximum, and parses stdout only.

`Termination` distinguishes a numeric exit from caller cancellation, timeout, and external signal. Exit 0 maps to `pass`, exit 2 to `error`, every other numeric exit including 130 to `fail`, explicit cancellation to `cancelled`, and timeout or signal to `error`. Timeout, cancellation, failures after spawn, and remaining descendants trigger process-group TERM, a bounded grace, then KILL. The direct child is reaped before pipe shutdown. Nonblocking readers receive a bounded final drain window, observe a shutdown flag, and are always joined, so a `setsid` descendant retaining a pipe cannot hang execution. A `setsid` descendant can escape group termination on Linux and may continue running, but closing the local pipes bounds its impact.

Optional progress events use a caller-provided bounded `SyncSender` with nonblocking `try_send`. Full and disconnected consumers cause counted drops and cannot delay timeout, cancellation, reap, drain, or join. The final `ExecutionResult` is authoritative. The CLI registers SIGINT and SIGTERM against the execution's `CancellationToken`. Before SQLite recording, after runner threads have joined, it blocks both signals on the remaining main thread. It records once, inspects the cancellation flag and pending signals, unregisters the handlers, and updates that exact attempt to `cancelled` with exit code 130 or 143 when needed. The signals remain blocked through process exit, closing the post-recording race. The synchronous API is intended to run on a TUI worker thread and does not require an async runtime.

Adapter `--list` discovery uses the same process-group implementation with a five second CLI timeout and 64 KiB retained-output cap before any database mutation. It parses only sanitized stdout, ignores stderr as slug input while including it in execution errors, and rejects truncated stdout instead of parsing partial lines. This plan/result boundary lets the future TUI reuse statement and runner APIs without parsing CLI tables.

## Extension path

Adding NeetCode 150 or 250 consists of importing any missing global problems, installing their language adapters, then creating a new ordered problem set that references both existing and new slugs. Overlapping problems retain one implementation and one completion identity.

## Solve-mode boundary

The main terminal thread owns `AppState`, Ratatui, and the repository connection. Pure reducer effects carry `OperationId`, source generation, and `RunIntent`. One bounded worker thread performs atomic source saves and synchronous process-group execution; explicit submit opens a separate SQLite connection only after execution terminates to record exactly one attempt. The runtime cancels and joins this worker before restoring the terminal. Stale operation IDs are ignored, and stale source generations may be displayed but cannot describe the current buffer.

`source` accepts only the planned regular solution file beneath the canonical root and rejects symlinks, escapes, oversized data, and invalid UTF-8. `editor` owns a bounded Unicode document and 32-snapshot undo history. The built-in Rust/Python lexer emits only keyword, string, comment, and plain spans. Interview state is an offline placeholder with no network or persistence boundary until Stack 7.
