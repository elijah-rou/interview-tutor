//! Reverse Bits (Easy).

pub struct Solution;

impl Solution {
    pub fn reverse_bits(x: i32) -> i32 {
        unimplemented!("reverse-bits")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::reverse_bits(0b00000010100101000001111010011100),
        0b00111001011110000010100101000000
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
