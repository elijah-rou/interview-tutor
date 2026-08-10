//! Encode and Decode Strings (Medium).

pub struct Codec;

impl Codec {
    pub fn new() -> Self {
        unimplemented!("encode-and-decode-strings")
    }

    pub fn encode(&self, strs: Vec<String>) -> String {
        unimplemented!("encode-and-decode-strings")
    }

    pub fn decode(&self, s: String) -> Vec<String> {
        unimplemented!("encode-and-decode-strings")
    }
}

pub(crate) fn run_case() {
    let codec = Codec::new();
    let input = vec!["lint".into(), "code".into(), "love".into(), "you".into()];
    assert_eq!(codec.decode(codec.encode(input.clone())), input);
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
