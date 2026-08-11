#!/usr/bin/env python3
"""Linux PTY checks for bounded interview runtime teardown."""

from __future__ import annotations

import os
from pathlib import Path
import signal
import sys
import tempfile
import termios
import time

from pty_harness import PtySession, hard_deadline

ENTER_SCREEN = b"\x1b[?1049h"
LEAVE_SCREEN = b"\x1b[?1049l"
SHOW_CURSOR = b"\x1b[?25h"
CASE_TIMEOUT_SECONDS = 4.0


def run_case(
    name: str,
    interview_binary: Path,
    repository_root: Path,
    temporary: Path,
    action: str,
    expected_status: int,
) -> float:
    codex_home = temporary / f"codex-{name}"
    codex_home.mkdir(mode=0o700)
    (codex_home / "fake-mode").write_text("normal", encoding="utf-8")
    environment = os.environ.copy()
    environment.pop("OPENAI_API_KEY", None)
    environment["PRACTICE_ROOT"] = str(repository_root)
    environment["CODEX_HOME"] = str(codex_home)
    environment["INTERVIEW_TUTOR_CODEX_EXECUTABLE"] = str(
        repository_root / "cli" / "tests" / "fixtures" / "fake_codex_app_server.py"
    )
    if action == "error":
        environment["INTERVIEW_TUTOR_TEST_ERROR_AFTER_ENTER"] = "1"
    if action == "panic":
        environment["INTERVIEW_TUTOR_TEST_PANIC_AFTER_ENTER"] = "1"
    disposition_file = temporary / f"{name}-signal-dispositions"
    if action == "quit":
        environment["INTERVIEW_TUTOR_TEST_SIGNAL_DISPOSITION_FILE"] = str(
            disposition_file
        )

    command = [
        str(interview_binary),
        "--db",
        str(temporary / f"{name}.db"),
        "--set",
        "blind75",
        "--language",
        "python",
        "--no-codex",
    ]
    with PtySession(
        command,
        environment,
        columns=100,
        rows=30,
        case_timeout=CASE_TIMEOUT_SECONDS,
        keep_slave=True,
    ) as session:
        terminal_before = session.terminal_before
        if action in {"quit", "sigint", "sigterm"}:
            session.wait_history(ENTER_SCREEN)
            if action == "quit":
                session.send(b"q")
            else:
                session.process.send_signal(
                    signal.SIGINT if action == "sigint" else signal.SIGTERM
                )
        elapsed = session.wait_exit(expected_status)
        output = bytes(session.output)
        assert ENTER_SCREEN in output, (name, output[-2000:])
        assert LEAVE_SCREEN in output, (name, output[-2000:])
        assert SHOW_CURSOR in output, (name, output[-2000:])
        assert session.slave is not None
        assert termios.tcgetattr(session.slave) == terminal_before, name
        assert not (codex_home / "fake-version-probe").exists(), name
        assert not (codex_home / "fake-capture.jsonl").exists(), name
        if action == "quit":
            assert disposition_file.read_text(encoding="utf-8") == (
                "dispositions=restored mask=restored\n"
            )
        return elapsed


def run_all(
    interview_binary: Path, repository_root: Path
) -> list[tuple[str, float, int]]:
    if sys.platform != "linux":
        raise AssertionError("the Linux PTY cleanup gate must not silently skip")
    evidence = []
    with tempfile.TemporaryDirectory(prefix="interview-runtime-cleanup-") as directory:
        temporary = Path(directory)
        for name, action, expected_status in [
            ("clean-quit", "quit", 0),
            ("startup-error", "error", 1),
            ("panic", "panic", 1),
            ("sigint", "sigint", 130),
            ("sigterm", "sigterm", 143),
        ]:
            started = time.monotonic()
            with hard_deadline(CASE_TIMEOUT_SECONDS, f"runtime cleanup {name}"):
                elapsed = run_case(
                    name,
                    interview_binary,
                    repository_root,
                    temporary,
                    action,
                    expected_status,
                )
            assert time.monotonic() - started <= CASE_TIMEOUT_SECONDS
            evidence.append((name, elapsed, expected_status))
    return evidence


def main() -> int:
    interview_binary = Path(sys.argv[1]).resolve()
    repository_root = Path(sys.argv[2]).resolve()
    evidence = run_all(interview_binary, repository_root)
    statuses = ",".join(f"{name}:{status}" for name, _, status in evidence)
    print(f"terminal_cleanup={len(evidence)} statuses={statuses} no_codex_probes=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
