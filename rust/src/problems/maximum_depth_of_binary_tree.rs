//! Maximum Depth of Binary Tree (Easy).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn max_depth(root: TreeLink) -> i32 {
        unimplemented!("maximum-depth-of-binary-tree")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::max_depth(tree(&[
            Some(3),
            Some(9),
            Some(20),
            None,
            None,
            Some(15),
            Some(7)
        ])),
        3
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
