//! Number of Connected Components in an Undirected Graph (Medium).

pub struct Solution;

impl Solution {
    pub fn count_components(n: i32, edges: Vec<Vec<i32>>) -> i32 {
        unimplemented!("number-of-connected-components-in-an-undirected-graph")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::count_components(5, vec![vec![0, 1], vec![1, 2], vec![3, 4]]),
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
