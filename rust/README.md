# Rust runner

A dependency-free stable Rust crate.

```console
./run --list
./run two-sum
./run all
```

Edit the matching file under `src/problems/`. Each module contains the LeetCode-compatible interface and its contract assertions. The wrapper runs only the selected problem and records the result in the root Turso-compatible progress database.

The authoritative inventory and order are in `../problem_sets/blind75/problems.json`; `src/problems/mod.rs` adds Rust dispatch metadata. LeetCode publishes no Rust template for cyclic-list and clone-graph APIs, so `src/types.rs` documents the local `Rc<RefCell<_>>` representations. Premium problems use their conventional NeetCode/LintCode-compatible Rust interfaces.

Compile all starters without executing their intentionally failing cases:

```console
cargo check
cargo test --no-run
```
