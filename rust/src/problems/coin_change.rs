//! Coin Change (Medium).

pub struct Solution;

impl Solution {
    pub fn coin_change(coins: Vec<i32>, amount: i32) -> i32 {
        unimplemented!("coin-change")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::coin_change(vec![1, 2, 5], 11), 3);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
