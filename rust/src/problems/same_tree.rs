//! Same Tree (Easy).

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn is_same_tree(p: TreeLink, q: TreeLink) -> bool {
        unimplemented!("same-tree")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_same_tree(
        tree(&[Some(1), Some(2), Some(3)]),
        tree(&[Some(1), Some(2), Some(3)])
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
