//! Unique Paths (Medium).

pub struct Solution;

impl Solution {
    pub fn unique_paths(m: i32, n: i32) -> i32 {
        unimplemented!("unique-paths")
    }
}

pub(crate) fn run_case() {
    assert_eq!(Solution::unique_paths(3, 7), 28);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
