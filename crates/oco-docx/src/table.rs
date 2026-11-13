//! Properties of tables, their rows and their cells.
//!
//! Only what decides where the text of a table goes is read: the widths of
//! the columns, the margins inside cells, the heights of rows and whether
//! they may split across pages, and how cells span and merge. Whether
//! borders are drawn is read, but not their styles, and that a table floats,
//! but not where; shading and conditional formatting are not.

use crate::xmltree::Node;
#[cfg(feature = "serde")]
use serde::Serialize;

use crate::coverage::Coverage;
use crate::props::Border;
use crate::values::Color;
use crate::xml::{elements, int, on_off, twips, val, w_attr, w_name};

/// The blank between a cell's edges and its content, in twips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct CellMargins {
    pub top: Option<i32>,
    pub left: Option<i32>,
    pub bottom: Option<i32>,
    pub right: Option<i32>,
}

impl CellMargins {
    fn parse(node: Node) -> Self {
        let mut margins = CellMargins::default();
        for child in elements(node) {
            // Only lengths: a margin cannot be a percentage.
            let width = twips(w_attr(child, "w")).filter(|_| w_attr(child, "type") != Some("pct"));
            match w_name(child) {
                Some("top") => margins.top = width,
                Some("left" | "start") => margins.left = width,
                Some("bottom") => margins.bottom = width,
                Some("right" | "end") => margins.right = width,
                _ => {}
            }
        }
        margins
    }

    fn apply(&mut self, over: &CellMargins) {
        self.top = over.top.or(self.top);
        self.left = over.left.or(self.left);
        self.bottom = over.bottom.or(self.bottom);
        self.right = over.right.or(self.right);
    }
}

/// `w:tblPr`, of a table or of a table style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TableProps {
    pub style_id: Option<String>,
    /// `w:tblW` when it is a length, in twips.
    pub width: Option<i32>,
    /// `w:tblW` when it is a share of the room there is, in fiftieths of a
    /// percent: 5000 is all of it.
    pub width_share: Option<i32>,
    /// `w:tblInd`: how far the table starts from the margin, in twips.
    pub indent: Option<i32>,
    /// `w:jc`: `left`, `center` or `right`.
    pub align: Option<String>,
    /// `w:tblLayout`: the column widths are as stated, not fitted to content.
    pub fixed_layout: Option<bool>,
    pub cell_margins: CellMargins,
    /// The width of the table's borders in eighths of a point: that of the
    /// widest edge of `w:tblBorders` that is drawn, and 0 when none is.
    pub borders: Option<i32>,
    /// `w:tblBorders` edge by edge: the lines around the table and between its cells.
    pub edges: Edges,
    /// `w:shd`: the fill of every cell that has none of its own.
    pub shade: Option<Color>,
    /// `w:tblpPr`: the table floats, out of the flow of the text. Where it
    /// floats is not read.
    pub floating: bool,
    /// `w:tblCellSpacing`: the cells stand apart, each within rules of its
    /// own; this much blank, in twips, is on every side of each.
    pub cell_spacing: Option<i32>,
}

/// The borders of a table (`w:tblBorders`) or of one cell (`w:tcBorders`),
/// edge by edge. An edge that is absent says nothing; one whose style is
/// `nil` or `none` says that there is no line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Edges {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    /// Between the rows inside (a table, or a cell that covers several).
    pub inside_h: Option<Border>,
    /// Between the columns inside.
    pub inside_v: Option<Border>,
}

impl Edges {
    fn parse(node: Node) -> Self {
        let mut edges = Edges::default();
        for child in elements(node) {
            let edge = Some(Border::parse_edge(child));
            match w_name(child) {
                Some("top") => edges.top = edge,
                // ISO Strict documents say where the text starts and ends rather than left and right.
                Some("left" | "start") => edges.left = edge,
                Some("bottom") => edges.bottom = edge,
                Some("right" | "end") => edges.right = edge,
                Some("insideH") => edges.inside_h = edge,
                Some("insideV") => edges.inside_v = edge,
                _ => {}
            }
        }
        edges
    }

    /// Lays `over` on top: each edge it states wins.
    pub fn apply(&mut self, over: &Edges) {
        for (mine, theirs) in [
            (&mut self.top, &over.top),
            (&mut self.left, &over.left),
            (&mut self.bottom, &over.bottom),
            (&mut self.right, &over.right),
            (&mut self.inside_h, &over.inside_h),
            (&mut self.inside_v, &over.inside_v),
        ] {
            if theirs.is_some() {
                *mine = theirs.clone();
            }
        }
    }
}

/// The width of the widest line a set of borders (`w:tblBorders`) draws,
/// in eighths of a point; 0 when it draws none.
fn widest_border(node: Node) -> i32 {
    elements(node).map(Border::parse_edge).filter(Border::is_visible).map(|edge| edge.size.max(0)).max().unwrap_or(0)
}

/// A length that is a number of twips, not a percentage or `auto`.
fn length(node: Node) -> Option<i32> {
    match w_attr(node, "type") {
        Some("pct" | "auto" | "nil") => None,
        _ => twips(w_attr(node, "w")),
    }
}

/// A width that is a share of what is available, in fiftieths of a percent.
/// It is written as that number, or as a percentage with its sign.
fn share(node: Node) -> Option<i32> {
    if w_attr(node, "type") != Some("pct") {
        return None;
    }
    let written = w_attr(node, "w")?.trim();
    let share = match written.strip_suffix('%') {
        Some(percent) => (percent.trim().parse::<f32>().ok()? * 50.0).round() as i32,
        None => int(Some(written))?,
    };
    (share > 0).then_some(share)
}

impl TableProps {
    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut props = TableProps::default();
        for child in elements(node) {
            match w_name(child) {
                Some("tblStyle") => props.style_id = val(child).map(str::to_owned),
                Some("tblW") => {
                    props.width = length(child).filter(|w| *w > 0);
                    props.width_share = share(child);
                }
                Some("tblInd") => props.indent = length(child),
                Some("jc") => props.align = val(child).map(str::to_owned),
                Some("tblLayout") => props.fixed_layout = Some(w_attr(child, "type") == Some("fixed")),
                Some("tblCellMar") => props.cell_margins = CellMargins::parse(child),
                Some("tblBorders") => {
                    props.borders = Some(widest_border(child));
                    props.edges = Edges::parse(child);
                }
                Some("shd") => props.shade = w_attr(child, "fill").and_then(Color::parse),
                Some("tblCellSpacing") => props.cell_spacing = length(child).filter(|w| *w > 0),
                Some("tblpPr") => {
                    props.floating = true;
                    // Its position is not modelled.
                    coverage.miss(Some("w:tblPr"), child);
                }
                // Which conditional formats of the style apply; none are read.
                Some("tblLook" | "tblStyleRowBandSize" | "tblStyleColBandSize") => {}
                // Accessibility text.
                Some("tblCaption" | "tblDescription") => {}
                _ => coverage.miss(Some("w:tblPr"), child),
            }
        }
        props
    }

    /// Lays `over` on top of these properties: what it states wins.
    pub fn apply(&mut self, over: &TableProps) {
        self.style_id = over.style_id.clone().or(self.style_id.take());
        if over.width.is_some() || over.width_share.is_some() {
            (self.width, self.width_share) = (over.width, over.width_share);
        }
        self.indent = over.indent.or(self.indent);
        self.align = over.align.clone().or(self.align.take());
        self.fixed_layout = over.fixed_layout.or(self.fixed_layout);
        self.cell_margins.apply(&over.cell_margins);
        self.borders = over.borders.or(self.borders);
        self.edges.apply(&over.edges);
        self.shade = over.shade.or(self.shade);
        self.floating |= over.floating;
        self.cell_spacing = over.cell_spacing.or(self.cell_spacing);
    }
}

/// `w:trPr`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct RowProps {
    /// `w:trHeight` in twips: the least the row is tall, or exactly how tall.
    pub height: Option<i32>,
    /// The height is exact (`w:hRule="exact"`); content beyond it is cut off.
    pub exact_height: bool,
    /// `w:cantSplit`: the row stays whole on one page.
    pub cant_split: bool,
    /// `w:tblHeader`: the row repeats at the top of each page of the table.
    pub header: bool,
    /// `w:gridBefore`, `w:gridAfter`: how many columns of the grid the row
    /// leaves empty before its first cell and after its last.
    pub grid_before: u32,
    pub grid_after: u32,
    /// `w:wBefore`, `w:wAfter`: how wide those columns are to be, in twips,
    /// where the row says.
    pub width_before: Option<i32>,
    pub width_after: Option<i32>,
}

impl RowProps {
    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut props = RowProps::default();
        for child in elements(node) {
            match w_name(child) {
                Some("trHeight") => {
                    props.height = twips(val(child)).filter(|h| *h > 0);
                    props.exact_height = w_attr(child, "hRule") == Some("exact");
                }
                Some("gridBefore") => props.grid_before = int(val(child)).map_or(0, |n| n.clamp(0, 63) as u32),
                Some("gridAfter") => props.grid_after = int(val(child)).map_or(0, |n| n.clamp(0, 63) as u32),
                Some("wBefore") => props.width_before = length(child).filter(|w| *w > 0),
                Some("wAfter") => props.width_after = length(child).filter(|w| *w > 0),
                Some("cantSplit") => props.cant_split = on_off(child),
                Some("tblHeader") => props.header = on_off(child),
                // Revision marks on the row itself.
                Some("ins" | "del") => {}
                _ => coverage.miss(Some("w:trPr"), child),
            }
        }
        props
    }
}

/// How a cell takes part in a vertical merge (`w:vMerge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum VerticalMerge {
    /// The first cell of the merged group, which holds its content.
    Restart,
    /// A cell below it, joined to the one above.
    Continue,
}

/// `w:tcPr`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct CellProps {
    /// `w:tcW` when it is a length, in twips.
    pub width: Option<i32>,
    /// `w:tcW` when it is a share of the table's width, in fiftieths of a percent.
    pub width_share: Option<i32>,
    /// `w:gridSpan`: how many columns of the grid the cell covers.
    pub grid_span: u32,
    pub merge: Option<VerticalMerge>,
    /// Margins of this cell alone, over the table's.
    pub margins: CellMargins,
    /// `w:vAlign`: where the content stands in a cell taller than it:
    /// `top` (as when nothing is said), `center` or `bottom`.
    pub vertical_align: Option<String>,
    /// `w:hideMark`: an empty cell does not make its row as tall as its
    /// paragraph mark.
    pub hide_mark: bool,
    /// `w:tcBorders`: the cell's own lines, where it has a say over the table's.
    pub borders: Edges,
    /// `w:shd`: the cell's fill.
    pub shade: Option<Color>,
}

impl Default for CellProps {
    fn default() -> Self {
        CellProps {
            width: None,
            width_share: None,
            grid_span: 1,
            merge: None,
            margins: CellMargins::default(),
            vertical_align: None,
            hide_mark: false,
            borders: Edges::default(),
            shade: None,
        }
    }
}

impl CellProps {
    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut props = CellProps::default();
        for child in elements(node) {
            match w_name(child) {
                Some("tcW") => {
                    props.width = length(child).filter(|w| *w > 0);
                    props.width_share = share(child);
                }
                Some("gridSpan") => props.grid_span = int(val(child)).map_or(1, |n| n.clamp(1, 63) as u32),
                Some("vMerge") => {
                    props.merge = Some(match val(child) {
                        Some("restart") => VerticalMerge::Restart,
                        _ => VerticalMerge::Continue,
                    });
                }
                Some("tcMar") => props.margins = CellMargins::parse(child),
                Some("vAlign") => props.vertical_align = val(child).map(str::to_owned),
                Some("hideMark") => props.hide_mark = on_off(child),
                Some("tcBorders") => props.borders = Edges::parse(child),
                Some("shd") => props.shade = w_attr(child, "fill").and_then(Color::parse),
                _ => coverage.miss(Some("w:tcPr"), child),
            }
        }
        props
    }
}

/// `w:tblGrid`: the widths of the table's columns, in twips.
pub(crate) fn parse_grid(node: Node) -> Vec<i32> {
    elements(node)
        .filter(|child| w_name(*child) == Some("gridCol"))
        .map(|column| twips(w_attr(column, "w")).unwrap_or(0).max(0))
        .collect()
}
