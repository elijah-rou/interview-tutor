//! Longest Palindromic Substring (Medium).

pub struct Solution;

impl Solution {
    pub fn longest_palindrome(s: String) -> String {
        unimplemented!("longest-palindromic-substring")
    }
}

pub(crate) fn run_case() {
    let answer = Solution::longest_palindrome("babad".into());
    assert!(answer == "bab" || answer == "aba");
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
