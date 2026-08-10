//! Decode Ways (Medium).

pub struct Solution;

impl Solution {
    pub fn num_decodings(s: String) -> i32 {
        unimplemented!("decode-ways")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::num_decodings("226".into()), 3);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
