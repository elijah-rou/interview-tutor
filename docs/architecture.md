# Architecture

## Ownership

The checked-in distribution has two independent layers:

1. `catalog/problems.json` defines global problems and installed language adapter source paths.
2. `problem_sets/*.json` defines ordered references to global problem slugs.

A catalog revision seeds or upgrades the local database once. Each upgrade reconciles the exact managed catalog and retires managed resources that were omitted, without changing custom resources or attempt history. The database is the runtime authority for user-created problems and sets; opening the CLI does not continuously overwrite local CRUD. Shipped problems and sets are marked managed and read-only through local CRUD. Custom resources are fully editable.

## Runtime schema

SQLite v2 uses durable integer keys internally:

- `problems`: global metadata, statement Markdown, test revision, and archival state.
- `problem_sets`: named collections.
- `problem_set_members`: many-to-many membership plus a contiguous 1-based ordinal.
- `languages`: installed runner commands.
- `problem_implementations`: per-problem source paths and adapter availability.
- `attempts`: global problem/language results with optional invoked-set context.

Completion is derived from a passing attempt whose revision matches the global problem's current test revision. It is never stored per set. Removing or reordering a membership cannot delete learning history.

## Execution

`practice_cli::database::resolve_problem` resolves either a global slug or an exact set slug/index. `practice_cli::runner::plan_execution` produces the runner and solution paths. `execute_plan` invokes the set-agnostic language protocol, and `record_execution` writes one central attempt. Language-local wrappers suppress their compatibility recorder during central execution.

This plan/result boundary is the integration point for the future TUI. A Rust TUI can reuse the CLI crate modules to load statement Markdown and registered paths without parsing human CLI tables. The current executor inherits child output and has no timeout or cancellation API. Bounded output streaming belongs to the later runner layer.

## Extension path

Adding NeetCode 150 or 250 consists of importing any missing global problems, installing their language adapters, then creating a new ordered problem set that references both existing and new slugs. Overlapping problems retain one implementation and one completion identity.
