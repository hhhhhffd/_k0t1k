//! Minimal NBT (Minecraft's binary tag format) reader and writer, enough to read `level.dat` and to
//! edit `level.dat` and `servers.dat` without losing the tags this code does not know about.
//!
//! A tiny hand-written codec instead of a crate: the files must be written back, so every tag type has
//! to round-trip in order and strings need Java's "modified UTF-8" (supplementary characters such as
//! emoji in server names are stored as surrogate pairs, which a plain UTF-8 codec would corrupt).

use std::io::Read;
use std::path::Path;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::error::{Error, IoContext, Result};

/// Compound tags nest at most this deep; Minecraft itself rejects deeper data, and it keeps the
/// recursive reader from overflowing the stack on a hostile file.
const MAX_DEPTH: usize = 512;
/// Decompressed size cap for gzip files (a `level.dat` is a few KiB; this only stops zip bombs).
const MAX_DECOMPRESSED: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    /// Element type id and the elements (all of that type).
    List(u8, Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// An ordered set of named tags.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound(pub Vec<(String, Tag)>);

impl Compound {
    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.0.iter().find(|(key, _)| key == name).map(|(_, tag)| tag)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Tag> {
        self.0.iter_mut().find(|(key, _)| key == name).map(|(_, tag)| tag)
    }

    /// Replaces the tag of that name in place, or appends it.
    pub fn set(&mut self, name: &str, tag: Tag) {
        match self.get_mut(name) {
            Some(existing) => *existing = tag,
            None => self.0.push((name.to_owned(), tag)),
        }
    }

    pub fn compound(&self, name: &str) -> Option<&Compound> {
        match self.get(name) {
            Some(Tag::Compound(compound)) => Some(compound),
            _ => None,
        }
    }

    pub fn compound_mut(&mut self, name: &str) -> Option<&mut Compound> {
        match self.get_mut(name) {
            Some(Tag::Compound(compound)) => Some(compound),
            _ => None,
        }
    }

    pub fn string(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(Tag::String(value)) => Some(value),
            _ => None,
        }
    }

    /// Any integer tag, widened; Minecraft stores flags as bytes and counters as ints or longs.
    pub fn integer(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            Tag::Byte(value) => Some(i64::from(*value)),
            Tag::Short(value) => Some(i64::from(*value)),
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::Other(format!("invalid NBT: {message}"))
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.data.len() < len {
            return Err(invalid("unexpected end of data"));
        }
        let (head, rest) = self.data.split_at(len);
        self.data = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }

    fn length(&mut self) -> Result<usize> {
        usize::try_from(i32::from_be_bytes(self.array()?)).map_err(|_| invalid("negative length"))
    }

    fn string(&mut self) -> Result<String> {
        let len = usize::from(u16::from_be_bytes(self.array()?));
        decode_modified_utf8(self.take(len)?)
    }

    /// A payload of `count` fixed-size items; the size is checked against the data left first so a
    /// forged count cannot make the reader allocate gigabytes.
    fn items<T, const N: usize>(&mut self, count: usize, parse: fn([u8; N]) -> T) -> Result<Vec<T>> {
        let bytes = self.take(count.checked_mul(N).ok_or_else(|| invalid("length overflow"))?)?;
        let (chunks, _) = bytes.as_chunks::<N>();
        Ok(chunks.iter().map(|&chunk| parse(chunk)).collect())
    }

    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag> {
        if depth > MAX_DEPTH {
            return Err(invalid("nesting is too deep"));
        }
        Ok(match id {
            1 => Tag::Byte(i8::from_be_bytes(self.array()?)),
            2 => Tag::Short(i16::from_be_bytes(self.array()?)),
            3 => Tag::Int(i32::from_be_bytes(self.array()?)),
            4 => Tag::Long(i64::from_be_bytes(self.array()?)),
            5 => Tag::Float(f32::from_be_bytes(self.array()?)),
            6 => Tag::Double(f64::from_be_bytes(self.array()?)),
            7 => {
                let count = self.length()?;
                Tag::ByteArray(self.items(count, i8::from_be_bytes)?)
            }
            8 => Tag::String(self.string()?),
            9 => {
                let element = self.array::<1>()?[0];
                let count = self.length()?;
                if element == 0 && count > 0 {
                    return Err(invalid("list of End tags"));
                }
                // Every element takes at least one byte, so this bounds the allocation.
                if count > self.data.len() {
                    return Err(invalid("unexpected end of data"));
                }
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.payload(element, depth + 1)?);
                }
                Tag::List(element, items)
            }
            10 => Tag::Compound(self.compound(depth + 1)?),
            11 => {
                let count = self.length()?;
                Tag::IntArray(self.items(count, i32::from_be_bytes)?)
            }
            12 => {
                let count = self.length()?;
                Tag::LongArray(self.items(count, i64::from_be_bytes)?)
            }
            other => return Err(invalid(&format!("unknown tag type {other}"))),
        })
    }

    fn compound(&mut self, depth: usize) -> Result<Compound> {
        let mut entries = Vec::new();
        loop {
            let id = self.array::<1>()?[0];
            if id == 0 {
                return Ok(Compound(entries));
            }
            let name = self.string()?;
            entries.push((name, self.payload(id, depth)?));
        }
    }
}

/// Java's modified UTF-8: NUL is `C0 80` and supplementary characters are two 3-byte surrogates.
fn decode_modified_utf8(bytes: &[u8]) -> Result<String> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let continuation = |offset: usize| {
            bytes
                .get(index + offset)
                .filter(|byte| *byte & 0xC0 == 0x80)
                .map(|byte| u16::from(byte & 0x3F))
                .ok_or_else(|| invalid("malformed string"))
        };
        match first {
            0x01..=0x7F => {
                units.push(u16::from(first));
                index += 1;
            }
            0xC0..=0xDF => {
                units.push(u16::from(first & 0x1F) << 6 | continuation(1)?);
                index += 2;
            }
            0xE0..=0xEF => {
                units.push(u16::from(first & 0x0F) << 12 | continuation(1)? << 6 | continuation(2)?);
                index += 3;
            }
            _ => return Err(invalid("malformed string")),
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

fn encode_modified_utf8(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for unit in text.encode_utf16() {
        match unit {
            0x01..=0x7F => bytes.push(unit as u8),
            0x00 | 0x80..=0x7FF => {
                bytes.push(0xC0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                bytes.push(0xE0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    bytes
}

fn write_string(out: &mut Vec<u8>, text: &str) -> Result<()> {
    let bytes = encode_modified_utf8(text);
    let len = u16::try_from(bytes.len()).map_err(|_| Error::Other("NBT string is longer than 65535 bytes".into()))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}

fn write_length(out: &mut Vec<u8>, len: usize) -> Result<()> {
    let len = i32::try_from(len).map_err(|_| Error::Other("NBT array is too long".into()))?;
    out.extend_from_slice(&len.to_be_bytes());
    Ok(())
}

fn type_id(tag: &Tag) -> u8 {
    match tag {
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 3,
        Tag::Long(_) => 4,
        Tag::Float(_) => 5,
        Tag::Double(_) => 6,
        Tag::ByteArray(_) => 7,
        Tag::String(_) => 8,
        Tag::List(..) => 9,
        Tag::Compound(_) => 10,
        Tag::IntArray(_) => 11,
        Tag::LongArray(_) => 12,
    }
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag) -> Result<()> {
    match tag {
        Tag::Byte(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Short(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Int(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Long(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Float(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::Double(value) => out.extend_from_slice(&value.to_be_bytes()),
        Tag::ByteArray(values) => {
            write_length(out, values.len())?;
            out.extend(values.iter().map(|value| *value as u8));
        }
        Tag::String(text) => write_string(out, text)?,
        Tag::List(element, items) => {
            out.push(*element);
            write_length(out, items.len())?;
            for item in items {
                if type_id(item) != *element {
                    return Err(Error::Other("NBT list holds tags of different types".into()));
                }
                write_payload(out, item)?;
            }
        }
        Tag::Compound(compound) => write_compound(out, compound)?,
        Tag::IntArray(values) => {
            write_length(out, values.len())?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Tag::LongArray(values) => {
            write_length(out, values.len())?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(())
}

fn write_compound(out: &mut Vec<u8>, compound: &Compound) -> Result<()> {
    for (name, tag) in &compound.0 {
        out.push(type_id(tag));
        write_string(out, name)?;
        write_payload(out, tag)?;
    }
    out.push(0);
    Ok(())
}

/// Parses an uncompressed NBT document: the root compound and its name.
pub fn parse(data: &[u8]) -> Result<(String, Compound)> {
    let mut reader = Reader { data };
    if reader.array::<1>()?[0] != 10 {
        return Err(invalid("the root is not a compound"));
    }
    let name = reader.string()?;
    Ok((name, reader.compound(1)?))
}

pub fn serialize(name: &str, root: &Compound) -> Result<Vec<u8>> {
    let mut out = vec![10];
    write_string(&mut out, name)?;
    write_compound(&mut out, root)?;
    Ok(out)
}

/// Reads an NBT file, gzip-compressed (`level.dat`) or plain (`servers.dat`), detected by the gzip magic.
pub fn read_file(path: &Path) -> Result<(String, Compound)> {
    let raw = std::fs::read(path).at(path)?;
    let data = if raw.starts_with(&[0x1F, 0x8B]) {
        let mut data = Vec::new();
        GzDecoder::new(raw.as_slice()).take(MAX_DECOMPRESSED + 1).read_to_end(&mut data).at(path)?;
        if data.len() as u64 > MAX_DECOMPRESSED {
            return Err(Error::Other(format!("{} is too large when decompressed", path.display())));
        }
        data
    } else {
        raw
    };
    parse(&data).map_err(|error| Error::Other(format!("{}: {error}", path.display())))
}

/// Writes through a temporary file and a rename, so a crash cannot leave a half-written save file.
pub fn write_file(path: &Path, name: &str, root: &Compound, gzip: bool) -> Result<()> {
    let data = serialize(name, root)?;
    let data = if gzip {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &data).at(path)?;
        encoder.finish().at(path)?
    } else {
        data
    };
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, data).at(&temp)?;
    std::fs::rename(&temp, path).at(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Compound {
        Compound(vec![
            ("byte".into(), Tag::Byte(-3)),
            ("long".into(), Tag::Long(i64::MIN)),
            ("float".into(), Tag::Float(1.5)),
            ("double".into(), Tag::Double(-2.25)),
            ("text".into(), Tag::String("Мир 🌍\0".into())),
            ("bytes".into(), Tag::ByteArray(vec![1, -2])),
            ("ints".into(), Tag::IntArray(vec![7, 8])),
            ("longs".into(), Tag::LongArray(vec![9])),
            ("list".into(), Tag::List(8, vec![Tag::String("a".into())])),
            ("empty".into(), Tag::List(0, vec![])),
            ("nested".into(), Tag::Compound(Compound(vec![("x".into(), Tag::Short(5))]))),
        ])
    }

    #[test]
    fn every_tag_type_round_trips_plain_and_gzip() {
        let dir = tempfile::tempdir().unwrap();
        for (file, gzip) in [("plain.dat", false), ("packed.dat", true)] {
            let path = dir.path().join(file);
            write_file(&path, "root", &sample(), gzip).unwrap();
            let (name, root) = read_file(&path).unwrap();
            assert_eq!(name, "root");
            assert_eq!(root, sample());
        }
    }

    #[test]
    fn strings_use_java_modified_utf8() {
        // NUL is C0 80, and U+1F30D is the surrogate pair D83C DF0D as two 3-byte sequences.
        assert_eq!(encode_modified_utf8("\0"), [0xC0, 0x80]);
        assert_eq!(encode_modified_utf8("🌍"), [0xED, 0xA0, 0xBC, 0xED, 0xBC, 0x8D]);
        assert_eq!(decode_modified_utf8(&[0xED, 0xA0, 0xBC, 0xED, 0xBC, 0x8D]).unwrap(), "🌍");
    }

    #[test]
    fn malformed_data_is_an_error_not_a_panic() {
        let bytes = serialize("r", &sample()).unwrap();
        for cut in 0..bytes.len() {
            assert!(parse(&bytes[..cut]).is_err(), "truncated at {cut}");
        }
        // A list claiming a billion elements must fail on the length check, not allocate.
        let forged = [10, 0, 0, 9, 0, 1, b'a', 3, 0x3B, 0x9A, 0xCA, 0x00];
        assert!(parse(&forged).is_err());
        assert!(parse(&[1, 0, 0, 0]).is_err());
    }

    #[test]
    fn accessors_widen_integers() {
        let root = sample();
        assert_eq!(root.integer("byte"), Some(-3));
        assert_eq!(root.string("text"), Some("Мир 🌍\0"));
        assert_eq!(root.compound("nested").and_then(|c| c.integer("x")), Some(5));
        assert_eq!(root.integer("text"), None);
    }
}
