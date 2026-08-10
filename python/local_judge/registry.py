from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class Problem:
    slug: str
    path: str
    class_name: str
    method_name: str | None


PROBLEMS = (
    Problem("3sum", "problems/medium/3sum.py", "Solution", "threeSum"),
    Problem("alien-dictionary", "problems/hard/alien_dictionary.py", "Solution", "alienOrder"),
    Problem(
        "best-time-to-buy-and-sell-stock",
        "problems/easy/best_time_to_buy_and_sell_stock.py",
        "Solution",
        "maxProfit",
    ),
    Problem(
        "binary-tree-level-order-traversal",
        "problems/medium/binary_tree_level_order_traversal.py",
        "Solution",
        "levelOrder",
    ),
    Problem(
        "binary-tree-maximum-path-sum",
        "problems/hard/binary_tree_maximum_path_sum.py",
        "Solution",
        "maxPathSum",
    ),
    Problem("climbing-stairs", "problems/easy/climbing_stairs.py", "Solution", "climbStairs"),
    Problem("clone-graph", "problems/medium/clone_graph.py", "Solution", "cloneGraph"),
    Problem("coin-change", "problems/medium/coin_change.py", "Solution", "coinChange"),
    Problem("combination-sum", "problems/medium/combination_sum.py", "Solution", "combinationSum"),
    Problem(
        "construct-binary-tree-from-preorder-and-inorder-traversal",
        "problems/medium/construct_binary_tree_from_preorder_and_inorder_traversal.py",
        "Solution",
        "buildTree",
    ),
    Problem(
        "container-with-most-water",
        "problems/medium/container_with_most_water.py",
        "Solution",
        "maxArea",
    ),
    Problem(
        "contains-duplicate", "problems/easy/contains_duplicate.py", "Solution", "containsDuplicate"
    ),
    Problem("counting-bits", "problems/easy/counting_bits.py", "Solution", "countBits"),
    Problem("course-schedule", "problems/medium/course_schedule.py", "Solution", "canFinish"),
    Problem("decode-ways", "problems/medium/decode_ways.py", "Solution", "numDecodings"),
    Problem(
        "design-add-and-search-words-data-structure",
        "problems/medium/design_add_and_search_words_data_structure.py",
        "WordDictionary",
        None,
    ),
    Problem(
        "encode-and-decode-strings", "problems/medium/encode_and_decode_strings.py", "Codec", None
    ),
    Problem(
        "find-median-from-data-stream",
        "problems/hard/find_median_from_data_stream.py",
        "MedianFinder",
        None,
    ),
    Problem(
        "find-minimum-in-rotated-sorted-array",
        "problems/medium/find_minimum_in_rotated_sorted_array.py",
        "Solution",
        "findMin",
    ),
    Problem("graph-valid-tree", "problems/medium/graph_valid_tree.py", "Solution", "validTree"),
    Problem("group-anagrams", "problems/medium/group_anagrams.py", "Solution", "groupAnagrams"),
    Problem("house-robber", "problems/medium/house_robber.py", "Solution", "rob"),
    Problem("house-robber-ii", "problems/medium/house_robber_ii.py", "Solution", "rob"),
    Problem(
        "implement-trie-prefix-tree", "problems/medium/implement_trie_prefix_tree.py", "Trie", None
    ),
    Problem("insert-interval", "problems/medium/insert_interval.py", "Solution", "insert"),
    Problem("invert-binary-tree", "problems/easy/invert_binary_tree.py", "Solution", "invertTree"),
    Problem("jump-game", "problems/medium/jump_game.py", "Solution", "canJump"),
    Problem(
        "kth-smallest-element-in-a-bst",
        "problems/medium/kth_smallest_element_in_a_bst.py",
        "Solution",
        "kthSmallest",
    ),
    Problem("linked-list-cycle", "problems/easy/linked_list_cycle.py", "Solution", "hasCycle"),
    Problem(
        "longest-common-subsequence",
        "problems/medium/longest_common_subsequence.py",
        "Solution",
        "longestCommonSubsequence",
    ),
    Problem(
        "longest-consecutive-sequence",
        "problems/medium/longest_consecutive_sequence.py",
        "Solution",
        "longestConsecutive",
    ),
    Problem(
        "longest-increasing-subsequence",
        "problems/medium/longest_increasing_subsequence.py",
        "Solution",
        "lengthOfLIS",
    ),
    Problem(
        "longest-palindromic-substring",
        "problems/medium/longest_palindromic_substring.py",
        "Solution",
        "longestPalindrome",
    ),
    Problem(
        "longest-repeating-character-replacement",
        "problems/medium/longest_repeating_character_replacement.py",
        "Solution",
        "characterReplacement",
    ),
    Problem(
        "longest-substring-without-repeating-characters",
        "problems/medium/longest_substring_without_repeating_characters.py",
        "Solution",
        "lengthOfLongestSubstring",
    ),
    Problem(
        "lowest-common-ancestor-of-a-binary-search-tree",
        "problems/medium/lowest_common_ancestor_of_a_binary_search_tree.py",
        "Solution",
        "lowestCommonAncestor",
    ),
    Problem(
        "maximum-depth-of-binary-tree",
        "problems/easy/binary_tree_maximum_depth.py",
        "Solution",
        "maxDepth",
    ),
    Problem(
        "maximum-product-subarray",
        "problems/medium/maximum_product_subarray.py",
        "Solution",
        "maxProduct",
    ),
    Problem("maximum-subarray", "problems/medium/maximum_subarray.py", "Solution", "maxSubArray"),
    Problem("meeting-rooms", "problems/easy/meeting_rooms.py", "Solution", "canAttendMeetings"),
    Problem(
        "meeting-rooms-ii", "problems/medium/meeting_rooms_ii.py", "Solution", "minMeetingRooms"
    ),
    Problem("merge-intervals", "problems/medium/merge_intervals.py", "Solution", "merge"),
    Problem(
        "merge-k-sorted-lists", "problems/hard/merge_k_sorted_lists.py", "Solution", "mergeKLists"
    ),
    Problem(
        "merge-two-sorted-lists",
        "problems/easy/merge_two_sorted_lists.py",
        "Solution",
        "mergeTwoLists",
    ),
    Problem(
        "minimum-window-substring",
        "problems/hard/minimum_window_substring.py",
        "Solution",
        "minWindow",
    ),
    Problem("missing-number", "problems/easy/missing_number.py", "Solution", "missingNumber"),
    Problem(
        "non-overlapping-intervals",
        "problems/medium/non_overlapping_intervals.py",
        "Solution",
        "eraseOverlapIntervals",
    ),
    Problem("number-of-1-bits", "problems/easy/number_of_1_bits.py", "Solution", "hammingWeight"),
    Problem(
        "number-of-connected-components-in-an-undirected-graph",
        "problems/medium/number_of_connected_components_in_an_undirected_graph.py",
        "Solution",
        "countComponents",
    ),
    Problem("number-of-islands", "problems/medium/number_of_islands.py", "Solution", "numIslands"),
    Problem(
        "pacific-atlantic-water-flow",
        "problems/medium/pacific_atlantic_water_flow.py",
        "Solution",
        "pacificAtlantic",
    ),
    Problem(
        "palindromic-substrings",
        "problems/medium/palindromic_substrings.py",
        "Solution",
        "countSubstrings",
    ),
    Problem(
        "product-of-array-except-self",
        "problems/medium/product_of_array_except_self.py",
        "Solution",
        "productExceptSelf",
    ),
    Problem(
        "remove-nth-node-from-end-of-list",
        "problems/medium/remove_nth_node_from_end_of_list.py",
        "Solution",
        "removeNthFromEnd",
    ),
    Problem("reorder-list", "problems/medium/reorder_list.py", "Solution", "reorderList"),
    Problem("reverse-bits", "problems/easy/reverse_bits.py", "Solution", "reverseBits"),
    Problem(
        "reverse-linked-list", "problems/easy/reverse_linked_list.py", "Solution", "reverseList"
    ),
    Problem("rotate-image", "problems/medium/rotate_image.py", "Solution", "rotate"),
    Problem("same-tree", "problems/easy/same_tree.py", "Solution", "isSameTree"),
    Problem(
        "search-in-rotated-sorted-array",
        "problems/medium/search_in_rotated_sorted_array.py",
        "Solution",
        "search",
    ),
    Problem(
        "serialize-and-deserialize-binary-tree",
        "problems/hard/serialize_and_deserialize_binary_tree.py",
        "Codec",
        None,
    ),
    Problem("set-matrix-zeroes", "problems/medium/set_matrix_zeroes.py", "Solution", "setZeroes"),
    Problem("spiral-matrix", "problems/medium/spiral_matrix.py", "Solution", "spiralOrder"),
    Problem(
        "subtree-of-another-tree",
        "problems/easy/subtree_of_another_tree.py",
        "Solution",
        "isSubtree",
    ),
    Problem("sum-of-two-integers", "problems/medium/sum_of_two_integers.py", "Solution", "getSum"),
    Problem(
        "top-k-frequent-elements",
        "problems/medium/top_k_frequent_elements.py",
        "Solution",
        "topKFrequent",
    ),
    Problem("two-sum", "problems/easy/two_sum.py", "Solution", "twoSum"),
    Problem("unique-paths", "problems/medium/unique_paths.py", "Solution", "uniquePaths"),
    Problem("valid-anagram", "problems/easy/valid_anagram.py", "Solution", "isAnagram"),
    Problem("valid-palindrome", "problems/easy/valid_palindrome.py", "Solution", "isPalindrome"),
    Problem("valid-parentheses", "problems/easy/valid_parentheses.py", "Solution", "isValid"),
    Problem(
        "validate-binary-search-tree",
        "problems/medium/validate_binary_search_tree.py",
        "Solution",
        "isValidBST",
    ),
    Problem("word-break", "problems/medium/word_break.py", "Solution", "wordBreak"),
    Problem("word-search", "problems/medium/word_search.py", "Solution", "exist"),
    Problem("word-search-ii", "problems/hard/word_search_ii.py", "Solution", "findWords"),
)
BY_SLUG = {problem.slug: problem for problem in PROBLEMS}

assert PROBLEMS
assert len(BY_SLUG) == len(PROBLEMS)
