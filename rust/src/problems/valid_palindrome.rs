//! Valid Palindrome (Easy).

pub struct Solution;

impl Solution {
    pub fn is_palindrome(s: String) -> bool {
        unimplemented!("valid-palindrome")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::is_palindrome(
        "A man, a plan, a canal: Panama".into()
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
