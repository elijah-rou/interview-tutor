//! Top K Frequent Elements (Medium).

pub struct Solution;

impl Solution {
    pub fn top_k_frequent(nums: Vec<i32>, k: i32) -> Vec<i32> {
        unimplemented!("top-k-frequent-elements")
    }
}

pub(crate) fn run_case() {
    let mut actual = Solution::top_k_frequent(vec![1, 1, 1, 2, 2, 3], 2);
    actual.sort();
    assert_eq!(actual, vec![1, 2]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
