//! Alien Dictionary (Hard).

pub struct Solution;

impl Solution {
    pub fn alien_order(words: Vec<String>) -> String {
        unimplemented!("alien-dictionary")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::alien_order(vec![
            "wrt".into(),
            "wrf".into(),
            "er".into(),
            "ett".into(),
            "rftt".into()
        ]),
        "wertf"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
