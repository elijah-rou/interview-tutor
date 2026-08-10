//! Climbing Stairs (Easy).

pub struct Solution;

impl Solution {
    pub fn climb_stairs(n: i32) -> i32 {
        unimplemented!("climbing-stairs")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::climb_stairs(5), 8);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
