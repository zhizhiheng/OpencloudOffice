//! Reader for WordprocessingML (`.docx`) documents.
//!
//! [`Docx::open`] loads the package and parses the document, its styles and
//! its theme fonts. [`Docx::resolver`] then answers the question a layout
//! engine needs answered: given the style hierarchy, what does each paragraph
//! and run actually look like?
//!
//! This crate only reads. Markup it does not understand is counted in
//! [`Docx::coverage`] rather than preserved, so it is not yet a basis for
//! writing documents back.

mod coverage;
mod document;
mod error;
pub mod opc;
mod props;
mod resolve;
mod styles;
mod theme;
mod xml;

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

pub use coverage::Coverage;
pub use document::{
    Block, BreakKind, Document, Paragraph, Revision, RevisionKind, Run, RunContent, Table,
    TableCell, TableRow,
};
pub use error::{Error, Result};
pub use props::{FontRef, Fonts, Indent, NumRef, ParaProps, RunProps, Spacing};
pub use resolve::Resolver;
pub use styles::{Style, StyleKind, StyleSheet};
pub use theme::{ThemeFontLang, ThemeFonts};

use opc::Package;

/// A parsed `.docx` file.
#[derive(Debug, Clone)]
pub struct Docx {
    pub document: Document,
    pub styles: StyleSheet,
    pub theme: Option<ThemeFonts>,
    pub theme_lang: ThemeFontLang,
    /// Markup that was encountered but is not modelled yet.
    pub coverage: Coverage,
}

impl Docx {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_reader(BufReader::new(File::open(path)?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_reader(Cursor::new(bytes))
    }

    pub fn from_reader<R: Read + Seek>(reader: R) -> Result<Self> {
        let mut package = Package::new(reader)?;
        let mut coverage = Coverage::default();

        // The main part is found through the package relationships; its name
        // is conventional, not fixed. Tolerate packages that omit them.
        let main = package
            .relationships(None)?
            .into_iter()
            .find(|r| !r.external && r.rel_type.ends_with("/officeDocument"))
            .map(|r| r.target)
            .or_else(|| {
                package
                    .has_part("word/document.xml")
                    .then(|| "word/document.xml".to_owned())
            })
            .ok_or(Error::NoMainDocument)?;
        let relationships = package.relationships(Some(&main))?;
        let related = |kind: &str| {
            relationships
                .iter()
                .find(|r| !r.external && r.rel_type.ends_with(kind))
                .map(|r| r.target.clone())
        };
        let xml_error = |part: &str| {
            let part = part.to_owned();
            move |source| Error::Xml { part, source }
        };

        let xml = package
            .read_xml(&main)?
            .ok_or_else(|| Error::MissingPart(main.clone()))?;
        let document = Document::parse(&xml, &mut coverage).map_err(xml_error(&main))?;

        // The remaining parts are optional: a document without them still opens.
        let mut read_related = |kind: &str| -> Result<Option<(String, String)>> {
            let Some(part) = related(kind) else {
                return Ok(None);
            };
            Ok(package.read_xml(&part)?.map(|xml| (part, xml)))
        };
        let styles = match read_related("/styles")? {
            Some((part, xml)) => {
                StyleSheet::parse(&xml, &mut coverage).map_err(xml_error(&part))?
            }
            None => StyleSheet::default(),
        };
        let theme = match read_related("/theme")? {
            Some((part, xml)) => Some(ThemeFonts::parse(&xml).map_err(xml_error(&part))?),
            None => None,
        };
        let theme_lang = match read_related("/settings")? {
            Some((part, xml)) => ThemeFontLang::parse_settings(&xml).map_err(xml_error(&part))?,
            None => ThemeFontLang::default(),
        };

        Ok(Docx {
            document,
            styles,
            theme,
            theme_lang,
            coverage,
        })
    }

    pub fn resolver(&self) -> Resolver<'_> {
        Resolver {
            styles: &self.styles,
            theme: self.theme.as_ref(),
            theme_lang: &self.theme_lang,
        }
    }
}
