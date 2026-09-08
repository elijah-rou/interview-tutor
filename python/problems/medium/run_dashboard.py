"""Problem 10: backend product API, a run dashboard that survives retries.

Implement Dashboard. See the runtime-practice set for the full prompt.
Returned records must be copies; cursors are (tenant_id, watermark, last).
"""

from typing import Literal, TypedDict

State = Literal["queued", "running", "cancel_requested", "succeeded", "failed", "cancelled"]
Cursor = tuple[str, int, int]


class Run(TypedDict):
    run_id: int
    tenant_id: str
    request_id: str
    payload: str
    state: State


class Dashboard:
    def __init__(self) -> None:
        raise NotImplementedError

    def create_run(
        self, tenant_id: str, request_id: str, payload: str
    ) -> Run | Literal["CONFLICT"]:
        raise NotImplementedError

    def get_run(self, tenant_id: str, run_id: int) -> Run | Literal["NOT_FOUND"]:
        raise NotImplementedError

    def cancel_run(self, tenant_id: str, run_id: int) -> Run | Literal["NOT_FOUND"]:
        raise NotImplementedError

    def list_runs(
        self, tenant_id: str, limit: int, cursor: Cursor | None = None
    ) -> tuple[list[Run], Cursor | None]:
        raise NotImplementedError

    def _set_state(self, run_id: int, state: State) -> None:
        """Test-only worker hook. Not a public handler."""
        raise NotImplementedError
