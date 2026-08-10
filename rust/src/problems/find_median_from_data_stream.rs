//! Find Median from Data Stream (Hard).

pub struct MedianFinder;

impl MedianFinder {
    pub fn new() -> Self {
        unimplemented!("find-median-from-data-stream")
    }

    pub fn add_num(&self, num: i32) {
        unimplemented!("find-median-from-data-stream")
    }

    pub fn find_median(&self) -> f64 {
        unimplemented!("find-median-from-data-stream")
    }
}

pub(crate) fn run_case() {
    let finder = MedianFinder::new();
    finder.add_num(1);
    finder.add_num(2);
    assert_eq!(finder.find_median(), 1.5);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
