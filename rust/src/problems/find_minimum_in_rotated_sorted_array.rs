//! Find Minimum in Rotated Sorted Array (Medium).

pub struct Solution;

impl Solution {
    pub fn find_min(nums: Vec<i32>) -> i32 {
        unimplemented!("find-minimum-in-rotated-sorted-array")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::find_min(vec![3, 4, 5, 1, 2]), 1);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
