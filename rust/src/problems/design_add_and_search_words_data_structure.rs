//! Design Add and Search Words Data Structure (Medium).

pub struct WordDictionary;

impl WordDictionary {
    pub fn new() -> Self {
        unimplemented!("design-add-and-search-words-data-structure")
    }

    pub fn add_word(&self, word: String) {
        unimplemented!("design-add-and-search-words-data-structure")
    }

    pub fn search(&self, word: String) -> bool {
        unimplemented!("design-add-and-search-words-data-structure")
    }
}

pub(crate) fn run_case() {
    let dictionary = WordDictionary::new();
    dictionary.add_word("bad".into());
    assert!(dictionary.search("b.d".into()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
