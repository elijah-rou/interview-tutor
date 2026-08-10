//! Implement Trie (Prefix Tree) (Medium).

pub struct Trie;

impl Trie {
    pub fn new() -> Self {
        unimplemented!("implement-trie-prefix-tree")
    }

    pub fn insert(&self, word: String) {
        unimplemented!("implement-trie-prefix-tree")
    }

    pub fn search(&self, word: String) -> bool {
        unimplemented!("implement-trie-prefix-tree")
    }

    pub fn starts_with(&self, prefix: String) -> bool {
        unimplemented!("implement-trie-prefix-tree")
    }
}

pub(crate) fn run_case() {
    let trie = Trie::new();
    trie.insert("apple".into());
    assert!(trie.search("apple".into()));
    assert!(trie.starts_with("app".into()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
