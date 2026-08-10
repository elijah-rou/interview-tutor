//! Longest Repeating Character Replacement (Medium).

pub struct Solution;

impl Solution {
    pub fn character_replacement(s: String, k: i32) -> i32 {
        unimplemented!("longest-repeating-character-replacement")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::character_replacement("AABABBA".into(), 1), 4);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
