//! 3Sum (Medium).

pub struct Solution;

impl Solution {
    pub fn three_sum(nums: Vec<i32>) -> Vec<Vec<i32>> {
        unimplemented!("3sum")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::three_sum(vec![-1, 0, 1, 2, -1, -4]);
    for triplet in &mut actual {
        triplet.sort();
    }
    actual.sort();
    let mut expected = vec![vec![-1, -1, 2], vec![-1, 0, 1]];
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
