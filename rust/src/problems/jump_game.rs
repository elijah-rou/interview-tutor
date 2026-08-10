//! Jump Game (Medium).

pub struct Solution;

impl Solution {
    pub fn can_jump(nums: Vec<i32>) -> bool {
        unimplemented!("jump-game")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::can_jump(vec![2, 3, 1, 1, 4]));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
