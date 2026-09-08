"""Problem 1: deadlock-aware lock manager. All lock access goes through one manager."""

from _thread import LockType

DeadlockedAttempt = RuntimeError


class LockManager:
    def __init__(self) -> None:
        raise NotImplementedError

    def safe_lock(self, lock: LockType) -> None:
        raise NotImplementedError

    def safe_unlock(self, lock: LockType) -> None:
        raise NotImplementedError
