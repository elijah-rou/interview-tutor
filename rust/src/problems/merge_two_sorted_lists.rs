//! Merge Two Sorted Lists (Easy).

use crate::types::{list, list_to_vec, ListNode};
pub struct Solution;

impl Solution {
    pub fn merge_two_lists(
        list1: Option<Box<ListNode>>,
        list2: Option<Box<ListNode>>,
    ) -> Option<Box<ListNode>> {
        unimplemented!("merge-two-sorted-lists")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        list_to_vec(&Solution::merge_two_lists(
            list(&[1, 2, 4]),
            list(&[1, 3, 4])
        )),
        vec![1, 1, 2, 3, 4, 4]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
