//! Set-agnostic Rust problem adapter registry.

pub mod alien_dictionary;
pub mod best_time_to_buy_and_sell_stock;
pub mod binary_tree_level_order_traversal;
pub mod binary_tree_maximum_path_sum;
pub mod climbing_stairs;
pub mod clone_graph;
pub mod coin_change;
pub mod combination_sum;
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

pub struct Problem {
    pub slug: &'static str,
    execute: fn(),
}

impl Problem {
    const fn new(slug: &'static str, execute: fn()) -> Self {
        Self { slug, execute }
    }

    pub fn execute(&self) {
        (self.execute)();
    }
}

pub const PROBLEMS: &[Problem] = &[
    Problem::new("3sum", three_sum::run_case),
    Problem::new("alien-dictionary", alien_dictionary::run_case),
    Problem::new(
        "best-time-to-buy-and-sell-stock",
        best_time_to_buy_and_sell_stock::run_case,
    ),
    Problem::new(
        "binary-tree-level-order-traversal",
        binary_tree_level_order_traversal::run_case,
    ),
    Problem::new(
        "binary-tree-maximum-path-sum",
        binary_tree_maximum_path_sum::run_case,
    ),
    Problem::new("climbing-stairs", climbing_stairs::run_case),
    Problem::new("clone-graph", clone_graph::run_case),
    Problem::new("coin-change", coin_change::run_case),
    Problem::new("combination-sum", combination_sum::run_case),
    Problem::new(
        "construct-binary-tree-from-preorder-and-inorder-traversal",
        construct_binary_tree_from_preorder_and_inorder_traversal::run_case,
    ),
    Problem::new(
        "container-with-most-water",
        container_with_most_water::run_case,
    ),
    Problem::new("contains-duplicate", contains_duplicate::run_case),
    Problem::new("counting-bits", counting_bits::run_case),
    Problem::new("course-schedule", course_schedule::run_case),
    Problem::new("decode-ways", decode_ways::run_case),
    Problem::new(
        "design-add-and-search-words-data-structure",
        design_add_and_search_words_data_structure::run_case,
    ),
    Problem::new(
        "encode-and-decode-strings",
        encode_and_decode_strings::run_case,
    ),
    Problem::new(
        "find-median-from-data-stream",
        find_median_from_data_stream::run_case,
    ),
    Problem::new(
        "find-minimum-in-rotated-sorted-array",
        find_minimum_in_rotated_sorted_array::run_case,
    ),
    Problem::new("graph-valid-tree", graph_valid_tree::run_case),
    Problem::new("group-anagrams", group_anagrams::run_case),
    Problem::new("house-robber", house_robber::run_case),
    Problem::new("house-robber-ii", house_robber_ii::run_case),
    Problem::new(
        "implement-trie-prefix-tree",
        implement_trie_prefix_tree::run_case,
    ),
    Problem::new("insert-interval", insert_interval::run_case),
    Problem::new("invert-binary-tree", invert_binary_tree::run_case),
    Problem::new("jump-game", jump_game::run_case),
    Problem::new(
        "kth-smallest-element-in-a-bst",
        kth_smallest_element_in_a_bst::run_case,
    ),
    Problem::new("linked-list-cycle", linked_list_cycle::run_case),
    Problem::new(
        "longest-common-subsequence",
        longest_common_subsequence::run_case,
    ),
    Problem::new(
        "longest-consecutive-sequence",
        longest_consecutive_sequence::run_case,
    ),
    Problem::new(
        "longest-increasing-subsequence",
        longest_increasing_subsequence::run_case,
    ),
    Problem::new(
        "longest-palindromic-substring",
        longest_palindromic_substring::run_case,
    ),
    Problem::new(
        "longest-repeating-character-replacement",
        longest_repeating_character_replacement::run_case,
    ),
    Problem::new(
        "longest-substring-without-repeating-characters",
        longest_substring_without_repeating_characters::run_case,
    ),
    Problem::new(
        "lowest-common-ancestor-of-a-binary-search-tree",
        lowest_common_ancestor_of_a_binary_search_tree::run_case,
    ),
    Problem::new(
        "maximum-depth-of-binary-tree",
        maximum_depth_of_binary_tree::run_case,
    ),
    Problem::new(
        "maximum-product-subarray",
        maximum_product_subarray::run_case,
    ),
    Problem::new("maximum-subarray", maximum_subarray::run_case),
    Problem::new("meeting-rooms", meeting_rooms::run_case),
    Problem::new("meeting-rooms-ii", meeting_rooms_ii::run_case),
    Problem::new("merge-intervals", merge_intervals::run_case),
    Problem::new("merge-k-sorted-lists", merge_k_sorted_lists::run_case),
    Problem::new("merge-two-sorted-lists", merge_two_sorted_lists::run_case),
    Problem::new(
        "minimum-window-substring",
        minimum_window_substring::run_case,
    ),
    Problem::new("missing-number", missing_number::run_case),
    Problem::new(
        "non-overlapping-intervals",
        non_overlapping_intervals::run_case,
    ),
    Problem::new("number-of-1-bits", number_of_1_bits::run_case),
    Problem::new(
        "number-of-connected-components-in-an-undirected-graph",
        number_of_connected_components_in_an_undirected_graph::run_case,
    ),
    Problem::new("number-of-islands", number_of_islands::run_case),
    Problem::new(
        "pacific-atlantic-water-flow",
        pacific_atlantic_water_flow::run_case,
    ),
    Problem::new("palindromic-substrings", palindromic_substrings::run_case),
    Problem::new(
        "product-of-array-except-self",
        product_of_array_except_self::run_case,
    ),
    Problem::new(
        "remove-nth-node-from-end-of-list",
        remove_nth_node_from_end_of_list::run_case,
    ),
    Problem::new("reorder-list", reorder_list::run_case),
    Problem::new("reverse-bits", reverse_bits::run_case),
    Problem::new("reverse-linked-list", reverse_linked_list::run_case),
    Problem::new("rotate-image", rotate_image::run_case),
    Problem::new("same-tree", same_tree::run_case),
    Problem::new(
        "search-in-rotated-sorted-array",
        search_in_rotated_sorted_array::run_case,
    ),
    Problem::new(
        "serialize-and-deserialize-binary-tree",
        serialize_and_deserialize_binary_tree::run_case,
    ),
    Problem::new("set-matrix-zeroes", set_matrix_zeroes::run_case),
    Problem::new("spiral-matrix", spiral_matrix::run_case),
    Problem::new("subtree-of-another-tree", subtree_of_another_tree::run_case),
    Problem::new("sum-of-two-integers", sum_of_two_integers::run_case),
    Problem::new("top-k-frequent-elements", top_k_frequent_elements::run_case),
    Problem::new("two-sum", two_sum::run_case),
    Problem::new("unique-paths", unique_paths::run_case),
    Problem::new("valid-anagram", valid_anagram::run_case),
    Problem::new("valid-palindrome", valid_palindrome::run_case),
    Problem::new("valid-parentheses", valid_parentheses::run_case),
    Problem::new(
        "validate-binary-search-tree",
        validate_binary_search_tree::run_case,
    ),
    Problem::new("word-break", word_break::run_case),
    Problem::new("word-search", word_search::run_case),
    Problem::new("word-search-ii", word_search_ii::run_case),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PROBLEMS;

    #[test]
    fn registry_contains_unique_slugs() {
        assert!(!PROBLEMS.is_empty());
        let slugs: HashSet<_> = PROBLEMS.iter().map(|problem| problem.slug).collect();
        assert_eq!(slugs.len(), PROBLEMS.len());
    }
}
