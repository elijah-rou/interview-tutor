//! Reorder List (Medium).

use crate::types::{list, list_to_vec, ListNode};
pub struct Solution;

impl Solution {
    pub fn reorder_list(head: &mut Option<Box<ListNode>>) {
        unimplemented!("reorder-list")
    }
}

pub(crate) fn run_case() {
    let mut head = list(&[1, 2, 3, 4]);
    Solution::reorder_list(&mut head);
    assert_eq!(list_to_vec(&head), vec![1, 4, 2, 3]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
