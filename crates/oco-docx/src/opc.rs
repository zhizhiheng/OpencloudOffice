//! Open Packaging Conventions (ECMA-376 Part 2): the zip container and the
//! relationship parts that link its contents together.

use std::collections::HashMap;
use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::xmltree;
use crate::zip::{self, Archive};

/// Upper bound on the uncompressed size of a single part. Guards against
/// decompression bombs; real document parts are orders of magnitude smaller.
pub const MAX_PART_SIZE: u64 = 256 * 1024 * 1024;

pub struct Package<R> {
    archive: Archive<R>,
    /// Normalised part name -> zip entry index.
    index: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    /// Part name inside the package (no leading slash), or the raw URI when `external`.
    pub target: String,
    pub external: bool,
}

impl<R: Read + Seek> Package<R> {
    pub fn new(reader: R) -> Result<Self> {
        let archive = Archive::new(reader)?;
        let mut index = HashMap::new();
        for (i, entry) in archive.entries().iter().enumerate() {
            index.entry(normalize(&entry.name)).or_insert(i);
        }
        Ok(Self { archive, index })
    }

    pub fn has_part(&self, name: &str) -> bool {
        self.index.contains_key(&normalize(name))
    }

    /// Reads a part's bytes, or `None` if the package has no such part.
    pub fn read_part(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(&i) = self.index.get(&normalize(name)) else {
            return Ok(None);
        };
        // The size an entry declares is not to be trusted; the archive caps the actual read too.
        match self.archive.read(i, MAX_PART_SIZE) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(zip::Error::TooLarge) => Err(Error::PartTooLarge { part: name.to_owned(), limit: MAX_PART_SIZE }),
            Err(error) => Err(error.into()),
        }
    }

    /// A part still as the archive holds it, or `None` if the package has no such part.
    pub fn read_packed(&mut self, name: &str) -> Result<Option<zip::Packed>> {
        match self.index.get(&normalize(name)) {
            Some(&i) => Ok(Some(self.archive.read_packed(i)?)),
            None => Ok(None),
        }
    }

    /// Reads an XML part as text. Parts are UTF-8 or UTF-16 (Part 2 §8.1.4).
    pub fn read_xml(&mut self, name: &str) -> Result<Option<String>> {
        let Some(bytes) = self.read_part(name)? else {
            return Ok(None);
        };
        decode_xml(bytes).map(Some).ok_or_else(|| Error::Encoding(name.to_owned()))
    }

    /// Relationships of `source`, or of the package itself when `source` is `None`.
    /// A missing relationships part means "no relationships".
    pub fn relationships(&mut self, source: Option<&str>) -> Result<Vec<Relationship>> {
        let rels_part = match source {
            None => "_rels/.rels".to_owned(),
            Some(source) => {
                let (dir, file) = split_dir(source);
                format!("{dir}_rels/{file}.rels")
            }
        };
        let Some(xml) = self.read_xml(&rels_part)? else {
            return Ok(Vec::new());
        };
        let doc = xmltree::Document::parse(&xml).map_err(|source| Error::Xml { part: rels_part.clone(), source })?;
        let base_dir = source.map(|s| split_dir(s).0).unwrap_or("");
        let relationships = doc
            .root_element()
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
            .filter_map(|n| {
                let raw_target = n.attribute("Target")?;
                let external = n.attribute("TargetMode") == Some("External");
                Some(Relationship {
                    id: n.attribute("Id").unwrap_or_default().to_owned(),
                    rel_type: n.attribute("Type").unwrap_or_default().to_owned(),
                    target: if external { raw_target.to_owned() } else { resolve_target(base_dir, raw_target) },
                    external,
                })
            })
            .collect();
        Ok(relationships)
    }
}

/// Part names compare case-insensitively (Part 2 §6.2.2.3), and some producers
/// write backslashes or a leading slash into zip entry names.
fn normalize(name: &str) -> String {
    name.trim_start_matches(['/', '\\']).replace('\\', "/").to_ascii_lowercase()
}

/// Splits `word/document.xml` into (`word/`, `document.xml`).
fn split_dir(part: &str) -> (&str, &str) {
    match part.rfind('/') {
        Some(i) => part.split_at(i + 1),
        None => ("", part),
    }
}

/// Resolves a relationship target against the directory of its source part.
fn resolve_target(base_dir: &str, target: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let path = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => format!("{base_dir}{target}"),
    };
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

fn decode_xml(bytes: Vec<u8>) -> Option<String> {
    match bytes.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).ok(),
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8(bytes).ok(),
    }
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relationship_targets() {
        assert_eq!(resolve_target("word/", "styles.xml"), "word/styles.xml");
        assert_eq!(resolve_target("word/", "theme/theme1.xml"), "word/theme/theme1.xml");
        assert_eq!(resolve_target("word/", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve_target("word/", "/word/styles.xml"), "word/styles.xml");
        assert_eq!(resolve_target("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve_target("", "../../escape.xml"), "escape.xml");
    }

    #[test]
    fn decodes_utf16_parts() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("<a/>".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("<a/>".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_xml(le).as_deref(), Some("<a/>"));
        assert_eq!(decode_xml(be).as_deref(), Some("<a/>"));
        assert_eq!(decode_xml(b"\xEF\xBB\xBF<a/>".to_vec()).as_deref(), Some("<a/>"));
    }
}
