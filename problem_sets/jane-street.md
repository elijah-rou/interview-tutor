# Jane Street interview set

This set follows the weekend screening plan. Use Python, narrate the design, state invariants and complexity, test the baseline, then accept extensions one at a time.

## Best four

1. **Time Based Key-Value Store (981):** Add deletion and tombstones, bounded history, and out-of-order writes.
2. **Insert Delete GetRandom O(1) (380):** Add duplicate values, weighted sampling, and arbitrary-occurrence removal.
3. **Number of Islands (200):** Preserve the input, return component sizes, then add land dynamically.
4. **Insert Interval (57):** Handle unsorted intervals, deletion, online insertion, and overlap queries.

## Useful follow-ups

5. **Find Median from Data Stream (295):** Use two heaps, then support a sliding window or deletion.
6. **Design Add and Search Words (211):** Add prefix queries, deletion, and additional wildcards.
7. **Design Hit Counter (362):** Add arbitrary windows, per-key counts, out-of-order events, and bounded memory.
8. **Accounts Merge (721):** Compare graph traversal with union-find and explain the trade-off.

## Mechanics

9. **LRU Cache (146):** Dictionary plus doubly linked list. Treat this as a warm-up, not a mock.
10. **Kth Largest Element in a Stream (703):** `heapq` and bounded heaps.
11. **Search in Rotated Sorted Array (33):** Binary search with explicit boundary invariants.

## Final unseen mock pool

Choose one problem you have not solved before. Keep the others unseen as backups.

12. **Construct Binary Tree from Preorder and Inorder Traversal (105):** Trees.
13. **Graph Valid Tree (261):** Graphs.
14. **Longest Substring Without Repeating Characters (3):** Strings.
15. **Merge Intervals (56):** Intervals.

For each mock: clarify constraints, give an example, propose the simplest correct design, state invariants and complexity, implement, test boundaries, identify the next limitation, then improve.
