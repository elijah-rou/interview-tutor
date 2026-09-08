"""Problem 3: bounded asynchronous job runner.

Implement the Runner API. See the runtime-practice set for the full prompt.
Do not create a job coroutine until a worker is ready to execute it.
"""

from collections.abc import Callable, Coroutine
from typing import Any, Literal

State = Literal["queued", "running", "succeeded", "failed", "cancelled"]
Job = Callable[[], Coroutine[Any, Any, Any]]


class Runner:
    def __init__(self, worker_count: int, queue_capacity: int) -> None:
        raise NotImplementedError

    async def start(self) -> None:
        raise NotImplementedError

    def submit(self, job_id: str, async_callable: Job) -> Literal["accepted", "queue_full"]:
        raise NotImplementedError

    def status(self, job_id: str) -> State | Literal["NOT_FOUND"]:
        raise NotImplementedError

    def cancel(self, job_id: str) -> State | Literal["NOT_FOUND"]:
        raise NotImplementedError

    async def close(self) -> dict[str, State]:
        """Return terminal states, not job return values or exception objects."""
        raise NotImplementedError
