//! Word Search II (Hard).

pub struct Solution;

impl Solution {
    pub fn find_words(board: Vec<Vec<char>>, words: Vec<String>) -> Vec<String> {
        unimplemented!("word-search-ii")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::find_words(
        vec![
            vec!['o', 'a', 'a', 'n'],
            vec!['e', 't', 'a', 'e'],
            vec!['i', 'h', 'k', 'r'],
            vec!['i', 'f', 'l', 'v'],
        ],
        vec!["oath".into(), "pea".into(), "eat".into(), "rain".into()],
    );
    actual.sort();
    assert_eq!(actual, vec!["eat".to_string(), "oath".to_string()]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
