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
mod drawing;
mod error;
mod inflate;
mod numbering;
pub mod opc;
mod props;
mod resolve;
mod section;
mod statistics;
mod styles;
mod table;
mod theme;
mod values;
mod xml;
pub mod xmltree;
pub mod zip;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

pub use coverage::Coverage;
pub use document::{
    Block, BreakKind, Document, Paragraph, Revision, RevisionKind, Run, RunContent, Table, TableCell, TableRow,
};
pub use drawing::{
    Alignment, Anchor, Drawing, Offset, Picture, Placement, RelativeTo, TextBox, Wrap, WrapSide, EMU_PER_POINT,
};
pub use error::{Error, OtherFormat, Result};
pub use numbering::{format_number, Counters, Label, LabelAlign, Level, Numbering, Suffix};
pub use props::{Border, FontRef, Fonts, Indent, NumRef, ParaBorders, ParaProps, RunProps, Spacing, TabKind, TabStop};
pub use resolve::Resolver;
pub use section::{DocGrid, GridKind, Section, SectionStart, StoryRefs};
pub use statistics::Statistics;
pub use styles::{Style, StyleKind, StyleSheet};
pub use table::{CellMargins, CellProps, RowProps, TableProps, VerticalMerge};
pub use theme::{Settings, ThemeFontLang, ThemeFonts};
pub use values::{Color, Emphasis, Highlight, Underline, VertAlign};

use opc::Package;

/// A file embedded in the document, such as a picture. It is kept as the
/// package holds it and unpacked when asked for.
#[derive(Debug, Clone)]
pub struct Media(zip::Packed);

impl Media {
    /// The file's bytes.
    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self.0.unpack(opc::MAX_PART_SIZE)?)
    }
}

/// Replaces the relationship ids that the pictures of a story were read
/// with by the names of the parts they stand for, and collects those parts.
fn resolve_pictures(
    blocks: &mut [Block],
    relationships: &[opc::Relationship],
    package: &mut Package<impl Read + Seek>,
    media: &mut BTreeMap<String, Media>,
) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                // Pictures inside a text box belong to the same part.
                for text_box in paragraph.runs.iter_mut().flat_map(|run| &mut run.text_boxes) {
                    resolve_pictures(&mut text_box.blocks, relationships, package, media);
                }
                for piece in paragraph.runs.iter_mut().flat_map(|run| &mut run.content) {
                    let RunContent::Drawing(drawing) = piece else { continue };
                    let Some(picture) = &mut drawing.picture else { continue };
                    let id = picture.part.take();
                    let target = relationships.iter().find(|r| Some(&r.id) == id.as_ref() && !r.external);
                    let Some(target) = target.map(|r| r.target.clone()) else { continue };
                    if !media.contains_key(&target) {
                        // A picture that cannot be read is shown as missing, not as an error.
                        let Ok(Some(packed)) = package.read_packed(&target) else { continue };
                        media.insert(target.clone(), Media(packed));
                    }
                    picture.part = Some(target);
                }
            }
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                    resolve_pictures(&mut cell.blocks, relationships, package, media);
                }
            }
        }
    }
}

/// A parsed `.docx` file.
#[derive(Debug, Clone)]
pub struct Docx {
    pub document: Document,
    pub styles: StyleSheet,
    pub theme: Option<ThemeFonts>,
    pub numbering: Numbering,
    pub settings: Settings,
    /// The content of the headers and footers, by the relationship id that
    /// a section names them with ([`Section::headers`], [`Section::footers`]).
    pub stories: BTreeMap<String, Vec<Block>>,
    /// The picture files the document shows, by the name of their package
    /// part ([`Picture::part`]).
    pub media: BTreeMap<String, Media>,
    /// Page, line and character counts recorded by the application that
    /// last saved the document.
    pub recorded: Statistics,
    /// Markup that was encountered but is not modelled yet.
    pub coverage: Coverage,
    /// Problems that did not prevent opening, such as an unreadable styles
    /// part. The document is usable but may not look as its author intended.
    pub warnings: Vec<String>,
}

impl Docx {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_reader(BufReader::new(File::open(path)?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_reader(Cursor::new(bytes))
    }

    pub fn from_reader<R: Read + Seek>(mut reader: R) -> Result<Self> {
        const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        let mut magic = [0; 8];
        let is_ole = reader.read_exact(&mut magic).is_ok() && magic == OLE_MAGIC;
        reader.seek(SeekFrom::Start(0))?;
        if is_ole {
            return Err(Error::OtherFormat(OtherFormat::OleCompoundFile));
        }

        let mut package = Package::new(reader)?;
        let mut coverage = Coverage::default();
        let mut warnings = Vec::new();

        // The main part is found through the package relationships; its name
        // is conventional, not fixed. Tolerate packages that omit them.
        let package_relationships = package.relationships(None)?;
        let main = package_relationships
            .iter()
            // Word takes the last one when a package wrongly has several.
            .rfind(|r| !r.external && r.rel_type.ends_with("/officeDocument"))
            .map(|r| r.target.clone())
            .or_else(|| package.has_part("word/document.xml").then(|| "word/document.xml".to_owned()));
        let Some(main) = main else {
            let is_odf = package.has_part("mimetype") && package.has_part("content.xml");
            return Err(if is_odf { Error::OtherFormat(OtherFormat::OpenDocument) } else { Error::NoMainDocument });
        };

        let xml = package.read_xml(&main)?.ok_or_else(|| Error::MissingPart(main.clone()))?;
        let mut document =
            Document::parse(&xml, &mut coverage).map_err(|source| Error::Xml { part: main.clone(), source })?;

        // Everything else is optional. A document whose styles or settings are
        // missing or damaged still opens, with defaults and a warning: showing
        // the text plainly beats refusing to show it.
        let relationships = package.relationships(Some(&main)).unwrap_or_else(|error| {
            warnings.push(error.to_string());
            Vec::new()
        });
        let mut media = BTreeMap::new();
        resolve_pictures(&mut document.body, &relationships, &mut package, &mut media);
        let mut read_related = |kind: &str| -> Option<String> {
            let part = &relationships.iter().find(|r| !r.external && r.rel_type.ends_with(kind))?.target;
            match package.read_xml(part) {
                Ok(xml) => xml,
                Err(error) => {
                    warnings.push(error.to_string());
                    None
                }
            }
        };
        let styles_xml = read_related("/styles");
        let theme_xml = read_related("/theme");
        let settings_xml = read_related("/settings");
        let numbering_xml = read_related("/numbering");

        let mut parsed = |kind: &str, error: crate::xmltree::Error| {
            warnings.push(format!("the {kind} part is malformed and was ignored: {error}"));
        };
        let styles = styles_xml
            .and_then(|xml| {
                // A half-read style sheet must not leave its tallies behind.
                let mut seen = Coverage::default();
                match StyleSheet::parse(&xml, &mut seen) {
                    Ok(styles) => {
                        coverage.merge(seen);
                        Some(styles)
                    }
                    Err(error) => {
                        parsed("styles", error);
                        None
                    }
                }
            })
            .unwrap_or_default();
        let theme = theme_xml.and_then(|xml| ThemeFonts::parse(&xml).map_err(|e| parsed("theme", e)).ok());
        let settings = settings_xml
            .and_then(|xml| Settings::parse(&xml).map_err(|e| parsed("settings", e)).ok())
            .unwrap_or_default();
        let numbering = numbering_xml
            .and_then(|xml| {
                let mut seen = Coverage::default();
                match Numbering::parse(&xml, &mut seen) {
                    Ok(numbering) => {
                        coverage.merge(seen);
                        Some(numbering)
                    }
                    Err(error) => {
                        parsed("numbering", error);
                        None
                    }
                }
            })
            .unwrap_or_default();

        // Headers and footers. One that cannot be read is left out, and the
        // pages that would have shown it go without.
        let mut stories = BTreeMap::new();
        for relationship in &relationships {
            let kind = relationship.rel_type.rsplit('/').next().unwrap_or_default();
            if relationship.external || !matches!(kind, "header" | "footer") {
                continue;
            }
            let read = package.read_xml(&relationship.target).map_err(|error| error.to_string()).and_then(|xml| {
                let mut seen = Coverage::default();
                let xml = xml.ok_or_else(|| format!("the {kind} part {} is missing", relationship.target))?;
                let blocks = Document::parse_story(&xml, &mut seen)
                    .map_err(|error| format!("the {kind} part is malformed and was ignored: {error}"))?;
                Ok((blocks, seen))
            });
            match read {
                Ok((mut blocks, seen)) => {
                    // A header's pictures are named in the relationships of the header's own part.
                    let own = package.relationships(Some(&relationship.target)).unwrap_or_default();
                    resolve_pictures(&mut blocks, &own, &mut package, &mut media);
                    coverage.merge(seen);
                    stories.insert(relationship.id.clone(), blocks);
                }
                Err(warning) => warnings.push(warning),
            }
        }

        // Statistics are a convenience; a missing or damaged part is no cause for a warning.
        let recorded = package_relationships
            .iter()
            .find(|r| !r.external && r.rel_type.ends_with("/extended-properties"))
            .map_or("docProps/app.xml", |r| r.target.as_str())
            .to_owned();
        let recorded =
            package.read_xml(&recorded).ok().flatten().and_then(|xml| Statistics::parse(&xml).ok()).unwrap_or_default();

        Ok(Docx { document, styles, theme, numbering, settings, stories, media, recorded, coverage, warnings })
    }

    pub fn resolver(&self) -> Resolver<'_> {
        Resolver {
            styles: &self.styles,
            theme: self.theme.as_ref(),
            theme_lang: &self.settings.theme_lang,
            numbering: &self.numbering,
        }
    }
}
