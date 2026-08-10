from __future__ import annotations

from collections.abc import Callable
from types import ModuleType
from typing import Any

from blind75.structures import (
    graph,
    linked,
    linked_values,
    tree,
    tree_values,
)


# One small public-contract example per problem. Add cases here while solving;
# starter modules remain clean and directly pasteable into LeetCode.
SIMPLE_CASES: dict[str, tuple[tuple[Any, ...], Any]] = {
    "best-time-to-buy-and-sell-stock": (([7, 1, 5, 3, 6, 4],), 5),
    "climbing-stairs": ((5,), 8),
    "coin-change": (([1, 2, 5], 11), 3),
    "container-with-most-water": (([1, 8, 6, 2, 5, 4, 8, 3, 7],), 49),
    "contains-duplicate": (([1, 2, 3, 1],), True),
    "counting-bits": ((5,), [0, 1, 1, 2, 1, 2]),
    "course-schedule": ((2, [[1, 0]]), True),
    "decode-ways": (("226",), 3),
    "find-minimum-in-rotated-sorted-array": (([3, 4, 5, 1, 2],), 1),
    "graph-valid-tree": ((5, [[0, 1], [0, 2], [0, 3], [1, 4]]), True),
    "house-robber": (([1, 2, 3, 1],), 4),
    "house-robber-ii": (([2, 3, 2],), 3),
    "insert-interval": (([[1, 3], [6, 9]], [2, 5]), [[1, 5], [6, 9]]),
    "jump-game": (([2, 3, 1, 1, 4],), True),
    "longest-common-subsequence": (("abcde", "ace"), 3),
    "longest-consecutive-sequence": (([100, 4, 200, 1, 3, 2],), 4),
    "longest-increasing-subsequence": (([10, 9, 2, 5, 3, 7, 101, 18],), 4),
    "longest-palindromic-substring": (("cbbd",), "bb"),
    "longest-repeating-character-replacement": (("AABABBA", 1), 4),
    "longest-substring-without-repeating-characters": (("abcabcbb",), 3),
    "maximum-product-subarray": (([2, 3, -2, 4],), 6),
    "maximum-subarray": (([-2, 1, -3, 4, -1, 2, 1, -5, 4],), 6),
    "meeting-rooms": (([[0, 30], [5, 10], [15, 20]],), False),
    "meeting-rooms-ii": (([[0, 30], [5, 10], [15, 20]],), 2),
    "merge-intervals": (([[1, 3], [2, 6], [8, 10], [15, 18]],), [[1, 6], [8, 10], [15, 18]]),
    "minimum-window-substring": (("ADOBECODEBANC", "ABC"), "BANC"),
    "missing-number": (([3, 0, 1],), 2),
    "non-overlapping-intervals": (([[1, 2], [2, 3], [3, 4], [1, 3]],), 1),
    "number-of-1-bits": ((11,), 3),
    "number-of-connected-components-in-an-undirected-graph": ((5, [[0, 1], [1, 2], [3, 4]]), 2),
    "number-of-islands": (([["1", "1", "0"], ["1", "0", "0"], ["0", "0", "1"]],), 2),
    "palindromic-substrings": (("aaa",), 6),
    "product-of-array-except-self": (([1, 2, 3, 4],), [24, 12, 8, 6]),
    "reverse-bits": ((43261596,), 964176192),
    "search-in-rotated-sorted-array": (([4, 5, 6, 7, 0, 1, 2], 0), 4),
    "spiral-matrix": (([[1, 2, 3], [4, 5, 6], [7, 8, 9]],), [1, 2, 3, 6, 9, 8, 7, 4, 5]),
    "sum-of-two-integers": ((2, 3), 5),
    "unique-paths": ((3, 7), 28),
    "valid-anagram": (("anagram", "nagaram"), True),
    "valid-palindrome": (("A man, a plan, a canal: Panama",), True),
    "valid-parentheses": (("()[]{}",), True),
    "word-break": (("leetcode", ["leet", "code"]), True),
    "word-search": (
        ([["A", "B", "C", "E"], ["S", "F", "C", "S"], ["A", "D", "E", "E"]], "ABCCED"),
        True,
    ),
}


def assert_equal(actual: Any, expected: Any) -> None:
    if actual != expected:
        raise AssertionError(f"expected {expected!r}, got {actual!r}")


def call_solution(module: ModuleType, method_name: str, *args: Any) -> Any:
    solution = module.Solution()
    return getattr(solution, method_name)(*args)


def test_simple(module: ModuleType, method_name: str, case: tuple[tuple[Any, ...], Any]) -> None:
    args, expected = case
    assert_equal(call_solution(module, method_name, *args), expected)


def test_two_sum(module: ModuleType) -> None:
    nums = [2, 7, 11, 15]
    target = 9
    actual = call_solution(module, "twoSum", nums, target)
    if len(actual) != 2:
        raise AssertionError(f"expected two indices, got {actual!r}")
    first, second = actual
    if first == second or not (0 <= first < len(nums)) or not (0 <= second < len(nums)):
        raise AssertionError(f"invalid indices: {actual!r}")
    assert_equal(nums[first] + nums[second], target)


def test_3sum(module: ModuleType) -> None:
    actual = call_solution(module, "threeSum", [-1, 0, 1, 2, -1, -4])
    assert_equal(sorted(sorted(group) for group in actual), [[-1, -1, 2], [-1, 0, 1]])


def test_alien_dictionary(module: ModuleType) -> None:
    assert_equal(call_solution(module, "alienOrder", ["wrt", "wrf", "er", "ett", "rftt"]), "wertf")


def test_binary_tree_level_order_traversal(module: ModuleType) -> None:
    assert_equal(
        call_solution(module, "levelOrder", tree([3, 9, 20, None, None, 15, 7])),
        [[3], [9, 20], [15, 7]],
    )


def test_binary_tree_maximum_depth(module: ModuleType) -> None:
    assert_equal(call_solution(module, "maxDepth", tree([3, 9, 20, None, None, 15, 7])), 3)


def test_binary_tree_maximum_path_sum(module: ModuleType) -> None:
    assert_equal(call_solution(module, "maxPathSum", tree([-10, 9, 20, None, None, 15, 7])), 42)


def test_clone_graph(module: ModuleType) -> None:
    original = graph([[2, 4], [1, 3], [2, 4], [1, 3]])
    cloned = call_solution(module, "cloneGraph", original)

    def snapshot(root):
        nodes = {}
        queue = [root]
        while queue:
            node = queue.pop()
            if node.val in nodes:
                assert nodes[node.val][0] is node
                continue
            nodes[node.val] = (node, sorted(neighbor.val for neighbor in node.neighbors))
            queue.extend(node.neighbors)
        return nodes

    original_nodes = snapshot(original)
    cloned_nodes = snapshot(cloned)
    assert_equal(
        {value: neighbors for value, (_, neighbors) in cloned_nodes.items()},
        {value: neighbors for value, (_, neighbors) in original_nodes.items()},
    )
    if {id(node) for node, _ in original_nodes.values()} & {
        id(node) for node, _ in cloned_nodes.values()
    }:
        raise AssertionError("clone must not reuse any original node")


def test_combination_sum(module: ModuleType) -> None:
    actual = call_solution(module, "combinationSum", [2, 3, 6, 7], 7)
    normalized = sorted(sorted(combination) for combination in actual)
    assert_equal(normalized, [[2, 2, 3], [7]])


def test_construct_tree(module: ModuleType) -> None:
    result = call_solution(module, "buildTree", [3, 9, 20, 15, 7], [9, 3, 15, 20, 7])
    assert_equal(tree_values(result), [3, 9, 20, None, None, 15, 7])


def test_word_dictionary(module: ModuleType) -> None:
    value = module.WordDictionary()
    value.addWord("bad")
    value.addWord("dad")
    assert_equal(value.search("pad"), False)
    assert_equal(value.search(".ad"), True)


def test_encode_decode(module: ModuleType) -> None:
    values = ["lint", "code", "", "a#b"]
    codec = module.Codec()
    encoded = codec.encode(values)
    if not isinstance(encoded, str):
        raise AssertionError("encode must return str")
    assert_equal(codec.decode(encoded), values)


def test_median_finder(module: ModuleType) -> None:
    finder = module.MedianFinder()
    finder.addNum(1)
    finder.addNum(2)
    assert_equal(finder.findMedian(), 1.5)
    finder.addNum(3)
    assert_equal(finder.findMedian(), 2.0)


def test_group_anagrams(module: ModuleType) -> None:
    actual = call_solution(module, "groupAnagrams", ["eat", "tea", "tan", "ate", "nat", "bat"])

    def normalize(groups):
        return sorted(sorted(group) for group in groups)

    assert_equal(normalize(actual), normalize([["bat"], ["nat", "tan"], ["ate", "eat", "tea"]]))


def test_invert_binary_tree(module: ModuleType) -> None:
    assert_equal(
        tree_values(call_solution(module, "invertTree", tree([4, 2, 7, 1, 3, 6, 9]))),
        [4, 7, 2, 9, 6, 3, 1],
    )


def test_kth_smallest(module: ModuleType) -> None:
    assert_equal(call_solution(module, "kthSmallest", tree([3, 1, 4, None, 2]), 1), 1)


def test_linked_list_cycle(module: ModuleType) -> None:
    head = linked([3, 2, 0, -4])
    assert head is not None and head.next is not None
    tail = head
    while tail.next is not None:
        tail = tail.next
    tail.next = head.next
    assert_equal(call_solution(module, "hasCycle", head), True)


def test_lca(module: ModuleType) -> None:
    root = tree([6, 2, 8, 0, 4, 7, 9, None, None, 3, 5])
    assert root is not None and root.left is not None and root.right is not None
    result = call_solution(module, "lowestCommonAncestor", root, root.left, root.right)
    assert result is root


def test_merge_k_lists(module: ModuleType) -> None:
    result = call_solution(
        module, "mergeKLists", [linked([1, 4, 5]), linked([1, 3, 4]), linked([2, 6])]
    )
    assert_equal(linked_values(result), [1, 1, 2, 3, 4, 4, 5, 6])


def test_merge_two_lists(module: ModuleType) -> None:
    result = call_solution(module, "mergeTwoLists", linked([1, 2, 4]), linked([1, 3, 4]))
    assert_equal(linked_values(result), [1, 1, 2, 3, 4, 4])


def test_pacific_atlantic(module: ModuleType) -> None:
    heights = [[1, 2, 2, 3, 5], [3, 2, 3, 4, 4], [2, 4, 5, 3, 1], [6, 7, 1, 4, 5], [5, 1, 1, 2, 4]]
    actual = call_solution(module, "pacificAtlantic", heights)
    expected = [[0, 4], [1, 3], [1, 4], [2, 2], [3, 0], [3, 1], [4, 0]]
    assert_equal(sorted(map(tuple, actual)), sorted(map(tuple, expected)))


def test_remove_nth(module: ModuleType) -> None:
    assert_equal(
        linked_values(call_solution(module, "removeNthFromEnd", linked([1, 2, 3, 4, 5]), 2)),
        [1, 2, 3, 5],
    )


def test_reorder_list(module: ModuleType) -> None:
    head = linked([1, 2, 3, 4])
    result = call_solution(module, "reorderList", head)
    if result is not None:
        raise AssertionError("reorderList must mutate in place and return None")
    assert_equal(linked_values(head), [1, 4, 2, 3])


def test_reverse_linked_list(module: ModuleType) -> None:
    assert_equal(
        linked_values(call_solution(module, "reverseList", linked([1, 2, 3, 4, 5]))),
        [5, 4, 3, 2, 1],
    )


def test_rotate_image(module: ModuleType) -> None:
    matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
    result = call_solution(module, "rotate", matrix)
    if result is not None:
        raise AssertionError("rotate must mutate in place and return None")
    assert_equal(matrix, [[7, 4, 1], [8, 5, 2], [9, 6, 3]])


def test_same_tree(module: ModuleType) -> None:
    assert_equal(call_solution(module, "isSameTree", tree([1, 2, 3]), tree([1, 2, 3])), True)


def test_serialize_tree(module: ModuleType) -> None:
    codec = module.Codec()
    root = tree([1, 2, 3, None, None, 4, 5])
    data = codec.serialize(root)
    if not isinstance(data, str):
        raise AssertionError("serialize must return str")
    assert_equal(tree_values(codec.deserialize(data)), [1, 2, 3, None, None, 4, 5])


def test_set_matrix_zeroes(module: ModuleType) -> None:
    matrix = [[1, 1, 1], [1, 0, 1], [1, 1, 1]]
    result = call_solution(module, "setZeroes", matrix)
    if result is not None:
        raise AssertionError("setZeroes must mutate in place and return None")
    assert_equal(matrix, [[1, 0, 1], [0, 0, 0], [1, 0, 1]])


def test_subtree(module: ModuleType) -> None:
    assert_equal(call_solution(module, "isSubtree", tree([3, 4, 5, 1, 2]), tree([4, 1, 2])), True)


def test_top_k(module: ModuleType) -> None:
    assert_equal(sorted(call_solution(module, "topKFrequent", [1, 1, 1, 2, 2, 3], 2)), [1, 2])


def test_trie(module: ModuleType) -> None:
    value = module.Trie()
    value.insert("apple")
    assert_equal(value.search("apple"), True)
    assert_equal(value.search("app"), False)
    assert_equal(value.startsWith("app"), True)


def test_validate_bst(module: ModuleType) -> None:
    assert_equal(call_solution(module, "isValidBST", tree([2, 1, 3])), True)


def test_word_search_ii(module: ModuleType) -> None:
    board = [["o", "a", "a", "n"], ["e", "t", "a", "e"], ["i", "h", "k", "r"], ["i", "f", "l", "v"]]
    actual = call_solution(module, "findWords", board, ["oath", "pea", "eat", "rain"])
    assert_equal(sorted(actual), ["eat", "oath"])


CUSTOM_TESTS: dict[str, Callable[[ModuleType], None]] = {
    "two-sum": test_two_sum,
    "3sum": test_3sum,
    "alien-dictionary": test_alien_dictionary,
    "binary-tree-level-order-traversal": test_binary_tree_level_order_traversal,
    "maximum-depth-of-binary-tree": test_binary_tree_maximum_depth,
    "binary-tree-maximum-path-sum": test_binary_tree_maximum_path_sum,
    "clone-graph": test_clone_graph,
    "combination-sum": test_combination_sum,
    "construct-binary-tree-from-preorder-and-inorder-traversal": test_construct_tree,
    "design-add-and-search-words-data-structure": test_word_dictionary,
    "encode-and-decode-strings": test_encode_decode,
    "find-median-from-data-stream": test_median_finder,
    "group-anagrams": test_group_anagrams,
    "implement-trie-prefix-tree": test_trie,
    "invert-binary-tree": test_invert_binary_tree,
    "kth-smallest-element-in-a-bst": test_kth_smallest,
    "linked-list-cycle": test_linked_list_cycle,
    "lowest-common-ancestor-of-a-binary-search-tree": test_lca,
    "merge-k-sorted-lists": test_merge_k_lists,
    "merge-two-sorted-lists": test_merge_two_lists,
    "pacific-atlantic-water-flow": test_pacific_atlantic,
    "remove-nth-node-from-end-of-list": test_remove_nth,
    "reorder-list": test_reorder_list,
    "reverse-linked-list": test_reverse_linked_list,
    "rotate-image": test_rotate_image,
    "same-tree": test_same_tree,
    "serialize-and-deserialize-binary-tree": test_serialize_tree,
    "set-matrix-zeroes": test_set_matrix_zeroes,
    "subtree-of-another-tree": test_subtree,
    "top-k-frequent-elements": test_top_k,
    "validate-binary-search-tree": test_validate_bst,
    "word-search-ii": test_word_search_ii,
}
