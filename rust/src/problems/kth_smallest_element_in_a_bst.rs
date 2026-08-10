//! Kth Smallest Element in a BST (Medium).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn kth_smallest(root: TreeLink, k: i32) -> i32 {
        unimplemented!("kth-smallest-element-in-a-bst")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::kth_smallest(tree(&[Some(3), Some(1), Some(4), None, Some(2)]), 1),
        1
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
