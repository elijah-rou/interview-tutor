//! Word Break (Medium).

pub struct Solution;

impl Solution {
    pub fn word_break(s: String, word_dict: Vec<String>) -> bool {
        unimplemented!("word-break")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::word_break(
        "leetcode".into(),
        vec!["leet".into(), "code".into()]
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
