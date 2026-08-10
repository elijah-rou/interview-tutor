//! Longest Substring Without Repeating Characters (Medium).

pub struct Solution;

impl Solution {
    pub fn length_of_longest_substring(s: String) -> i32 {
        unimplemented!("longest-substring-without-repeating-characters")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::length_of_longest_substring("abcabcbb".into()), 3);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
