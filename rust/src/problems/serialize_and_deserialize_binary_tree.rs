//! Serialize and Deserialize Binary Tree (Hard).

use crate::types::{tree, TreeLink};

pub struct Codec;

impl Codec {
    pub fn new() -> Self {
        unimplemented!("serialize-and-deserialize-binary-tree")
    }

    pub fn serialize(&self, root: TreeLink) -> String {
        unimplemented!("serialize-and-deserialize-binary-tree")
    }

    pub fn deserialize(&self, data: String) -> TreeLink {
        unimplemented!("serialize-and-deserialize-binary-tree")
    }
}

pub(crate) fn run_case() {
    let codec = Codec::new();
    let input = tree(&[Some(1), Some(2), Some(3), None, None, Some(4), Some(5)]);
    assert_eq!(codec.deserialize(codec.serialize(input.clone())), input);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
