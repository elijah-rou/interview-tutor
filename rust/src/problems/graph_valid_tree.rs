//! Graph Valid Tree (Medium).

pub struct Solution;

impl Solution {
    pub fn valid_tree(n: i32, edges: Vec<Vec<i32>>) -> bool {
        unimplemented!("graph-valid-tree")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::valid_tree(
        5,
        vec![vec![0, 1], vec![0, 2], vec![0, 3], vec![1, 4]]
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
