//! Counting Bits (Easy).

pub struct Solution;

impl Solution {
    pub fn count_bits(n: i32) -> Vec<i32> {
        unimplemented!("counting-bits")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::count_bits(5), vec![0, 1, 1, 2, 1, 2]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
