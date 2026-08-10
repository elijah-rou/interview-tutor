//! Clone Graph (Medium).
//!
//! LeetCode does not publish a Rust template for this problem. The local
//! `GraphLink` representation is defined in `crate::types`.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use crate::types::{graph, GraphLink, GraphNode};

pub struct Solution;

impl Solution {
    pub fn clone_graph(node: GraphLink) -> GraphLink {
        unimplemented!("clone-graph")
    }
}

type GraphSnapshot = BTreeMap<i32, (usize, Vec<i32>)>;

fn graph_snapshot(root: &Rc<RefCell<GraphNode>>) -> GraphSnapshot {
    let mut snapshot = BTreeMap::new();
    let mut queue = VecDeque::from([Rc::clone(root)]);
    while let Some(node) = queue.pop_front() {
        let address = Rc::as_ptr(&node) as usize;
        let (value, mut neighbor_values, neighbors) = {
            let node = node.borrow();
            (
                node.val,
                node.neighbors
                    .iter()
                    .map(|neighbor| neighbor.borrow().val)
                    .collect::<Vec<_>>(),
                node.neighbors.clone(),
            )
        };
        neighbor_values.sort();
        if let Some((existing_address, _)) = snapshot.get(&value) {
            assert_eq!(*existing_address, address, "duplicate graph node value");
            continue;
        }
        snapshot.insert(value, (address, neighbor_values));
        queue.extend(neighbors);
    }
    snapshot
}

pub(crate) fn run_case() {
    let original = graph(&[&[1, 3], &[0, 2], &[1, 3], &[0, 2]]).expect("original root");
    let cloned = Solution::clone_graph(Some(Rc::clone(&original))).expect("clone root");
    let original_snapshot = graph_snapshot(&original);
    let cloned_snapshot = graph_snapshot(&cloned);

    let original_adjacency: Vec<_> = original_snapshot
        .iter()
        .map(|(&value, (_, neighbors))| (value, neighbors.clone()))
        .collect();
    let cloned_adjacency: Vec<_> = cloned_snapshot
        .iter()
        .map(|(&value, (_, neighbors))| (value, neighbors.clone()))
        .collect();
    assert_eq!(cloned_adjacency, original_adjacency);

    let original_addresses: Vec<_> = original_snapshot.values().map(|entry| entry.0).collect();
    assert!(cloned_snapshot
        .values()
        .all(|entry| !original_addresses.contains(&entry.0)));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
