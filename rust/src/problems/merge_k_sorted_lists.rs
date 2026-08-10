//! Merge K Sorted Lists (Hard).

use crate::types::{list, list_to_vec, ListNode};
pub struct Solution;

impl Solution {
    pub fn merge_k_lists(lists: Vec<Option<Box<ListNode>>>) -> Option<Box<ListNode>> {
        unimplemented!("merge-k-sorted-lists")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        list_to_vec(&Solution::merge_k_lists(vec![
            list(&[1, 4, 5]),
            list(&[1, 3, 4]),
            list(&[2, 6])
        ])),
        vec![1, 1, 2, 3, 4, 4, 5, 6]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
