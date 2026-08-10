//! Meeting Rooms II (Medium).

use crate::types::Interval;
pub struct Solution;

impl Solution {
    pub fn min_meeting_rooms(intervals: Vec<Interval>) -> i32 {
        unimplemented!("meeting-rooms-ii")
    }
}

pub(crate) fn run_case() {
    assert_eq!(
        Solution::min_meeting_rooms(vec![
            Interval::new(0, 30),
            Interval::new(5, 10),
            Interval::new(15, 20)
        ]),
        2
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
