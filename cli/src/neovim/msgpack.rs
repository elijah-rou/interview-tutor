use rmp::Marker;
use std::io::{self, Read};

pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DEPTH: usize = 32;
pub const MAX_CONTAINER_ITEMS: usize = 150_000;
pub const MAX_NODES: usize = 400_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    Binary(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Ext(i8, Vec<u8>),
}

impl Value {
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(Value, Value)]> {
        match self {
            Self::Map(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(*value),
            Self::Integer(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Unsigned(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn map_get(&self, key: &str) -> Option<&Value> {
        self.as_map()?
            .iter()
            .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
    }
}

struct Decoder<'a, R> {
    reader: &'a mut R,
    bytes: usize,
    nodes: usize,
}

impl<R: Read> Decoder<'_, R> {
    fn read_exact(&mut self, output: &mut [u8]) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(output.len())
            .ok_or("Neovim MessagePack byte count overflow")?;
        if self.bytes > MAX_MESSAGE_BYTES {
            return Err("Neovim MessagePack value exceeds 4 MiB".into());
        }
        self.reader
            .read_exact(output)
            .map_err(|error| match error.kind() {
                io::ErrorKind::UnexpectedEof => "Neovim MessagePack stream ended mid-value".into(),
                _ => format!("cannot read Neovim MessagePack stream: {error}"),
            })
    }

    fn byte(&mut self) -> Result<u8, String> {
        let mut byte = [0_u8; 1];
        self.read_exact(&mut byte)?;
        Ok(byte[0])
    }

    fn be_u16(&mut self) -> Result<u16, String> {
        let mut bytes = [0_u8; 2];
        self.read_exact(&mut bytes)?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn be_u32(&mut self) -> Result<u32, String> {
        let mut bytes = [0_u8; 4];
        self.read_exact(&mut bytes)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn be_u64(&mut self) -> Result<u64, String> {
        let mut bytes = [0_u8; 8];
        self.read_exact(&mut bytes)?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn length(&mut self, bytes: usize) -> Result<usize, String> {
        let value = match bytes {
            1 => u64::from(self.byte()?),
            2 => u64::from(self.be_u16()?),
            4 => u64::from(self.be_u32()?),
            _ => unreachable!("MessagePack length widths are fixed"),
        };
        usize::try_from(value).map_err(|_| "Neovim MessagePack length does not fit usize".into())
    }

    fn blob(&mut self, length: usize) -> Result<Vec<u8>, String> {
        if length > MAX_MESSAGE_BYTES.saturating_sub(self.bytes) {
            return Err("Neovim MessagePack blob exceeds 4 MiB".into());
        }
        let mut bytes = vec![0_u8; length];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn container_len(&self, length: usize) -> Result<(), String> {
        if length > MAX_CONTAINER_ITEMS {
            Err("Neovim MessagePack container exceeds item bound".into())
        } else {
            Ok(())
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err("Neovim MessagePack nesting exceeds 32 levels".into());
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or("Neovim MessagePack node count overflow")?;
        if self.nodes > MAX_NODES {
            return Err("Neovim MessagePack value exceeds node bound".into());
        }
        let marker = Marker::from_u8(self.byte()?);
        match marker {
            Marker::FixPos(value) => Ok(Value::Unsigned(u64::from(value))),
            Marker::FixNeg(value) => Ok(Value::Integer(i64::from(value))),
            Marker::Null => Ok(Value::Nil),
            Marker::False => Ok(Value::Bool(false)),
            Marker::True => Ok(Value::Bool(true)),
            Marker::U8 => Ok(Value::Unsigned(u64::from(self.byte()?))),
            Marker::U16 => Ok(Value::Unsigned(u64::from(self.be_u16()?))),
            Marker::U32 => Ok(Value::Unsigned(u64::from(self.be_u32()?))),
            Marker::U64 => Ok(Value::Unsigned(self.be_u64()?)),
            Marker::I8 => Ok(Value::Integer(i64::from(self.byte()? as i8))),
            Marker::I16 => Ok(Value::Integer(i64::from(self.be_u16()? as i16))),
            Marker::I32 => Ok(Value::Integer(i64::from(self.be_u32()? as i32))),
            Marker::I64 => Ok(Value::Integer(self.be_u64()? as i64)),
            Marker::F32 => Ok(Value::Float(f64::from(f32::from_bits(self.be_u32()?)))),
            Marker::F64 => Ok(Value::Float(f64::from_bits(self.be_u64()?))),
            Marker::FixStr(length) => self.string(usize::from(length)),
            Marker::Str8 => {
                let length = self.length(1)?;
                self.string(length)
            }
            Marker::Str16 => {
                let length = self.length(2)?;
                self.string(length)
            }
            Marker::Str32 => {
                let length = self.length(4)?;
                self.string(length)
            }
            Marker::Bin8 => {
                let length = self.length(1)?;
                Ok(Value::Binary(self.blob(length)?))
            }
            Marker::Bin16 => {
                let length = self.length(2)?;
                Ok(Value::Binary(self.blob(length)?))
            }
            Marker::Bin32 => {
                let length = self.length(4)?;
                Ok(Value::Binary(self.blob(length)?))
            }
            Marker::FixArray(length) => self.array(usize::from(length), depth),
            Marker::Array16 => {
                let length = self.length(2)?;
                self.array(length, depth)
            }
            Marker::Array32 => {
                let length = self.length(4)?;
                self.array(length, depth)
            }
            Marker::FixMap(length) => self.map(usize::from(length), depth),
            Marker::Map16 => {
                let length = self.length(2)?;
                self.map(length, depth)
            }
            Marker::Map32 => {
                let length = self.length(4)?;
                self.map(length, depth)
            }
            Marker::FixExt1 => self.ext(1),
            Marker::FixExt2 => self.ext(2),
            Marker::FixExt4 => self.ext(4),
            Marker::FixExt8 => self.ext(8),
            Marker::FixExt16 => self.ext(16),
            Marker::Ext8 => {
                let length = self.length(1)?;
                self.ext(length)
            }
            Marker::Ext16 => {
                let length = self.length(2)?;
                self.ext(length)
            }
            Marker::Ext32 => {
                let length = self.length(4)?;
                self.ext(length)
            }
            Marker::Reserved => Err("Neovim MessagePack stream used reserved marker".into()),
        }
    }

    fn string(&mut self, length: usize) -> Result<Value, String> {
        let bytes = self.blob(length)?;
        let string = String::from_utf8(bytes)
            .map_err(|_| "Neovim MessagePack string is not valid UTF-8".to_string())?;
        Ok(Value::String(string))
    }

    fn array(&mut self, length: usize, depth: usize) -> Result<Value, String> {
        self.container_len(length)?;
        let mut values = Vec::with_capacity(length);
        for _ in 0..length {
            values.push(self.value(depth + 1)?);
        }
        Ok(Value::Array(values))
    }

    fn map(&mut self, length: usize, depth: usize) -> Result<Value, String> {
        self.container_len(length)?;
        let mut values = Vec::with_capacity(length);
        for _ in 0..length {
            let key = self.value(depth + 1)?;
            let value = self.value(depth + 1)?;
            values.push((key, value));
        }
        Ok(Value::Map(values))
    }

    fn ext(&mut self, length: usize) -> Result<Value, String> {
        let kind = self.byte()? as i8;
        Ok(Value::Ext(kind, self.blob(length)?))
    }
}

pub fn decode<R: Read>(reader: &mut R) -> Result<Value, String> {
    Decoder {
        reader,
        bytes: 0,
        nodes: 0,
    }
    .value(0)
}

pub fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    encode_value(value, &mut output, 0)?;
    if output.len() > MAX_MESSAGE_BYTES {
        return Err("Neovim MessagePack request exceeds 4 MiB".into());
    }
    Ok(output)
}

fn encode_value(value: &Value, output: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("Neovim MessagePack request nesting exceeds 32 levels".into());
    }
    match value {
        Value::Nil => output.push(0xc0),
        Value::Bool(false) => output.push(0xc2),
        Value::Bool(true) => output.push(0xc3),
        Value::Integer(value) if (-32..=127).contains(value) => output.push(*value as u8),
        Value::Integer(value) => {
            output.push(0xd3);
            output.extend_from_slice(&value.to_be_bytes());
        }
        Value::Unsigned(value) if *value <= 127 => output.push(*value as u8),
        Value::Unsigned(value) => {
            output.push(0xcf);
            output.extend_from_slice(&value.to_be_bytes());
        }
        Value::Float(value) => {
            output.push(0xcb);
            output.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Value::String(value) => encode_blob(value.as_bytes(), output, BlobKind::String)?,
        Value::Binary(value) => encode_blob(value, output, BlobKind::Binary)?,
        Value::Array(values) => {
            encode_array_len(values.len(), output)?;
            for value in values {
                encode_value(value, output, depth + 1)?;
            }
        }
        Value::Map(values) => {
            encode_map_len(values.len(), output)?;
            for (key, value) in values {
                encode_value(key, output, depth + 1)?;
                encode_value(value, output, depth + 1)?;
            }
        }
        Value::Ext(kind, bytes) => {
            match bytes.len() {
                1 => output.push(0xd4),
                2 => output.push(0xd5),
                4 => output.push(0xd6),
                8 => output.push(0xd7),
                16 => output.push(0xd8),
                length if u8::try_from(length).is_ok() => {
                    output.push(0xc7);
                    output.push(length as u8);
                }
                length if u16::try_from(length).is_ok() => {
                    output.push(0xc8);
                    output.extend_from_slice(&(length as u16).to_be_bytes());
                }
                length => {
                    let length = u32::try_from(length).map_err(|_| "Neovim ext value too large")?;
                    output.push(0xc9);
                    output.extend_from_slice(&length.to_be_bytes());
                }
            }
            output.push(*kind as u8);
            output.extend_from_slice(bytes);
        }
    }
    if output.len() > MAX_MESSAGE_BYTES {
        return Err("Neovim MessagePack request exceeds 4 MiB".into());
    }
    Ok(())
}

enum BlobKind {
    String,
    Binary,
}

fn encode_blob(bytes: &[u8], output: &mut Vec<u8>, kind: BlobKind) -> Result<(), String> {
    let length = bytes.len();
    match kind {
        BlobKind::String if length <= 31 => output.push(0xa0 | length as u8),
        BlobKind::String if u8::try_from(length).is_ok() => {
            output.push(0xd9);
            output.push(length as u8);
        }
        BlobKind::String if u16::try_from(length).is_ok() => {
            output.push(0xda);
            output.extend_from_slice(&(length as u16).to_be_bytes());
        }
        BlobKind::String => {
            output.push(0xdb);
            output.extend_from_slice(
                &u32::try_from(length)
                    .map_err(|_| "string too large")?
                    .to_be_bytes(),
            );
        }
        BlobKind::Binary if u8::try_from(length).is_ok() => {
            output.push(0xc4);
            output.push(length as u8);
        }
        BlobKind::Binary if u16::try_from(length).is_ok() => {
            output.push(0xc5);
            output.extend_from_slice(&(length as u16).to_be_bytes());
        }
        BlobKind::Binary => {
            output.push(0xc6);
            output.extend_from_slice(
                &u32::try_from(length)
                    .map_err(|_| "binary too large")?
                    .to_be_bytes(),
            );
        }
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn encode_array_len(length: usize, output: &mut Vec<u8>) -> Result<(), String> {
    if length <= 15 {
        output.push(0x90 | length as u8);
    } else if u16::try_from(length).is_ok() {
        output.push(0xdc);
        output.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        output.push(0xdd);
        output.extend_from_slice(
            &u32::try_from(length)
                .map_err(|_| "array too large")?
                .to_be_bytes(),
        );
    }
    Ok(())
}

fn encode_map_len(length: usize, output: &mut Vec<u8>) -> Result<(), String> {
    if length <= 15 {
        output.push(0x80 | length as u8);
    } else if u16::try_from(length).is_ok() {
        output.push(0xde);
        output.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        output.push(0xdf);
        output.extend_from_slice(
            &u32::try_from(length)
                .map_err(|_| "map too large")?
                .to_be_bytes(),
        );
    }
    Ok(())
}

pub fn array(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Array(values.into_iter().collect())
}

pub fn map(values: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        values
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_rpc_value_markers_and_extensions() {
        let value = array([
            Value::Nil,
            Value::Bool(true),
            Value::Integer(-34),
            Value::Unsigned(u64::from(u32::MAX) + 1),
            Value::Float(1.25),
            Value::String("界".into()),
            Value::Binary(vec![0, 1, 2]),
            map([("key", Value::Unsigned(7))]),
            Value::Ext(0, vec![42]),
        ]);
        let encoded = encode(&value).unwrap();
        assert_eq!(decode(&mut Cursor::new(encoded)).unwrap(), value);
    }

    struct OneByteReader {
        bytes: Cursor<Vec<u8>>,
    }

    impl Read for OneByteReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let limit = output.len().min(1);
            self.bytes.read(&mut output[..limit])
        }
    }

    fn fragmented(bytes: Vec<u8>) -> Value {
        decode(&mut OneByteReader {
            bytes: Cursor::new(bytes),
        })
        .unwrap()
    }

    #[test]
    fn decodes_every_supported_marker_from_fragmented_input() {
        let cases = vec![
            (vec![0x2a], Value::Unsigned(42)),
            (vec![0xff], Value::Integer(-1)),
            (vec![0xc0], Value::Nil),
            (vec![0xc2], Value::Bool(false)),
            (vec![0xc3], Value::Bool(true)),
            (vec![0xcc, 0xff], Value::Unsigned(255)),
            (vec![0xcd, 0x01, 0x00], Value::Unsigned(256)),
            (vec![0xce, 0, 1, 0, 0], Value::Unsigned(65_536)),
            (vec![0xcf, 0, 0, 0, 1, 0, 0, 0, 0], Value::Unsigned(1 << 32)),
            (vec![0xd0, 0xfe], Value::Integer(-2)),
            (vec![0xd1, 0xff, 0xfe], Value::Integer(-2)),
            (vec![0xd2, 0xff, 0xff, 0xff, 0xfe], Value::Integer(-2)),
            (
                vec![0xd3, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe],
                Value::Integer(-2),
            ),
            (vec![0xca, 0x3f, 0xc0, 0, 0], Value::Float(1.5)),
            (vec![0xcb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0], Value::Float(1.5)),
            (vec![0xa1, b'a'], Value::String("a".into())),
            (vec![0xd9, 1, b'a'], Value::String("a".into())),
            (vec![0xda, 0, 1, b'a'], Value::String("a".into())),
            (vec![0xdb, 0, 0, 0, 1, b'a'], Value::String("a".into())),
            (vec![0xc4, 1, 7], Value::Binary(vec![7])),
            (vec![0xc5, 0, 1, 7], Value::Binary(vec![7])),
            (vec![0xc6, 0, 0, 0, 1, 7], Value::Binary(vec![7])),
            (vec![0x91, 0xc0], Value::Array(vec![Value::Nil])),
            (vec![0xdc, 0, 1, 0xc0], Value::Array(vec![Value::Nil])),
            (vec![0xdd, 0, 0, 0, 1, 0xc0], Value::Array(vec![Value::Nil])),
            (
                vec![0x81, 0xa1, b'k', 0xc0],
                Value::Map(vec![(Value::String("k".into()), Value::Nil)]),
            ),
            (
                vec![0xde, 0, 1, 0xa1, b'k', 0xc0],
                Value::Map(vec![(Value::String("k".into()), Value::Nil)]),
            ),
            (
                vec![0xdf, 0, 0, 0, 1, 0xa1, b'k', 0xc0],
                Value::Map(vec![(Value::String("k".into()), Value::Nil)]),
            ),
        ];
        for (bytes, expected) in cases {
            assert_eq!(fragmented(bytes), expected);
        }

        for (marker, length) in [(0xd4, 1), (0xd5, 2), (0xd6, 4), (0xd7, 8), (0xd8, 16)] {
            let mut bytes = vec![marker, 3];
            bytes.extend(vec![9; length]);
            assert_eq!(fragmented(bytes), Value::Ext(3, vec![9; length]));
        }
        for (header, length) in [
            (vec![0xc7, 1], 1),
            (vec![0xc8, 0, 1], 1),
            (vec![0xc9, 0, 0, 0, 1], 1),
        ] {
            let mut bytes = header;
            bytes.extend([3, 9]);
            assert_eq!(fragmented(bytes), Value::Ext(3, vec![9; length]));
        }
    }

    #[test]
    fn exact_message_depth_container_and_node_bounds_are_enforced() {
        let payload_length = MAX_MESSAGE_BYTES - 5;
        let mut exact_message = vec![0xdb];
        exact_message.extend_from_slice(&(payload_length as u32).to_be_bytes());
        exact_message.extend(vec![b'a'; payload_length]);
        assert_eq!(
            decode(&mut Cursor::new(exact_message))
                .unwrap()
                .as_str()
                .unwrap()
                .len(),
            payload_length
        );
        let oversized_length = payload_length + 1;
        let mut oversized_message = vec![0xdb];
        oversized_message.extend_from_slice(&(oversized_length as u32).to_be_bytes());
        assert!(decode(&mut Cursor::new(oversized_message)).is_err());

        let mut exact_depth = vec![0x91; MAX_DEPTH];
        exact_depth.push(0xc0);
        assert!(decode(&mut Cursor::new(exact_depth)).is_ok());
        let mut excessive_depth = vec![0x91; MAX_DEPTH + 1];
        excessive_depth.push(0xc0);
        assert!(
            decode(&mut Cursor::new(excessive_depth))
                .unwrap_err()
                .contains("nesting")
        );

        let mut exact_container = vec![0xdd];
        exact_container.extend_from_slice(&(MAX_CONTAINER_ITEMS as u32).to_be_bytes());
        exact_container.extend(vec![0xc0; MAX_CONTAINER_ITEMS]);
        assert_eq!(
            decode(&mut Cursor::new(exact_container))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            MAX_CONTAINER_ITEMS
        );
        let mut excessive_container = vec![0xdd];
        excessive_container.extend_from_slice(&((MAX_CONTAINER_ITEMS + 1) as u32).to_be_bytes());
        assert!(
            decode(&mut Cursor::new(excessive_container))
                .unwrap_err()
                .contains("container")
        );

        let mut excessive_nodes = vec![0xdf];
        excessive_nodes.extend_from_slice(&(MAX_CONTAINER_ITEMS as u32).to_be_bytes());
        excessive_nodes.push(0xc0);
        excessive_nodes.push(0xdd);
        excessive_nodes.extend_from_slice(&(100_000_u32).to_be_bytes());
        excessive_nodes.extend(vec![0xc0; 100_000]);
        excessive_nodes.extend(vec![0xc0; (MAX_CONTAINER_ITEMS - 1) * 2]);
        assert!(
            decode(&mut Cursor::new(excessive_nodes))
                .unwrap_err()
                .contains("node bound")
        );
    }

    #[test]
    fn rejects_reserved_truncated_deep_and_oversized_values() {
        assert!(
            decode(&mut Cursor::new([0xc1]))
                .unwrap_err()
                .contains("reserved")
        );
        assert!(
            decode(&mut Cursor::new([0xda, 0, 3, b'a']))
                .unwrap_err()
                .contains("mid-value")
        );
        let mut deep = vec![0x91; MAX_DEPTH + 2];
        deep.push(0xc0);
        assert!(
            decode(&mut Cursor::new(deep))
                .unwrap_err()
                .contains("nesting")
        );
        let oversized = [0xdb, 0, 64, 0, 1];
        assert!(decode(&mut Cursor::new(oversized)).is_err());
    }
}
