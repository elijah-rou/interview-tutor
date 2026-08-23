# Interview Tutor

Interview Tutor is a Linux-first, local algorithm practice catalog and judge. It combines a Rust catalog/progress CLI, a terminal problem browser and native solve editor, Python and Rust adapters, and an optional backend-neutral interviewer with Pi as the default transport and Codex as an explicit compatibility option. The 82-problem global catalog ships the Blind 75, Convex, Core, anti-metal, and Depot sets; problems, solutions, attempts, and completion have one global identity even when a problem belongs to several sets.

## Requirements and build

Required on Linux:

- a current stable Rust toolchain with Cargo, rustfmt, and Clippy
- Python 3.12 or newer
- GNU Make and GNU coreutils (`timeout` and `readlink -f`)
- a UTF-8, `xterm-256color`-compatible terminal for the TUI

SQLite is bundled into the Rust CLI. Neovim 0.9 through 1.x is required for the embedded interview editor; Turso and the Pi/Codex interviewer executables are optional. Pi integration accepts exactly Pi 0.84.2.

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

Relative paths are resolved from the repository root; `file:` URLs and `~/` are accepted. TUI startup flags take precedence over defaults: `--set ID` opens that set, `--language ID` selects an enabled language, and `--neovim PATH` selects the required clean embedded Neovim executable. `--interviewer pi|codex|none` selects one interviewer transport. Resolution is CLI, legacy `--no-codex` as `none`, `INTERVIEW_TUTOR_INTERVIEWER`, then default `pi`; conflicts, empty values, and invalid values fail without fallback. `INTERVIEW_TUTOR_PI_EXECUTABLE`, `INTERVIEW_TUTOR_CODEX_EXECUTABLE`, and `INTERVIEW_TUTOR_NEOVIM_EXECUTABLE` select trusted executables. `none` and `--no-codex` probe or spawn neither interviewer. The `./run` launcher also accepts `--db`.

## Practice flow

Browse a set, inspect progress, open a problem, then solve it in the TUI:

```console
./practice sets list
./practice --set convex list
./practice --set anti-metal show 1
./practice --set depot list
./practice --set blind75 show 16
./practice --set blind75 stats --language python
./practice stats --global --language rust
./interview --set blind75 --language python --no-codex
```

In `./interview`, choose a set and problem with `j`/`k` and Enter, then press Enter from problem detail to open the planned source in required clean embedded Neovim. `Space t` atomically saves and tests without recording progress; `Space s` saves, tests, and records exactly one attempt after execution terminates. `Space b` or `:TutorBack` atomically autosaves a dirty draft without testing or recording and returns to the selected set's problem list. `Space c` toggles the focused Problem, Output, or Interview accessory, `Space ?` opens Solve help, and Ctrl-Q provides a guarded quit from every solve context. F5/F9 remain test/submit compatibility aliases. See [the TUI guide](docs/interview-tui.md) for the full Neovim surface, integration commands, responsive layouts, stale/error states, and guarded exit behavior.

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

## Optional interviewer

`./interview` defaults to Pi after an in-app disclosure. Install trusted Pi 0.84.2 and configure its selected model provider. Each accepted application turn uses a fresh no-session RPC process with tools/bash, extensions, skills, prompt templates, themes, context files, approvals, telemetry, update checks, and startup network operations disabled. Pi reads its own bounded configuration/auth inputs and may contact the selected model provider for the disclosed turn. Pi `auth.json` credentials beginning with `!command` execute that command through a shell, per Pi's credential resolution contract.

Codex remains available only through `--interviewer codex` or `INTERVIEW_TUTOR_INTERVIEWER=codex`. Install a trusted Codex CLI and authenticate with `codex login`; versions 0.146.0 and 0.147.0 remain accepted exactly. There is no silent Pi/Codex fallback.

After consent, the selected process receives the statement, source, bounded latest test output, bounded memory-only transcript, and question. Interview Tutor writes no transcript log. Local editing, tests, submission, and navigation remain available in `none` mode or when the selected interviewer is declined, unauthenticated, offline, interrupted, or incompatible. Executable version checks establish compatibility, not provenance. See [interviewer compatibility and privacy](docs/codex-compatibility.md).

## Repository and contracts

```text
catalog/problems.json       shipped global metadata, statements, and adapter paths
problem_sets/*.json         ordered references to global problem slugs
cli/                        Rust CLI, TUI, runner, database, and interviewer transports
python/                     Python starters, adapters, and representative cases
rust/                       Rust starters, adapters, and representative cases
.turso/progress.db          local runtime database (created on first use)
```

Starter APIs follow LeetCode where an official public template exists; otherwise they use the documented conventional local representation. Local tests are representative public contracts, not LeetCode's private hidden corpus. Starter solutions intentionally remain incomplete and can fail the full language suite.

Shipped statement briefs remain in `catalog/problems.json`. They were independently written from checked-in interfaces, data structures, and public executable cases; executable cases are authoritative if a brief conflicts. [Catalog provenance](catalog/README.md), the [problem-set index](problem_sets/README.md), the [anti-metal selection rationales](problem_sets/anti-metal.md), and the [original Blind 75 local brief](problem_sets/blind75.md) remain checked in and visible.

See [architecture](docs/architecture.md), [testing](docs/testing.md), and [security](SECURITY.md) for implementation boundaries and verification gates.

## License

Interview Tutor is available under the [MIT License](LICENSE). Copyright © 2026 Elijah Roussos.
