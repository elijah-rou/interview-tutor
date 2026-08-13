# Interview Tutor

Interview Tutor is a Linux-first, local algorithm practice catalog and judge. It combines a Rust catalog/progress CLI, a terminal problem browser and native solve editor, Python and Rust adapters, and an optional Codex interviewer. Blind 75 is the first shipped set; problems, solutions, attempts, and completion have one global identity even when a problem belongs to several sets.

## Requirements and build

Required on Linux:

- a current stable Rust toolchain with Cargo, rustfmt, and Clippy
- Python 3.12 or newer
- GNU Make and GNU coreutils (`timeout` and `readlink -f`)
- a UTF-8, `xterm-256color`-compatible terminal for the TUI

SQLite is bundled into the Rust CLI. Turso and Codex are optional.

```console
git clone https://github.com/elijah-rou/interview-tutor.git
cd interview-tutor
rustup component add rustfmt clippy
cargo build --manifest-path cli/Cargo.toml --bins --locked
cargo build --manifest-path rust/Cargo.toml --locked
./practice --help
./interview --help
```

The `./practice`, `./run`, and `./interview` launchers also build their Rust binary on demand.

## First run and configuration

`./practice` and `./interview` create `.turso/progress.db` on first use, migrate an older supported schema, and reconcile the checked-in global catalog and ordered problem sets. The database stores local custom catalog changes, attempts, and progress; the checked-in files remain the source for shipped metadata. Inspect the selected path with `./practice db`. A Turso server is not required, but `turso dev --db-file .turso/progress.db` can expose the same local file.

Database precedence is:

1. `--db PATH`
2. `PRACTICE_DATABASE_URL`
3. `PRACTICE_DB_PATH`
4. legacy `BLIND75_DATABASE_URL` and `BLIND75_DB_PATH`
5. `.turso/progress.db`

Relative paths are resolved from the repository root; `file:` URLs and `~/` are accepted. TUI startup flags take precedence over defaults: `--set ID` opens that set instead of the set menu, `--language ID` selects an enabled language instead of Python (or the first enabled language), and `--no-codex` prevents any Codex version probe or process spawn. The `./run` launcher also accepts `--db`.

## Practice flow

Browse a set, inspect progress, open a problem, then solve it in the TUI:

```console
./practice sets list
./practice --set blind75 list
./practice --set blind75 show 16
./practice --set blind75 stats --language python
./practice stats --global --language rust
./interview --set blind75 --language python --no-codex
```

In `./interview`, choose a set and problem with `j`/`k` and Enter, then press Enter from problem detail to open the planned source. F5 or Ctrl-S atomically saves and tests without recording progress. F9 or `:submit` saves, tests, and records exactly one attempt after execution terminates. See [the TUI guide](docs/interview-tui.md) for all keys, responsive layouts, stale/error states, and guarded exit behavior.

Run a problem directly by global slug, or by set plus slug/1-based index:

```console
./run python two-sum
./run rust blind75 16
```

The two-argument form is always `LANGUAGE GLOBAL_SLUG`. The three-argument form is always `LANGUAGE SET SLUG_OR_INDEX`. A zero exit status means the local suite passed. Direct runs record one attempt after execution; spawn/preflight failures do not.

Completion belongs to a global problem and language at the problem's current test revision. Passing a shared problem counts in every set containing it. Set indexes are selectors only; attempt history retains stable problem identity.

## Catalog administration

Problem metadata is independent of ordered set membership:

```console
./practice problems add custom-pair-sum \
  --title "Custom Pair Sum" --difficulty Easy --topic "Arrays & Hashing" \
  --statement-file ./custom-pair-sum.md
./practice problems adapter custom-pair-sum python python/problems/easy/custom_pair_sum.py
./practice sets create favorites --name "Favorites"
./practice sets add favorites two-sum
./practice sets add favorites custom-pair-sum --index 1
./practice --set favorites list
```

`problems` and `sets` also provide list, show, update, move/remove, and guarded delete operations. A custom metadata-only problem is valid but cannot run until a language adapter and local test dispatch exist. Shipped resources are read-only through local CRUD; custom resources remain editable.

## Optional Codex interviewer

Codex is opt-in after an in-app disclosure. Install a trusted Codex CLI, authenticate it with `codex login`, verify with `codex login status`, and run `./interview`. This stack accepts exactly Codex CLI 0.146.0 and 0.147.0. `INTERVIEW_TUTOR_CODEX_EXECUTABLE` can select another trusted executable path; version compatibility is not executable provenance.

Interview Tutor does not read API keys or login tokens. After consent, the configured Codex process sends the disclosed statement, source, bounded latest test output, bounded memory-only transcript, and question to OpenAI. Its read-only tools, MCP servers, and configuration may access other readable local paths; this is not total process isolation. Use a dedicated Codex profile/home with minimal configuration when that boundary is too broad. Local editing, tests, and submission remain available with `--no-codex` or when Codex is declined, unauthenticated, offline, interrupted, or incompatible. See [Codex compatibility and privacy](docs/codex-compatibility.md).

## Repository and contracts

```text
catalog/problems.json       shipped global metadata, statements, and adapter paths
problem_sets/*.json         ordered references to global problem slugs
cli/                        Rust CLI, TUI, runner, database, and Codex boundary
python/                     Python starters, adapters, and representative cases
rust/                       Rust starters, adapters, and representative cases
.turso/progress.db          local runtime database (created on first use)
```

Starter APIs follow LeetCode where an official public template exists; otherwise they use the documented conventional local representation. Local tests are representative public contracts, not LeetCode's private hidden corpus. Starter solutions intentionally remain incomplete and can fail the full language suite.

Shipped statement briefs remain in `catalog/problems.json`. They were independently written from checked-in interfaces, data structures, and public executable cases; executable cases are authoritative if a brief conflicts. [Catalog provenance](catalog/README.md) and the [original Blind 75 local brief](problem_sets/blind75.md) remain checked in and visible.

See [architecture](docs/architecture.md), [testing](docs/testing.md), and [security](SECURITY.md) for implementation boundaries and verification gates.

## License

Interview Tutor is available under the [MIT License](LICENSE). Copyright © 2026 Elijah Roussos.
