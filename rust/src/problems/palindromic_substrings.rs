//! Palindromic Substrings (Medium).

pub struct Solution;

impl Solution {
    pub fn count_substrings(s: String) -> i32 {
        unimplemented!("palindromic-substrings")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::count_substrings("aaa".into()), 6);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
