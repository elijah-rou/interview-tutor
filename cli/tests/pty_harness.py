#!/usr/bin/env python3
"""Bounded Linux PTY helpers with a small screen model. Stdlib only."""

from __future__ import annotations

import codecs
import fcntl
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import termios
import time
import unicodedata

MAX_CAPTURE_BYTES = 4 * 1024 * 1024
FAILURE_TAIL_CHARS = 4000
POLL_SECONDS = 0.025


def _cell_width(character: str) -> int:
    if unicodedata.combining(character):
        return 0
    return 2 if unicodedata.east_asian_width(character) in {"F", "W"} else 1


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
        self._decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self._state = "text"
        self._csi = ""

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

    def feed(self, data: bytes) -> None:
        for character in self._decoder.decode(data):
            if self._state == "osc":
                if character == "\a":
                    self._state = "text"
                elif character == "\x1b":
                    self._state = "osc-escape"
                continue
            if self._state == "osc-escape":
                self._state = "text" if character == "\\" else "osc"
                continue
            if self._state == "escape":
                if character == "[":
                    self._state = "csi"
                    self._csi = ""
                elif character == "]":
                    self._state = "osc"
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

    def _apply_csi(self, sequence: str) -> None:
        final = sequence[-1]
        values = self._numbers(sequence[:-1])
        first = values[0] if values else 0
        distance = max(1, first)
        if final in {"H", "f"}:
            row = max(1, values[0] if values else 1)
            column = max(1, values[1] if len(values) > 1 else 1)
            self.y = min(self.rows - 1, row - 1)
            self.x = min(self.columns - 1, column - 1)
        elif final == "A":
            self.y = max(0, self.y - distance)
        elif final in {"B", "e"}:
            self.y = min(self.rows - 1, self.y + distance)
        elif final == "C":
            self.x = min(self.columns - 1, self.x + distance)
        elif final == "D":
            self.x = max(0, self.x - distance)
        elif final in {"G", "`"}:
            self.x = min(self.columns - 1, max(1, first) - 1)
        elif final == "d":
            self.y = min(self.rows - 1, max(1, first) - 1)
        elif final == "J":
            if first in {2, 3}:
                self.cells = [[" " for _ in range(self.columns)] for _ in range(self.rows)]
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
            self.x, self.y = self.saved

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
        self.process = subprocess.Popen(
            command,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=environment,
            close_fds=True,
            process_group=0,
        )
        if not keep_slave:
            os.close(slave)

    def __enter__(self) -> "PtySession":
        return self

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        self.close()

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
            self.output.extend(chunk)
            if len(self.output) > MAX_CAPTURE_BYTES:
                del self.output[: len(self.output) - MAX_CAPTURE_BYTES]
            self.screen.feed(chunk)
            readable, _, _ = select.select([self.master], [], [], 0)
        return received

    def wait_screen(self, marker: str, timeout: float | None = None) -> None:
        deadline = min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        while marker not in self.screen.text():
            if time.monotonic() >= deadline:
                raise AssertionError(f"timed out waiting for screen marker {marker!r}; {self._failure_tail()}")
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(
                    f"process exited with {status} before screen marker {marker!r}; {self._failure_tail()}"
                )

    def wait_history(self, marker: bytes, timeout: float | None = None) -> None:
        deadline = min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        while marker not in self.output:
            if time.monotonic() >= deadline:
                raise AssertionError(f"timed out waiting for output marker {marker!r}; {self._failure_tail()}")
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(
                    f"process exited with {status} before output marker {marker!r}; {self._failure_tail()}"
                )

    def wait_predicate(self, description: str, predicate: object, timeout: float | None = None) -> None:
        assert callable(predicate)
        deadline = min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        while not predicate():
            if time.monotonic() >= deadline:
                raise AssertionError(f"timed out waiting for {description}; {self._failure_tail()}")
            self.drain(POLL_SECONDS)
            status = self.process.poll()
            if status is not None:
                self.drain()
                raise AssertionError(f"process exited with {status} while waiting for {description}; {self._failure_tail()}")

    def send(self, keys: bytes) -> None:
        assert self.process.poll() is None
        os.write(self.master, keys)

    def resize(self, columns: int, rows: int) -> None:
        assert columns > 0
        assert rows > 0
        self.screen.resize(columns, rows)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        os.killpg(self.process.pid, signal.SIGWINCH)

    def settle(self, debounce: float = 0.075) -> None:
        assert 0 < debounce <= 0.1
        end = min(self.deadline, time.monotonic() + debounce)
        while time.monotonic() < end:
            self.drain(min(POLL_SECONDS, end - time.monotonic()))

    def wait_exit(self, expected_status: int, timeout: float | None = None) -> float:
        deadline = min(self.deadline, time.monotonic() + timeout) if timeout else self.deadline
        remaining = max(0.05, deadline - time.monotonic())
        self.process.wait(timeout=remaining)
        self.drain()
        if self.process.returncode != expected_status:
            raise AssertionError(
                f"expected status {expected_status}, got {self.process.returncode}; {self._failure_tail()}"
            )
        elapsed = time.monotonic() - self.started
        assert elapsed <= 20.0, elapsed
        return elapsed

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
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


def run_checked(command: list[str], environment: dict[str, str], expected_status: int = 0) -> subprocess.CompletedProcess[str]:
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
        return
    deadline = time.monotonic() + 1.0
    while Path(f"/proc/{pid}").exists() and time.monotonic() < deadline:
        time.sleep(0.01)
    if Path(f"/proc/{pid}").exists():
        raise AssertionError(f"fixture process {pid} survived explicit cleanup")
