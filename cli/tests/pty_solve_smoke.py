#!/usr/bin/env python3
"""Linux PTY smoke for the concrete solve workflow. Uses only the Python stdlib."""

import fcntl
import os
from pathlib import Path
import select
import shutil
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time


def run_checked(command: list[str], env: dict[str, str]) -> None:
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
    if result.returncode != 0:
        raise AssertionError(f"command failed: {command!r}\nstdout={result.stdout}\nstderr={result.stderr}")


def wait_for(master: int, process: subprocess.Popen[bytes], output: bytearray, needle: bytes, deadline: float) -> None:
    while needle not in output:
        if time.monotonic() >= deadline:
            raise AssertionError(f"timed out waiting for {needle!r}; tail={bytes(output[-2000:])!r}")
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                pass
        if process.poll() is not None:
            raise AssertionError(f"TUI exited early with {process.returncode}; tail={bytes(output[-2000:])!r}")


def main() -> int:
    if sys.platform != "linux":
        return 0
    interview_binary = Path(sys.argv[1]).resolve()
    practice_binary = Path(sys.argv[2]).resolve()
    repository_root = Path(sys.argv[3]).resolve()
    with tempfile.TemporaryDirectory(prefix="interview-pty-") as temporary:
        root = Path(temporary)
        shutil.copytree(repository_root / "catalog", root / "catalog")
        shutil.copytree(repository_root / "problem_sets", root / "problem_sets")
        shutil.copytree(repository_root / "python", root / "python")
        shutil.copytree(repository_root / "rust", root / "rust")
        fake_runner = root / "python" / "run"
        fake_runner.write_text("#!/bin/sh\nif [ \"${1:-}\" = --list ]; then printf 'smoke-problem\\n'; else printf 'PASS\\n'; fi\n")
        fake_runner.chmod(0o755)
        solution = root / "python" / "smoke.py"
        solution.write_text("print('initial')\n")
        database = root / "smoke.db"
        env = os.environ.copy()
        env["PRACTICE_ROOT"] = str(root)
        base = [str(practice_binary), "--db", str(database)]
        run_checked(base + ["problems", "add", "smoke-problem", "--title", "Smoke Problem", "--difficulty", "Easy", "--topic", "Smoke", "--statement", "Edit and run."], env)
        run_checked(base + ["problems", "adapter", "smoke-problem", "python", "python/smoke.py"], env)
        run_checked(base + ["sets", "create", "smoke-set", "--name", "Smoke Set"], env)
        run_checked(base + ["sets", "add", "smoke-set", "smoke-problem"], env)

        master, slave = os.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        process = subprocess.Popen(
            [str(interview_binary), "--db", str(database), "--set", "smoke-set", "--language", "python"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            close_fds=True,
        )
        os.close(slave)
        output = bytearray()
        deadline = time.monotonic() + 12
        try:
            wait_for(master, process, output, b"Smoke Problem", deadline)
            os.write(master, b"\r")
            wait_for(master, process, output, b"Edit and run", deadline)
            os.write(master, b"\r")
            wait_for(master, process, output, b"Editor", deadline)
            os.write(master, b"iX")
            time.sleep(0.1)
            os.write(master, b"\x1b")
            time.sleep(0.1)
            os.write(master, b"\x13")
            wait_for(master, process, output, b"PASS", deadline)
            with sqlite3.connect(database) as connection:
                assert connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0] == 0
            os.write(master, b"\x1b[20~")
            wait_for(master, process, output, b"progress refreshed", deadline)
            with sqlite3.connect(database) as connection:
                attempts = connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
                assert attempts == 1, attempts
            os.write(master, b" q")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
            assert solution.read_text().startswith("X"), solution.read_text()
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=2)
            os.close(master)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
