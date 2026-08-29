//! Insert Delete GetRandom O(1) (Medium).

pub struct RandomizedSet;

impl RandomizedSet {
    pub fn new() -> Self {
        unimplemented!("insert-delete-getrandom-o1")
    }

    pub fn insert(&mut self, val: i32) -> bool {
        unimplemented!("insert-delete-getrandom-o1")
    }

    pub fn remove(&mut self, val: i32) -> bool {
        unimplemented!("insert-delete-getrandom-o1")
    }

    pub fn get_random(&self) -> i32 {
        unimplemented!("insert-delete-getrandom-o1")
    }
}

pub(crate) fn run_case() {
    let mut values = RandomizedSet::new();
    assert!(values.insert(1));
    assert!(!values.remove(2));
    assert!(values.insert(2));
    assert!(matches!(values.get_random(), 1 | 2));
    assert!(values.remove(1));
    assert!(!values.insert(2));
    assert_eq!(values.get_random(), 2);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
