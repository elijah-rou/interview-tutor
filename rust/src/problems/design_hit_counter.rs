//! Design Hit Counter (Medium).

pub struct HitCounter;

impl HitCounter {
    pub fn new() -> Self {
        unimplemented!("design-hit-counter")
    }

    pub fn hit(&mut self, timestamp: i32) {
        unimplemented!("design-hit-counter")
    }

    pub fn get_hits(&mut self, timestamp: i32) -> i32 {
        unimplemented!("design-hit-counter")
    }
}

pub(crate) fn run_case() {
    let mut counter = HitCounter::new();
    counter.hit(1);
    counter.hit(2);
    counter.hit(3);
    assert_eq!(counter.get_hits(4), 3);
    counter.hit(300);
    assert_eq!(counter.get_hits(300), 4);
    assert_eq!(counter.get_hits(301), 3);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
