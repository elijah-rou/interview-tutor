//! Best Time to Buy and Sell Stock (Easy).

pub struct Solution;

impl Solution {
    pub fn max_profit(prices: Vec<i32>) -> i32 {
        unimplemented!("best-time-to-buy-and-sell-stock")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::max_profit(vec![7, 1, 5, 3, 6, 4]), 5);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
