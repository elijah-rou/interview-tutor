//! Number of 1 Bits (Easy).

pub struct Solution;

impl Solution {
    pub fn hamming_weight(n: i32) -> i32 {
        unimplemented!("number-of-1-bits")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::hamming_weight(0b00000000000000000000000000001011),
        3
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
