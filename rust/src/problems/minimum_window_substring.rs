//! Minimum Window Substring (Hard).

pub struct Solution;

impl Solution {
    pub fn min_window(s: String, t: String) -> String {
        unimplemented!("minimum-window-substring")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::min_window("ADOBECODEBANC".into(), "ABC".into()),
        "BANC"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
