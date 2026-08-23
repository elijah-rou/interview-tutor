//! Subarray Sum Equals K (Medium).

pub struct Solution;

impl Solution {
    pub fn subarray_sum(nums: Vec<i32>, k: i32) -> i32 {
        unimplemented!("subarray-sum-equals-k")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::subarray_sum(vec![1, 1, 1], 2), 2);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
