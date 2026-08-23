//! Subsets (Medium).

pub struct Solution;

impl Solution {
    pub fn subsets(nums: Vec<i32>) -> Vec<Vec<i32>> {
        unimplemented!("subsets")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::subsets(vec![1, 2]);
    for subset in &mut actual {
        subset.sort();
    }
    actual.sort();
    assert_eq!(actual, vec![vec![], vec![1], vec![1, 2], vec![2]]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
