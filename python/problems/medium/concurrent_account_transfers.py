"""Convex concurrency drill: atomic transfers under one shared lock.

Account membership is fixed. Copy the initial balances and return snapshots.
Per-account locking with a consistent order is an optional follow-up.
"""


class Accounts:
    def __init__(self, balances: dict[str, int]) -> None:
        raise NotImplementedError

    def transfer(self, source: str, destination: str, amount: int) -> bool:
        """Return False for insufficient funds; otherwise transfer atomically."""
        raise NotImplementedError

    def snapshot(self) -> dict[str, int]:
        """Return a consistent copy of all balances."""
        raise NotImplementedError
