//! Section properties (`w:sectPr`): page size, margins and the document grid.

use roxmltree::Node;
use serde::Serialize;

use crate::coverage::Coverage;
use crate::xml::{elements, int, twips, val, w_attr, w_name};

/// The page setup of one section. All lengths are in twips (1/20 pt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub page_width: i32,
    pub page_height: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub header: i32,
    pub footer: i32,
    pub gutter: i32,
    pub columns: i32,
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
}

/// The header or footer for each kind of page, by relationship id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
            header: 720,
            footer: 720,
            gutter: 0,
            columns: 1,
            grid: DocGrid::default(),
            start: SectionStart::NextPage,
            headers: StoryRefs::default(),
            footers: StoryRefs::default(),
            title_page: false,
            page_number_start: None,
            page_number_format: None,
        }
    }
}

/// The document grid (`w:docGrid`), which East Asian documents use to fix
/// the number of lines per page, and sometimes characters per line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DocGrid {
    pub kind: GridKind,
    /// Distance between grid lines in twips.
    pub line_pitch: i32,
    /// Adjustment to the character pitch, in 4096ths of a point.
    pub char_space: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GridKind {
    #[default]
    None,
    /// Lines snap to the line pitch; characters are placed freely.
    Lines,
    /// Lines and characters both snap.
    LinesAndChars,
    SnapToChars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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
                    // even if the header is taller"; the size is what matters here.
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
                    section.columns = int(w_attr(child, "num")).unwrap_or(1).max(1);
                    if section.columns > 1 {
                        coverage.note("w:sectPr/w:cols (more than one column)");
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
                // Editing aids with no effect on layout.
                Some("formProt" | "rsid" | "noEndnote" | "bidi" | "rtlGutter") => {}
                _ => coverage.miss(Some("w:sectPr"), child),
            }
        }
        section
    }

    /// Width available to text between the left and right margins.
    pub fn text_width(&self) -> i32 {
        self.page_width - self.margin_left - self.margin_right - self.gutter
    }

    /// Height available to text between the top and bottom margins.
    pub fn text_height(&self) -> i32 {
        self.page_height - self.margin_top - self.margin_bottom
    }
}
