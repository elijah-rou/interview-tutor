# Blind 75 local practice harness

A local, language-independent practice environment for the canonical Blind 75 set. Problems are ordered by current LeetCode difficulty (Easy, Medium, Hard), then alphabetically within each difficulty tier.

Every solution file already contains the judge-facing class or method signature and the supplied data structures. Edit only the selected solution. The checked-in tests use the public LeetCode contract and representative published examples. LeetCode's private hidden test corpus is not public and is therefore not copied here.

## Quick start

List the problems:

```console
./practice list
./practice list --difficulty easy
./practice list --topic graph
```

Solve and check a Python problem:

```console
cd python
$EDITOR blind75/problems/easy/two_sum.py
./run two-sum
```

Solve and check a Rust problem:

```console
cd rust
$EDITOR src/problems/two_sum.rs
./run two-sum
```

A run executes the complete local suite for that problem. Its exit status is zero only when every case passes. `./run all` evaluates the full set. You can also launch either runner from the project root:

```console
./practice test python two-sum
./practice test rust two-sum
```

No Python packages are required. Rust uses the stable toolchain and Cargo.

## Progress

Successful and failed practice runs are recorded in `.turso/progress.db`:

```console
./practice stats
./practice stats --language python
./practice stats --language rust
./practice db
```

`stats` reports overall completion and breakdowns by difficulty and DSA topic. A problem is complete after its current test revision passes. A later test revision does not silently inherit an older pass.

The database is a local SQLite/libSQL file and can be opened directly by Turso. To expose it through Turso's local development server:

```console
turso dev --db-file .turso/progress.db
```

The runners use the embedded file directly, so the server is optional. Override the location with `BLIND75_DATABASE_URL=file:/path/to/progress.db` or `BLIND75_DB_PATH=/path/to/progress.db`.

## Layout

```text
problem_sets/blind75/problems.json  authoritative ordered catalog
practice                            root progress and dispatch CLI
python/                             Python starter solutions and runner
rust/                               Rust starter solutions and runner
.turso/progress.db                  local progress, created on demand
```

The catalog stores stable slugs, display metadata, source links, and a `test_revision`. Language runners map each catalog slug to its language-specific adapter. This keeps the problem set independent of an execution framework and leaves room for new languages or sets.

To add a problem set, add a versioned catalog under `problem_sets/<set-id>/`, select it with `./practice --set <set-id> ...`, then provide the corresponding solution stubs and test adapters in each supported language. Do not infer the inventory by scanning solution files: the catalog is the source of truth.

## Interface policy

Method names, argument conventions, return values, in-place behavior, and supplied structures follow LeetCode. Python uses LeetCode's camelCase API; Rust uses its snake_case API. Premium problems use their conventional LeetCode/NeetCode-compatible interfaces. LeetCode does not publish Rust templates for a few cyclic or graph APIs, so those files document the local representation explicitly.

Tests intentionally validate outcomes rather than a particular implementation. Outputs with multiple valid orderings are normalized where the LeetCode contract permits it.

## Later: CLI hints

LLM hints are intentionally not wired into the first version. A future `hint` command can use GPT-5.6-Luna-xhigh behind an explicit API configuration, keep the current solution local by default, and record no completion attempt. The core harness and progress database do not depend on an LLM.
