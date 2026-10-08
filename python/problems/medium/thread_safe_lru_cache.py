"""Distributed database concurrency drill: adapt lru_cache.py without changing that exercise.

Protect each complete get/put operation with one lock, including recency updates.
The get_or_load follow-up is discussion-only until the core passes.
"""


class LRUCache:
    def __init__(self, capacity: int) -> None:
        raise NotImplementedError

    def get(self, key: int) -> int:
        """Return the value, or -1 when absent. A hit makes the key most recent."""
        raise NotImplementedError

    def put(self, key: int, value: int) -> None:
        raise NotImplementedError
