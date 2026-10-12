# Attempt test output plan

Status: shape approved 2026-10-12; building.

## Request

Store every practice attempt with its full test output (each test case's stdout, stderr, and result) and show that output in the attempt history.

## The one-way doors

1. **Persisted data.** Schema v3 adds per-case output rows to `.turso/progress.db`. An older binary refuses a v3 file.
2. **Judge contract.** Python and Rust judges gain a per-case invocation protocol that the CLI depends on.

Reversible work around them: the renderer, CLI command names, TUI keys and screen layout, and the per-stream byte cap value.

## Facts the shape rests on

- `attempts` stores only outcome, duration, exit code, revision, and context (`cli/src/database.rs:102`). Output is printed or shown in the Output pane, then dropped.
- Judges have no test cases. Python runs one `SIMPLE_CASES` tuple or one monolithic `CUSTOM_TESTS` function per problem and stops at the first exception (`python/local_judge/cli.py:35`). Rust runs one `run_case()` per problem (`rust/src/problems/*.rs`). The only result signal is the exit code.
- The runner merges stdout and stderr into one tagged, sanitized, 256 KiB display string (`cli/src/runner.rs:143`, `:478`). Separate streams exist only for `--list` discovery.
- Three paths record attempts: `practice run`/`test` (`runner::record_execution`), TUI submit (runner worker, then the signal cutoff `finalize_attempt_cancelled`), and the direct `python/run <slug>` / `rust/run <slug>` launchers, which print to the terminal and call hidden `practice _record` with no output.
- No attempt history view exists in the CLI or the TUI.

## Shape

### A test case is a named unit the judge runs in its own process

The judge registry is the single source of truth for a problem's cases. Each case runs in a fresh process, so output attribution comes from pipe boundaries and needs no parsing of user output. The case result is that process's termination under today's mapping: exit 0 pass, 2 error, other exit fail, cancel cancelled, timeout or signal error.

Judge protocol (argv and exit codes only):

| Invocation | Contract |
|---|---|
| `RUNNER --list` | unchanged |
| `RUNNER --cases SLUG` | Setup step. Reads only the registry; never imports or runs the solution in Python. Rust builds the judge here, so compiler diagnostics land in setup stderr. stdout is exactly 1..=15 case names in execution order, one per line, matching `[a-z0-9][a-z0-9-]{0,63}`. |
| `RUNNER --problem SLUG --case NAME` | Runs exactly one case. stdout and stderr are free-form. The exit code is the result. |
| `RUNNER SLUG` | Human launcher. Delegates to `practice run <lang> [<set>] SLUG`; with `PRACTICE_NO_RECORD=1` it runs all cases in-process to the terminal. |

Day one migration is mechanical: each `SIMPLE_CASES` tuple becomes case `example-1`, each `CUSTOM_TESTS` function becomes `scenario-1`, and each Rust `run_case` becomes `scenario-1`. **History will show one case per problem until test content is split into more cases.** Wrapping does not change `test_revision`. Adding assertions does.

### Execution

`runner::execute_attempt` sets one deadline (today's 30 s wall timeout across setup and all cases), runs setup, parses its stdout strictly, then runs every case in order. It does not fail fast: every case runs so history shows every failure. On cancellation or deadline, the remaining cases become `NotRun`. Each step reuses today's process-group, reader, TERM/KILL, and drain machinery, with separate stdout and stderr retention. At most one child is alive at a time. Live progress events still stream to the TUI Output pane during a run.

A setup protocol violation (truncated, malformed, duplicate, empty, or more than 15 names) is a preflight error and records nothing, matching today's rule for preflight failures.

### Domain types (`cli/src/attempt.rs`, pure)

```rust
pub const MAX_CASES: usize = 15;
pub const MAX_STORED_STREAM_BYTES: usize = 8 * 1024; // head 4 KiB + tail 4 KiB once truncated

pub struct CaseName(String);                   // validated at the protocol boundary

pub enum CapturedText {                        // sanitized UTF-8
    Complete(String),
    Truncated { head: String, omitted_bytes: NonZeroU64, tail: String },
}

pub struct StepRun {
    pub termination: Termination,              // moved here from runner
    pub duration_ms: u32,
    pub stdout: CapturedText,
    pub stderr: CapturedText,
}

pub enum CaseRun { Ran(StepRun), NotRun(NotRunReason) }
pub enum NotRunReason { Cancelled, DeadlineExceeded }

pub struct TestOutput { setup: StepRun, cases: Vec<(CaseName, CaseRun)> }
impl TestOutput {
    /// <= MAX_CASES, unique names, failed setup implies no cases, NotRun entries are a suffix.
    pub fn new(setup: StepRun, cases: Vec<(CaseName, CaseRun)>) -> Result<Self, String> { todo!() }
    pub fn outcome(&self) -> AttemptOutcome { todo!() } // cancelled > error > fail > pass
}

pub struct AttemptReport { pub output: TestOutput, pub duration_ms: i64, cancelled: bool }
impl AttemptReport {
    pub fn outcome(&self) -> AttemptOutcome { todo!() }
    pub fn status_code(&self) -> i32 { todo!() }      // first non-passing step's code, 130 if cancelled
    pub fn mark_cancelled(&mut self) { todo!() }
}

pub enum RecordedOutput { PredatesCapture, Captured(TestOutput) }

pub fn render_report(report: &AttemptReport) -> String { todo!() }   // practice run, TUI Output
pub fn render_record(record: &AttemptRecord) -> String { todo!() }   // attempts show, TUI Attempt
```

`Termination` and `AttemptOutcome` move into `attempt.rs`. Their callers are updated, with no re-exports.

### Storage: schema v3 adds `attempt_steps`; `attempts` is unchanged

```sql
CREATE TABLE IF NOT EXISTS attempt_steps (
    attempt_id INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 15),   -- 0 = setup
    case_name TEXT,
    termination TEXT NOT NULL CHECK (termination IN
        ('exited','signalled','timed_out','cancelled','not_run_cancelled','not_run_deadline')),
    status INTEGER,                                              -- exit code or signal number
    duration_ms INTEGER CHECK (duration_ms >= 0),
    stdout_omitted_bytes INTEGER NOT NULL CHECK (stdout_omitted_bytes >= 0),
    stderr_omitted_bytes INTEGER NOT NULL CHECK (stderr_omitted_bytes >= 0),
    stdout_head TEXT NOT NULL, stdout_tail TEXT NOT NULL,
    stderr_head TEXT NOT NULL, stderr_tail TEXT NOT NULL,
    PRIMARY KEY (attempt_id, ordinal),
    UNIQUE (attempt_id, case_name),
    CHECK ((ordinal = 0) = (case_name IS NULL)),
    CHECK ((termination IN ('exited','signalled')) = (status IS NOT NULL)),
    CHECK ((termination LIKE 'not_run%') = (duration_ms IS NULL)),
    CHECK (stdout_omitted_bytes > 0 OR stdout_tail = ''),
    CHECK (stderr_omitted_bytes > 0 OR stderr_tail = ''),
    CHECK (length(CAST(stdout_head AS BLOB)) + length(CAST(stdout_tail AS BLOB)) <= 8192),
    CHECK (length(CAST(stderr_head AS BLOB)) + length(CAST(stderr_tail AS BLOB)) <= 8192)
) STRICT;
CREATE INDEX IF NOT EXISTS attempts_problem_language_id ON attempts(problem_id, language_id, id);
```

- The per-case result is derived from `(termination, status)` by one function and is not stored twice. `attempts.result` stays the authority for completion queries, the signal cutoff, and pre-v3 rows. It is computed by the fold inside the same transaction as the steps.
- An attempt with zero step rows reads as `PredatesCapture`. Every v3 recording writes at least the setup row in the same transaction, so absence is unambiguous.
- Truncation is a column pair plus an exact omitted-byte count, not a marker string inside the text.
- Row codecs stay private to `database.rs`. Reads re-validate through `TestOutput::new`. A corrupt row is a loud error.

**Migration v2 to v3** is additive: one transaction counts `attempts`, creates the table and index, checks the count is unchanged and `attempt_steps` is empty, and sets `user_version = 3`. v0 and v1 chains already end on `SCHEMA_VERSION`. A crash rolls back, and the next open retries.

### Recording

- `practice run`/`test` and TUI submit call `database::record_attempt(connection, &plan, &report)`. One transaction inserts the attempt row and 1..=16 step rows; outcome, exit code, and duration come from the report. `ExecutionPlan` carries the problem id and `test_revision` captured at planning, so a catalog reconciliation mid-run cannot relabel the evidence.
- `finalize_attempt_cancelled` is unchanged and touches only `attempts`. Step rows keep describing what ran, so history can show "cancelled after completion".
- `python/run <slug>` and `rust/run <slug>` delegate to `practice run`, so every recorded attempt goes through the one capturing path. **Hidden `practice _record` is deleted.** Its test callers (`tests/test_admin.py`, `tests/test_generalized_cli.py`, `cli/tests/cli_parse_contracts.rs`) move to a SQL fixture helper or to `practice run` against fixture runners.
- Single writer per attempt: the CLI process or the TUI runner worker that inserted it, which also finalizes it. Readers see only committed snapshots.

### History surfaces

CLI, nested like `sets` and `problems`:

```console
$ ./practice attempts list two-sum --language python [--limit N] [--before ID]
ID  RUN AT                LANGUAGE  RESULT  CASES  MS   REVISION
42  2026-10-12T20:46:03Z  python    fail    0/1    190  current
17  2026-09-01T10:00:00Z  python    pass    -      90   current
(older: --before 17)
$ ./practice attempts show 42
$ ./practice attempts show 42 --case example-1 --stream stderr   # raw stored text, pipeable
$ ./practice attempts show 17
Attempt 17 · two-sum · python · pass · 2026-09-01T10:00:00Z · 90 ms · revision 1 (current)
output: not recorded (attempt predates output capture)
```

TUI: two browser screens, `History` and `Attempt`. `a` on Problem detail opens History for the selected problem and language. Up/Down move, PageDown/PageUp page with keyset cursors, Enter opens Attempt (one scrollable render of setup and every case), Esc goes back, and `l` cycles language. Loads run on the main-thread connection through `Effect::LoadHistory` / `Effect::LoadAttempt`, with stale operation ids ignored. Solve gets no new pane.

### Bounds

| Item | Bound |
|---|---|
| Cases per problem | 15, enforced at parse, in `TestOutput::new`, by SQL CHECK, and by a judge registry test |
| Steps per attempt | 16, sequential, at most one live child |
| Stored text per stream per step | 8 KiB (4 KiB head + 4 KiB tail when truncated) plus exact omitted bytes |
| Stored text per attempt | at most 256 KiB, equal to today's display cap, so a full render fits the TUI Output bound |
| CLI `attempts list --limit` | 1..=1000, default 20 |
| TUI history page | 100 rows |

## Synthesis

Two candidates were generated independently: A (Claude Opus 5.5) and B (Codex GPT-6.1-Sol). They converged on process-per-case execution, a child table with head/tail and omitted counts, an explicit "predates capture" state, launcher delegation to `practice run`, and history in the browser rather than Solve. Base: **A**, for one child table, an additive migration, one recording path, and an attempt that fits today's display cap.

Grafted from B: setup never imports the Python solution, which replaces A's fd `dup2` trick; `ExecutionPlan` freezes problem id and `test_revision` at planning; and `a` as the History key, because `h` is already Hint in Solve.

Rejected from B: per-recording random idempotency keys and immutable `recorded_result` columns (no retry path exists today); an `output_kind` enum plus a separate setup-output table (one table with ordinal 0 covers it); exit-2 fallback negotiation for legacy adapters (both adapters are in-repo and move in lockstep); JSON manifests with labels; raw-byte and drain-completion counters; and a 64-case, 2 MiB bound. A's case-name re-exports were also dropped in favor of moving callers.

## Tradeoffs accepted

- We accept one process per case (about 30-60 ms in Python and a cargo freshness check in Rust, unmeasured) in exchange for exact output attribution that user code cannot forge or corrupt.
- We accept losing cross-stream interleaving in stored output in exchange for separate per-stream text. Live interleaving remains in the Output pane during a run.
- We accept an 8 KiB per-stream stored cap in exchange for keeping every attempt within the existing 256 KiB display bound.
- We accept that program output becomes durable in the gitignored database in exchange for history. It was memory-only before.
- We accept running every case on failure in exchange for showing every failure.

## Alternatives rejected

- One judge process emitting a per-case report on a side channel: a crash in one case loses every case, user code inherits and can forge the channel, and each judge needs fd-level capture.
- Output columns on `attempts` with no cases: smallest change, but it cannot represent "each test case" and would need a second migration.
- Keeping `_record` with a "no output" marker: a permanent second writer that contradicts "every attempt keeps its output".

## Approved decisions (2026-10-12)

1. Build the judge protocol and process-per-case execution now.
2. Store at most 8 KiB per stream per case, so a whole attempt stays within 256 KiB.
3. Delete hidden `practice _record`.
4. Leave the database file mode unchanged and document that output is now persisted.

## First implementation step

A thin CLI-only slice: `attempt.rs` types with invariant and fold tests; `execute_step`/`execute_attempt` against an extended `runner_fixture.sh` implementing `--cases`; schema v3 with a v2 fixture migration test; `record_attempt`; and `practice attempts show`. It is verified by `./practice run python two-sum` in the verify-interview-tutor container recording an attempt whose stdout, stderr, and result `practice attempts show` prints.
