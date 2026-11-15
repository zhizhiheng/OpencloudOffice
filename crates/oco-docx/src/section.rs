//! Section properties (`w:sectPr`): page size, margins and the document grid.

use crate::xmltree::Node;
#[cfg(feature = "serde")]
use serde::Serialize;

use crate::coverage::Coverage;
use crate::xml::{elements, int, twips, val, w_attr, w_name};

/// The page setup of one section. All lengths are in twips (1/20 pt).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Section {
    pub page_width: i32,
    pub page_height: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    /// The top (bottom) margin was given as a negative number, which means
    /// that the body begins (ends) exactly there even when the header
    /// (footer) reaches further; they then overlap.
    pub margin_top_fixed: bool,
    pub margin_bottom_fixed: bool,
    pub header: i32,
    pub footer: i32,
    pub gutter: i32,
    /// How many columns the text stands in (`w:cols`).
    pub columns: i32,
    /// The blank between two columns that are equally wide.
    pub column_space: i32,
    /// The columns' own widths, each with the blank after it, where they
    /// are not equally wide (`w:col`).
    pub column_widths: Vec<(i32, i32)>,
    /// A line is drawn between the columns.
    pub column_separator: bool,
    pub grid: DocGrid,
    pub start: SectionStart,
    /// The headers and footers the section names, by relationship id; see
    /// [`crate::Docx::stories`]. One it does not name is the same as in the
    /// section before.
    pub headers: StoryRefs,
    pub footers: StoryRefs,
    /// `w:titlePg`: the first page of the section has its own header and footer.
    pub title_page: bool,
    /// `w:pgNumType`: the number the section's first page takes, when it
    /// does not simply carry on, and the format page numbers are shown in.
    pub page_number_start: Option<i32>,
    pub page_number_format: Option<String>,
    /// `w:footnotePr` and `w:endnotePr`: how notes are numbered, where the
    /// section says.
    pub footnotes: NoteProps,
    pub endnotes: NoteProps,
}

/// How footnotes or endnotes are numbered (`w:footnotePr`, `w:endnotePr`),
/// in a section or for the whole document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct NoteProps {
    /// The number format, as `w:numFmt` names it: `decimal`, `lowerRoman`, `decimalEnclosedCircleChinese` ...
    pub format: Option<String>,
    /// The number of the first note.
    pub start: Option<i32>,
    /// `continuous`, `eachSect` or `eachPage`.
    pub restart: Option<String>,
    /// For footnotes: `pageBottom` (the default) or `beneathText`.
    pub position: Option<String>,
}

impl NoteProps {
    pub(crate) fn parse(node: Node) -> Self {
        let mut props = NoteProps::default();
        for child in elements(node) {
            match w_name(child) {
                Some("numFmt") => props.format = val(child).map(str::to_owned),
                Some("numStart") => props.start = int(val(child)),
                Some("numRestart") => props.restart = val(child).map(str::to_owned),
                Some("pos") => props.position = val(child).map(str::to_owned),
                _ => {}
            }
        }
        props
    }

    /// Lays `over` on top of these properties: what it states wins.
    pub fn apply(&mut self, over: &NoteProps) {
        self.format = over.format.clone().or(self.format.take());
        self.start = over.start.or(self.start);
        self.restart = over.restart.clone().or(self.restart.take());
        self.position = over.position.clone().or(self.position.take());
    }
}

/// The header or footer for each kind of page, by relationship id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct StoryRefs {
    /// For every page that has none of its own.
    pub default: Option<String>,
    /// For the first page, when the section has a title page.
    pub first: Option<String>,
    /// For even pages, when the document tells odd and even pages apart.
    pub even: Option<String>,
}

impl Default for Section {
    /// What Word assumes when a document states nothing: US Letter with
    /// one-inch margins and no grid.
    fn default() -> Self {
        Section {
            page_width: 12240,
            page_height: 15840,
            margin_top: 1440,
            margin_right: 1440,
            margin_bottom: 1440,
            margin_left: 1440,
            margin_top_fixed: false,
            margin_bottom_fixed: false,
            header: 720,
            footer: 720,
            gutter: 0,
            columns: 1,
            column_space: 720,
            column_widths: Vec::new(),
            column_separator: false,
            grid: DocGrid::default(),
            start: SectionStart::NextPage,
            headers: StoryRefs::default(),
            footers: StoryRefs::default(),
            title_page: false,
            page_number_start: None,
            page_number_format: None,
            footnotes: NoteProps::default(),
            endnotes: NoteProps::default(),
        }
    }
}

/// The document grid (`w:docGrid`), which East Asian documents use to fix
/// the number of lines per page, and sometimes characters per line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct DocGrid {
    pub kind: GridKind,
    /// Distance between grid lines in twips.
    pub line_pitch: i32,
    /// Adjustment to the character pitch, in 4096ths of a point.
    pub char_space: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum GridKind {
    #[default]
    None,
    /// Lines snap to the line pitch; characters are placed freely.
    Lines,
    /// Lines and characters both snap.
    LinesAndChars,
    SnapToChars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SectionStart {
    NextPage,
    Continuous,
    EvenPage,
    OddPage,
    NextColumn,
}

impl Section {
    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut section = Section::default();
        for child in elements(node) {
            let length = |name: &str| twips(w_attr(child, name));
            match w_name(child) {
                Some("pgSz") => {
                    section.page_width = length("w").unwrap_or(section.page_width);
                    section.page_height = length("h").unwrap_or(section.page_height);
                }
                Some("pgMar") => {
                    // A negative top or bottom margin means "exactly this,
                    // even if the header is taller".
                    section.margin_top_fixed = length("top").is_some_and(|top| top < 0);
                    section.margin_bottom_fixed = length("bottom").is_some_and(|bottom| bottom < 0);
                    section.margin_top = length("top").map_or(section.margin_top, i32::abs);
                    section.margin_bottom = length("bottom").map_or(section.margin_bottom, i32::abs);
                    section.margin_left = length("left").unwrap_or(section.margin_left);
                    section.margin_right = length("right").unwrap_or(section.margin_right);
                    section.header = length("header").unwrap_or(section.header);
                    section.footer = length("footer").unwrap_or(section.footer);
                    section.gutter = length("gutter").unwrap_or(section.gutter);
                }
                Some("docGrid") => {
                    section.grid = DocGrid {
                        kind: match w_attr(child, "type") {
                            Some("lines") => GridKind::Lines,
                            Some("linesAndChars") => GridKind::LinesAndChars,
                            Some("snapToChars") => GridKind::SnapToChars,
                            _ => GridKind::None,
                        },
                        line_pitch: length("linePitch").unwrap_or(0),
                        char_space: int(w_attr(child, "charSpace")).unwrap_or(0),
                    };
                }
                Some("type") => {
                    section.start = match val(child) {
                        Some("continuous") => SectionStart::Continuous,
                        Some("evenPage") => SectionStart::EvenPage,
                        Some("oddPage") => SectionStart::OddPage,
                        Some("nextColumn") => SectionStart::NextColumn,
                        _ => SectionStart::NextPage,
                    };
                }
                Some("cols") => {
                    section.columns = int(w_attr(child, "num")).unwrap_or(1).clamp(1, 45);
                    section.column_space = length("space").unwrap_or(720).max(0);
                    section.column_separator = crate::xml::on_off_attr(child, "sep").unwrap_or(false);
                    // Columns of widths of their own, unless the section
                    // says its columns are equally wide all the same.
                    let own: Vec<(i32, i32)> = elements(child)
                        .filter(|column| w_name(*column) == Some("col"))
                        .filter_map(|column| {
                            let width = twips(w_attr(column, "w")).filter(|width| *width > 0)?;
                            Some((width, twips(w_attr(column, "space")).unwrap_or(0).max(0)))
                        })
                        .take(45)
                        .collect();
                    if crate::xml::on_off_attr(child, "equalWidth") != Some(true) && !own.is_empty() {
                        section.columns = own.len() as i32;
                        section.column_widths = own;
                    }
                }
                Some(kind @ ("headerReference" | "footerReference")) => {
                    let id = child
                        .attributes()
                        .find(|attribute| attribute.name() == "id" && attribute.namespace().is_some())
                        .map(|attribute| attribute.value().to_owned());
                    let refs = if kind == "headerReference" { &mut section.headers } else { &mut section.footers };
                    match w_attr(child, "type") {
                        Some("first") => refs.first = id,
                        Some("even") => refs.even = id,
                        _ => refs.default = id,
                    }
                }
                Some("titlePg") => section.title_page = crate::xml::on_off(child),
                Some("pgNumType") => {
                    section.page_number_start = int(w_attr(child, "start"));
                    section.page_number_format = w_attr(child, "fmt").map(str::to_owned);
                }
                Some("footnotePr") => section.footnotes = NoteProps::parse(child),
                Some("endnotePr") => section.endnotes = NoteProps::parse(child),
                // Editing aids with no effect on layout.
                Some("formProt" | "rsid" | "noEndnote" | "bidi" | "rtlGutter") => {}
                // Text that runs from left to right, line under line, is what there is anyway.
                Some("textDirection") if matches!(val(child), Some("lrTb" | "lrTbV")) => {}
                _ => coverage.miss(Some("w:sectPr"), child),
            }
        }
        section
    }

    /// Width available to text between the left and right margins.
    pub fn text_width(&self) -> i32 {
        self.page_width - self.margin_left - self.margin_right - self.gutter
    }

    /// The columns the text stands in: for each, how far its left edge is
    /// from the left edge of the text, and how wide it is. Columns that
    /// are equally wide share what the blanks between them leave of the
    /// text's width; a section without columns has one, as wide as the text.
    pub fn column_boxes(&self) -> Vec<(i32, i32)> {
        let whole = self.text_width();
        if !self.column_widths.is_empty() {
            let mut left = 0;
            return self
                .column_widths
                .iter()
                .map(|&(width, space)| {
                    let place = (left, width);
                    left += width + space;
                    place
                })
                .collect();
        }
        let count = self.columns.max(1);
        let width = ((whole - (count - 1) * self.column_space) / count).max(1);
        (0..count).map(|column| (column * (width + self.column_space), width)).collect()
    }

    /// Height available to text between the top and bottom margins.
    pub fn text_height(&self) -> i32 {
        self.page_height - self.margin_top - self.margin_bottom
    }
}
