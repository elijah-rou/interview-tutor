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
from types import SimpleNamespace
from typing import Callable

import pty_harness
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


def controlling_terminal_self_test() -> None:
    command = [
        sys.executable,
        "-c",
        (
            "import os; "
            "tty = os.open('/dev/tty', os.O_RDWR); "
            "size = os.get_terminal_size(tty); "
            "foreground = os.tcgetpgrp(tty) == os.getpgrp() == os.tcgetpgrp(0); "
            "print(f'{size.columns}x{size.lines} foreground={foreground}', "
            "flush=True)"
        ),
    ]
    with PtySession(command, os.environ.copy(), 73, 19) as session:
        session.wait_history(b"73x19 foreground=True")
        session.wait_exit(0)


def output_before_exit_self_test() -> None:
    """Each waiter accepts a marker read in the same iteration that sees the exit."""
    marker = "output-before-exit"
    command = [sys.executable, "-c", f"print({marker!r}, flush=True)"]
    waiters: list[Callable[[PtySession], None]] = [
        lambda session: session.wait_history(marker.encode()),
        lambda session: session.wait_screen(marker),
        lambda session: session.wait_predicate(
            "output marker", lambda: marker.encode() in session.output
        ),
    ]
    for wait in waiters:
        with PtySession(command, os.environ.copy(), 80, 24) as session:
            session.process.wait(timeout=5)
            wait(session)
            session.wait_exit(0)

    with PtySession(
        [sys.executable, "-c", "pass"], os.environ.copy(), 80, 24
    ) as session:
        session.process.wait(timeout=5)
        try:
            session.wait_history(b"never-produced")
        except AssertionError as error:
            assert str(error).startswith("process exited with 0 "), error
        else:
            raise AssertionError("exit without the marker did not fail the wait")


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


RESIZE_TRANSIENT_CHILD = r"""
import os, signal
signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGWINCH})
tty = os.open("/dev/tty", os.O_RDWR)

def size():
    current = os.get_terminal_size(tty)
    return current.columns, current.lines

def corner(columns, rows):
    os.write(1, f"\x1b[{rows};{columns}H#".encode())

sized = size()
os.write(1, b"\x1b[2J")
corner(*sized)
os.write(1, f"\x1b[1;1Hdrawn-{sized[0]}x{sized[1]}".encode())
while sized != (40, 10):
    signal.sigwait({signal.SIGWINCH})
    current = size()
    if current == sized:
        continue
    # Finish the frame sized before the resize, like a draw straddling it.
    corner(*sized)
    sized = current
    os.write(1, b"\x1b[2J")
    corner(*sized)
    os.write(1, f"\x1b[1;1Hdrawn-{sized[0]}x{sized[1]}".encode())
"""


def resize_transient_self_test() -> None:
    """A frame sized before a resize and written after it is not a violation."""
    command = [sys.executable, "-c", RESIZE_TRANSIENT_CHILD]
    with PtySession(command, os.environ.copy(), 80, 24) as session:
        session.wait_history(b"drawn-80x24")
        session.resize(60, 20)
        session.wait_history(b"drawn-60x20")
        assert session.screen.invalid_cursor_request is None, (
            session.screen.invalid_cursor_request
        )
        session.resize(40, 10)
        session.wait_history(b"drawn-40x10")
        session.wait_exit(0)
        requests = list(session.screen.cursor_requests)
        assert CursorRequest("24;80", "H", 79, 23, 60, 20) in requests, requests
        assert CursorRequest("20;60", "H", 59, 19, 40, 10) in requests, requests
        assert session.screen.cursor_violation is None, session.screen.cursor_violation


def resize_transient_screen_self_test() -> None:
    """Stale-size requests count unless the next full clear supersedes them."""
    stale = CursorRequest("24;80", "H", 79, 23, 60, 20)
    unrepainted = Screen(80, 24)
    unrepainted.resize(60, 20)
    unrepainted.feed(b"\x1b[24;80H\x1b[J\x1b[1J\x1b[3J")
    assert unrepainted.invalid_cursor_request is None
    assert unrepainted.size_before_resize == (80, 24)
    assert unrepainted.cursor_violation == stale

    repainted = Screen(80, 24)
    repainted.resize(60, 20)
    repainted.feed(b"\x1b[24;80H\x1b[2J")
    assert repainted.cursor_violation is None
    repainted.feed(b"\x1b[24;80H")
    assert repainted.invalid_cursor_request == stale

    outside_both = Screen(80, 24)
    outside_both.resize(60, 20)
    outside_both.feed(b"\x1b[25;1H")
    assert outside_both.invalid_cursor_request == CursorRequest(
        "25;1", "H", 0, 24, 60, 20
    )

    unchanged = Screen(80, 24)
    unchanged.resize(80, 24)
    assert unchanged.size_before_resize is None
    unchanged.resize(60, 20)
    try:
        unchanged.resize(40, 10)
    except AssertionError as error:
        assert "previous resize" in str(error), error
    else:
        raise AssertionError("a second outstanding resize was accepted")


def settle_deadline_self_test() -> None:
    """settle never polls with a negative wait when its end passes mid-iteration."""
    with PtySession(
        [sys.executable, "-c", "import time; time.sleep(5)"], os.environ.copy(), 80, 24
    ) as session:
        start = time.monotonic()
        debounce = 0.075
        # The loop check sees 1 ms left; the next clock read is 1 ms past the end.
        readings = iter([start, start + debounce - 0.001, start + debounce + 0.001])
        clock = SimpleNamespace(monotonic=lambda: next(readings, start + 1.0))
        real_time = pty_harness.time
        pty_harness.time = clock
        try:
            session.settle(debounce)
        finally:
            pty_harness.time = real_time
        assert next(readings, None) is None, "settle stopped before its end"


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
        controlling_terminal_self_test()
        output_before_exit_self_test()
        cursor_request_self_test()
        resize_transient_self_test()
        resize_transient_screen_self_test()
        settle_deadline_self_test()
        make_fail_fast_self_test(repository_root, temporary)
    print(
        "PTY_HARNESS_SELF_TEST_PASS timeout_cleanup=1 injected_failure_cleanup=1 "
        "controlling_terminal=73x19 output_before_exit_waiters=3 cursor_sequences=10 "
        "resize_transients=2 resize_transient_screen_cases=4 settle_deadline=1 "
        "make_fail_fast_iterations=2/5"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
