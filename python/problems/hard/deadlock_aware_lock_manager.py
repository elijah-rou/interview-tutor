"""Problem 1: deadlock-aware lock manager. All lock access goes through one manager."""

from _thread import LockType
from threading import get_ident as thread_id
from threading import Lock, Condition


DeadlockedAttempt = RuntimeError


class LockManager:
    def __init__(self) -> None:
        self.waiting: dict[int, LockType] = {}
        self.owned: dict[LockType, int] = {}
        self.mutex = Lock()
        self.changed = Condition(self.mutex)

    def safe_lock(self, lock: LockType) -> None:
        tid = thread_id()
        
        with self.changed:
            try:
                while lock in self.owned:
                    # Check whether this would create a cycle
                    probe = lock
                    while probe in self.owned:
                        owner = self.owned[probe]
                        if owner == tid:
                            raise DeadlockedAttempt("detected cyle")
                        if owner not in self.waiting:
                            break
                        probe = self.waiting[owner]
                    self.waiting[tid] = lock
                    self.changed.wait()

                lock.acquire()
                self.owned[lock] = tid
            finally:
                self.waiting.pop(tid, None)

    def safe_unlock(self, lock: LockType) -> None:
        tid = thread_id()

        with self.changed:
            if self.owned.get(lock) != tid:
                raise ValueError("caller doesn't own this lock")
            lock.release()
            del self.owned[lock]
            self.changed.notify_all()
