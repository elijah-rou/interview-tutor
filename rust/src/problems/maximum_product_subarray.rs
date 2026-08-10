//! Maximum Product Subarray (Medium).

pub struct Solution;

impl Solution {
    pub fn max_product(nums: Vec<i32>) -> i32 {
        unimplemented!("maximum-product-subarray")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::max_product(vec![2, 3, -2, 4]), 6);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
