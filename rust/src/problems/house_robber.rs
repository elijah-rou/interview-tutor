//! House Robber (Medium).

pub struct Solution;

impl Solution {
    pub fn rob(nums: Vec<i32>) -> i32 {
        unimplemented!("house-robber")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::rob(vec![1, 2, 3, 1]), 4);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
