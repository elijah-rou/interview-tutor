//! Linked List Cycle (Easy).
//!
//! LeetCode does not supply a Rust template for this problem. The local
//! `CycleLink` representation uses `Rc<RefCell<_>>` so fixtures can contain a cycle.

use crate::types::{cyclic_list, CycleLink};

pub struct Solution;

impl Solution {
    pub fn has_cycle(head: CycleLink) -> bool {
        unimplemented!("linked-list-cycle")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::has_cycle(cyclic_list(&[3, 2, 0, -4], Some(1))));
    assert!(!Solution::has_cycle(cyclic_list(&[1, 2], None)));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
