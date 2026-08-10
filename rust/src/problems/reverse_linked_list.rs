//! Reverse Linked List (Easy).

use crate::types::{list, list_to_vec, ListNode};
pub struct Solution;

impl Solution {
    pub fn reverse_list(head: Option<Box<ListNode>>) -> Option<Box<ListNode>> {
        unimplemented!("reverse-linked-list")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        list_to_vec(&Solution::reverse_list(list(&[1, 2, 3]))),
        vec![3, 2, 1]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
