//! Course Schedule (Medium).

pub struct Solution;

impl Solution {
    pub fn can_finish(num_courses: i32, prerequisites: Vec<Vec<i32>>) -> bool {
        unimplemented!("course-schedule")
    }
}

pub(crate) fn run_case() {
    assert!(Solution::can_finish(2, vec![vec![1, 0]]));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
