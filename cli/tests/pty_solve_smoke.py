#!/usr/bin/env python3
"""Linux PTY smoke for local solve plus the fake Codex app-server. Stdlib only."""

import fcntl
import json
import os
from pathlib import Path
import select
import shutil
import signal
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time

from pty_harness import MAX_CAPTURE_BYTES, Screen, claim_controlling_terminal


def run_checked(command: list[str], env: dict[str, str]) -> None:
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
    if result.returncode != 0:
        raise AssertionError(f"command failed: {command!r}\nstdout={result.stdout}\nstderr={result.stderr}")


def drain_terminal(master: int, output: bytearray, screen: Screen) -> None:
    readable, _, _ = select.select([master], [], [], 0.05)
    if not readable:
        return
    try:
        chunk = os.read(master, 65536)
    except OSError:
        return
    output.extend(chunk)
    if len(output) > MAX_CAPTURE_BYTES:
        del output[: len(output) - MAX_CAPTURE_BYTES]
    screen.feed(chunk)


def wait_for(
    master: int,
    process: subprocess.Popen[bytes],
    output: bytearray,
    screen: Screen,
    marker: str,
    deadline: float,
) -> None:
    while marker not in screen.text():
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"timed out waiting for rendered {marker!r}; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )
        drain_terminal(master, output, screen)
        if process.poll() is not None:
            raise AssertionError(
                f"TUI exited early with {process.returncode}; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )


def launch(
    interview_binary: Path,
    database: Path,
    env: dict[str, str],
) -> tuple[int, subprocess.Popen[bytes], bytearray, Screen]:
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    process = subprocess.Popen(
        [
            str(interview_binary),
            "--db",
            str(database),
            "--set",
            "smoke-set",
            "--language",
            "python",
        ],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        close_fds=True,
        preexec_fn=claim_controlling_terminal,
    )
    os.close(slave)
    return master, process, bytearray(), Screen(120, 40)


def open_solve(
    master: int,
    process: subprocess.Popen[bytes],
    output: bytearray,
    screen: Screen,
    deadline: float,
) -> None:
    wait_for(master, process, output, screen, "Smoke Problem", deadline)
    os.write(master, b"\r")
    wait_for(master, process, output, screen, "Edit and run", deadline)
    os.write(master, b"\r")
    wait_for(master, process, output, screen, "Editor", deadline)


def captured_turns(capture: Path) -> list[dict]:
    if not capture.exists():
        return []
    records = [json.loads(line) for line in capture.read_text().splitlines()]
    return [record["json"] for record in records if record.get("json", {}).get("method") == "turn/start"]


def wait_for_turn_count(
    master: int,
    process: subprocess.Popen[bytes],
    output: bytearray,
    screen: Screen,
    capture: Path,
    count: int,
    deadline: float,
) -> None:
    while len(captured_turns(capture)) < count:
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"timed out waiting for {count} fake turns; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )
        drain_terminal(master, output, screen)
        if process.poll() is not None:
            raise AssertionError(
                f"TUI exited early with {process.returncode}; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )


def wait_for_attempt_count(
    master: int,
    process: subprocess.Popen[bytes],
    output: bytearray,
    screen: Screen,
    database: Path,
    count: int,
    deadline: float,
) -> None:
    attempts = 0
    while attempts < count:
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"timed out waiting for {count} attempts; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )
        with sqlite3.connect(database) as connection:
            attempts = connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
        drain_terminal(master, output, screen)
        if process.poll() is not None:
            raise AssertionError(
                f"TUI exited early with {process.returncode}; "
                f"screen={screen.text()[-3000:]!r}; raw_tail={bytes(output[-3000:])!r}"
            )
    assert attempts == count, attempts


def turn_payload(turn: dict) -> dict:
    text = turn["params"]["input"][0]["text"]
    return json.loads(text.split("INPUT_JSON:", 1)[1])


def stop_process(master: int, process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
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
    fake_pi = repository_root / "cli" / "tests" / "fixtures" / "fake_pi_rpc.py"
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
        (codex_home / "fake-mode").write_text("hold-interviewer")
        capture = codex_home / "fake-capture.jsonl"
        env = os.environ.copy()
        env["PRACTICE_ROOT"] = str(root)
        env["CODEX_HOME"] = str(codex_home)
        env["INTERVIEW_TUTOR_CODEX_EXECUTABLE"] = str(fake_codex)
        env["INTERVIEW_TUTOR_INTERVIEWER"] = "codex"
        base = [str(practice_binary), "--db", str(database)]
        run_checked(base + ["problems", "add", "smoke-problem", "--title", "Smoke Problem", "--difficulty", "Easy", "--topic", "Smoke", "--statement", "Edit and run."], env)
        run_checked(base + ["problems", "adapter", "smoke-problem", "python", "python/smoke.py"], env)
        run_checked(base + ["sets", "create", "smoke-set", "--name", "Smoke Set"], env)
        run_checked(base + ["sets", "add", "smoke-set", "smoke-problem"], env)

        pi_home = root / "pi-home"
        pi_home.mkdir(mode=0o700)
        (pi_home / "fake-mode").write_text("normal")
        pi_env = env.copy()
        pi_env.pop("INTERVIEW_TUTOR_INTERVIEWER")
        pi_env["PI_CODING_AGENT_DIR"] = str(pi_home)
        pi_env["INTERVIEW_TUTOR_PI_EXECUTABLE"] = str(fake_pi)
        master, process, output, screen = launch(interview_binary, database, pi_env)
        try:
            open_solve(master, process, output, screen, deadline)
            os.write(master, b"\t\t\ti")
            wait_for(master, process, output, screen, "Privacy disclosure", deadline)
            os.write(master, b"y")
            wait_for(master, process, output, screen, "Pi: ready", deadline)
            os.write(master, b"Why?\r")
            wait_for(master, process, output, screen, "What invariant holds?", deadline)
            os.write(master, b"\x11")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
        finally:
            stop_process(master, process)
        pi_records = [
            json.loads(line)
            for line in (pi_home / "fake-capture.jsonl").read_text().splitlines()
        ]
        pi_processes = [record for record in pi_records if record.get("kind") == "process"]
        assert len(pi_processes) == 1, pi_processes
        assert all(not Path(record["cwd"]).exists() for record in pi_processes)

        large_source = "payload = '" + ("x" * (256 * 1024)) + "'\n"
        solution.write_text(large_source)
        (pi_home / "fake-mode").write_text("no-read-after-get-state")
        master, process, output, screen = launch(interview_binary, database, pi_env)
        try:
            open_solve(master, process, output, screen, deadline)
            os.write(master, b"\t\t\ti")
            wait_for(master, process, output, screen, "Privacy disclosure", deadline)
            os.write(master, b"y")
            wait_for(master, process, output, screen, "Pi: ready", deadline)
            os.write(master, b"Why?\r")
            capture_path = pi_home / "fake-capture.jsonl"
            while '"kind": "state-sent"' not in capture_path.read_text():
                if time.monotonic() >= deadline:
                    raise AssertionError("fake Pi did not send state before Ctrl-C")
                time.sleep(0.01)
            process.send_signal(signal.SIGINT)
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 130, process.returncode
        finally:
            stop_process(master, process)
        blocked_records = [
            json.loads(line)
            for line in (pi_home / "fake-capture.jsonl").read_text().splitlines()
        ]
        blocked_process = [
            record for record in blocked_records if record.get("kind") == "process"
        ][-1]
        assert not Path(blocked_process["cwd"]).exists(), blocked_process
        assert not Path(f"/proc/{blocked_process['pid']}").exists(), blocked_process
        solution.write_text(recorded_source)
        (pi_home / "fake-mode").write_text("normal")

        master, process, output, screen = launch(interview_binary, database, env)
        try:
            open_solve(master, process, output, screen, deadline)
            assert captured_turns(capture) == []
            os.write(master, b"\t\t\t")
            time.sleep(0.1)
            os.write(master, b"i")
            wait_for(master, process, output, screen, "Privacy disclosure", deadline)
            assert captured_turns(capture) == []
            os.write(master, b"y")
            wait_for(master, process, output, screen, "ready · memory", deadline)
            assert captured_turns(capture) == []

            os.write(master, b"Why this invariant?\r")
            wait_for_turn_count(master, process, output, screen, capture, 1, deadline)
            held = codex_home / "fake-held-turn"
            while not held.exists():
                if time.monotonic() >= deadline:
                    raise AssertionError("fake interviewer turn was not held")
                time.sleep(0.01)

            os.write(master, b"\x1b[20~")
            wait_for_attempt_count(
                master, process, output, screen, database, 1, deadline
            )
            assert len(captured_turns(capture)) == 1
            os.write(master, b"\tiX")
            wait_for(master, process, output, screen, "Xprint", deadline)
            os.write(master, b"\x1b")
            wait_for(master, process, output, screen, "Space t test", deadline)
            assert solution.read_text() == recorded_source
            (codex_home / "fake-release-turn").write_text("release")

            wait_for_turn_count(master, process, output, screen, capture, 2, deadline)
            wait_for(
                master,
                process,
                output,
                screen,
                "RECORDED_SUBMISSION_REVIEW",
                deadline,
            )
            wait_for(
                master,
                process,
                output,
                screen,
                "Submission review · recorded",
                deadline,
            )
            wait_for(master, process, output, screen, "revision 0", deadline)
            with sqlite3.connect(database) as connection:
                attempts = connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
            assert attempts == 1, attempts

            turns = captured_turns(capture)
            assert len(turns) == 2, len(turns)
            payloads = [turn_payload(turn) for turn in turns]
            expected_fields = {"statement", "source", "latestTestOutput", "transcript", "userQuestion"}
            assert all(set(payload) == expected_fields for payload in payloads)
            assert payloads[0]["userQuestion"] == "Why this invariant?"
            assert payloads[1]["source"] == recorded_source
            assert payloads[1]["source"] != "X" + recorded_source
            assert payloads[1]["userQuestion"] == ""
            assert turns[0]["params"]["threadId"] != turns[1]["params"]["threadId"]

            os.write(master, b"\t")
            wait_for(master, process, output, screen, "Problem / Examples [active]", deadline)
            os.write(master, b"\t")
            wait_for(master, process, output, screen, "Output / Test [active]", deadline)
            os.write(master, b"\t")
            wait_for(master, process, output, screen, "Interview [active]", deadline)
            os.write(master, b" r")
            wait_for(master, process, output, screen, "offline · memory", deadline)
            os.write(master, b"\t q q")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
        finally:
            stop_process(master, process)

        records = [json.loads(line) for line in capture.read_text().splitlines()]
        process_cwds = [Path(record["cwd"]) for record in records if record.get("kind") == "process"]
        assert process_cwds and all(not cwd.exists() for cwd in process_cwds)
        persisted = database.read_bytes() + solution.read_bytes()
        for transcript_text in [b"Why this invariant?", b"What invariant holds?", b"Level 1 invariant", b"Submission reviewed", b"RECORDED_SUBMISSION_REVIEW"]:
            assert transcript_text not in persisted

        master, process, output, screen = launch(interview_binary, database, env)
        try:
            open_solve(master, process, output, screen, deadline)
            os.write(master, b"\t\t\t")
            wait_for(master, process, output, screen, "Interview [active]", deadline)
            rendered = screen.text()
            assert "What invariant holds?" not in rendered
            assert "Level 1 invariant" not in rendered
            assert "Submission reviewed" not in rendered
            assert "RECORDED_SUBMISSION_REVIEW" not in rendered
            os.write(master, b"i")
            wait_for(master, process, output, screen, "Privacy disclosure", deadline)
            assert len(captured_turns(capture)) == 2
            os.write(master, b"n\t q")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
            assert process.returncode == 0, process.returncode
        finally:
            stop_process(master, process)

        assert solution.read_text() == recorded_source
        assert time.monotonic() - started <= 24
        print("default_pi_turns=1 pi_blocked_write_sigint=130 explicit_codex_turns=2 attempts=1 recorded_revision=0 transcript_after_relaunch=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
