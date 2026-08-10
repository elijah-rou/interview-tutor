//! Word Search (Medium).

pub struct Solution;

impl Solution {
    pub fn exist(board: Vec<Vec<char>>, word: String) -> bool {
        unimplemented!("word-search")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::exist(
        vec![
            vec!['A', 'B', 'C', 'E'],
            vec!['S', 'F', 'C', 'S'],
            vec!['A', 'D', 'E', 'E']
        ],
        "ABCCED".into()
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
