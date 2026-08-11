# Practice CLI

The administrative and execution control plane is a standalone Rust crate. It owns catalog validation, SQLite migrations and CRUD, selector resolution, progress queries, adapter discovery, subprocess execution, and attempt recording.

The repository-level `practice` and `run` scripts are stable launchers for this crate:

```console
./practice --help
./run python two-sum
```

Develop the crate directly with:

```console
cargo fmt --manifest-path cli/Cargo.toml --check
cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path cli/Cargo.toml
```

## Modules

- `catalog.rs`: checked-in catalog parsing and invariant validation.
- `database.rs`: schema v2, atomic v1 migration, seed reconciliation, CRUD, and progress queries.
- `runner.rs`: execution planning, bounded Linux process-group execution and discovery, cancellation, sanitized output events, and explicit attempt recording.
- `main.rs`: the `clap` command grammar and presentation layer.

The CLI crate is independent of the Python solution environment. Python remains one problem-adapter language alongside Rust.

`runner::execute` is synchronous so callers can place it on a worker thread. Its defaults are one child, a 30 second wall timeout, 250 millisecond TERM grace, 256 KiB final rendered output, 8 KiB reads, and 64 queued events. Optional event delivery uses `SyncSender::try_send`; full or disconnected consumers cause counted event drops and never delay lifecycle cleanup. The bounded final `ExecutionResult` is authoritative. See `docs/architecture.md` for termination and outcome semantics.
