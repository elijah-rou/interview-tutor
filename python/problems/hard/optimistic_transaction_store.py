"""Problem 4: point-access optimistic concurrency control, not full snapshot isolation."""

from typing import Literal

MISSING = "MISSING"
CONFLICT = "CONFLICT"


class Store:
    def __init__(self) -> None:
        raise NotImplementedError

    def begin(self) -> "Transaction":
        raise NotImplementedError


class Transaction:
    def __init__(self, store: Store) -> None:
        raise NotImplementedError

    def read(self, key: str) -> int | Literal["MISSING"]:
        raise NotImplementedError

    def write(self, key: str, value: int) -> None:
        raise NotImplementedError

    def commit(self) -> int | Literal["CONFLICT"]:
        raise NotImplementedError

    def abort(self) -> None:
        raise NotImplementedError
