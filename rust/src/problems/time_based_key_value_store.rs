//! Time Based Key-Value Store (Medium).

pub struct TimeMap;

impl TimeMap {
    pub fn new() -> Self {
        unimplemented!("time-based-key-value-store")
    }

    pub fn set(&mut self, key: String, value: String, timestamp: i32) {
        unimplemented!("time-based-key-value-store")
    }

    pub fn get(&self, key: String, timestamp: i32) -> String {
        unimplemented!("time-based-key-value-store")
    }
}

pub(crate) fn run_case() {
    let mut time_map = TimeMap::new();
    time_map.set("foo".into(), "bar".into(), 1);
    assert_eq!(time_map.get("foo".into(), 1), "bar");
    assert_eq!(time_map.get("foo".into(), 3), "bar");
    time_map.set("foo".into(), "bar2".into(), 4);
    assert_eq!(time_map.get("foo".into(), 4), "bar2");
    assert_eq!(time_map.get("foo".into(), 5), "bar2");
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
