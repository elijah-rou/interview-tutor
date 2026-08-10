//! Container With Most Water (Medium).

pub struct Solution;

impl Solution {
    pub fn max_area(height: Vec<i32>) -> i32 {
        unimplemented!("container-with-most-water")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::max_area(vec![1, 8, 6, 2, 5, 4, 8, 3, 7]), 49);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
