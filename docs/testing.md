# Testing

All repository gates are Linux-first and deterministic. Install stable Rust with rustfmt/Clippy, Python 3.12+, GNU Make/coreutils, and the pinned CI formatter/linter when reproducing CI:

```console
python3 -m pip install ruff==0.14.10
cargo fetch --manifest-path cli/Cargo.toml --locked
cargo fetch --manifest-path rust/Cargo.toml --locked
export CARGO_NET_OFFLINE=true TERM=xterm-256color RUSTFLAGS=-Dwarnings
```

## Gates

### `make check`

Runs, in order:

1. `make test-harness`
2. Python bytecode compilation for `python`, `tests`, and `cli/tests`
3. Ruff format and lint for `python` and `tests` when Ruff is installed
4. CLI rustfmt, all-target Clippy with warnings denied, and the complete CLI Cargo suite serially
5. adapter-crate rustfmt and `cargo check`
6. the Rust adapter registry test
7. root Python administrative/catalog/CLI unit tests

The language starter implementations are intentionally incomplete, so running every Python or Rust problem case is expected to fail. That is not a repository gate. The relevant distribution invariant is the registry/catalog gate: adapters, slugs, metadata, and dispatch must stay coherent while starter answers remain exercises.

### `make test-harness`

Exercises the Python PTY harness itself: an injected hard timeout with child cleanup, cleanup-registry behavior after an injected failure, ten cursor-control boundary sequences, and Make's race-loop fail-fast behavior. This target does not build the binaries.

### `make test-pty`

Fetches the locked CLI graph, reuses/builds the two debug binaries in offline mode, then runs the serial Linux PTY matrix. Its 32 cases are:

- three full/compact/resize workflows
- six local-runner fail/timeout/cancel/output/process-boundary cases
- four SIGINT/SIGTERM attempt-recording lock cases
- thirteen Codex auth/decline/protocol/approval/reconnect/backpressure cases
- one configuration-boundary case
- five terminal lifecycle cases: clean quit, startup error, panic, SIGINT, and SIGTERM

Each ordinary PTY case has a 20-second hard deadline. Lifecycle cases use 4 seconds. The Python matrix has a 90-second hard deadline and asserts its own total is at most 90 seconds; the Make command adds GNU `timeout` at 95 seconds with a 5-second kill-after grace.

The gate checks 120x40 and 80x24 rendering, below-minimum resize behavior and state preservation, save/test/submit attempt rows, stale revisions, bounded output, fake Codex degradation/privacy/reconnect, terminal mode/cursor/alternate-screen restoration, prior signal dispositions, direct-child reap, process-group cleanup, pipe-reader shutdown, and temporary artifact cleanup. A hostile descendant that calls `setsid` is outside process-group containment; the fixture records and explicitly kills/reaps its PID so the limitation remains visible.

### `make test-race`

Builds/reuses the same debug binaries, then runs a 20-case PTY matrix: ten local-runner cancellation repetitions and ten Codex interrupt-without-ack repetitions. It uses the same 20-second per-case, 90-second inner-matrix, and 95-second Make wrapper bounds.

After the PTY matrix, Make runs ten fail-fast iterations. Each iteration runs the exact queued Codex completion/cancel race unit test and the local runner cancellation-at-timeout-boundary integration test, both single-threaded. These Cargo invocations have no separate Make timeout; CI's 20-minute job timeout is their outer ceiling.

## CI contract

`.github/workflows/ci.yml` runs for every pull request and push on `ubuntu-latest`. Workflow permissions are `contents: read`; concurrency is grouped by workflow/ref and older in-progress runs are cancelled. The single Linux job has a 20-minute timeout and sets `TERM=xterm-256color` plus `RUSTFLAGS=-Dwarnings`.

CI installs stable Rust with rustfmt/Clippy, Python 3.12, and exactly Ruff 0.14.10. Its Cargo cache contains download indexes/archives and Git databases only, uses an exact OS/architecture/lockfile key, and never caches Cargo credentials or build outputs. CI fetches both lockfiles online once, then sets `CARGO_NET_OFFLINE=true` for these exact gates:

```console
make check
make test-harness
make test-pty
make test-race
cargo check --manifest-path cli/Cargo.toml --bins --release --locked --offline
```

The explicit `make test-harness` repeats the self-test already entered by `make check` because it is a named CI gate. Cargo target directories remain in the same job, so debug artifacts from `make check` are reused by both PTY gates rather than rebuilt from scratch. Only the final release check uses the separate release profile. `make check`, harness self-tests, Cargo tests, and the release check have no narrower workflow timeout; the 20-minute job ceiling applies.

CI passes no GitHub or OpenAI secrets. `INTERVIEW_TUTOR_CODEX_EXECUTABLE` points to the checked-in fake app-server. PTY environments remove `OPENAI_API_KEY`, and the fake fixture fails if it receives that variable. No gate performs a live Codex/model turn. Fixture isolation is not a kernel network namespace and does not claim to contain a compromised arbitrary executable.

## Adding deterministic fixtures

1. Reproduce behavior with a fake local runner mode in `cli/tests/pty_matrix.py` or a fake app-server mode in `cli/tests/fixtures/fake_codex_app_server.py`. Never require credentials, the installed `codex`, a live model, wall-clock dates, or external network data.
2. Create all database, source, home, and capture state beneath the per-case temporary fixture root. Remove `OPENAI_API_KEY` and explicitly set the fake executable.
3. Drive observable terminal input/output through `PtySession`; assert the screen, database rows, process state, captured allowlisted payload, and final exit status instead of implementation strings alone.
4. Register every PID file and temporary path with `CleanupRegistry` before the case can fail. For deliberate `setsid`, record the escaped PID and assert explicit kill/reap.
5. Add the case through `Matrix.run` so the 20-second case deadline, terminal finalizer, and cleanup checks apply. Keep the whole selected matrix below 90 seconds and update the documented case count if it changes.
6. Run `make test-harness`, the narrow new fixture, then the full `make test-pty`; use `make test-race` for cancellation/finalization changes.
