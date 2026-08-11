#!/usr/bin/env python3
"""Bounded, serial Linux PTY acceptance matrix for Interview Tutor."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import sys
import tempfile
import time
from typing import Callable

from pty_harness import CleanupRegistry, PtySession, hard_deadline, run_checked
import tui_runtime_cleanup

ENTER = b"\r"
ESCAPE = b"\x1b"
F5 = b"\x1b[15~"
F9 = b"\x1b[20~"
SHIFT_TAB = b"\x1b[Z"
CTRL_C = b"\x03"


class MatrixFixture:
    def __init__(
        self,
        temporary: Path,
        interview_binary: Path,
        practice_binary: Path,
        repository_root: Path,
    ) -> None:
        self.temporary = temporary
        self.interview_binary = interview_binary
        self.practice_binary = practice_binary
        self.repository_root = repository_root
        self.root = temporary / "root"
        shutil.copytree(repository_root / "catalog", self.root / "catalog")
        shutil.copytree(repository_root / "problem_sets", self.root / "problem_sets")
        shutil.copytree(repository_root / "python", self.root / "python")
        shutil.copytree(repository_root / "rust", self.root / "rust")
        self.solution = self.root / "python" / "smoke.py"
        self.solution.write_text("print('initial')\n", encoding="utf-8")
        self.runner_mode = self.root / "python" / "runner-mode"
        self.runner = self.root / "python" / "run"
        self.runner.write_text(self._runner_source(), encoding="utf-8")
        self.runner.chmod(0o700)
        self.template_database = temporary / "template.db"
        self.cleanup = CleanupRegistry()
        self.case_sessions: list[PtySession] = []
        self.base_environment = os.environ.copy()
        self.base_environment.pop("OPENAI_API_KEY", None)
        self.base_environment["PRACTICE_ROOT"] = str(self.root)
        self.base_environment["TERM"] = "xterm-256color"
        self.fake_codex = (
            repository_root / "cli" / "tests" / "fixtures" / "fake_codex_app_server.py"
        )
        self._prepare_database()
        self.database_index = 0
        self.codex_index = 0

    @staticmethod
    def _runner_source() -> str:
        return r"""#!/usr/bin/env python3
import os
from pathlib import Path
import signal
import sys
import time

root = Path(__file__).resolve().parent
if sys.argv[1:] == ["--list"]:
    print("smoke-problem")
    raise SystemExit(0)
mode = (root / "runner-mode").read_text(encoding="utf-8").strip()
pid_file = os.environ.get("PRACTICE_FIXTURE_PID_FILE")
if mode == "normal":
    for index in range(60):
        print(f"output-line-{index:02d}")
    print("PASS")
elif mode == "fail":
    print("ordinary-failure")
    raise SystemExit(7)
elif mode == "timeout":
    time.sleep(30)
elif mode == "cancel":
    if pid_file:
        Path(pid_file).write_text(str(os.getpid()), encoding="utf-8")
    time.sleep(30)
elif mode == "record-lock":
    if pid_file:
        Path(pid_file).write_text(str(os.getpid()), encoding="utf-8")
    marker = os.environ.get("PRACTICE_RECORD_READY_FILE")
    if marker:
        Path(marker).write_text("runner-finished\n", encoding="utf-8")
    print("record-lock-runner-finished")
elif mode == "flood":
    sys.stdout.buffer.write(b"safe-start\n")
    sys.stdout.buffer.write(b"\033]0;hostile-title\007")
    sys.stdout.buffer.write(b"split-euro:\342")
    sys.stdout.buffer.flush()
    sys.stdout.buffer.write(b"\202\254\n")
    for index in range(6000):
        sys.stdout.write(f"flood-{index:04d}:" + "x" * 400 + "\n")
    sys.stdout.write("safe-tail\n")
elif mode in {"group-descendant", "escaped-descendant"}:
    child = os.fork()
    if child == 0:
        if mode == "escaped-descendant":
            os.setsid()
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        while True:
            time.sleep(60)
    if pid_file:
        Path(pid_file).write_text(str(child), encoding="utf-8")
    print(f"fixture-child:{child}")
else:
    print(f"unknown runner mode: {mode}", file=sys.stderr)
    raise SystemExit(2)
"""

    def _practice(self, arguments: list[str]) -> None:
        run_checked(
            [
                str(self.practice_binary),
                "--db",
                str(self.template_database),
                *arguments,
            ],
            self.base_environment,
        )

    def _prepare_database(self) -> None:
        statement = "\n".join(
            ["## Matrix statement", ""]
            + [
                f"detail-line-{index:02d} bounded scrolling content"
                for index in range(55)
            ]
            + ["DETAIL-END-SENTINEL"]
        )
        self._practice(
            [
                "problems",
                "add",
                "smoke-problem",
                "--title",
                "Smoke Problem",
                "--difficulty",
                "Easy",
                "--topic",
                "Matrix",
                "--statement",
                statement,
            ]
        )
        self._practice(
            ["problems", "adapter", "smoke-problem", "python", "python/smoke.py"]
        )
        self._practice(["sets", "create", "a00-matrix", "--name", "A00 Matrix Set"])
        self._practice(["sets", "add", "a00-matrix", "smoke-problem"])

        with sqlite3.connect(self.template_database) as connection:
            timestamp = "2026-01-01T00:00:00Z"
            for index in range(1, 46):
                connection.execute(
                    "INSERT INTO problem_sets(slug, name, description, managed, created_at, updated_at) "
                    "VALUES (?, ?, ?, 0, ?, ?)",
                    (
                        f"matrix-set-{index:02d}",
                        f"Matrix Set {index:02d}",
                        "scroll fixture",
                        timestamp,
                        timestamp,
                    ),
                )
            set_id = connection.execute(
                "SELECT id FROM problem_sets WHERE slug = 'a00-matrix'"
            ).fetchone()[0]
            problems = connection.execute(
                "SELECT id, title FROM problems WHERE slug != 'smoke-problem' AND archived = 0 "
                "ORDER BY slug LIMIT 40"
            ).fetchall()
            assert len(problems) == 40
            self.problem_titles = [title for _, title in problems]
            for ordinal, (problem_id, _) in enumerate(problems, start=2):
                connection.execute(
                    "INSERT INTO problem_set_members(problem_set_id, problem_id, ordinal, section) "
                    "VALUES (?, ?, ?, NULL)",
                    (set_id, problem_id, ordinal),
                )
            connection.commit()
            connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")

    def database(self, label: str) -> Path:
        self.database_index += 1
        path = self.temporary / f"{self.database_index:02d}-{label}.db"
        with (
            sqlite3.connect(self.template_database) as source,
            sqlite3.connect(path) as target,
        ):
            source.backup(target)
        return path

    def codex_home(self, mode: str) -> Path:
        self.codex_index += 1
        path = self.temporary / f"codex-{self.codex_index:02d}-{mode}"
        path.mkdir(mode=0o700)
        (path / "fake-mode").write_text(mode, encoding="utf-8")
        self.cleanup.register_codex_home(path)
        return path

    def environment(
        self,
        codex_mode: str = "normal",
        no_codex: bool = False,
        timeout_ms: int | None = None,
        executable: Path | None = None,
        pid_file: Path | None = None,
    ) -> tuple[dict[str, str], Path]:
        environment = self.base_environment.copy()
        home = self.codex_home(codex_mode)
        environment["CODEX_HOME"] = str(home)
        environment["INTERVIEW_TUTOR_CODEX_EXECUTABLE"] = str(
            executable or self.fake_codex
        )
        if timeout_ms is not None:
            environment["INTERVIEW_TUTOR_TEST_RUN_TIMEOUT_MS"] = str(timeout_ms)
        if pid_file is not None:
            self.cleanup.register_pid_file(pid_file)
            environment["PRACTICE_FIXTURE_PID_FILE"] = str(pid_file)
        if no_codex:
            environment.pop("INTERVIEW_TUTOR_CODEX_EXECUTABLE", None)
        return environment, home

    def launch(
        self,
        database: Path,
        environment: dict[str, str],
        columns: int = 120,
        rows: int = 40,
        problem_set: bool = True,
        no_codex: bool = False,
        case_timeout: float = 20.0,
    ) -> PtySession:
        command = [str(self.interview_binary), "--db", str(database)]
        if problem_set:
            command += ["--set", "a00-matrix"]
        command += ["--language", "python"]
        if no_codex:
            command.append("--no-codex")
        session = PtySession(command, environment, columns, rows, case_timeout)
        self.case_sessions.append(session)
        return session

    def finalize_case(self) -> None:
        invalid_requests = [
            session.screen.invalid_cursor_request
            for session in self.case_sessions
            if session.screen.invalid_cursor_request is not None
        ]
        self.case_sessions.clear()
        self.cleanup.finalize_case()
        assert not invalid_requests, f"invalid cursor requests: {invalid_requests!r}"

    def set_runner(self, mode: str) -> None:
        self.runner_mode.write_text(mode, encoding="utf-8")
        self.solution.write_text("print('initial')\n", encoding="utf-8")


def capture_records(home: Path) -> list[dict]:
    path = home / "fake-capture.jsonl"
    if not path.exists():
        return []
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return records


def capture_messages(home: Path) -> list[dict]:
    return [
        record["json"]
        for record in capture_records(home)
        if record.get("kind") == "message"
    ]


def captured_turns(home: Path) -> list[dict]:
    return [
        message
        for message in capture_messages(home)
        if message.get("method") == "turn/start"
    ]


def captured_completions(home: Path) -> list[dict]:
    return [
        record
        for record in capture_records(home)
        if record.get("kind") == "turn-completed"
    ]


def wait_turns(session: PtySession, home: Path, count: int) -> None:
    session.wait_predicate(
        f"{count} captured Codex turns",
        lambda: len(captured_turns(home)) >= count,
    )


def wait_turn_completion(session: PtySession, home: Path, turn: int) -> None:
    session.wait_predicate(
        f"unique Codex completion marker for turn {turn}",
        lambda: any(
            record.get("turn") == turn for record in captured_completions(home)
        ),
    )
    session.wait_screen(f"[turn-{turn}]")


def captured_turn_sequence(home: Path) -> list[tuple[str, str]]:
    sequence = []
    for turn in captured_turns(home):
        text = turn["params"]["input"][0]["text"]
        payload = json.loads(text.split("INPUT_JSON:", 1)[1])
        if "Review the explicitly recorded local submission" in text:
            sequence.append(("submission-review", payload["userQuestion"]))
            continue
        sequence.append(("interviewer", payload["userQuestion"]))
    return sequence


def query_attempts(database: Path) -> list[tuple[str, int | None]]:
    with sqlite3.connect(database) as connection:
        return connection.execute(
            "SELECT result, exit_code FROM attempts ORDER BY id"
        ).fetchall()


def assert_codex_cleanup(home: Path) -> None:
    for record in capture_records(home):
        if record.get("kind") != "process":
            continue
        cwd = Path(record["cwd"])
        pid = int(record["pid"])
        assert not cwd.exists(), f"Codex temporary directory leaked: {cwd}"
        assert not Path(f"/proc/{pid}").exists(), f"Codex process leaked: {pid}"


def open_solve(session: PtySession) -> None:
    session.wait_screen("Smoke Problem")
    session.send(ENTER)
    session.wait_screen("Matrix statement")
    session.send(ENTER)
    session.wait_screen("Editor")
    session.wait_screen("No test run yet")


def focus_interview_and_disclose(session: PtySession) -> None:
    session.send(b"\t\t\t")
    session.wait_screen("Interview [active]")
    session.send(b"i")
    session.wait_screen("Privacy disclosure")


def accept_disclosure(session: PtySession) -> None:
    session.send(b"y")
    session.wait_screen("Codex: ready")


def quit_from_editor(session: PtySession, dirty: bool = False) -> None:
    session.send(b" q")
    if dirty:
        session.wait_screen("Unsaved changes")
        session.send(b" q")
    session.wait_exit(0)


def local_test_after_interview(session: PtySession) -> None:
    session.send(ESCAPE)
    session.settle()
    session.send(b"\t")
    session.wait_screen("Editor [active]")
    session.send(F5)
    session.wait_screen("Run complete")


def full_workflow_case(fixture: MatrixFixture) -> str:
    fixture.set_runner("normal")
    database = fixture.database("full")
    environment, home = fixture.environment()
    with fixture.launch(database, environment, problem_set=False) as session:
        session.wait_screen("A00 Matrix Se")
        session.send(b"j" * 40)
        session.wait_screen("Matrix Set 39")
        session.send(b"k" * 40)
        session.wait_screen("A00 Matrix Se")
        session.send(ENTER)
        session.wait_screen("Smoke Problem")
        session.send(b"j" * 30)
        session.wait_screen(fixture.problem_titles[29])
        session.send(b"k" * 30)
        session.wait_screen("Smoke Problem")
        session.send(ENTER)
        session.wait_screen("Matrix statement")
        session.send(b"j" * 40)
        session.wait_screen("DETAIL-END-SENTINEL")
        session.send(ENTER)
        session.wait_screen("Editor")

        session.send("i🦀界".encode("utf-8"))
        session.wait_screen("🦀界")
        session.send(ESCAPE)
        session.wait_screen("Normal · DIRTY")
        session.send(b":bogus" + ENTER)
        session.wait_screen("unsupported command: :bogus")
        session.send(b"h")
        session.wait_predicate(
            "command error dismissal",
            lambda: "unsupported command" not in session.screen.text(),
        )
        session.send(b"u")
        session.wait_predicate(
            "Unicode insertion undo",
            lambda: "🦀界" not in session.screen.text(),
        )
        session.send(b"iX")
        session.wait_screen("Xprint")
        session.send(ESCAPE)
        session.wait_screen("Normal · DIRTY")
        session.send(b" b")
        session.wait_screen("Unsaved changes")
        assert "Editor" in session.screen.text()

        session.send(F5)
        session.wait_screen("Run complete")
        assert query_attempts(database) == []
        session.send(b"\t\t")
        session.wait_screen("Output / Test [active]")
        session.send(b"j" * 60)
        session.wait_screen("PASS")
        session.send(b"i")
        session.wait_screen("Privacy disclosure")
        assert captured_turns(home) == []
        assert not (home / "fake-version-probe").exists()
        session.send(b"y")
        session.wait_screen("Codex: ready")
        assert captured_turns(home) == []

        session.send(b"Why does this invariant hold?" + ENTER)
        wait_turns(session, home, 1)
        session.wait_screen("Interviewer:")
        for level in range(1, 4):
            session.send(b" h")
            wait_turns(session, home, 1 + level)
            session.wait_screen(f"Level {level} invariant")
        session.send(b" h")
        session.wait_screen("maximum three hints reached")
        session.settle()
        assert len(captured_turns(home)) == 4
        session.send(b"k")

        session.send(F9)
        wait_turns(session, home, 5)
        session.wait_screen("Submission review · recorded")
        attempts = query_attempts(database)
        assert attempts == [("pass", 0)], attempts
        session.send(b" r")
        session.wait_screen("Codex: offline")
        current = session.screen.text()
        assert "What invariant holds?" not in current
        assert "Level 1 invariant" not in current
        assert "Submission reviewed" not in current
        session.send(b"\t")
        quit_from_editor(session)

    assert_codex_cleanup(home)
    persisted = database.read_bytes() + fixture.solution.read_bytes()
    for private in [
        b"Why does this invariant hold?",
        b"What invariant holds?",
        b"Level 1 invariant",
        b"Submission reviewed",
    ]:
        assert private not in persisted

    environment, relaunch_home = fixture.environment()
    with fixture.launch(database, environment) as session:
        open_solve(session)
        focus_interview_and_disclose(session)
        screen = session.screen.text()
        assert "What invariant holds?" not in screen
        assert "Level 1 invariant" not in screen
        assert "Submission reviewed" not in screen
        assert captured_turns(relaunch_home) == []
        session.send(b"n")
        session.wait_screen("Codex declined")
        session.send(b"\t")
        quit_from_editor(session)
    assert_codex_cleanup(relaunch_home)
    return (
        "attempts=1(pass) turns=5 hint_levels=1,2,3 relaunch_transcript=0 statuses=0,0"
    )


def compact_case(fixture: MatrixFixture) -> str:
    fixture.set_runner("normal")
    database = fixture.database("compact")
    environment, home = fixture.environment()
    with fixture.launch(database, environment, columns=80, rows=24) as session:
        session.wait_screen("Smoke Problem")
        assert "Ready" in session.screen.text()
        assert "View" in session.screen.text()
        session.send(b"\t")
        session.wait_screen("Progress [active]")
        session.send(SHIFT_TAB)
        session.wait_screen("Problems [active]")
        session.send(b"j" * 20)
        session.wait_screen(fixture.problem_titles[19])
        session.send(b"k" * 20 + ENTER)
        session.wait_screen("Matrix statement")
        session.send(b"j" * 45)
        session.wait_screen("DETAIL-END-SENTINEL")
        session.send(ENTER)
        session.wait_screen("Solve panes")
        session.wait_screen("Codex: offline · memory only")

        session.send(b"\t")
        session.wait_screen("Problem / Examples [active]")
        session.send(b"j" * 45)
        session.wait_screen("DETAIL-END-SENTINEL")
        session.send(b"\t")
        session.wait_screen("Output / Test [active]")
        session.send(b"\t")
        session.wait_screen("Interview [active]")
        session.send(b"i")
        session.wait_screen("Privacy disclosure")
        accept_disclosure(session)
        for index in range(6):
            if index > 0:
                session.send(b"i")
            question = f"compact-question-{index}-" + "wrapped-content-" * 3
            session.send(question.encode("utf-8") + ENTER)
            wait_turn_completion(session, home, index + 1)
        session.send(b"k" * 40)
        session.wait_screen("compact-question-0-")
        session.send(b"j" * 40)
        session.wait_screen("compact-question-5-")

        session.send(b"\t")
        session.wait_screen("Editor [active]")
        session.send(F5)
        session.wait_screen("Run complete")
        assert query_attempts(database) == []
        session.send(b"\t\t")
        session.wait_screen("Output / Test [active]")
        session.wait_screen("output-line-00")
        footer = session.screen.text().splitlines()[-1]
        assert len(footer) <= 80
        assert "Tab panes" in footer
        session.send(b"j" * 60)
        session.wait_screen("output-line-59")
        session.send(F9)
        session.wait_predicate(
            "one compact submit attempt",
            lambda: query_attempts(database) == [("pass", 0)],
        )
        session.send(b"\t")
        session.wait_screen("Interview [active]")
        wait_turn_completion(session, home, 7)
        session.wait_screen("Submission review · recorded revision")
        expected_sequence = [
            (
                "interviewer",
                f"compact-question-{index}-" + "wrapped-content-" * 3,
            )
            for index in range(6)
        ] + [("submission-review", "")]
        assert captured_turn_sequence(home) == expected_sequence
        assert len(captured_turns(home)) == 7
        session.wait_screen("Submit recorded")
        session.send(b"\t")
        session.wait_screen("Editor [active]")
        quit_from_editor(session)
    assert_codex_cleanup(home)
    return "attempts test=0 submit=1(pass) turns=7 compact=80x24 status=0"


def resize_case(fixture: MatrixFixture) -> str:
    fixture.set_runner("normal")
    database = fixture.database("resize")
    environment, home = fixture.environment(no_codex=True)
    with fixture.launch(database, environment, no_codex=True) as session:
        open_solve(session)
        session.send("iRESIZE-界".encode("utf-8"))
        session.wait_screen("RESIZE-界")
        session.send(ESCAPE)
        session.wait_screen("Normal · DIRTY")
        session.resize(59, 19)
        session.wait_screen("Terminal too small")
        session.wait_screen("Resize to at least 60 × 20")
        session.wait_screen("Dirty editor: press twice to confirm")
        assert session.process.poll() is None
        session.resize(80, 24)
        session.wait_screen("Solve panes")
        session.wait_screen("RESIZE-界")
        session.resize(120, 40)
        session.wait_screen("Problem / Examples")
        session.wait_screen("RESIZE-界")
        quit_from_editor(session, dirty=True)
    assert not (home / "fake-version-probe").exists()
    return "sizes=120x40,59x19,80x24,120x40 buffer=preserved status=0"


def run_runner_case(fixture: MatrixFixture, mode: str) -> str:
    fixture.set_runner(mode)
    database = fixture.database(f"runner-{mode}")
    pid_file = fixture.temporary / f"runner-{mode}.pid"
    environment, home = fixture.environment(
        no_codex=True,
        timeout_ms=200 if mode == "timeout" else None,
        pid_file=pid_file,
    )
    escaped_pid = 0
    try:
        with fixture.launch(database, environment, no_codex=True) as session:
            open_solve(session)
            if mode == "flood":
                session.watch_raw(b"\x1b]0;hostile-title\x07")
                session.watch_raw(b"hostile-title")
            session.send(F9)
            if mode == "cancel":
                session.wait_predicate("runner pid fixture", pid_file.exists)
                session.send(CTRL_C)
            session.wait_predicate(
                "recorded targeted runner attempt",
                lambda: len(query_attempts(database)) == 1,
            )
            session.wait_screen("Submit recorded")
            session.send(b"\t\t")
            session.wait_screen("Output / Test [active]")
            expected = {
                "fail": ("Exited(7)", ("fail", 7)),
                "timeout": ("TimedOut", ("error", None)),
                "cancel": ("Cancelled", ("cancelled", 130)),
                "flood": ("output truncated", ("pass", 0)),
                "group-descendant": ("fixture-child", ("pass", 0)),
                "escaped-descendant": ("fixture-child", ("pass", 0)),
            }[mode]
            session.wait_screen(expected[0])
            if mode == "flood":
                session.wait_screen("euro:€")
                session.send(b"j" * 20)
                session.wait_screen("flood-0005")
                assert "hostile-title" not in session.screen.text()
                assert not session.raw_sequence_seen(b"\x1b]0;hostile-title\x07")
                assert not session.raw_sequence_seen(b"hostile-title")
                assert all(
                    "hostile-title" not in event for event in session.screen.osc_events
                )
                assert len(session.output) <= 4 * 1024 * 1024
                assert len(session.screen.osc_events) <= 128
            if mode in {"group-descendant", "escaped-descendant"}:
                session.wait_predicate("descendant pid fixture", pid_file.exists)
                escaped_pid = int(pid_file.read_text(encoding="utf-8"))
            attempts = query_attempts(database)
            assert attempts == [expected[1]], (mode, attempts)
            session.send(b"\t\t")
            quit_from_editor(session)
        if mode == "group-descendant":
            deadline = time.monotonic() + 1.0
            while Path(f"/proc/{escaped_pid}").exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            assert not Path(f"/proc/{escaped_pid}").exists()
        if mode == "escaped-descendant":
            assert Path(f"/proc/{escaped_pid}").exists()
    finally:
        # Matrix.run owns the outer cleanup registry finalizer so PID files are reread even if an
        # assertion above aborts before escaped_pid is assigned.
        pass
    assert not (home / "fake-version-probe").exists()
    outcome = query_attempts(database)[0]
    boundary = " explicit-escaped-pid-cleanup" if mode == "escaped-descendant" else ""
    return f"attempt={outcome[0]} exit={outcome[1]} responsive=yes{boundary}"


def run_signal_during_record_lock_case(
    fixture: MatrixFixture,
    signal_number: int,
    iteration: int,
) -> str:
    assert signal_number in {signal.SIGINT, signal.SIGTERM}
    expected_status = 130 if signal_number == signal.SIGINT else 143
    fixture.set_runner("record-lock")
    label = f"signal-lock-{signal_number}-{iteration}"
    database = fixture.database(label)
    pid_file = fixture.temporary / f"{label}.pid"
    ready_file = fixture.temporary / f"{label}.ready"
    disposition_file = fixture.temporary / f"{label}.dispositions"
    environment, _home = fixture.environment(no_codex=True, pid_file=pid_file)
    environment["PRACTICE_RECORD_READY_FILE"] = str(ready_file)
    environment["INTERVIEW_TUTOR_TEST_SIGNAL_DISPOSITION_FILE"] = str(disposition_file)

    lock = sqlite3.connect(database, timeout=1.0, isolation_level=None)
    try:
        with fixture.launch(database, environment, no_codex=True) as session:
            open_solve(session)
            lock.execute("BEGIN IMMEDIATE")
            try:
                session.send(F9)
                session.wait_predicate(
                    "record-lock runner completion", ready_file.exists
                )
                session.wait_predicate(
                    "record-lock runner exit",
                    lambda: (
                        pid_file.exists()
                        and not Path(
                            f"/proc/{int(pid_file.read_text(encoding='utf-8'))}"
                        ).exists()
                    ),
                )
                time.sleep(0.05)
                os.kill(session.process.pid, signal_number)
                time.sleep(0.05)
            finally:
                lock.execute("COMMIT")
            session.wait_exit(expected_status)
    finally:
        lock.close()

    assert query_attempts(database) == [("cancelled", expected_status)]
    assert disposition_file.read_text(encoding="utf-8") == (
        "dispositions=restored mask=restored\n"
    )
    return (
        f"signal={signal.Signals(signal_number).name} attempt=cancelled "
        f"exit={expected_status} dispositions=restored mask=restored iteration={iteration}"
    )


def codex_local_recovery(session: PtySession, database: Path) -> None:
    local_test_after_interview(session)
    assert query_attempts(database) == []
    quit_from_editor(session)


def run_codex_case(fixture: MatrixFixture, mode: str) -> str:
    fixture.set_runner("normal")
    database = fixture.database(f"codex-{mode}")
    environment, home = fixture.environment(codex_mode=mode)
    with fixture.launch(database, environment) as session:
        open_solve(session)
        focus_interview_and_disclose(session)
        assert captured_turns(home) == []
        if mode == "decline":
            session.send(b"n")
            session.wait_screen("Codex declined")
            assert captured_turns(home) == []
            codex_local_recovery(session, database)
            turns = 0
        else:
            session.send(b"y")
            if mode == "auth-required":
                session.wait_screen("Authentication required")
                assert captured_turns(home) == []
                codex_local_recovery(session, database)
                turns = 0
            elif mode == "queue-flood":
                session.wait_predicate(
                    "queue flood failure",
                    lambda: (
                        "Codex: disconnected" in session.screen.text()
                        or "Codex: protocol error" in session.screen.text()
                    ),
                )
                codex_local_recovery(session, database)
                turns = len(captured_turns(home))
            else:
                session.wait_screen("Codex: ready")
                session.send(b"targeted failure" + ENTER)
                wait_turns(session, home, 1)
                if mode in {"timeout-ack", "interrupt-no-ack"}:
                    session.send(CTRL_C)
                    session.wait_screen("Codex: disconnected")
                    session.wait_predicate(
                        "captured turn interrupt",
                        lambda: any(
                            message.get("method") == "turn/interrupt"
                            for message in capture_messages(home)
                        ),
                    )
                elif mode == "restart":
                    session.wait_screen("Codex: protocol error")
                    session.send(b"i")
                    session.wait_screen("Codex: ready")
                    assert len(captured_turns(home)) == 1
                    session.send(b"after explicit reconnect" + ENTER)
                    wait_turns(session, home, 2)
                    session.wait_screen("Interviewer:")
                    session.wait_predicate(
                        "single Codex reconnect",
                        lambda: (
                            (home / "fake-start-count").exists()
                            and (home / "fake-start-count").read_text(encoding="utf-8")
                            == "2"
                        ),
                    )
                elif mode == "stderr-flood":
                    session.wait_screen("Interviewer:")
                else:
                    session.wait_screen("Codex: protocol error")
                    if mode in {
                        "approval-command",
                        "approval-user-input",
                        "approval-mcp",
                    }:
                        session.wait_screen("Codex requested forbidden operation")
                turns = len(captured_turns(home))
                codex_local_recovery(session, database)
    assert_codex_cleanup(home)
    probe_path = home / "fake-version-probe"
    probes = (
        probe_path.read_text(encoding="utf-8").count("version")
        if probe_path.exists()
        else 0
    )
    reconnects = (
        int((home / "fake-start-count").read_text(encoding="utf-8"))
        if (home / "fake-start-count").exists()
        else 0
    )
    return f"turns={turns} version_probes={probes} starts={reconnects} local_test=pass attempts=0 status=0"


def config_case(fixture: MatrixFixture) -> str:
    database = fixture.database("config")
    environment, no_codex_home = fixture.environment(no_codex=True)
    binary = str(fixture.interview_binary)

    invalid_db = run_checked(
        [binary, "--db", "", "--no-codex"], environment, expected_status=1
    )
    assert "database path must not be empty" in invalid_db.stderr
    valid_db = run_checked(
        [binary, "--db", str(database), "--no-codex"],
        environment,
        expected_status=1,
    )
    assert "interactive terminal" in valid_db.stderr
    invalid_set = run_checked(
        [binary, "--db", str(database), "--set", "missing", "--no-codex"],
        environment,
        expected_status=1,
    )
    assert "unknown problem set" in invalid_set.stderr
    valid_set = run_checked(
        [binary, "--db", str(database), "--set", "a00-matrix", "--no-codex"],
        environment,
        expected_status=1,
    )
    assert "interactive terminal" in valid_set.stderr
    invalid_language = run_checked(
        [binary, "--db", str(database), "--language", "brainfry", "--no-codex"],
        environment,
        expected_status=1,
    )
    assert "unknown or disabled language" in invalid_language.stderr
    valid_language = run_checked(
        [binary, "--db", str(database), "--language", "python", "--no-codex"],
        environment,
        expected_status=1,
    )
    assert "interactive terminal" in valid_language.stderr

    with fixture.launch(database, environment, no_codex=True) as session:
        session.wait_screen("Smoke Problem")
        session.send(b"q")
        session.wait_exit(0)
    assert not (no_codex_home / "fake-version-probe").exists()
    assert not (no_codex_home / "fake-capture.jsonl").exists()

    for label, source, expected in [
        (
            "incompatible",
            "#!/bin/sh\nif [ \"${1:-}\" = --version ]; then echo 'codex-cli 9.9.9'; exit 0; fi\nexit 2\n",
            "unsupported Codex CLI",
        ),
        (
            "untrusted",
            "#!/bin/sh\necho 'codex-cli 0.146.0'\n",
            "group- or world-writable",
        ),
    ]:
        executable = fixture.temporary / f"codex-{label}.sh"
        executable.write_text(source, encoding="utf-8")
        executable.chmod(0o722 if label == "untrusted" else 0o700)
        case_database = fixture.database(f"config-{label}")
        case_environment, home = fixture.environment(executable=executable)
        with fixture.launch(case_database, case_environment) as session:
            open_solve(session)
            focus_interview_and_disclose(session)
            session.send(b"y")
            session.wait_screen(expected)
            codex_local_recovery(session, case_database)
        assert_codex_cleanup(home)
    return "db/set/language invalid+valid=6 no_codex_probes=0 incompatible+untrusted local_test=pass statuses=1/0"


class Matrix:
    def __init__(self, cleanup: Callable[[], None]) -> None:
        self.started = time.monotonic()
        self.cleanup = cleanup
        self.results: list[tuple[str, float, str]] = []

    def run(self, name: str, operation: Callable[[], str]) -> None:
        started = time.monotonic()
        try:
            with hard_deadline(20.0, f"PTY case {name}"):
                evidence = operation()
        finally:
            self.cleanup()
        elapsed = time.monotonic() - started
        assert elapsed <= 20.0, f"case {name} exceeded 20s: {elapsed:.3f}s"
        self.results.append((name, elapsed, evidence))
        print(f"PASS {name} {elapsed:.3f}s {evidence}", flush=True)

    def finish(self, maximum: float = 90.0) -> None:
        elapsed = time.monotonic() - self.started
        assert elapsed <= maximum, f"matrix exceeded {maximum:.0f}s: {elapsed:.3f}s"
        print(
            f"PTY_MATRIX_PASS cases={len(self.results)} elapsed={elapsed:.3f}s "
            f"max_case={max(item[1] for item in self.results):.3f}s",
            flush=True,
        )


def full_matrix(fixture: MatrixFixture, matrix: Matrix) -> None:
    matrix.run("workflow-120x40", lambda: full_workflow_case(fixture))
    matrix.run("compact-80x24", lambda: compact_case(fixture))
    matrix.run("resize-preservation", lambda: resize_case(fixture))

    for mode in [
        "fail",
        "timeout",
        "cancel",
        "flood",
        "group-descendant",
        "escaped-descendant",
    ]:
        matrix.run(f"runner-{mode}", lambda mode=mode: run_runner_case(fixture, mode))

    for iteration in range(1, 3):
        for signal_number in [signal.SIGINT, signal.SIGTERM]:
            matrix.run(
                f"signal-record-lock-{signal.Signals(signal_number).name.lower()}-{iteration}",
                lambda signal_number=signal_number, iteration=iteration: (
                    run_signal_during_record_lock_case(
                        fixture, signal_number, iteration
                    )
                ),
            )

    for mode in [
        "auth-required",
        "decline",
        "malformed",
        "oversize",
        "eof",
        "timeout-ack",
        "interrupt-no-ack",
        "approval-command",
        "approval-user-input",
        "approval-mcp",
        "restart",
        "stderr-flood",
        "queue-flood",
    ]:
        matrix.run(f"codex-{mode}", lambda mode=mode: run_codex_case(fixture, mode))

    matrix.run("config-boundaries", lambda: config_case(fixture))

    lifecycle = tui_runtime_cleanup.run_all(
        fixture.interview_binary, fixture.repository_root
    )
    for name, elapsed, status in lifecycle:
        matrix.results.append(
            (f"lifecycle-{name}", elapsed, f"status={status} terminal=restored")
        )
        print(
            f"PASS lifecycle-{name} {elapsed:.3f}s status={status} terminal=restored",
            flush=True,
        )


def race_matrix(fixture: MatrixFixture, matrix: Matrix) -> None:
    for iteration in range(10):
        matrix.run(
            f"runner-cancel-repeat-{iteration + 1}",
            lambda: run_runner_case(fixture, "cancel"),
        )
    for iteration in range(10):
        matrix.run(
            f"codex-cancel-repeat-{iteration + 1}",
            lambda: run_codex_case(fixture, "interrupt-no-ack"),
        )


def main() -> int:
    if sys.platform != "linux":
        raise AssertionError("make test-pty is a Linux gate and must not silently skip")
    arguments = sys.argv[1:]
    race = False
    if arguments and arguments[0] == "--race":
        race = True
        arguments = arguments[1:]
    if len(arguments) != 3:
        raise SystemExit(
            "usage: pty_matrix.py [--race] INTERVIEW_BINARY PRACTICE_BINARY REPOSITORY_ROOT"
        )
    interview_binary, practice_binary, repository_root = map(
        lambda value: Path(value).resolve(), arguments
    )
    with hard_deadline(90.0, "complete PTY matrix"):
        with tempfile.TemporaryDirectory(prefix="interview-pty-matrix-") as directory:
            fixture = MatrixFixture(
                Path(directory), interview_binary, practice_binary, repository_root
            )
            matrix = Matrix(fixture.finalize_case)
            try:
                if race:
                    race_matrix(fixture, matrix)
                    matrix.finish(maximum=90.0)
                else:
                    full_matrix(fixture, matrix)
                    matrix.finish(maximum=90.0)
            finally:
                fixture.cleanup.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
