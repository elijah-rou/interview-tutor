//! Longest Common Subsequence (Medium).

pub struct Solution;

impl Solution {
    pub fn longest_common_subsequence(text1: String, text2: String) -> i32 {
        unimplemented!("longest-common-subsequence")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::longest_common_subsequence("abcde".into(), "ace".into()),
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
