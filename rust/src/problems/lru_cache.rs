//! LRU Cache (Medium).

pub struct LRUCache;

impl LRUCache {
    pub fn new(capacity: i32) -> Self {
        unimplemented!("lru-cache")
    }

    pub fn get(&mut self, key: i32) -> i32 {
        unimplemented!("lru-cache")
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
