//! Remove Nth Node From End of List (Medium).

use crate::types::{list, list_to_vec, ListNode};
pub struct Solution;

impl Solution {
    pub fn remove_nth_from_end(head: Option<Box<ListNode>>, n: i32) -> Option<Box<ListNode>> {
        unimplemented!("remove-nth-node-from-end-of-list")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        list_to_vec(&Solution::remove_nth_from_end(list(&[1, 2, 3, 4, 5]), 2)),
        vec![1, 2, 3, 5]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
