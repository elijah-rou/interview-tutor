//! Missing Number (Easy).

pub struct Solution;

impl Solution {
    pub fn missing_number(nums: Vec<i32>) -> i32 {
        unimplemented!("missing-number")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::missing_number(vec![3, 0, 1]), 2);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
