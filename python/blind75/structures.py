from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from typing import Iterable


@dataclass(eq=False, slots=True)
class ListNode:
    val: int = 0
    next: ListNode | None = None


@dataclass(eq=False, slots=True)
class TreeNode:
    val: int = 0
    left: TreeNode | None = None
    right: TreeNode | None = None


@dataclass(eq=False, slots=True)
class Node:
    val: int = 0
    neighbors: list[Node] = field(default_factory=list)


def linked(values: Iterable[int]) -> ListNode | None:
    dummy = ListNode()
    tail = dummy
    for value in values:
        tail.next = ListNode(value)
        tail = tail.next
    return dummy.next


def linked_values(head: ListNode | None, *, limit: int = 10_000) -> list[int]:
    values: list[int] = []
    seen: set[int] = set()
    while head is not None:
        identity = id(head)
        if identity in seen:
            raise AssertionError("unexpected cycle in linked-list result")
        seen.add(identity)
        values.append(head.val)
        if len(values) > limit:
            raise AssertionError("linked-list result exceeded node limit")
        head = head.next
    return values


def tree(values: list[int | None]) -> TreeNode | None:
    if not values or values[0] is None:
        return None
    root = TreeNode(values[0])
    queue = deque([root])
    index = 1
    while queue and index < len(values):
        parent = queue.popleft()
        if index < len(values) and values[index] is not None:
            parent.left = TreeNode(values[index])
            queue.append(parent.left)
        index += 1
        if index < len(values) and values[index] is not None:
            parent.right = TreeNode(values[index])
            queue.append(parent.right)
        index += 1
    return root


def tree_values(root: TreeNode | None, *, limit: int = 10_000) -> list[int | None]:
    if root is None:
        return []
    result: list[int | None] = []
    queue: deque[TreeNode | None] = deque([root])
    visited = 0
    while queue:
        node = queue.popleft()
        if node is None:
            result.append(None)
            continue
        result.append(node.val)
        queue.extend((node.left, node.right))
        visited += 1
        if visited > limit:
            raise AssertionError("tree result exceeded node limit")
    while result and result[-1] is None:
        result.pop()
    return result


def graph(adjacency: list[list[int]]) -> Node | None:
    if not adjacency:
        return None
    nodes = [Node(index + 1) for index in range(len(adjacency))]
    for node, neighbors in zip(nodes, adjacency, strict=True):
        node.neighbors = [nodes[value - 1] for value in neighbors]
    return nodes[0]


def graph_adjacency(node: Node | None, *, limit: int = 10_000) -> list[list[int]]:
    if node is None:
        return []
    nodes: dict[int, Node] = {}
    queue = deque([node])
    while queue:
        current = queue.popleft()
        if current.val in nodes:
            if nodes[current.val] is not current:
                raise AssertionError("graph contains duplicate node values")
            continue
        nodes[current.val] = current
        if len(nodes) > limit:
            raise AssertionError("graph result exceeded node limit")
        queue.extend(current.neighbors)
    maximum = max(nodes)
    if set(nodes) != set(range(1, maximum + 1)):
        raise AssertionError("graph node values must be contiguous starting at 1")
    return [
        [neighbor.val for neighbor in nodes[value].neighbors] for value in range(1, maximum + 1)
    ]
