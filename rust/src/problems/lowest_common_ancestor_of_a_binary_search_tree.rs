//! Lowest Common Ancestor of a Binary Search Tree (Medium).

use std::rc::Rc;

use crate::types::{tree, TreeLink};
pub struct Solution;

impl Solution {
    pub fn lowest_common_ancestor(root: TreeLink, p: TreeLink, q: TreeLink) -> TreeLink {
        unimplemented!("lowest-common-ancestor-of-a-binary-search-tree")
    }
}

pub(crate) fn run_case() {
    let root = tree(&[
        Some(6),
        Some(2),
        Some(8),
        Some(0),
        Some(4),
        Some(7),
        Some(9),
        None,
        None,
        Some(3),
        Some(5),
    ]);
    let root_node = root.as_ref().expect("root");
    let (p, q) = {
        let root = root_node.borrow();
        (root.left.clone(), root.right.clone())
    };
    let answer = Solution::lowest_common_ancestor(root.clone(), p, q).expect("ancestor");
    assert!(Rc::ptr_eq(root_node, &answer));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
