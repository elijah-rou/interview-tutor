# Rust adapter runner

The stable Rust crate is a set-agnostic adapter registry.

```console
./run --list
./run two-sum
./run --cases two-sum
./run --problem two-sum --case scenario-1
cargo check
cargo test --no-run
```

`./run SLUG` records through `../practice run`; with `PRACTICE_NO_RECORD=1` it runs every case without recording. `--cases` (which also builds the judge) and `--problem SLUG --case NAME` are the judge protocol `practice` uses. Each problem's `run_case` is its one case, `scenario-1`.

Solutions live under `src/problems/`. The root `../run` command is preferred for set/index resolution and central attempt recording. LeetCode publishes no Rust template for cyclic-list and clone-graph APIs, so `src/types.rs` documents the local representations.
