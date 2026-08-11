#!/usr/bin/env python3
"""Linux PTY smoke for local solve plus the fake Codex app-server. Stdlib only."""

import fcntl
import json
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
            raise AssertionError(f"timed out waiting for {needle!r}; tail={bytes(output[-3000:])!r}")
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                pass
        if process.poll() is not None:
            raise AssertionError(f"TUI exited early with {process.returncode}; tail={bytes(output[-3000:])!r}")


def wait_for_after(master: int, process: subprocess.Popen[bytes], output: bytearray, start: int, needle: bytes, deadline: float) -> None:
    while needle not in output[start:]:
        if time.monotonic() >= deadline:
            raise AssertionError(f"timed out waiting for new {needle!r}; tail={bytes(output[-3000:])!r}")
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                pass
        if process.poll() is not None:
            raise AssertionError(f"TUI exited early with {process.returncode}; tail={bytes(output[-3000:])!r}")


def launch(interview_binary: Path, database: Path, env: dict[str, str]) -> tuple[int, subprocess.Popen[bytes], bytearray]:
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
    return master, process, bytearray()


def open_solve(master: int, process: subprocess.Popen[bytes], output: bytearray, deadline: float) -> None:
    wait_for(master, process, output, b"Smoke Problem", deadline)
    os.write(master, b"\r")
    wait_for(master, process, output, b"Edit and run", deadline)
    os.write(master, b"\r")
    wait_for(master, process, output, b"Editor", deadline)


def captured_turns(capture: Path) -> list[dict]:
    if not capture.exists():
        return []
    records = [json.loads(line) for line in capture.read_text().splitlines()]
    return [record["json"] for record in records if record.get("json", {}).get("method") == "turn/start"]


def wait_for_turn_count(master: int, process: subprocess.Popen[bytes], output: bytearray, capture: Path, count: int, deadline: float) -> None:
    while len(captured_turns(capture)) < count:
        if time.monotonic() >= deadline:
            raise AssertionError(f"timed out waiting for {count} fake turns; tail={bytes(output[-3000:])!r}")
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                pass
        if process.poll() is not None:
            raise AssertionError(f"TUI exited early with {process.returncode}; tail={bytes(output[-3000:])!r}")


def turn_payload(turn: dict) -> dict:
    text = turn["params"]["input"][0]["text"]
    return json.loads(text.split("INPUT_JSON:", 1)[1])


def stop_process(master: int, process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        process.kill()
        process.wait(timeout=2)
    os.close(master)


def main() -> int:
    if sys.platform != "linux":
        return 0
    started = time.monotonic()
    deadline = started + 18
    interview_binary = Path(sys.argv[1]).resolve()
    practice_binary = Path(sys.argv[2]).resolve()
    repository_root = Path(sys.argv[3]).resolve()
    fake_codex = repository_root / "cli" / "tests" / "fixtures" / "fake_codex_app_server.py"
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
        recorded_source = "print('initial')\n"
        solution.write_text(recorded_source)
        database = root / "smoke.db"
        codex_home = root / "codex-home"
        codex_home.mkdir(mode=0o700)
        (codex_home / "fake-mode").write_text("normal")
        capture = codex_home / "fake-capture.jsonl"
        env = os.environ.copy()
        env["PRACTICE_ROOT"] = str(root)
        env["CODEX_HOME"] = str(codex_home)
        env["INTERVIEW_TUTOR_CODEX_EXECUTABLE"] = str(fake_codex)
        base = [str(practice_binary), "--db", str(database)]
        run_checked(base + ["problems", "add", "smoke-problem", "--title", "Smoke Problem", "--difficulty", "Easy", "--topic", "Smoke", "--statement", "Edit and run."], env)
        run_checked(base + ["problems", "adapter", "smoke-problem", "python", "python/smoke.py"], env)
        run_checked(base + ["sets", "create", "smoke-set", "--name", "Smoke Set"], env)
        run_checked(base + ["sets", "add", "smoke-set", "smoke-problem"], env)

        master, process, output = launch(interview_binary, database, env)
        try:
            open_solve(master, process, output, deadline)
            assert captured_turns(capture) == []
            os.write(master, b"\t\t\t")
            time.sleep(0.1)
            os.write(master, b"i")
            wait_for(master, process, output, b"Privacy disclosure", deadline)
            assert captured_turns(capture) == []
            mark = len(output)
            os.write(master, b"y")
            wait_for_after(master, process, output, mark, b"ready \xc2\xb7 memory", deadline)
            assert captured_turns(capture) == []

            os.write(master, b"Why this invariant?\r")
            wait_for_turn_count(master, process, output, capture, 1, deadline)
            wait_for(master, process, output, b"Interviewer:", deadline)
            os.write(master, b" h")
            wait_for_turn_count(master, process, output, capture, 2, deadline)
            wait_for(master, process, output, b"Hinter:", deadline)
            os.write(master, b"\x1b[20~")
            wait_for_turn_count(master, process, output, capture, 3, deadline)
            wait_for(master, process, output, b"Submission review", deadline)
            with sqlite3.connect(database) as connection:
                attempts = connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
            assert attempts == 1, attempts

            turns = captured_turns(capture)
            assert len(turns) == 3, len(turns)
            payloads = [turn_payload(turn) for turn in turns]
            expected_fields = {"statement", "source", "latestTestOutput", "transcript", "userQuestion"}
            assert all(set(payload) == expected_fields for payload in payloads)
            assert payloads[0]["userQuestion"] == "Why this invariant?"
            assert payloads[1]["transcript"] == ""
            assert payloads[2]["source"] == recorded_source
            assert payloads[2]["userQuestion"] == ""
            assert turns[0]["params"]["threadId"] == turns[2]["params"]["threadId"]
            assert turns[0]["params"]["threadId"] != turns[1]["params"]["threadId"]

            mark = len(output)
            os.write(master, b" r")
            wait_for_after(master, process, output, mark, b"offline \xc2\xb7 memory", deadline)
            os.write(master, b"\t q")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
        finally:
            stop_process(master, process)

        records = [json.loads(line) for line in capture.read_text().splitlines()]
        process_cwds = [Path(record["cwd"]) for record in records if record.get("kind") == "process"]
        assert process_cwds and all(not cwd.exists() for cwd in process_cwds)
        persisted = database.read_bytes() + solution.read_bytes()
        for transcript_text in [b"Why this invariant?", b"What invariant holds?", b"Level 1 invariant", b"Submission reviewed"]:
            assert transcript_text not in persisted

        master, process, output = launch(interview_binary, database, env)
        try:
            open_solve(master, process, output, deadline)
            os.write(master, b"\t\t\t")
            time.sleep(0.1)
            assert b"What invariant holds?" not in output
            assert b"Level 1 invariant" not in output
            assert b"Submission reviewed" not in output
            os.write(master, b"i")
            wait_for(master, process, output, b"Privacy disclosure", deadline)
            assert len(captured_turns(capture)) == 3
            os.write(master, b"n\t q")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
        finally:
            stop_process(master, process)

        assert solution.read_text() == recorded_source
        assert time.monotonic() - started <= 20
        print("fake_turns=3 attempts=1 transcript_after_relaunch=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
