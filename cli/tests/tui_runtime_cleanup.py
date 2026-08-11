#!/usr/bin/env python3
"""Linux PTY checks for bounded interview runtime teardown."""

import fcntl
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


ENTER_SCREEN = b"\x1b[?1049h"
LEAVE_SCREEN = b"\x1b[?1049l"
SHOW_CURSOR = b"\x1b[?25h"
CASE_TIMEOUT_SECONDS = 4.0


def drain(master: int, output: bytearray) -> None:
    while True:
        readable, _, _ = select.select([master], [], [], 0)
        if not readable:
            return
        try:
            output.extend(os.read(master, 65536))
        except OSError:
            return


def wait_for_screen(master: int, process: subprocess.Popen[bytes], output: bytearray, deadline: float) -> None:
    while ENTER_SCREEN not in output:
        if time.monotonic() >= deadline:
            raise AssertionError(f"terminal startup exceeded bound; output={bytes(output[-2000:])!r}")
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                pass
        if process.poll() is not None:
            drain(master, output)
            raise AssertionError(f"TUI exited before entering screen: {process.returncode}; output={bytes(output[-2000:])!r}")


def run_case(
    name: str,
    interview_binary: Path,
    repository_root: Path,
    temporary: Path,
    action: str,
    expected_status: int,
) -> None:
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    terminal_before = termios.tcgetattr(slave)
    codex_home = temporary / f"codex-{name}"
    codex_home.mkdir(mode=0o700)
    (codex_home / "fake-mode").write_text("normal", encoding="utf-8")
    environment = os.environ.copy()
    environment["PRACTICE_ROOT"] = str(repository_root)
    environment["CODEX_HOME"] = str(codex_home)
    environment["INTERVIEW_TUTOR_CODEX_EXECUTABLE"] = str(
        repository_root / "cli" / "tests" / "fixtures" / "fake_codex_app_server.py"
    )
    if action == "error":
        environment["INTERVIEW_TUTOR_TEST_ERROR_AFTER_ENTER"] = "1"
    if action == "panic":
        environment["INTERVIEW_TUTOR_TEST_PANIC_AFTER_ENTER"] = "1"

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
    process = subprocess.Popen(
        command,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=environment,
        close_fds=True,
    )
    output = bytearray()
    started = time.monotonic()
    deadline = started + CASE_TIMEOUT_SECONDS
    try:
        if action in {"quit", "sigint", "sigterm"}:
            wait_for_screen(master, process, output, deadline)
            if action == "quit":
                os.write(master, b"q")
            else:
                process.send_signal(signal.SIGINT if action == "sigint" else signal.SIGTERM)
        process.wait(timeout=max(0.1, deadline - time.monotonic()))
        drain(master, output)
        elapsed = time.monotonic() - started
        assert elapsed < CASE_TIMEOUT_SECONDS, (name, elapsed)
        assert process.returncode == expected_status, (
            name,
            process.returncode,
            bytes(output[-2000:]),
        )
        assert ENTER_SCREEN in output, (name, bytes(output[-2000:]))
        assert LEAVE_SCREEN in output, (name, bytes(output[-2000:]))
        assert SHOW_CURSOR in output, (name, bytes(output[-2000:]))
        assert termios.tcgetattr(slave) == terminal_before, name
        assert not (codex_home / "fake-version-probe").exists(), name
        assert not (codex_home / "fake-capture.jsonl").exists(), name
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=1)
        os.close(master)
        os.close(slave)


def main() -> int:
    if sys.platform != "linux":
        return 0
    interview_binary = Path(sys.argv[1]).resolve()
    repository_root = Path(sys.argv[2]).resolve()
    with tempfile.TemporaryDirectory(prefix="interview-runtime-cleanup-") as directory:
        temporary = Path(directory)
        for name, action, expected_status in [
            ("clean-quit", "quit", 0),
            ("startup-error", "error", 1),
            ("panic", "panic", 1),
            ("sigint", "sigint", 130),
            ("sigterm", "sigterm", 143),
        ]:
            run_case(
                name,
                interview_binary,
                repository_root,
                temporary,
                action,
                expected_status,
            )
    print("terminal_cleanup=5 no_codex_probes=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
