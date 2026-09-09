from __future__ import annotations
from typing import Any

class DataNode:
    next: DataNode | None
    prev: DataNode | None
    data: Any
    key: int | None

    def __init__(self, data: Any=None, key: int=None):
        self.data = data 
        self.key = key 
        self.next = None
        self.prev = None

class LRUCache:
    def __init__(self, capacity: int):
        assert capacity > 0
        self.capacity = capacity
        self.value_map: dict[int, DataNode] = {}

        self.head_sentinel = DataNode()
        self.tail_sentinel = DataNode()
        self.head_sentinel.next = self.tail_sentinel
        self.tail_sentinel.prev = self.head_sentinel
    
    def _insert_or_promote(self, node: DataNode) -> None:
        head = self.head_sentinel.next
        next = node.next
        prev = node.prev

        assert (next is None and prev is None) or (next is not None and prev is not None)

        # Insert head
        if head == node:
            return
        # Relink head
        self.head_sentinel.next = node 
        head.prev = node

        # Relink node
        node.prev = self.head_sentinel
        node.next = head

        # Join old node links
        if next is not None and prev is not None:
            prev.next = next
            next.prev = prev

    def get(self, key: int) -> int:
        # Grab current node
        node = self.value_map.get(key)
        if node:
            self._insert_or_promote(node)
            return node.data
        return -1

    def put(self, key: int, value: int) -> None:
        if key in self.value_map:
            node = self.value_map[key]
            node.data = value
        else:
            node = DataNode(value, key)
            if len(self.value_map) == self.capacity:
                tail = self.tail_sentinel.prev
                prev = tail.prev
                self.value_map.pop(tail.key, None)
                self.tail_sentinel.prev = prev
                prev.next = self.tail_sentinel
            self.value_map[key] = node
        self._insert_or_promote(node)
