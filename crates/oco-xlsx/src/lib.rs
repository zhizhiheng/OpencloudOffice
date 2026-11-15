//! Reads a spreadsheet (SpreadsheetML: `.xlsx`, `.xlsm`) and lays its sheets out as pages.
//!
//! A spreadsheet is not shown from the document form that Word documents,
//! binary Word documents and Markdown share: a sheet is a grid of cells, and is laid out as one. What it does share
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
    /// The package is not a spreadsheet, or lacks a part a spreadsheet must have.
    Malformed(String),
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(error) => error.fmt(f),
            Error::Malformed(what) => write!(f, "damaged spreadsheet: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported spreadsheet: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// A parsed spreadsheet.
#[derive(Debug, Clone, Default)]
pub struct Workbook {
    /// The picture files the spreadsheet shows, by the name of their package
    /// part: what [`oco_layout::svg::render`] looks pictures up in.
    pub media: BTreeMap<String, Media>,
    /// Problems that did not prevent opening.
    pub warnings: Vec<String>,
}

impl Workbook {
    /// Reads a spreadsheet from the bytes of its file.
    pub fn from_bytes(_bytes: &[u8]) -> Result<Self, Error> {
        Err(Error::Unsupported("spreadsheets are not read yet"))
    }

    /// What the spreadsheet has that is not shown, or only roughly: each kind
    /// by a name that is part of the interface, with how often it occurs.
    pub fn not_shown(&self) -> Vec<(&'static str, usize)> {
        Vec::new()
    }
}

/// Lays the spreadsheet out as pages.
pub fn layout(_workbook: &Workbook, _fonts: &FontDb, _options: &Options) -> Layout {
    Layout::default()
}
