//! One way in for every kind of document the core shows.
//!
//! Whatever a file is, it is shown from the document form: the
//! WordprocessingML package that a `.docx` file already is. A binary Word
//! document or a Markdown text is first turned into such a package, and
//! from there on is read, laid out and drawn as any other.

use std::fmt;
use std::path::Path;

use oco_docx::Docx;

/// The kinds of document that can be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A WordprocessingML package: `.docx`, `.docm`, `.dotx`, `.dotm`.
    Docx,
    /// The binary format of Word 97 to 2003: `.doc`, `.dot`.
    Doc,
    /// Markdown, or plain text, which reads as Markdown.
    Markdown,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Docx(oco_docx::Error),
    Doc(oco_doc::Error),
    /// Neither a kind of document that is known nor text.
    Unknown,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "i/o error: {error}"),
            Error::Docx(error) => error.fmt(f),
            Error::Doc(error) => error.fmt(f),
            Error::Unknown => f.write_str("not a kind of document that can be shown"),
        }
    }
}

impl std::error::Error for Error {}

/// What kind of document `bytes` are. The content decides where it can: a
/// file is what it holds, whatever it is called. The name, if there is one,
/// decides between kinds that the content alone does not tell apart.
pub fn detect(bytes: &[u8], name: Option<&str>) -> Option<Format> {
    const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if bytes.starts_with(&COMPOUND_FILE) {
        return Some(Format::Doc);
    }
    if bytes.starts_with(b"PK") {
        return Some(Format::Docx);
    }
    let extension = name.and_then(|name| name.rsplit_once('.')).map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("md" | "markdown" | "mdown" | "mkd" | "txt") => return Some(Format::Markdown),
        // Named as a Word document but not one: the reader of that kind says what is wrong.
        Some("docx" | "docm" | "dotx" | "dotm") => return Some(Format::Docx),
        Some("doc" | "dot") => return Some(Format::Doc),
        _ => {}
    }
    let text = match bytes {
        [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..] => true,
        _ => !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok(),
    };
    text.then_some(Format::Markdown)
}

/// The text of a Markdown file: UTF-8, or UTF-16 with a byte order mark.
fn text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|pair| unit([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The document form of a file that is not in it already: a `.docx`
/// package made from a binary Word document or from Markdown. `None` for
/// a file that is a package as it is. `image` supplies the images a
/// Markdown text shows, by their addresses as written.
pub fn converted(
    bytes: &[u8],
    name: Option<&str>,
    image: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, Error> {
    match detect(bytes, name).ok_or(Error::Unknown)? {
        Format::Docx => Ok(None),
        Format::Doc => oco_doc::to_docx(bytes).map(Some).map_err(Error::Doc),
        Format::Markdown => Ok(Some(oco_markdown::to_docx(&text(bytes), image))),
    }
}

/// Opens a document of any kind that is known. See [`detect`] for `name`.
pub fn open(bytes: &[u8], name: Option<&str>) -> Result<Docx, Error> {
    open_with(bytes, name, &|_| None)
}

/// As [`open`], with the images a Markdown text shows supplied by `image`.
pub fn open_with(bytes: &[u8], name: Option<&str>, image: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Docx, Error> {
    let package = converted(bytes, name, image)?;
    Docx::from_bytes(package.as_deref().unwrap_or(bytes)).map_err(Error::Docx)
}

/// Opens a file. Images of a Markdown file are looked for beside it.
pub fn open_path(path: impl AsRef<Path>) -> Result<Docx, Error> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(Error::Io)?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
    let beside = |address: &str| {
        // Only files beside the document or below it: an address is not a way out of its folder.
        let relative = Path::new(address);
        let inside = relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
        inside.then(|| std::fs::read(path.parent()?.join(relative)).ok()).flatten()
    };
    open_with(&bytes, name.as_deref(), &beside)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_what_it_holds_and_its_name_decides_the_rest() {
        let compound = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0];
        assert_eq!(detect(&compound, Some("report.docx")), Some(Format::Doc));
        assert_eq!(detect(b"PK\x03\x04....", Some("notes.md")), Some(Format::Docx));
        assert_eq!(detect("# 标题\n".as_bytes(), None), Some(Format::Markdown));
        assert_eq!(detect("# 标题\n".as_bytes(), Some("README.MD")), Some(Format::Markdown));
        assert_eq!(detect(b"\xFF\xFE#\x00", None), Some(Format::Markdown));
        // Text that is called a Word document is a broken Word document, not Markdown.
        assert_eq!(detect(b"plain text", Some("notes.docx")), Some(Format::Docx));
        assert!(matches!(open(b"plain text", Some("notes.docx")), Err(Error::Docx(_))));
        assert_eq!(detect(b"\x00\x01\x02binary", None), None);
        assert!(matches!(open(b"\x00\x01\x02binary", Some("data.bin")), Err(Error::Unknown)));
    }

    #[test]
    fn markdown_opens_as_a_document_in_any_of_its_encodings() {
        let text = "第一段\nsecond";
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let with_mark: Vec<u8> = [0xEF, 0xBB, 0xBF].into_iter().chain(text.bytes()).collect();
        for bytes in [text.as_bytes().to_vec(), utf16, with_mark] {
            let docx = open(&bytes, Some("a.md")).unwrap();
            assert_eq!(docx.document.paragraphs().iter().map(|p| p.text()).collect::<Vec<_>>(), ["第一段", "second"]);
        }
    }
}
