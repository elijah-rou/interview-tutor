//! Sum of Two Integers (Medium).

pub struct Solution;

impl Solution {
    pub fn get_sum(a: i32, b: i32) -> i32 {
        unimplemented!("sum-of-two-integers")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::get_sum(2, 3), 5);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
