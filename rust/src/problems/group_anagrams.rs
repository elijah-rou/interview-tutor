//! Group Anagrams (Medium).

pub struct Solution;

impl Solution {
    pub fn group_anagrams(strs: Vec<String>) -> Vec<Vec<String>> {
        unimplemented!("group-anagrams")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::group_anagrams(vec![
        "eat".into(),
        "tea".into(),
        "tan".into(),
        "ate".into(),
        "nat".into(),
        "bat".into(),
    ]);
    for group in &mut actual {
        group.sort();
    }
    actual.sort();
    let mut expected: Vec<Vec<String>> = vec![
        vec!["ate".into(), "eat".into(), "tea".into()],
        vec!["nat".into(), "tan".into()],
        vec!["bat".into()],
    ];
    for group in &mut expected {
        group.sort();
    }
    expected.sort();
    assert_eq!(actual, expected);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
