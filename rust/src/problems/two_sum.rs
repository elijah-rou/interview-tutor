//! Two Sum (Easy).

pub struct Solution;

impl Solution {
    pub fn two_sum(nums: Vec<i32>, target: i32) -> Vec<i32> {
        unimplemented!("two-sum")
    }
}

pub(crate) fn run_case() {
    let nums = vec![2, 7, 11, 15];
    let actual = Solution::two_sum(nums.clone(), 9);
    assert_eq!(actual.len(), 2);
    let first = usize::try_from(actual[0]).expect("first index must be nonnegative");
    let second = usize::try_from(actual[1]).expect("second index must be nonnegative");
    assert_ne!(first, second);
    assert!(first < nums.len());
    assert!(second < nums.len());
    assert_eq!(nums[first] + nums[second], 9);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
