#!/usr/bin/env python3
"""Self-tests for PTY hard deadlines, cleanup, and Make race fail-fast behavior."""

from __future__ import annotations

import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

from pty_harness import (
    CleanupRegistry,
    CursorRequest,
    HarnessTimeout,
    PtySession,
    Screen,
    hard_deadline,
)


def wait_file(path: Path) -> None:
    deadline = time.monotonic() + 2.0
    while not path.exists():
        assert time.monotonic() < deadline, f"fixture did not create {path}"
        time.sleep(0.01)


def deadline_cleanup_self_test(temporary: Path) -> None:
    pid_file = temporary / "deadline.pid"
    command = [
        sys.executable,
        "-c",
        (
            "import os,pathlib,time; "
            f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid())); "
            "time.sleep(30)"
        ),
    ]
    prior_handler = signal.getsignal(signal.SIGALRM)
    prior_timer = signal.getitimer(signal.ITIMER_REAL)
    timed_out = False
    session_pid = 0
    try:
        with hard_deadline(0.2, "injected harness timeout"):
            with PtySession(command, os.environ.copy(), 80, 24) as session:
                session_pid = session.process.pid
                wait_file(pid_file)
                session.wait_history(b"never-produced")
    except HarnessTimeout:
        timed_out = True
    assert timed_out, "injected timeout did not fire"
    assert session_pid > 0
    assert not Path(f"/proc/{session_pid}").exists(), "timed-out PTY child leaked"
    assert signal.getsignal(signal.SIGALRM) == prior_handler
    assert signal.getitimer(signal.ITIMER_REAL) == prior_timer


def injected_failure_registry_self_test(temporary: Path) -> None:
    pid_file = temporary / "registry.pid"
    artifact = temporary / "registry-artifact"
    registry = CleanupRegistry()
    registry.register_pid_file(pid_file)
    registry.register_temp_path(artifact)
    artifact.mkdir()
    child = subprocess.Popen(
        [
            sys.executable,
            "-c",
            (
                "import os,pathlib,time; os.setsid(); "
                f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid())); "
                "time.sleep(30)"
            ),
        ],
        close_fds=True,
    )
    injected = False
    try:
        wait_file(pid_file)
        raise AssertionError("injected case failure")
    except AssertionError as error:
        assert str(error) == "injected case failure"
        injected = True
    finally:
        registry.close()
    assert injected
    assert child.poll() is not None, "registry did not reap direct fixture child"
    assert not Path(f"/proc/{child.pid}").exists(), "registry fixture child leaked"
    assert not artifact.exists(), "registry fixture path leaked"


def cursor_request_self_test() -> None:
    cases = [
        (b"\x1b[4A", (2, 2), None, CursorRequest("4", "A", 2, -2, 5, 5), (2, 0)),
        (b"\x1b[3B", (2, 2), None, CursorRequest("3", "B", 2, 5, 5, 5), (2, 4)),
        (b"\x1b[3C", (2, 2), None, CursorRequest("3", "C", 5, 2, 5, 5), (4, 2)),
        (b"\x1b[4D", (2, 2), None, CursorRequest("4", "D", -2, 2, 5, 5), (0, 2)),
        (b"\x1b[3e", (2, 2), None, CursorRequest("3", "e", 2, 5, 5, 5), (2, 4)),
        (b"\x1b[6;3H", (2, 2), None, CursorRequest("6;3", "H", 2, 5, 5, 5), (2, 4)),
        (b"\x1b[3;6f", (2, 2), None, CursorRequest("3;6", "f", 5, 2, 5, 5), (4, 2)),
        (b"\x1b[6G", (2, 2), None, CursorRequest("6", "G", 5, 2, 5, 5), (4, 2)),
        (b"\x1b[6d", (2, 2), None, CursorRequest("6", "d", 2, 5, 5, 5), (2, 4)),
        (b"\x1b[u", (2, 2), (5, 2), CursorRequest("", "u", 5, 2, 5, 5), (4, 2)),
    ]
    for sequence, initial, saved, expected_request, expected_cursor in cases:
        screen = Screen(5, 5)
        screen.x, screen.y = initial
        if saved is not None:
            screen.saved = saved
        screen.feed(sequence)
        assert list(screen.cursor_requests) == [expected_request]
        assert screen.invalid_cursor_request == expected_request
        assert (screen.x, screen.y) == expected_cursor

    valid = Screen(5, 5)
    valid.feed(
        b"\x1b[2;2H\x1b[1A\x1b[1B\x1b[1C\x1b[1D\x1b[1e\x1b[3G\x1b[3d\x1b[s\x1b[u"
    )
    assert len(valid.cursor_requests) == 9
    assert valid.invalid_cursor_request is None


def make_fail_fast_self_test(repository_root: Path, temporary: Path) -> None:
    trace = temporary / "race-trace"
    hook = temporary / "race-hook.sh"
    hook.write_text(
        '#!/bin/sh\nset -eu\nprintf \'%s\\n\' "$1" >> "$RACE_TRACE"\n[ "$1" -ne 1 ]\n',
        encoding="utf-8",
    )
    hook.chmod(0o700)
    environment = os.environ.copy()
    environment["INTERVIEW_TUTOR_RACE_TEST_HOOK"] = str(hook)
    environment["RACE_TRACE"] = str(trace)
    result = subprocess.run(
        [
            "make",
            "--no-print-directory",
            "test-race",
            "CARGO=true",
            "PTY_RACE_GATE=true",
            "RACE_ITERATIONS=5",
        ],
        cwd=repository_root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    assert result.returncode != 0, result
    assert trace.read_text(encoding="utf-8").splitlines() == ["0", "1"]


def main() -> int:
    repository_root = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else Path.cwd()
    with tempfile.TemporaryDirectory(prefix="pty-harness-self-test-") as directory:
        temporary = Path(directory)
        deadline_cleanup_self_test(temporary)
        injected_failure_registry_self_test(temporary)
        cursor_request_self_test()
        make_fail_fast_self_test(repository_root, temporary)
    print(
        "PTY_HARNESS_SELF_TEST_PASS timeout_cleanup=1 injected_failure_cleanup=1 "
        "cursor_sequences=10 make_fail_fast_iterations=2/5"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
