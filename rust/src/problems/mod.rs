//! Canonical Blind 75 registry, ordered Easy, Medium, then Hard.

pub mod alien_dictionary;
pub mod best_time_to_buy_and_sell_stock;
pub mod binary_tree_level_order_traversal;
pub mod binary_tree_maximum_path_sum;
pub mod climbing_stairs;
pub mod clone_graph;
pub mod coin_change;
pub mod combination_sum_iv;
pub mod construct_binary_tree_from_preorder_and_inorder_traversal;
pub mod container_with_most_water;
pub mod contains_duplicate;
pub mod counting_bits;
pub mod course_schedule;
pub mod decode_ways;
pub mod design_add_and_search_words_data_structure;
pub mod encode_and_decode_strings;
pub mod find_median_from_data_stream;
pub mod find_minimum_in_rotated_sorted_array;
pub mod graph_valid_tree;
pub mod group_anagrams;
pub mod house_robber;
pub mod house_robber_ii;
pub mod implement_trie_prefix_tree;
pub mod insert_interval;
pub mod invert_binary_tree;
pub mod jump_game;
pub mod kth_smallest_element_in_a_bst;
pub mod linked_list_cycle;
pub mod longest_common_subsequence;
pub mod longest_consecutive_sequence;
pub mod longest_increasing_subsequence;
pub mod longest_palindromic_substring;
pub mod longest_repeating_character_replacement;
pub mod longest_substring_without_repeating_characters;
pub mod lowest_common_ancestor_of_a_binary_search_tree;
pub mod maximum_depth_of_binary_tree;
pub mod maximum_product_subarray;
pub mod maximum_subarray;
pub mod meeting_rooms;
pub mod meeting_rooms_ii;
pub mod merge_intervals;
pub mod merge_k_sorted_lists;
pub mod merge_two_sorted_lists;
pub mod minimum_window_substring;
pub mod missing_number;
pub mod non_overlapping_intervals;
pub mod number_of_1_bits;
pub mod number_of_connected_components_in_an_undirected_graph;
pub mod number_of_islands;
pub mod pacific_atlantic_water_flow;
pub mod palindromic_substrings;
pub mod product_of_array_except_self;
pub mod remove_nth_node_from_end_of_list;
pub mod reorder_list;
pub mod reverse_bits;
pub mod reverse_linked_list;
pub mod rotate_image;
pub mod same_tree;
pub mod search_in_rotated_sorted_array;
pub mod serialize_and_deserialize_binary_tree;
pub mod set_matrix_zeroes;
pub mod spiral_matrix;
pub mod subtree_of_another_tree;
pub mod sum_of_two_integers;
pub mod three_sum;
pub mod top_k_frequent_elements;
pub mod two_sum;
pub mod unique_paths;
pub mod valid_anagram;
pub mod valid_palindrome;
pub mod valid_parentheses;
pub mod validate_binary_search_tree;
pub mod word_break;
pub mod word_search;
pub mod word_search_ii;

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl fmt::Display for Difficulty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Easy => formatter.write_str("Easy"),
            Self::Medium => formatter.write_str("Medium"),
            Self::Hard => formatter.write_str("Hard"),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Problem {
    pub slug: &'static str,
    pub title: &'static str,
    pub difficulty: Difficulty,
    pub category: &'static str,
    run_case: fn(),
}

impl Problem {
    const fn new(
        slug: &'static str,
        title: &'static str,
        difficulty: Difficulty,
        category: &'static str,
        run_case: fn(),
    ) -> Self {
        Self {
            slug,
            title,
            difficulty,
            category,
            run_case,
        }
    }

    pub(crate) fn execute(&self) {
        (self.run_case)();
    }
}

pub static PROBLEMS: &[Problem] = &[
    Problem::new(
        "best-time-to-buy-and-sell-stock",
        "Best Time to Buy and Sell Stock",
        Difficulty::Easy,
        "Array",
        best_time_to_buy_and_sell_stock::run_case,
    ),
    Problem::new(
        "climbing-stairs",
        "Climbing Stairs",
        Difficulty::Easy,
        "Dynamic Programming",
        climbing_stairs::run_case,
    ),
    Problem::new(
        "contains-duplicate",
        "Contains Duplicate",
        Difficulty::Easy,
        "Array",
        contains_duplicate::run_case,
    ),
    Problem::new(
        "counting-bits",
        "Counting Bits",
        Difficulty::Easy,
        "Binary",
        counting_bits::run_case,
    ),
    Problem::new(
        "invert-binary-tree",
        "Invert Binary Tree",
        Difficulty::Easy,
        "Tree",
        invert_binary_tree::run_case,
    ),
    Problem::new(
        "linked-list-cycle",
        "Linked List Cycle",
        Difficulty::Easy,
        "Linked List",
        linked_list_cycle::run_case,
    ),
    Problem::new(
        "maximum-depth-of-binary-tree",
        "Maximum Depth of Binary Tree",
        Difficulty::Easy,
        "Tree",
        maximum_depth_of_binary_tree::run_case,
    ),
    Problem::new(
        "meeting-rooms",
        "Meeting Rooms",
        Difficulty::Easy,
        "Interval",
        meeting_rooms::run_case,
    ),
    Problem::new(
        "merge-two-sorted-lists",
        "Merge Two Sorted Lists",
        Difficulty::Easy,
        "Linked List",
        merge_two_sorted_lists::run_case,
    ),
    Problem::new(
        "missing-number",
        "Missing Number",
        Difficulty::Easy,
        "Binary",
        missing_number::run_case,
    ),
    Problem::new(
        "number-of-1-bits",
        "Number of 1 Bits",
        Difficulty::Easy,
        "Binary",
        number_of_1_bits::run_case,
    ),
    Problem::new(
        "reverse-bits",
        "Reverse Bits",
        Difficulty::Easy,
        "Binary",
        reverse_bits::run_case,
    ),
    Problem::new(
        "reverse-linked-list",
        "Reverse Linked List",
        Difficulty::Easy,
        "Linked List",
        reverse_linked_list::run_case,
    ),
    Problem::new(
        "same-tree",
        "Same Tree",
        Difficulty::Easy,
        "Tree",
        same_tree::run_case,
    ),
    Problem::new(
        "subtree-of-another-tree",
        "Subtree of Another Tree",
        Difficulty::Easy,
        "Tree",
        subtree_of_another_tree::run_case,
    ),
    Problem::new(
        "two-sum",
        "Two Sum",
        Difficulty::Easy,
        "Array",
        two_sum::run_case,
    ),
    Problem::new(
        "valid-anagram",
        "Valid Anagram",
        Difficulty::Easy,
        "String",
        valid_anagram::run_case,
    ),
    Problem::new(
        "valid-palindrome",
        "Valid Palindrome",
        Difficulty::Easy,
        "String",
        valid_palindrome::run_case,
    ),
    Problem::new(
        "valid-parentheses",
        "Valid Parentheses",
        Difficulty::Easy,
        "String",
        valid_parentheses::run_case,
    ),
    Problem::new(
        "3sum",
        "3Sum",
        Difficulty::Medium,
        "Array",
        three_sum::run_case,
    ),
    Problem::new(
        "binary-tree-level-order-traversal",
        "Binary Tree Level Order Traversal",
        Difficulty::Medium,
        "Tree",
        binary_tree_level_order_traversal::run_case,
    ),
    Problem::new(
        "clone-graph",
        "Clone Graph",
        Difficulty::Medium,
        "Graph",
        clone_graph::run_case,
    ),
    Problem::new(
        "coin-change",
        "Coin Change",
        Difficulty::Medium,
        "Dynamic Programming",
        coin_change::run_case,
    ),
    Problem::new(
        "combination-sum-iv",
        "Combination Sum IV",
        Difficulty::Medium,
        "Dynamic Programming",
        combination_sum_iv::run_case,
    ),
    Problem::new(
        "construct-binary-tree-from-preorder-and-inorder-traversal",
        "Construct Binary Tree from Preorder and Inorder Traversal",
        Difficulty::Medium,
        "Tree",
        construct_binary_tree_from_preorder_and_inorder_traversal::run_case,
    ),
    Problem::new(
        "container-with-most-water",
        "Container With Most Water",
        Difficulty::Medium,
        "Array",
        container_with_most_water::run_case,
    ),
    Problem::new(
        "course-schedule",
        "Course Schedule",
        Difficulty::Medium,
        "Graph",
        course_schedule::run_case,
    ),
    Problem::new(
        "decode-ways",
        "Decode Ways",
        Difficulty::Medium,
        "Dynamic Programming",
        decode_ways::run_case,
    ),
    Problem::new(
        "design-add-and-search-words-data-structure",
        "Design Add and Search Words Data Structure",
        Difficulty::Medium,
        "Tree",
        design_add_and_search_words_data_structure::run_case,
    ),
    Problem::new(
        "encode-and-decode-strings",
        "Encode and Decode Strings",
        Difficulty::Medium,
        "String",
        encode_and_decode_strings::run_case,
    ),
    Problem::new(
        "find-minimum-in-rotated-sorted-array",
        "Find Minimum in Rotated Sorted Array",
        Difficulty::Medium,
        "Array",
        find_minimum_in_rotated_sorted_array::run_case,
    ),
    Problem::new(
        "graph-valid-tree",
        "Graph Valid Tree",
        Difficulty::Medium,
        "Graph",
        graph_valid_tree::run_case,
    ),
    Problem::new(
        "group-anagrams",
        "Group Anagrams",
        Difficulty::Medium,
        "String",
        group_anagrams::run_case,
    ),
    Problem::new(
        "house-robber",
        "House Robber",
        Difficulty::Medium,
        "Dynamic Programming",
        house_robber::run_case,
    ),
    Problem::new(
        "house-robber-ii",
        "House Robber II",
        Difficulty::Medium,
        "Dynamic Programming",
        house_robber_ii::run_case,
    ),
    Problem::new(
        "implement-trie-prefix-tree",
        "Implement Trie (Prefix Tree)",
        Difficulty::Medium,
        "Tree",
        implement_trie_prefix_tree::run_case,
    ),
    Problem::new(
        "insert-interval",
        "Insert Interval",
        Difficulty::Medium,
        "Interval",
        insert_interval::run_case,
    ),
    Problem::new(
        "jump-game",
        "Jump Game",
        Difficulty::Medium,
        "Dynamic Programming",
        jump_game::run_case,
    ),
    Problem::new(
        "kth-smallest-element-in-a-bst",
        "Kth Smallest Element in a BST",
        Difficulty::Medium,
        "Tree",
        kth_smallest_element_in_a_bst::run_case,
    ),
    Problem::new(
        "longest-common-subsequence",
        "Longest Common Subsequence",
        Difficulty::Medium,
        "Dynamic Programming",
        longest_common_subsequence::run_case,
    ),
    Problem::new(
        "longest-consecutive-sequence",
        "Longest Consecutive Sequence",
        Difficulty::Medium,
        "Graph",
        longest_consecutive_sequence::run_case,
    ),
    Problem::new(
        "longest-increasing-subsequence",
        "Longest Increasing Subsequence",
        Difficulty::Medium,
        "Dynamic Programming",
        longest_increasing_subsequence::run_case,
    ),
    Problem::new(
        "longest-palindromic-substring",
        "Longest Palindromic Substring",
        Difficulty::Medium,
        "String",
        longest_palindromic_substring::run_case,
    ),
    Problem::new(
        "longest-repeating-character-replacement",
        "Longest Repeating Character Replacement",
        Difficulty::Medium,
        "String",
        longest_repeating_character_replacement::run_case,
    ),
    Problem::new(
        "longest-substring-without-repeating-characters",
        "Longest Substring Without Repeating Characters",
        Difficulty::Medium,
        "String",
        longest_substring_without_repeating_characters::run_case,
    ),
    Problem::new(
        "lowest-common-ancestor-of-a-binary-search-tree",
        "Lowest Common Ancestor of a Binary Search Tree",
        Difficulty::Medium,
        "Tree",
        lowest_common_ancestor_of_a_binary_search_tree::run_case,
    ),
    Problem::new(
        "maximum-product-subarray",
        "Maximum Product Subarray",
        Difficulty::Medium,
        "Array",
        maximum_product_subarray::run_case,
    ),
    Problem::new(
        "maximum-subarray",
        "Maximum Subarray",
        Difficulty::Medium,
        "Array",
        maximum_subarray::run_case,
    ),
    Problem::new(
        "meeting-rooms-ii",
        "Meeting Rooms II",
        Difficulty::Medium,
        "Interval",
        meeting_rooms_ii::run_case,
    ),
    Problem::new(
        "merge-intervals",
        "Merge Intervals",
        Difficulty::Medium,
        "Interval",
        merge_intervals::run_case,
    ),
    Problem::new(
        "non-overlapping-intervals",
        "Non-overlapping Intervals",
        Difficulty::Medium,
        "Interval",
        non_overlapping_intervals::run_case,
    ),
    Problem::new(
        "number-of-connected-components-in-an-undirected-graph",
        "Number of Connected Components in an Undirected Graph",
        Difficulty::Medium,
        "Graph",
        number_of_connected_components_in_an_undirected_graph::run_case,
    ),
    Problem::new(
        "number-of-islands",
        "Number of Islands",
        Difficulty::Medium,
        "Graph",
        number_of_islands::run_case,
    ),
    Problem::new(
        "pacific-atlantic-water-flow",
        "Pacific Atlantic Water Flow",
        Difficulty::Medium,
        "Graph",
        pacific_atlantic_water_flow::run_case,
    ),
    Problem::new(
        "palindromic-substrings",
        "Palindromic Substrings",
        Difficulty::Medium,
        "String",
        palindromic_substrings::run_case,
    ),
    Problem::new(
        "product-of-array-except-self",
        "Product of Array Except Self",
        Difficulty::Medium,
        "Array",
        product_of_array_except_self::run_case,
    ),
    Problem::new(
        "remove-nth-node-from-end-of-list",
        "Remove Nth Node From End of List",
        Difficulty::Medium,
        "Linked List",
        remove_nth_node_from_end_of_list::run_case,
    ),
    Problem::new(
        "reorder-list",
        "Reorder List",
        Difficulty::Medium,
        "Linked List",
        reorder_list::run_case,
    ),
    Problem::new(
        "rotate-image",
        "Rotate Image",
        Difficulty::Medium,
        "Matrix",
        rotate_image::run_case,
    ),
    Problem::new(
        "search-in-rotated-sorted-array",
        "Search in Rotated Sorted Array",
        Difficulty::Medium,
        "Array",
        search_in_rotated_sorted_array::run_case,
    ),
    Problem::new(
        "set-matrix-zeroes",
        "Set Matrix Zeroes",
        Difficulty::Medium,
        "Matrix",
        set_matrix_zeroes::run_case,
    ),
    Problem::new(
        "spiral-matrix",
        "Spiral Matrix",
        Difficulty::Medium,
        "Matrix",
        spiral_matrix::run_case,
    ),
    Problem::new(
        "sum-of-two-integers",
        "Sum of Two Integers",
        Difficulty::Medium,
        "Binary",
        sum_of_two_integers::run_case,
    ),
    Problem::new(
        "top-k-frequent-elements",
        "Top K Frequent Elements",
        Difficulty::Medium,
        "Heap",
        top_k_frequent_elements::run_case,
    ),
    Problem::new(
        "unique-paths",
        "Unique Paths",
        Difficulty::Medium,
        "Dynamic Programming",
        unique_paths::run_case,
    ),
    Problem::new(
        "validate-binary-search-tree",
        "Validate Binary Search Tree",
        Difficulty::Medium,
        "Tree",
        validate_binary_search_tree::run_case,
    ),
    Problem::new(
        "word-break",
        "Word Break",
        Difficulty::Medium,
        "Dynamic Programming",
        word_break::run_case,
    ),
    Problem::new(
        "word-search",
        "Word Search",
        Difficulty::Medium,
        "Matrix",
        word_search::run_case,
    ),
    Problem::new(
        "alien-dictionary",
        "Alien Dictionary",
        Difficulty::Hard,
        "Graph",
        alien_dictionary::run_case,
    ),
    Problem::new(
        "binary-tree-maximum-path-sum",
        "Binary Tree Maximum Path Sum",
        Difficulty::Hard,
        "Tree",
        binary_tree_maximum_path_sum::run_case,
    ),
    Problem::new(
        "find-median-from-data-stream",
        "Find Median from Data Stream",
        Difficulty::Hard,
        "Heap",
        find_median_from_data_stream::run_case,
    ),
    Problem::new(
        "merge-k-sorted-lists",
        "Merge K Sorted Lists",
        Difficulty::Hard,
        "Linked List",
        merge_k_sorted_lists::run_case,
    ),
    Problem::new(
        "minimum-window-substring",
        "Minimum Window Substring",
        Difficulty::Hard,
        "String",
        minimum_window_substring::run_case,
    ),
    Problem::new(
        "serialize-and-deserialize-binary-tree",
        "Serialize and Deserialize Binary Tree",
        Difficulty::Hard,
        "Tree",
        serialize_and_deserialize_binary_tree::run_case,
    ),
    Problem::new(
        "word-search-ii",
        "Word Search II",
        Difficulty::Hard,
        "Tree",
        word_search_ii::run_case,
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PROBLEMS;

    #[test]
    fn registry_contains_exactly_75_unique_slugs() {
        let slugs: HashSet<_> = PROBLEMS.iter().map(|problem| problem.slug).collect();
        assert_eq!(PROBLEMS.len(), 75);
        assert_eq!(slugs.len(), 75);
    }

    #[test]
    fn registry_is_in_ascending_difficulty() {
        assert!(PROBLEMS
            .windows(2)
            .all(|pair| pair[0].difficulty <= pair[1].difficulty));
    }
}
