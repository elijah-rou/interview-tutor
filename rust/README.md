# Rust adapter runner

The stable Rust crate is a set-agnostic adapter registry.

```console
./run --list
./run two-sum
cargo check
cargo test --no-run
```

Solutions live under `src/problems/`. The root `../run` command is preferred for set/index resolution and central attempt recording. LeetCode publishes no Rust template for cyclic-list and clone-graph APIs, so `src/types.rs` documents the local representations.
