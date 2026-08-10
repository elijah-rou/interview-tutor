//! Validate Binary Search Tree (Medium).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn is_valid_bst(root: TreeLink) -> bool {
        unimplemented!("validate-binary-search-tree")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_valid_bst(tree(&[Some(2), Some(1), Some(3)])));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
