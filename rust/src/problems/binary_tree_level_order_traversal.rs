//! Binary Tree Level Order Traversal (Medium).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn level_order(root: TreeLink) -> Vec<Vec<i32>> {
        unimplemented!("binary-tree-level-order-traversal")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::level_order(tree(&[
            Some(3),
            Some(9),
            Some(20),
            None,
            None,
            Some(15),
            Some(7)
        ])),
        vec![vec![3], vec![9, 20], vec![15, 7]]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
