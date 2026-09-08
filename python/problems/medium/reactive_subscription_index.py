"""Problem 6: reactive subscription invalidation with consistent reverse edges."""

from collections.abc import Iterable
from typing import Literal


class SubscriptionIndex:
    def __init__(self) -> None:
        raise NotImplementedError

    def subscribe(self, subscription_id: str, dependency_keys: Iterable[str]) -> None:
        raise NotImplementedError

    def unsubscribe(self, subscription_id: str) -> bool:
        raise NotImplementedError

    def dependencies(self, subscription_id: str) -> set[str] | Literal["NOT_FOUND"]:
        raise NotImplementedError

    def invalidate(
        self, changed_keys: Iterable[str], max_results: int
    ) -> list[str] | Literal["OVERLOAD"]:
        raise NotImplementedError
