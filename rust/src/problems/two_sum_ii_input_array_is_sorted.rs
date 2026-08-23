//! Two Sum II - Input Array Is Sorted (Medium).

pub struct Solution;

impl Solution {
    pub fn two_sum(numbers: Vec<i32>, target: i32) -> Vec<i32> {
        unimplemented!("two-sum-ii-input-array-is-sorted")
    }
}

pub(crate) fn run_case() {
    let numbers = vec![2, 7, 11, 15];
    let actual = Solution::two_sum(numbers.clone(), 9);
    assert_eq!(actual.len(), 2);
    let first = usize::try_from(actual[0]).expect("first index must be positive");
    let second = usize::try_from(actual[1]).expect("second index must be positive");
    assert!(first > 0);
    assert!(first < second);
    let first = first - 1;
    let second = second - 1;
    assert!(second < numbers.len());
    assert_eq!(numbers[first] + numbers[second], 9);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
