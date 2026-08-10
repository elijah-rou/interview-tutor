//! Construct Binary Tree from Preorder and Inorder Traversal (Medium).

use crate::types::{tree_to_level_order, TreeLink};
pub struct Solution;

impl Solution {
    pub fn build_tree(preorder: Vec<i32>, inorder: Vec<i32>) -> TreeLink {
        unimplemented!("construct-binary-tree-from-preorder-and-inorder-traversal")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        tree_to_level_order(&Solution::build_tree(
            vec![3, 9, 20, 15, 7],
            vec![9, 3, 15, 20, 7]
        )),
        vec![Some(3), Some(9), Some(20), None, None, Some(15), Some(7)]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
