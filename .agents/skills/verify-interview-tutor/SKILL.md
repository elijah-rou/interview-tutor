---
name: verify-interview-tutor
description: Verify interview-tutor (the Rust practice catalog CLI, the Python and Rust judge, progress tracking, catalog administration, and the terminal browser with its embedded Neovim solve mode) by building and driving it in a disposable, network-less Linux container. Use after changes to cli/, python/, rust/, catalog/, or problem_sets/, or to prove a fix in the real CLI or TUI. Never drive it on the host's progress database.
---

# Verify interview-tutor

Read [features/README.md](features/README.md) before driving, then the feature file you are proving. Run every command below from the repository root. The CLI is `.agents/skills/verify-interview-tutor/scripts/verify-tutor`; call it by that literal path, not through a shell variable.

## Repository interview

- **Surface:** the terminal. Primary: the `./practice` catalog/progress CLI and the `./run LANGUAGE ...` judge. Secondary: the `./interview` TUI (set and problem browser, embedded clean Neovim, `Space t` test, `Space s` submit). The Pi and Codex interviewers are optional and need credentials and network, so this skill leaves them out (see the Gotchas in `features/interview-tui.md`).
- **Run:** `./practice`, `./run`, and `./interview` build on demand through `cargo run --locked`. The CLI is Linux-only: `cli/src/source.rs` uses `SYS_openat2`, and the process modules use `O_PATH`. A macOS host fails to compile, so every run happens in a container. Requirements come from `README.md` and `docs/testing.md`: stable Rust, Python 3.12 or newer, GNU coreutils, and Neovim from 0.9 up to (not including) 2.0.
- **Seed and state:** the checked-in `catalog/problems.json` and `problem_sets/*.json` seed a SQLite database on first use. Each drive passes `--db /proof/<feature>/progress.db`, so it never touches `.turso/progress.db`. No credentials are needed.
- **Drive:** the repository's own harnesses are `cli/tests/pty_matrix.py` and `make test-pty`/`make test-race`, which use fake fixtures. User paths are driven directly: CLI commands are run and their stdout, stderr, and exit codes captured; the TUI runs in tmux inside the container. `make test-race` is offered as the `race` drive.
- **Observe:** command output and exit codes, `attempts` rows read back from the SQLite file, solution files on disk, and tmux screen captures.
- **Isolate:** one container per run, labelled `interview-tutor.verify.run=<RUN_ID>`, with `--network none`, no published ports, and the checkout mounted read-only at `/src`. The product is copied to `/work` inside the container. Runs can coexist. The CLI refuses any container whose label does not match the run ID.
- **Recipe:** `create-verification/references/cli-tui.md`, plus the systems notes at the end of this file.

## Launch

Needs a running Docker daemon. The first launch builds the image `interview-tutor-verify:<hash>` from `scripts/Dockerfile` (Ubuntu 24.04, Rust 1.98.0 through rustup, Python 3.12, Neovim 0.9.5, tmux), with the locked crates prefetched. That build is the only step that uses the network. The hash covers the Dockerfile and the four Cargo manifests and lockfiles, so a dependency change produces a new image. To build the image on its own:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor image
```

Start a run:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor launch
```

It prints `RUN_ID=it-<date>-<time>-<pid>` and `EVIDENCE_DIR=...`. Pass that `RUN_ID` to every later command. Inside the container, `inside.sh serve` copies `/src` to `/work` (skipping `.git`, `.turso`, `.agents`, and build output) and builds `practice`, `interview-tutor`, and `local-judge-rust` offline. It then prints the readiness line `READY interview-tutor verifier`. Launch waits up to 600 seconds for that line; the build normally takes about 25 seconds. If launch fails, run Cleanup with the printed `RUN_ID` and start again. Teardown is always Cleanup.

## Doctor

Read-only. It answers whether this run's container is worth driving:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor doctor RUN_ID
```

It checks, from the host:

- the label matches the run, and the container is running;
- the network is `none`, with no published ports and only `/src` mounted, read-only;
- the image ID matches the one launched;
- the checkout is still at the launched revision.

Inside the container, it checks:

- the three binaries exist and match the hashes recorded at build time;
- Python is 3.12 or newer, Neovim is in range, and tmux and `timeout` are present;
- `/src` is not writable and `/work` has no `.turso`;
- the catalog and both `two-sum` solution files in `/work` match `/src`.

The last line must be `DOCTOR PASS ...`, and the full result goes to `EVIDENCE_DIR/doctor.txt`. Run Doctor before the first drive and again after any failed drive.

## Drive

One mapped feature per command. Each feature runs once per run; to repeat one, launch a new run.

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID catalog
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID judge
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID progress
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID admin
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID tui
```

Each command prints `DRIVE PASS <feature> (<n> checks)` or `DRIVE FAIL <feature> exit <status>` with the failing check. The exact user commands, expected results, and entry points for each feature are in `features/`. Each drive starts and ends by checking that the solution, catalog, and set files in `/work` match `/src`. Drives that rewrite `two-sum` restore it, even when they fail.

To run one user command by hand in the same container (for example, while you diagnose a failed check), read the container ID from `EVIDENCE_DIR/container.id` and use a scratch database:

```sh
docker exec -w /work CONTAINER_ID ./practice --db /tmp/manual.db sets list
```

The repository's deterministic cancellation gate is the `race` drive. It runs `make test-race`: 20 PTY cases (runner cancellation and fake-Codex interrupts) and then 10 fail-fast iterations of the two Rust race tests. It takes about 3 minutes and builds into `/work/cli/target`:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor drive RUN_ID race
```

For an unattended full pass, `all` runs launch, doctor, the five mapped features (not `race`), and cleanup, even after a failure. It exits 0 only if every feature passed:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor all
```

## Evidence

Each run has one evidence directory, `.agents/skills/verify-interview-tutor/evidence/RUN_ID/`. The skill's `.gitignore` ignores it, and Cleanup never removes it. It holds:

- `revision.txt`, `product-status.txt` (the product files' `git status`), `source.sha256`, `image.tag`, `image.id`, `image-build.log`;
- `launch/` with the toolchain versions, the build log, and the binary hashes, plus `readiness.stdout` and `doctor.txt`;
- `<feature>/drive.command`, `drive.stdout`, `drive.stderr`, `drive.exit`, and `<feature>/artifacts/`. The artifacts hold, for each step, `NN-step.cmd`, `.out`, `.err`, and `.exit`; each `NN-label.screen.txt` (TUI); the solution variants used; the feature's `progress.db`; and `summary.txt` (feature ID, entry points, and every passing check);
- `final-proof/` (all of `/proof` exported at cleanup), `container.log`, `processes-before-cleanup.txt`, `cleanup.stdout`, `remaining-run-containers.txt`, and `surviving-evidence.txt`.

Proof standards:

- Drive the real user commands (`./practice`, `./run`, `./interview`) from the copied checkout. Do not use internal functions, `practice _record`, or direct database writes as proof. The database is only read, never written, to check side effects.
- Capture the action and the resulting state. A judge run is proven by its exit code, its relayed output, and the `attempts` row it recorded. A catalog mutation is proven by a second read-only command. A TUI action is proven by a screen capture, plus the saved file and the attempt rows.
- The judge has no dry run. `Space t` is the closest thing, and the TUI drive checks it: zero attempt rows after two `Space t` runs, then exactly one after `Space s`.
- No mocks are used in the five features. The `race` drive uses the repository's own fake Codex app-server (`cli/tests/fixtures/fake_codex_app_server.py`), which stands in for an external process boundary.
- An entry point a feature file lists but that did not run is reported as skipped, with the reason. It is never covered by a different entry point.

## Cleanup

Run Cleanup even after failures:

```sh
.agents/skills/verify-interview-tutor/scripts/verify-tutor cleanup RUN_ID
```

It checks the run label, exports `/proof` to `final-proof/`, and saves `docker top` and `docker logs`. Then it runs `docker stop` on the recorded container ID; the container was started with `--rm`, so stopping removes it along with its scratch checkout, databases, tmux server, and build output. It waits until the ID is gone and checks that no container carries the run's label. Finally, it writes `surviving-evidence.txt` and fails if that list is empty. It never stops or removes anything by name or image, and never deletes evidence. The image stays for reuse; remove old `interview-tutor-verify:*` tags yourself when you no longer need them. The output must end with `CLEANUP PASS container <id> removed; <n> evidence files kept in <dir>`.

## Helpers

- `scripts/verify-tutor`: the host CLI (Bash 3.2 compatible) with the subcommands `image`, `launch`, `doctor`, `drive`, `cleanup`, and `all`. `verify-tutor --help` lists them.
- `scripts/inside.sh`: runs only inside the container (it refuses unless `INTERVIEW_TUTOR_VERIFY_CONTAINER=1`). `verify-tutor` invokes it as `bash /src/.agents/skills/verify-interview-tutor/scripts/inside.sh <serve|doctor|catalog|judge|progress|admin|tui|race>`.
- `scripts/Dockerfile`: the image recipe. `verify-tutor image` streams it with the Cargo manifests as the build context.

Check the feature map after editing it:

```sh
agentic feature-map-lint .agents/skills/verify-interview-tutor/features
```

## Systems notes

- Runs are deterministic: the network is `none`, the crates are locked, the toolchain is pinned, each feature gets a fresh database, the inputs are fixed, the solution files are restored, and the terminal is a fixed 120x40 `xterm-256color`. The product has no seeded randomness; durations and timestamps in output and attempt rows vary and are never asserted.
- Changes to the runner, signals, cancellation, or the TUI worker threads need the `race` drive as well as the features. The core data structures (database schema, catalog identity, attempt recording) and the concurrency model (runner process groups, cancellation, worker channels) stay with the user: show them the evidence and get their decision before changing either.
- Use `maintain-verification` when commands, keys, screen text, or catalog contents change.
