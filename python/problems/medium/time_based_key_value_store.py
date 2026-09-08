from __future__ import annotations
from typing import List, Tuple, Dict
from bisect import bisect_right


class TimeMap:
    def __init__(self):
        self.map: Dict[str, List[Tuple[str, int]]] = {}

    def set(self, key: str, value: str, timestamp: int) -> None:
        if key not in self.map:
            self.map[key] = [(value, timestamp)]
        else:
            self.map[key].append((value, timestamp))

    def get(self, key: str, timestamp: int) -> str:
        entries = self.map[key]
        # find the entry which is maxmially closest to the give timestamp
        index = bisect_right(entries, timestamp, key=lambda entry: entry[1])

        if index == 0:
            return ""
        # Return the value before the max since bisect right will return the maximal next not prev
        # bisect_left would exclude "equal_to" values
        return entries[index-1][0]
