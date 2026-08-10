//! Binary Tree Maximum Path Sum (Hard).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn max_path_sum(root: TreeLink) -> i32 {
        unimplemented!("binary-tree-maximum-path-sum")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::max_path_sum(tree(&[
            Some(-10),
            Some(9),
            Some(20),
            None,
            None,
            Some(15),
            Some(7)
        ])),
        42
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
