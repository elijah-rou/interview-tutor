from __future__ import annotations
from collections import deque

class Solution:

    def validTree(self, n: int, edges: list[list[int]]) -> bool:
        # immediately discard since there are too little/many edges
        if len(edges) != n-1:
            return False

        if n == 1 and edges == []:
            return True
        
        # build the graph
        first = -1
        adjacency_map = {}
        for i, e in enumerate(edges):
            if i == 0:
                first = e[0]
            if e[0] not in adjacency_map:
                adjacency_map[e[0]] = []
            if e[1] not in adjacency_map:
                adjacency_map[e[1]] = []
            adjacency_map[e[0]].append(e[1])
            adjacency_map[e[1]].append(e[0])
        assert(first != -1)

        to_visit: deque[int] = deque()
        to_visit.append(first)
        visited = set([first])
        while len(to_visit) != 0:
            current = to_visit.pop()
            for neighbour in adjacency_map[current]:
                if neighbour not in visited:
                    to_visit.append(neighbour)
                    visited.add(neighbour)

        return len(visited) == n







