//! Valid Parentheses (Easy).

pub struct Solution;

impl Solution {
    pub fn is_valid(s: String) -> bool {
        unimplemented!("valid-parentheses")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_valid("()[]{}".into()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
