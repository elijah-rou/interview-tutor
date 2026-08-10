//! Invert Binary Tree (Easy).

use crate::types::{tree, tree_to_level_order, TreeLink};
pub struct Solution;

impl Solution {
    pub fn invert_tree(root: TreeLink) -> TreeLink {
        unimplemented!("invert-binary-tree")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        tree_to_level_order(&Solution::invert_tree(tree(&[Some(2), Some(1), Some(3)]))),
        vec![Some(2), Some(3), Some(1)]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
