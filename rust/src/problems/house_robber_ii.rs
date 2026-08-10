//! House Robber II (Medium).

pub struct Solution;

impl Solution {
    pub fn rob(nums: Vec<i32>) -> i32 {
        unimplemented!("house-robber-ii")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::rob(vec![2, 3, 2]), 3);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
