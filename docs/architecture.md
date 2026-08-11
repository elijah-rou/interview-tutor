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

The Linux executor starts one direct child in a new process group and captures both pipes. Defaults are a 30 second wall timeout, 250 millisecond TERM grace, 256 KiB of final rendered display output, 8 KiB pipe reads, and 64 queued events. Limits are validated and bounded. Persistent per-stream VTE parsers decode arbitrary UTF-8 chunk boundaries and discard CSI, OSC, and dangerous controls while preserving newlines and tabs. The final UTF-8 result, including stream tags and its deterministic omission marker, never exceeds the display cap. `omitted_bytes` separately reports the exact raw bytes excluded from that result.

`Termination` distinguishes a numeric exit from caller cancellation, timeout, and external signal. Exit 0 maps to `pass`, exit 2 to `error`, every other numeric exit including 130 to `fail`, explicit cancellation to `cancelled`, and timeout or signal to `error`. Timeout, cancellation, failures after spawn, and remaining descendants trigger process-group TERM, a bounded grace, then KILL. The direct child is reaped before pipe shutdown. Nonblocking readers receive a bounded final drain window, observe a shutdown flag, and are always joined, so a `setsid` descendant retaining a pipe cannot hang execution. A `setsid` descendant can escape group termination on Linux and may continue running, but closing the local pipes bounds its impact.

Optional progress events use a caller-provided bounded `SyncSender` with nonblocking `try_send`. Full and disconnected consumers cause counted drops and cannot delay timeout, cancellation, reap, drain, or join. The final `ExecutionResult` is authoritative. The CLI registers SIGINT and SIGTERM against the execution's `CancellationToken`, unregisters both handlers after cleanup, records one cancelled attempt, and exits 130 or 143 respectively. The synchronous API is intended to run on a TUI worker thread and does not require an async runtime.

Adapter `--list` discovery uses the same process-group implementation with a five second CLI timeout and 64 KiB retained-output cap before any database mutation. This plan/result boundary lets the future TUI reuse statement and runner APIs without parsing CLI tables.

## Extension path

Adding NeetCode 150 or 250 consists of importing any missing global problems, installing their language adapters, then creating a new ordered problem set that references both existing and new slugs. Overlapping problems retain one implementation and one completion identity.
