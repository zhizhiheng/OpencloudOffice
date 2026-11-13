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

/// Content types of the parts of a word-processing package.
pub mod content_type {
    pub const DOCUMENT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
    pub const STYLES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
    pub const NUMBERING: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
    pub const SETTINGS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
    pub const FONT_TABLE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml";
    pub const HEADER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
    pub const FOOTER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
    pub const FOOTNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
    pub const ENDNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
    pub const THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
    pub const APP_PROPERTIES: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
    pub const CORE_PROPERTIES: &str = "application/vnd.openxmlformats-package.core-properties+xml";
}

/// Kinds of relationship between the parts of a word-processing package.
pub mod relationship {
    const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    pub const DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    pub const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    pub const NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
    pub const SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
    pub const FONT_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable";
    pub const HEADER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
    pub const FOOTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
    pub const FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
    pub const ENDNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
    pub const THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
    pub const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    pub const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    pub const APP_PROPERTIES: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
    pub const CORE_PROPERTIES: &str =
        "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
    /// The namespace the ids of relationships are written in (`r:id`, `r:embed`).
    pub const NAMESPACE: &str = OFFICE;
}

/// Puts a package together: the parts, what kind each is, and how they
/// refer to one another. What [`Builder::finish`] returns is a `.docx` file
/// that this crate reads back and that Word opens.
///
/// ```
/// use oco_docx::opc::{content_type, relationship, Builder};
/// let mut package = Builder::new();
/// let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// let document = format!(r#"<w:document xmlns:w="{w}"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#);
/// package.part("word/document.xml", content_type::DOCUMENT, document.as_bytes());
/// package.relate(None, relationship::DOCUMENT, "word/document.xml");
/// let docx = oco_docx::Docx::from_bytes(&package.finish()).unwrap();
/// assert_eq!(docx.document.paragraphs()[0].text(), "Hello");
/// ```
#[derive(Default)]
pub struct Builder {
    zip: crate::zip::Writer,
    /// Parts with a content type of their own.
    overrides: Vec<(String, String)>,
    /// Extensions of the files added, which are told by them.
    extensions: Vec<String>,
    /// The relationships of each source part; of the package itself under "".
    relationships: Vec<(String, Vec<String>)>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a part, named from the root of the package without a leading
    /// slash, such as `word/document.xml`, with its content type.
    pub fn part(&mut self, name: &str, content_type: &str, content: &[u8]) {
        self.overrides.push((name.to_owned(), content_type.to_owned()));
        self.zip.add(name, content);
    }

    /// Adds a file whose kind its extension tells, such as a picture:
    /// `word/media/image1.png`.
    pub fn file(&mut self, name: &str, content: &[u8]) {
        let extension = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
        if !self.extensions.contains(&extension) {
            self.extensions.push(extension);
        }
        self.zip.add(name, content);
    }

    /// Records that `source` refers to the part `target` (both named from
    /// the root of the package), and returns the id to refer to it by.
    /// The package itself is the source when `source` is `None`.
    pub fn relate(&mut self, source: Option<&str>, kind: &str, target: &str) -> String {
        self.add_relationship(source, kind, &format!("/{target}"), false)
    }

    /// As [`Builder::relate`], for something outside the package, such as
    /// the address a hyperlink leads to.
    pub fn relate_outside(&mut self, source: Option<&str>, kind: &str, address: &str) -> String {
        self.add_relationship(source, kind, address, true)
    }

    fn add_relationship(&mut self, source: Option<&str>, kind: &str, target: &str, outside: bool) -> String {
        let source = source.unwrap_or_default();
        let at = match self.relationships.iter().position(|(part, _)| part == source) {
            Some(at) => at,
            None => {
                self.relationships.push((source.to_owned(), Vec::new()));
                self.relationships.len() - 1
            }
        };
        let list = &mut self.relationships[at].1;
        let id = format!("rId{}", list.len() + 1);
        let mode = if outside { r#" TargetMode="External""# } else { "" };
        let (kind, target) = (crate::xmltree::escape(kind), crate::xmltree::escape(target));
        list.push(format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"{mode}/>"#));
        id
    }

    /// The finished package.
    pub fn finish(mut self) -> Vec<u8> {
        const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006";
        const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
        for (source, list) in &self.relationships {
            let name = match source.rsplit_once('/') {
                _ if source.is_empty() => "_rels/.rels".to_owned(),
                Some((folder, file)) => format!("{folder}/_rels/{file}.rels"),
                None => format!("_rels/{source}.rels"),
            };
            let xml =
                format!(r#"{HEAD}<Relationships xmlns="{PACKAGE}/relationships">{}</Relationships>"#, list.concat());
            self.zip.add(&name, xml.as_bytes());
        }
        let mut types = format!(
            r#"{HEAD}<Types xmlns="{PACKAGE}/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#
        );
        for extension in &self.extensions {
            let kind = match extension.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "bmp" => "image/bmp",
                "webp" => "image/webp",
                "svg" => "image/svg+xml",
                "tif" | "tiff" => "image/tiff",
                "emf" => "image/x-emf",
                "wmf" => "image/x-wmf",
                "rels" | "xml" => continue,
                _ => "application/octet-stream",
            };
            types.push_str(&format!(r#"<Default Extension="{extension}" ContentType="{kind}"/>"#));
        }
        for (part, kind) in &self.overrides {
            let (part, kind) = (crate::xmltree::escape(part), crate::xmltree::escape(kind));
            types.push_str(&format!(r#"<Override PartName="/{part}" ContentType="{kind}"/>"#));
        }
        types.push_str("</Types>");
        self.zip.add("[Content_Types].xml", types.as_bytes());
        self.zip.finish()
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
    fn a_built_package_reads_back_with_its_parts_and_relationships() {
        let mut builder = Builder::new();
        builder.part("word/document.xml", content_type::DOCUMENT, b"<document/>");
        builder.file("word/media/image1.PNG", b"picture");
        let document = builder.relate(None, relationship::DOCUMENT, "word/document.xml");
        let picture = builder.relate(Some("word/document.xml"), relationship::IMAGE, "word/media/image1.PNG");
        let link =
            builder.relate_outside(Some("word/document.xml"), relationship::HYPERLINK, "https://example.org/?a=1&b=2");
        assert_eq!((document.as_str(), picture.as_str(), link.as_str()), ("rId1", "rId1", "rId2"));

        let mut package = Package::new(std::io::Cursor::new(builder.finish())).unwrap();
        assert_eq!(package.relationships(None).unwrap()[0].target, "word/document.xml");
        let own = package.relationships(Some("word/document.xml")).unwrap();
        assert_eq!((own[0].target.as_str(), own[0].external), ("word/media/image1.PNG", false));
        assert_eq!((own[1].target.as_str(), own[1].external), ("https://example.org/?a=1&b=2", true));
        assert_eq!(package.read_part("word/media/image1.png").unwrap().as_deref(), Some(&b"picture"[..]));
        let types = package.read_xml("[Content_Types].xml").unwrap().unwrap();
        assert!(types.contains(r#"<Default Extension="png" ContentType="image/png"/>"#), "{types}");
        assert!(types.contains(&format!(
            r#"<Override PartName="/word/document.xml" ContentType="{}"/>"#,
            content_type::DOCUMENT
        )));
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
