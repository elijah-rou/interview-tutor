//! Number of Islands (Medium).

pub struct Solution;

impl Solution {
    pub fn num_islands(grid: Vec<Vec<char>>) -> i32 {
        unimplemented!("number-of-islands")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::num_islands(vec![
            vec!['1', '1', '0', '0'],
            vec!['1', '1', '0', '0'],
            vec!['0', '0', '1', '0']
        ]),
        2
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
