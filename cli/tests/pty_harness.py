#!/usr/bin/env python3
"""Bounded Linux PTY helpers with a small screen model. Stdlib only."""

from __future__ import annotations

import atexit
import codecs
from collections import deque
import fcntl
import json
import os
from pathlib import Path
import select
import shutil
import signal
import struct
import subprocess
import termios
import time
import unicodedata
from typing import Callable, NamedTuple

MAX_CAPTURE_BYTES = 4 * 1024 * 1024
MAX_OSC_EVENTS = 128
MAX_OSC_CHARS = 4096
MAX_CURSOR_REQUESTS = 4096
FAILURE_TAIL_CHARS = 4000
POLL_SECONDS = 0.025


class HarnessTimeout(TimeoutError):
    pass


class HardDeadline:
    """A nestable main-thread wall deadline that restores the prior alarm state."""

    def __init__(self, seconds: float, label: str) -> None:
        assert 0 < seconds <= 120.0
        assert label
        self.seconds = seconds
        self.label = label
        self.started = 0.0
        self.previous_handler: object = signal.SIG_DFL
        self.previous_timer = (0.0, 0.0)
        self.active = False

    def __enter__(self) -> "HardDeadline":
        assert not self.active
        self.started = time.monotonic()
        self.previous_handler = signal.getsignal(signal.SIGALRM)
        self.previous_timer = signal.getitimer(signal.ITIMER_REAL)

        def expired(_signal: int, _frame: object) -> None:
            raise HarnessTimeout(
                f"hard deadline exceeded: {self.label} ({self.seconds:.3f}s)"
            )

        signal.signal(signal.SIGALRM, expired)
        previous_remaining = self.previous_timer[0]
        effective = (
            min(self.seconds, previous_remaining)
            if previous_remaining > 0
            else self.seconds
        )
        signal.setitimer(signal.ITIMER_REAL, effective)
        self.active = True
        return self

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        if not self.active:
            return
        signal.setitimer(signal.ITIMER_REAL, 0.0)
        signal.signal(signal.SIGALRM, self.previous_handler)
        elapsed = time.monotonic() - self.started
        previous_remaining, previous_interval = self.previous_timer
        if previous_remaining > 0:
            remaining = max(0.000_001, previous_remaining - elapsed)
            signal.setitimer(signal.ITIMER_REAL, remaining, previous_interval)
        self.active = False


def hard_deadline(seconds: float, label: str) -> HardDeadline:
    return HardDeadline(seconds, label)


def _cell_width(character: str) -> int:
    if unicodedata.combining(character):
        return 0
    return 2 if unicodedata.east_asian_width(character) in {"F", "W"} else 1


class CursorRequest(NamedTuple):
    raw_parameters: str
    final: str
    requested_x: int
    requested_y: int
    columns: int
    rows: int


class Screen:
    """Enough VT state for crossterm/ratatui marker and cursor assertions."""

    def __init__(self, columns: int, rows: int) -> None:
        assert columns > 0
        assert rows > 0
        self.columns = columns
        self.rows = rows
        self.x = 0
        self.y = 0
        self.saved = (0, 0)
        self.cells = [[" " for _ in range(columns)] for _ in range(rows)]
        self.osc_events: deque[str] = deque(maxlen=MAX_OSC_EVENTS)
        self.osc_event_count = 0
        self.cursor_requests: deque[CursorRequest] = deque(maxlen=MAX_CURSOR_REQUESTS)
        self.invalid_cursor_request: CursorRequest | None = None
        self._decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self._state = "text"
        self._csi = ""
        self._osc = ""
        self._osc_overflow = False

    def resize(self, columns: int, rows: int) -> None:
        assert columns > 0
        assert rows > 0
        resized = [[" " for _ in range(columns)] for _ in range(rows)]
        for row in range(min(rows, self.rows)):
            for column in range(min(columns, self.columns)):
                resized[row][column] = self.cells[row][column]
        self.columns = columns
        self.rows = rows
        self.cells = resized
        self.x = min(self.x, columns - 1)
        self.y = min(self.y, rows - 1)
        self.saved = (min(self.saved[0], columns - 1), min(self.saved[1], rows - 1))

    def feed(self, data: bytes) -> None:
        for character in self._decoder.decode(data):
            if self._state == "osc":
                if character == "\a":
                    self._finish_osc()
                    self._state = "text"
                elif character == "\x1b":
                    self._state = "osc-escape"
                else:
                    self._append_osc(character)
                continue
            if self._state == "osc-escape":
                if character == "\\":
                    self._finish_osc()
                    self._state = "text"
                else:
                    self._append_osc(character)
                    self._state = "osc"
                continue
            if self._state == "escape":
                if character == "[":
                    self._state = "csi"
                    self._csi = ""
                elif character == "]":
                    self._state = "osc"
                    self._osc = ""
                    self._osc_overflow = False
                else:
                    self._state = "text"
                continue
            if self._state == "csi":
                self._csi += character
                if "@" <= character <= "~":
                    self._apply_csi(self._csi)
                    self._state = "text"
                    self._csi = ""
                elif len(self._csi) > 128:
                    self._state = "text"
                    self._csi = ""
                continue
            if character == "\x1b":
                self._state = "escape"
            elif character == "\r":
                self.x = 0
            elif character == "\n":
                self.y = min(self.rows - 1, self.y + 1)
            elif character == "\b":
                self.x = max(0, self.x - 1)
            elif character >= " ":
                self._put(character)

    def _append_osc(self, character: str) -> None:
        if self._osc_overflow:
            return
        if len(self._osc) >= MAX_OSC_CHARS:
            self._osc = ""
            self._osc_overflow = True
            return
        self._osc += character

    def _finish_osc(self) -> None:
        self.osc_event_count += 1
        event = "<overflow>" if self._osc_overflow else self._osc
        self.osc_events.append(event)
        self._osc = ""
        self._osc_overflow = False

    def _numbers(self, parameters: str) -> list[int]:
        parameters = parameters.lstrip("?<>=!")
        if not parameters:
            return [0]
        result = []
        for part in parameters.split(";"):
            try:
                result.append(int(part or "0"))
            except ValueError:
                result.append(0)
        return result

    def _request_cursor(self, x: int, y: int, sequence: str) -> None:
        request = CursorRequest(
            raw_parameters=sequence[:-1],
            final=sequence[-1],
            requested_x=x,
            requested_y=y,
            columns=self.columns,
            rows=self.rows,
        )
        self.cursor_requests.append(request)
        if self.invalid_cursor_request is None and not (
            0 <= x < self.columns and 0 <= y < self.rows
        ):
            self.invalid_cursor_request = request

    def _apply_csi(self, sequence: str) -> None:
        final = sequence[-1]
        values = self._numbers(sequence[:-1])
        first = values[0] if values else 0
        distance = max(1, first)
        if final in {"H", "f"}:
            row = max(1, values[0] if values else 1)
            column = max(1, values[1] if len(values) > 1 else 1)
            requested_x = column - 1
            requested_y = row - 1
            self._request_cursor(requested_x, requested_y, sequence)
            self.x = min(self.columns - 1, requested_x)
            self.y = min(self.rows - 1, requested_y)
        elif final == "A":
            requested_y = self.y - distance
            self._request_cursor(self.x, requested_y, sequence)
            self.y = max(0, requested_y)
        elif final in {"B", "e"}:
            requested_y = self.y + distance
            self._request_cursor(self.x, requested_y, sequence)
            self.y = min(self.rows - 1, requested_y)
        elif final == "C":
            requested_x = self.x + distance
            self._request_cursor(requested_x, self.y, sequence)
            self.x = min(self.columns - 1, requested_x)
        elif final == "D":
            requested_x = self.x - distance
            self._request_cursor(requested_x, self.y, sequence)
            self.x = max(0, requested_x)
        elif final in {"G", "`"}:
            requested_x = max(1, first) - 1
            self._request_cursor(requested_x, self.y, sequence)
            self.x = min(self.columns - 1, requested_x)
        elif final == "d":
            requested_y = max(1, first) - 1
            self._request_cursor(self.x, requested_y, sequence)
            self.y = min(self.rows - 1, requested_y)
        elif final == "J":
            if first in {2, 3}:
                self.cells = [
                    [" " for _ in range(self.columns)] for _ in range(self.rows)
                ]
                self.x = 0
                self.y = 0
            elif first == 0:
                self._erase_to_end()
            elif first == 1:
                for row in range(self.y):
                    self.cells[row] = [" " for _ in range(self.columns)]
                for column in range(self.x + 1):
                    self.cells[self.y][column] = " "
        elif final == "K":
            if first == 0:
                for column in range(self.x, self.columns):
                    self.cells[self.y][column] = " "
            elif first == 1:
                for column in range(self.x + 1):
                    self.cells[self.y][column] = " "
            elif first == 2:
                self.cells[self.y] = [" " for _ in range(self.columns)]
        elif final == "s":
            self.saved = (self.x, self.y)
        elif final == "u":
            requested_x, requested_y = self.saved
            self._request_cursor(requested_x, requested_y, sequence)
            self.x = min(self.columns - 1, max(0, requested_x))
            self.y = min(self.rows - 1, max(0, requested_y))

    def _erase_to_end(self) -> None:
        for column in range(self.x, self.columns):
            self.cells[self.y][column] = " "
        for row in range(self.y + 1, self.rows):
            self.cells[row] = [" " for _ in range(self.columns)]

    def _put(self, character: str) -> None:
        width = _cell_width(character)
        if width == 0:
            if self.x > 0:
                self.cells[self.y][self.x - 1] += character
            return
        if self.x >= self.columns:
            self.x = 0
            self.y = min(self.rows - 1, self.y + 1)
        self.cells[self.y][self.x] = character
        for continuation in range(1, width):
            if self.x + continuation < self.columns:
                self.cells[self.y][self.x + continuation] = ""
        self.x += width

    def text(self) -> str:
        return "\n".join("".join(row).rstrip() for row in self.cells)


class PtySession:
    def __init__(
        self,
        command: list[str],
        environment: dict[str, str],
        columns: int,
        rows: int,
        case_timeout: float = 20.0,
        keep_slave: bool = False,
    ) -> None:
        assert command
        assert columns > 0
        assert rows > 0
        assert 0 < case_timeout <= 20.0
        self.started = time.monotonic()
        self.deadline = self.started + case_timeout
        self.master, slave = os.openpty()
        self.slave = slave if keep_slave else None
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.terminal_before = termios.tcgetattr(slave)
        self.screen = Screen(columns, rows)
        self.output = bytearray()
        self._raw_watches: dict[bytes, bool] = {}
        self._watch_overlap = b""
        self._closed = False
        try:
            self.process = subprocess.Popen(
                command,
                stdin=slave,
                stdout=slave,
                stderr=slave,
                env=environment,
                close_fds=True,
                process_group=0,
            )
        except BaseException:
            os.close(self.master)
            os.close(slave)
            raise
        atexit.register(self.close)
        if not keep_slave:
            os.close(slave)

    def __enter__(self) -> "PtySession":
        return self

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        self.close()

    def watch_raw(self, sequence: bytes) -> None:
        assert sequence
        self._raw_watches[sequence] = sequence in self.output

    def raw_sequence_seen(self, sequence: bytes) -> bool:
        assert sequence in self._raw_watches, (
            "raw sequence must be watched before hostile output"
        )
        return self._raw_watches[sequence]

    def _track_raw(self, chunk: bytes) -> None:
        combined = self._watch_overlap + chunk
        for sequence, seen in self._raw_watches.items():
            if not seen and sequence in combined:
                self._raw_watches[sequence] = True
        longest = max((len(sequence) for sequence in self._raw_watches), default=1)
        overlap_count = max(0, longest - 1)
        self._watch_overlap = combined[-overlap_count:] if overlap_count else b""

    def _failure_tail(self) -> str:
        text = self.screen.text()
        history = bytes(self.output[-FAILURE_TAIL_CHARS:]).decode("utf-8", "replace")
        return f"screen={text[-FAILURE_TAIL_CHARS:]!r}; raw_tail={history!r}"

    def drain(self, wait: float = 0.0) -> bool:
        readable, _, _ = select.select([self.master], [], [], wait)
        received = False
        while readable:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                break
            if not chunk:
                break
            received = True
            self._track_raw(chunk)
            self.output.extend(chunk)
            if len(self.output) > MAX_CAPTURE_BYTES:
                del self.output[: len(self.output) - MAX_CAPTURE_BYTES]
            self.screen.feed(chunk)
            readable, _, _ = select.select([self.master], [], [], 0)
        return received

    def wait_screen(self, marker: str, timeout: float | None = None) -> None:
        deadline = (
            min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        )
        while marker not in self.screen.text():
            if time.monotonic() >= deadline:
                raise AssertionError(
                    f"timed out waiting for screen marker {marker!r}; {self._failure_tail()}"
                )
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(
                    f"process exited with {status} before screen marker {marker!r}; "
                    f"{self._failure_tail()}"
                )

    def wait_history(self, marker: bytes, timeout: float | None = None) -> None:
        deadline = (
            min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        )
        while marker not in self.output:
            if time.monotonic() >= deadline:
                raise AssertionError(
                    f"timed out waiting for output marker {marker!r}; {self._failure_tail()}"
                )
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(
                    f"process exited with {status} before output marker {marker!r}; "
                    f"{self._failure_tail()}"
                )

    def wait_predicate(
        self,
        description: str,
        predicate: Callable[[], bool],
        timeout: float | None = None,
    ) -> None:
        deadline = (
            min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        )
        while not predicate():
            if time.monotonic() >= deadline:
                raise AssertionError(
                    f"timed out waiting for {description}; {self._failure_tail()}"
                )
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(
                    f"process exited with {status} while waiting for {description}; "
                    f"{self._failure_tail()}"
                )

    def send(self, keys: bytes) -> None:
        assert self.process.poll() is None
        os.write(self.master, keys)

    def resize(self, columns: int, rows: int) -> None:
        assert columns > 0
        assert rows > 0
        self.screen.resize(columns, rows)
        fcntl.ioctl(
            self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0)
        )
        os.killpg(self.process.pid, signal.SIGWINCH)

    def settle(self, debounce: float = 0.075) -> None:
        assert 0 < debounce <= 0.1
        end = min(self.deadline, time.monotonic() + debounce)
        while time.monotonic() < end:
            self.drain(min(POLL_SECONDS, end - time.monotonic()))

    def wait_exit(self, expected_status: int, timeout: float | None = None) -> float:
        deadline = (
            min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        )
        remaining = max(0.05, deadline - time.monotonic())
        self.process.wait(timeout=remaining)
        self.drain()
        if self.process.returncode != expected_status:
            raise AssertionError(
                f"expected status {expected_status}, got {self.process.returncode}; "
                f"{self._failure_tail()}"
            )
        elapsed = time.monotonic() - self.started
        assert elapsed <= 20.0, elapsed
        return elapsed

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            atexit.unregister(self.close)
        except Exception:
            pass
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.kill(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait(timeout=2)
        try:
            os.close(self.master)
        except OSError:
            pass
        if self.slave is not None:
            try:
                os.close(self.slave)
            except OSError:
                pass
            self.slave = None


def run_checked(
    command: list[str],
    environment: dict[str, str],
    expected_status: int = 0,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        env=environment,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    if result.returncode != expected_status:
        stdout = result.stdout[-2000:]
        stderr = result.stderr[-2000:]
        raise AssertionError(
            f"command {command!r} returned {result.returncode}, expected {expected_status}; "
            f"stdout={stdout!r}; stderr={stderr!r}"
        )
    return result


def kill_fixture_process(pid: int) -> None:
    if pid <= 0:
        return
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            return
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    deadline = time.monotonic() + 1.0
    while Path(f"/proc/{pid}").exists() and time.monotonic() < deadline:
        time.sleep(0.01)
    if Path(f"/proc/{pid}").exists():
        raise AssertionError(f"fixture process {pid} survived explicit cleanup")


class CleanupRegistry:
    """Failure-safe registry for fixture PIDs and Codex process/temp artifacts."""

    def __init__(self) -> None:
        self.pid_files: set[Path] = set()
        self.codex_homes: set[Path] = set()
        self.temp_paths: set[Path] = set()
        self.closed = False
        atexit.register(self._atexit_cleanup)

    def register_pid_file(self, path: Path) -> None:
        self.pid_files.add(path)

    def register_codex_home(self, path: Path) -> None:
        self.codex_homes.add(path)

    def register_temp_path(self, path: Path) -> None:
        self.temp_paths.add(path)

    def _pid_values(self) -> set[int]:
        pids: set[int] = set()
        paths = set(self.pid_files)
        for home in self.codex_homes:
            if home.exists():
                paths.update(home.glob("*pid*"))
                capture = home / "fake-capture.jsonl"
                if capture.exists():
                    for line in capture.read_text(encoding="utf-8").splitlines():
                        try:
                            record = json.loads(line)
                        except json.JSONDecodeError:
                            continue
                        if record.get("kind") == "process":
                            pids.add(int(record["pid"]))
                            self.temp_paths.add(Path(record["cwd"]))
        for path in paths:
            try:
                value = path.read_text(encoding="utf-8").strip().split()[0]
                pids.add(int(value))
            except (FileNotFoundError, IndexError, ValueError):
                continue
        return pids

    def finalize_case(self) -> None:
        errors: list[str] = []
        for home in self.codex_homes:
            capture = home / "fake-capture.jsonl"
            if not capture.exists():
                continue
            for line in capture.read_text(encoding="utf-8").splitlines():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if record.get("kind") != "process":
                    continue
                cwd = Path(record["cwd"])
                pid = int(record["pid"])
                if cwd.exists():
                    errors.append(f"Codex temporary directory leaked: {cwd}")
                if Path(f"/proc/{pid}").exists():
                    errors.append(f"Codex process leaked: {pid}")
        self._cleanup_registered()
        self.pid_files.clear()
        self.codex_homes.clear()
        self.temp_paths.clear()
        if errors:
            raise AssertionError("; ".join(errors))

    def _cleanup_registered(self) -> None:
        cleanup_errors: list[str] = []
        for pid in self._pid_values():
            try:
                kill_fixture_process(pid)
            except AssertionError as error:
                cleanup_errors.append(str(error))
        for path in sorted(
            self.temp_paths, key=lambda item: len(item.parts), reverse=True
        ):
            try:
                if path.is_dir():
                    shutil.rmtree(path)
                else:
                    path.unlink(missing_ok=True)
            except OSError as error:
                cleanup_errors.append(f"cannot remove fixture path {path}: {error}")
        if cleanup_errors:
            raise AssertionError("; ".join(cleanup_errors))

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        try:
            atexit.unregister(self._atexit_cleanup)
        except Exception:
            pass
        self._cleanup_registered()

    def _atexit_cleanup(self) -> None:
        try:
            self._cleanup_registered()
        except BaseException:
            pass
