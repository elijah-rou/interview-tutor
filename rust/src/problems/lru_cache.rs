//! LRU Cache (Medium).
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::cell::RefCell;

enum LRUNode {
    Head,
    Tail,
    Entry { key: i32, value: i32}
}
struct Node {
    payload: LRUNode,
    next: Option<Rc<RefCell<Node>>>,
    prev: Option<Weak<RefCell<Node>>>
}

pub struct LRUCache {
    capacity: i32,
    map: HashMap<i32, Rc<RefCell<Node>>>,
    head: Rc<RefCell<Node>>,
    tail: Rc<RefCell<Node>>
}

impl LRUCache {
    pub fn new(capacity: i32) -> Self {
        let head = Rc::new(RefCell::new(Node { 
            payload: LRUNode::Head, 
            next: None,
            prev: None
        }));
        let tail = Rc::new(RefCell::new(Node { 
            payload: LRUNode::Tail, 
            next: None,
            prev: None
        }));
        head.borrow_mut().next = Some(tail.clone());
        tail.borrow_mut().prev = Some(Rc::downgrade(&head));
        Self {
            capacity: capacity,
            head: head,
            tail: tail,
            map: HashMap::new()
        }
    }

    pub fn get(&mut self, key: i32) -> i32 {
        match self.map.get(&key) {
            None => -1,
            Some(node) => {
                let old_head = self
                    .head
                    .borrow()
                    .next.unwrap()
                    .borrow_mut();
                old_head.prev = Some(Rc::downgrade(&node));
                self.head.borrow_mut().next = Some(node.clone());

                node.prev = self.head.borrow();
                node.next = old_head.borrow();

                // guaranteed to be non-sentinel
                node.payload.unwrap().value
            }
        }
    }

    pub fn put(&mut self, key: i32, value: i32) {
        unimplemented!("lru-cache")
    }
}

pub(crate) fn run_case() {
    let mut cache = LRUCache::new(2);
    cache.put(1, 1);
    cache.put(2, 2);
    assert_eq!(cache.get(1), 1);
    cache.put(3, 3);
    assert_eq!(cache.get(2), -1);
    cache.put(4, 4);
    assert_eq!(cache.get(1), -1);
    assert_eq!(cache.get(3), 3);
    assert_eq!(cache.get(4), 4);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
