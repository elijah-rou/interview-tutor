# Rust control plane

`cli/` is a standalone Rust crate containing the catalog/database API, bounded local runner, terminal browser and solve editor, and backend-neutral interviewer boundary with Pi RPC and Codex app-server transports. The repository launchers set the project root and invoke its two binaries:

```console
./practice --help
./interview --help
./run python two-sum
```

Linux development requires stable Rust with rustfmt and Clippy, Python 3.12+, GNU Make, and GNU coreutils. Build and check directly with:

```console
cargo build --manifest-path cli/Cargo.toml --bins --locked
cargo fmt --manifest-path cli/Cargo.toml --check
cargo clippy --manifest-path cli/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path cli/Cargo.toml --locked -- --test-threads=1
```

## Commands and configuration

`practice` owns catalog validation, SQLite v2 migration/reconciliation and CRUD, selectors, progress, adapter discovery, process execution, and attempt recording. Common commands are:

```console
./practice sets list
./practice --set blind75 list
./practice --set blind75 show two-sum
./practice --set blind75 stats --language python
./practice run python blind75 1
./practice db
```

`--db` is a global `practice` flag and an `interview` flag. Resolution is `--db` > `PRACTICE_DATABASE_URL` > `PRACTICE_DB_PATH` > legacy `BLIND75_DATABASE_URL`/`BLIND75_DB_PATH` > `.turso/progress.db`. On first open the CLI creates the parent directory and database, migrates a supported older schema, and reconciles `catalog/problems.json` plus `problem_sets/*.json`.

`./interview --set ID --language ID --neovim PATH --interviewer pi|codex|none` selects an initial set, enabled language, required clean Neovim executable, and one interviewer transport. Interviewer precedence is explicit CLI, legacy `--no-codex` as `none`, `INTERVIEW_TUTOR_INTERVIEWER`, then default `pi`; selected invalid/empty values and CLI conflicts fail. There is no backend fallback. Neovim resolves from `--neovim`, `INTERVIEW_TUTOR_NEOVIM_EXECUTABLE`, then `PATH`. Pi and Codex resolve from their `INTERVIEW_TUTOR_*_EXECUTABLE` variables then `PATH`. `none` probes and spawns neither interviewer; local solve does not depend on an interviewer.

## Solve behavior

From problem detail, Enter loads the catalog-planned source over bounded MessagePack-RPC into a host-owned scratch buffer rendered by required clean embedded Neovim. Ctrl-S/F5/`Space t` atomically save and test without an attempt. F9/`Space s`/`:TutorSubmit` save, test, and record one attempt after the runner terminates. A failed save starts no child. Explicit submit can request Codex review only after recording succeeds, and review uses that operation's exact captured source revision. See [the TUI guide](../docs/interview-tui.md).

The host accepts at most 1 MiB and 100,000 logical lines. An oversized native operation is rejected by restarting Neovim from the last valid exact source without changing the application revision. Linux `openat2` resolution confines loads and same-directory atomic saves to the planned regular source beneath the canonical project root; Neovim never receives that path.

## Components

- `catalog.rs`: checked-in catalog parsing and invariant validation.
- `database.rs`: SQLite schema, migrations, managed seed reconciliation, CRUD, attempts, and progress.
- `source.rs` and `editor.rs`: anchored atomic I/O and bounded Unicode-grapheme editing/highlighting.
- `runner.rs` and `signals.rs`: shell-free execution, process groups, bounded output/events, cancellation, and signal finalization.
- `app/` and `tui/`: pure effects/reducer state plus the terminal runtime and worker channels.
- `interviewer/`: backend selection, typed errors, shared prompt/envelope validation, hint accounting, and bounded memory-only transcript.
- `pi/`: pinned Pi 0.84.2 RPC transport with fresh per-turn processes and strict JSONL validation.
- `codex/`: exact-version Codex app-server compatibility transport.

`runner::execute` is synchronous so the CLI can call it directly and the TUI can place it on one worker thread. Defaults are one direct child, a 30-second wall timeout, 250-ms TERM grace, 256-KiB rendered output, 8-KiB reads, and 64 queued events. Event delivery is nonblocking; the bounded final `ExecutionResult` is authoritative. The TUI reuses the same execution plan and recording APIs rather than parsing CLI output. See [architecture](../docs/architecture.md) and [testing](../docs/testing.md).
