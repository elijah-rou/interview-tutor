//! Combination Sum (Medium).

pub struct Solution;

impl Solution {
    pub fn combination_sum(candidates: Vec<i32>, target: i32) -> Vec<Vec<i32>> {
        unimplemented!("combination-sum")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::combination_sum(vec![2, 3, 6, 7], 7);
    for combination in &mut actual {
        combination.sort();
    }
    actual.sort();
    assert_eq!(actual, vec![vec![2, 2, 3], vec![7]]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
