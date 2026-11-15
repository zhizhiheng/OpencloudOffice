//! Reads a presentation (PresentationML: `.pptx`, `.ppsx`, `.pptm`) and lays its slides out as pages.
//!
//! A presentation is not shown from the document form that Word documents,
//! binary Word documents and Markdown share: a slide is a canvas of shapes, each where it says, and is laid out as one. What it does share
//! with them is everything under that: the package and XML readers of
//! `oco-docx`, the fonts, the line breaking and the drawing instructions of
//! `oco-layout` ([`oco_layout::story`] sets its text), and the SVG a page is
//! drawn as.

use std::collections::BTreeMap;
use std::fmt;

use oco_docx::Media;
use oco_layout::{FontDb, Layout, Options};

#[derive(Debug)]
pub enum Error {
    /// The file is not a package that can be read.
    Package(oco_docx::Error),
    /// The package is not a presentation, or lacks a part a presentation must have.
    Malformed(String),
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(error) => error.fmt(f),
            Error::Malformed(what) => write!(f, "damaged presentation: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported presentation: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// A parsed presentation.
#[derive(Debug, Clone, Default)]
pub struct Presentation {
    /// The picture files the presentation shows, by the name of their package
    /// part: what [`oco_layout::svg::render`] looks pictures up in.
    pub media: BTreeMap<String, Media>,
    /// Problems that did not prevent opening.
    pub warnings: Vec<String>,
}

impl Presentation {
    /// Reads a presentation from the bytes of its file.
    pub fn from_bytes(_bytes: &[u8]) -> Result<Self, Error> {
        Err(Error::Unsupported("presentations are not read yet"))
    }

    /// What the presentation has that is not shown, or only roughly: each kind
    /// by a name that is part of the interface, with how often it occurs.
    pub fn not_shown(&self) -> Vec<(&'static str, usize)> {
        Vec::new()
    }
}

/// Lays the presentation out as pages.
pub fn layout(_presentation: &Presentation, _fonts: &FontDb, _options: &Options) -> Layout {
    Layout::default()
}
