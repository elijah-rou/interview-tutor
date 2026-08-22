//! Kth Largest Element in a Stream (Easy).

pub struct KthLargest;

impl KthLargest {
    pub fn new(k: i32, nums: Vec<i32>) -> Self {
        unimplemented!("kth-largest-element-in-a-stream")
    }

    pub fn add(&mut self, val: i32) -> i32 {
        unimplemented!("kth-largest-element-in-a-stream")
    }
}

pub(crate) fn run_case() {
    let mut kth_largest = KthLargest::new(3, vec![4, 5, 8, 2]);
    assert_eq!(kth_largest.add(3), 4);
    assert_eq!(kth_largest.add(5), 5);
    assert_eq!(kth_largest.add(10), 5);
    assert_eq!(kth_largest.add(9), 8);
    assert_eq!(kth_largest.add(4), 8);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
