from __future__ import annotations


class HitCounter:
    def __init__(self):
        pass

    def hit(self, timestamp: int) -> None:
        raise NotImplementedError

    def getHits(self, timestamp: int) -> int:
        raise NotImplementedError
