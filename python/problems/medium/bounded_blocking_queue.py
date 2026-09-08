"""Convex concurrency drill: use a deque and a threading.Condition.

Closing stops admission, wakes waiters, and permits draining accepted items.
The get(timeout) follow-up is not part of the core API.
"""

CLOSED = object()


class BoundedQueue:
    def __init__(self, capacity: int) -> None:
        raise NotImplementedError

    def put(self, item: object) -> bool:
        """Wait for capacity; return True if accepted, False if closed."""
        raise NotImplementedError

    def get(self) -> object:
        """Wait for an item; return CLOSED by identity only when closed and empty."""
        raise NotImplementedError

    def close(self) -> None:
        raise NotImplementedError
