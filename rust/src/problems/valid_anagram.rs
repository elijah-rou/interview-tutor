//! Valid Anagram (Easy).

pub struct Solution;

impl Solution {
    pub fn is_anagram(s: String, t: String) -> bool {
        unimplemented!("valid-anagram")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_anagram("anagram".into(), "nagaram".into()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
