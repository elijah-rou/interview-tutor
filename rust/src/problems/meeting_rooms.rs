//! Meeting Rooms (Easy).

use crate::types::Interval;
pub struct Solution;

impl Solution {
    pub fn can_attend_meetings(intervals: Vec<Interval>) -> bool {
        unimplemented!("meeting-rooms")
    }
}

pub(crate) fn run_case() {
    assert!(!Solution::can_attend_meetings(vec![
        Interval::new(0, 30),
        Interval::new(5, 10),
        Interval::new(15, 20)
    ]));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
