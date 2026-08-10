//! Subtree of Another Tree (Easy).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn is_subtree(root: TreeLink, sub_root: TreeLink) -> bool {
        unimplemented!("subtree-of-another-tree")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_subtree(
        tree(&[Some(3), Some(4), Some(5), Some(1), Some(2)]),
        tree(&[Some(4), Some(1), Some(2)])
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
