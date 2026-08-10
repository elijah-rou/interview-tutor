//! Contains Duplicate (Easy).

pub struct Solution;

impl Solution {
    pub fn contains_duplicate(nums: Vec<i32>) -> bool {
        unimplemented!("contains-duplicate")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::contains_duplicate(vec![1, 2, 3, 1]));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
