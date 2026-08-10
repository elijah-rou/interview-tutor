//! Shared LeetCode-style data structures and fixture helpers.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListNode {
    pub val: i32,
    pub next: Option<Box<ListNode>>,
}

impl ListNode {
    pub fn new(val: i32) -> Self {
        Self { val, next: None }
    }
}

pub fn list(values: &[i32]) -> Option<Box<ListNode>> {
    let mut head = None;
    for &value in values.iter().rev() {
        let mut node = Box::new(ListNode::new(value));
        node.next = head;
        head = Some(node);
    }
    head
}

pub fn list_to_vec(head: &Option<Box<ListNode>>) -> Vec<i32> {
    let mut values = Vec::new();
    let mut cursor = head.as_deref();
    while let Some(node) = cursor {
        values.push(node.val);
        cursor = node.next.as_deref();
    }
    values
}

/// Local representation for Linked List Cycle. LeetCode does not publish a
/// Rust template for this problem, and `Box<ListNode>` cannot express cycles.
#[derive(Debug)]
pub struct CycleListNode {
    pub val: i32,
    pub next: CycleLink,
}

pub type CycleLink = Option<Rc<RefCell<CycleListNode>>>;

pub fn cyclic_list(values: &[i32], cycle_index: Option<usize>) -> CycleLink {
    if values.is_empty() {
        assert!(cycle_index.is_none());
        return None;
    }
    let nodes: Vec<_> = values
        .iter()
        .map(|&value| {
            Rc::new(RefCell::new(CycleListNode {
                val: value,
                next: None,
            }))
        })
        .collect();
    for pair in nodes.windows(2) {
        pair[0].borrow_mut().next = Some(Rc::clone(&pair[1]));
    }
    if let Some(index) = cycle_index {
        assert!(index < nodes.len());
        nodes.last().unwrap().borrow_mut().next = Some(Rc::clone(&nodes[index]));
    }
    Some(Rc::clone(&nodes[0]))
}

#[derive(Debug, Eq, PartialEq)]
pub struct TreeNode {
    pub val: i32,
    pub left: TreeLink,
    pub right: TreeLink,
}

pub type TreeLink = Option<Rc<RefCell<TreeNode>>>;

impl TreeNode {
    pub fn new(val: i32) -> Self {
        Self {
            val,
            left: None,
            right: None,
        }
    }
}

/// Builds a tree from LeetCode's level-order representation.
pub fn tree(values: &[Option<i32>]) -> TreeLink {
    let Some(Some(root_value)) = values.first() else {
        return None;
    };

    let root = Rc::new(RefCell::new(TreeNode::new(*root_value)));
    let mut parents = VecDeque::from([Rc::clone(&root)]);
    let mut values = values.iter().skip(1);

    while let Some(parent) = parents.pop_front() {
        if let Some(Some(value)) = values.next() {
            let child = Rc::new(RefCell::new(TreeNode::new(*value)));
            parent.borrow_mut().left = Some(Rc::clone(&child));
            parents.push_back(child);
        }
        if let Some(Some(value)) = values.next() {
            let child = Rc::new(RefCell::new(TreeNode::new(*value)));
            parent.borrow_mut().right = Some(Rc::clone(&child));
            parents.push_back(child);
        }
    }

    Some(root)
}

pub fn tree_to_level_order(root: &TreeLink) -> Vec<Option<i32>> {
    let mut values = Vec::new();
    let mut nodes = VecDeque::from([root.clone()]);

    while let Some(node) = nodes.pop_front() {
        match node {
            Some(node) => {
                let node = node.borrow();
                values.push(Some(node.val));
                nodes.push_back(node.left.clone());
                nodes.push_back(node.right.clone());
            }
            None => values.push(None),
        }
    }
    while values.last() == Some(&None) {
        values.pop();
    }
    values
}

#[derive(Debug)]
pub struct GraphNode {
    pub val: i32,
    pub neighbors: Vec<Rc<RefCell<GraphNode>>>,
}

/// Alias matching judges that name the clone-graph structure `Node`.
pub type Node = GraphNode;
pub type GraphLink = Option<Rc<RefCell<GraphNode>>>;

impl GraphNode {
    pub fn new(val: i32) -> Self {
        Self {
            val,
            neighbors: Vec::new(),
        }
    }
}

/// Builds a 1-indexed graph; adjacency indexes are zero-based in this helper.
pub fn graph(adjacency: &[&[usize]]) -> GraphLink {
    if adjacency.is_empty() {
        return None;
    }
    let nodes: Vec<_> = (1..=adjacency.len())
        .map(|value| Rc::new(RefCell::new(GraphNode::new(value as i32))))
        .collect();
    for (index, neighbors) in adjacency.iter().enumerate() {
        assert!(neighbors.iter().all(|&neighbor| neighbor < nodes.len()));
        nodes[index].borrow_mut().neighbors = neighbors
            .iter()
            .map(|&neighbor| Rc::clone(&nodes[neighbor]))
            .collect();
    }
    Some(Rc::clone(&nodes[0]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interval {
    pub start: i32,
    pub end: i32,
}

impl Interval {
    pub fn new(start: i32, end: i32) -> Self {
        assert!(start <= end, "an interval must not end before it starts");
        Self { start, end }
    }
}
