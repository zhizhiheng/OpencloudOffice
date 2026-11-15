//! Lays a parsed document out into pages.
//!
//! [`layout`] takes a [`Docx`] and a set of fonts and returns pages of
//! drawing instructions: glyphs at positions, plus the rules, backgrounds and
//! marks that character effects need. [`svg::render`] turns a page into a
//! self-contained SVG. Nothing here depends on a platform's text system, so
//! the same input gives the same pages everywhere.
//!
//! This is a research prototype. It covers body text, tables, headers and
//! footers, page-number fields, pictures, DrawingML text boxes, footnotes and
//! endnotes. Columns and floating tables still need layout support.

pub mod fonts;
mod paragraph;
pub mod sfnt;
pub mod svg;
pub mod text;

use std::collections::HashMap;

use oco_docx::{
    format_number, Alignment, Block, BreakKind, Counters, Docx, GridKind, Offset, ParaBorders, ParaProps, RelativeTo,
    Resolver, RunContent, RunProps, Section, SectionStart, StoryRefs, VerticalMerge, Wrap, WrapSide,
};
#[cfg(feature = "serde")]
use serde::Serialize;

pub use fonts::{FontDb, FontId, Model};

pub type Rgb = [u8; 3];

/// One step of an outline. Coordinates are in points, as those of every item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Move(f32, f32),
    Line(f32, f32),
    /// A quadratic curve: its control point, then its end.
    Quad(f32, f32, f32, f32),
    /// A cubic curve: its two control points, then its end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

impl Segment {
    /// The points of the segment, moved by `map`.
    fn mapped(self, map: impl Fn(f32, f32) -> (f32, f32)) -> Segment {
        match self {
            Segment::Move(x, y) => {
                let (x, y) = map(x, y);
                Segment::Move(x, y)
            }
            Segment::Line(x, y) => {
                let (x, y) = map(x, y);
                Segment::Line(x, y)
            }
            Segment::Quad(x1, y1, x, y) => {
                let ((x1, y1), (x, y)) = (map(x1, y1), map(x, y));
                Segment::Quad(x1, y1, x, y)
            }
            Segment::Cubic(x1, y1, x2, y2, x, y) => {
                let ((x1, y1), (x2, y2), (x, y)) = (map(x1, y1), map(x2, y2), map(x, y));
                Segment::Cubic(x1, y1, x2, y2, x, y)
            }
            Segment::Close => Segment::Close,
        }
    }
}

/// A colour at a place along a gradient: 0 is where the gradient begins, 1 where it ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub at: f32,
    pub color: Rgb,
    /// From 0, not seen, to 1, solid.
    pub opacity: f32,
}

/// What an outline is filled with.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid {
        color: Rgb,
        opacity: f32,
    },
    /// Colours that change along the line from one point to another.
    Linear {
        stops: Vec<Stop>,
        from: (f32, f32),
        to: (f32, f32),
    },
    /// Colours that change outwards from a centre.
    Radial {
        stops: Vec<Stop>,
        centre: (f32, f32),
        radius: f32,
    },
}

impl Paint {
    pub fn solid(color: Rgb) -> Paint {
        Paint::Solid { color, opacity: 1.0 }
    }
}

/// The line an outline is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct Pen {
    pub width: f32,
    pub color: Rgb,
    pub opacity: f32,
    /// The lengths of dashes and gaps in turn; empty for a solid line.
    pub dash: Vec<f32>,
    /// Ends and corners are rounded rather than square.
    pub round: bool,
}

impl Pen {
    pub fn solid(width: f32, color: Rgb) -> Pen {
        Pen { width, color, opacity: 1.0, dash: Vec::new(), round: false }
    }
}

/// One drawing instruction. Coordinates are in points from the page's top-left corner.
#[derive(Debug, Clone)]
pub enum Item {
    Glyph {
        font: FontId,
        glyph: u16,
        x: f32,
        /// Baseline.
        y: f32,
        size: f32,
        x_scale: f32,
        color: Rgb,
        fake_bold: bool,
        fake_italic: bool,
        /// Only the outline of the glyph is drawn (`w:outline`).
        hollow: bool,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Rgb,
    },
    Rule {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: Rgb,
        /// Dash and gap lengths; zero for a solid line.
        dash: [f32; 2],
    },
    Wave {
        x1: f32,
        x2: f32,
        y: f32,
        amplitude: f32,
        width: f32,
        color: Rgb,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        color: Rgb,
        filled: bool,
    },
    /// A picture, or the box of an object that cannot be shown yet.
    Image {
        x: f32,
        /// Top.
        y: f32,
        width: f32,
        height: f32,
        /// The image among the document's media ([`oco_docx::Docx::media`]).
        part: Option<String>,
        /// The share of the image cut away at the left, top, right and bottom.
        crop: [f32; 4],
        /// Reflect the visible (already cropped) image horizontally and vertically.
        flip: [bool; 2],
        /// Degrees the picture is turned clockwise about the centre of its
        /// box, after it is reflected.
        rotation: f32,
    },
    /// What follows, as far as the [`Item::ClipEnd`] that answers it, is
    /// shown only where it falls inside this box: the content of a table
    /// row of exact height, or of a text box.
    ClipStart {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    ClipEnd,
    /// An outline, filled, drawn with a line, or both: a shape of a
    /// drawing, the grid of a sheet.
    Shape {
        path: Vec<Segment>,
        fill: Option<Paint>,
        stroke: Option<Pen>,
    },
    /// What follows, as far as the [`Item::GroupEnd`] that answers it, is
    /// drawn turned, scaled or sheared: a point (x, y) of it goes to
    /// (a·x + c·y + e, b·x + d·y + f) for the matrix `[a, b, c, d, e, f]`,
    /// and all of it is seen through as far as `opacity` says.
    GroupStart {
        matrix: [f32; 6],
        opacity: f32,
    },
    GroupEnd,
}

/// How far to either side a box that cuts off only what is above or below
/// it reaches: further than any page.
const CLIP_REACH: f32 = 20_000.0;

impl Item {
    /// Moves the item by `dx` across and `dy` down.
    pub fn moved(mut self, dx: f32, dy: f32) -> Item {
        self.translate(dx, dy);
        self
    }

    /// The start of a group that is turned `degrees` clockwise about (`cx`, `cy`).
    pub fn turned(degrees: f32, cx: f32, cy: f32) -> Item {
        // Quarter turns are exact, whatever the platform's sine makes of them.
        let quarters = degrees / 90.0;
        let (sin, cos) = if quarters == quarters.round() {
            [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][(quarters.round() as i64).rem_euclid(4) as usize]
        } else {
            degrees.to_radians().sin_cos()
        };
        let matrix = [cos, sin, -sin, cos, cx - cos * cx + sin * cy, cy - sin * cx - cos * cy];
        Item::GroupStart { matrix, opacity: 1.0 }
    }

    /// A box that shows what lies between `top` and `bottom`, however far
    /// to the left or right.
    fn clip_between(top: f32, bottom: f32) -> Item {
        Item::ClipStart { x: -CLIP_REACH, y: top, width: 2.0 * CLIP_REACH, height: (bottom - top).max(0.0) }
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Item::Glyph { x, y, .. }
            | Item::Rect { x, y, .. }
            | Item::Image { x, y, .. }
            | Item::ClipStart { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Item::ClipEnd | Item::GroupEnd => {}
            Item::Shape { path, fill, .. } => {
                path.iter_mut().for_each(|segment| *segment = segment.mapped(|x, y| (x + dx, y + dy)));
                match fill {
                    Some(Paint::Linear { from, to, .. }) => {
                        *from = (from.0 + dx, from.1 + dy);
                        *to = (to.0 + dx, to.1 + dy);
                    }
                    Some(Paint::Radial { centre, .. }) => *centre = (centre.0 + dx, centre.1 + dy),
                    _ => {}
                }
            }
            // The same turn about a point that has moved with everything else.
            Item::GroupStart { matrix: [a, b, c, d, e, f], .. } => {
                *e += dx - *a * dx - *c * dy;
                *f += dy - *b * dx - *d * dy;
            }
            Item::Rule { x1, y1, x2, y2, .. } => {
                *x1 += dx;
                *x2 += dx;
                *y1 += dy;
                *y2 += dy;
            }
            Item::Wave { x1, x2, y, .. } => {
                *x1 += dx;
                *x2 += dx;
                *y += dy;
            }
            Item::Circle { cx, cy, .. } => {
                *cx += dx;
                *cy += dy;
            }
        }
    }
}

/// Where a line of text ended up: for comparing against a reference, and
/// for the text a viewer lets one select and search.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct PlacedLine {
    pub baseline: f32,
    pub x0: f32,
    pub x1: f32,
    pub text: String,
    /// Where each character of `text` begins, one for each `char`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub xs: Vec<f32>,
    /// The size of the largest text on the line, in points.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub size: f32,
    /// The fields on the line that show a page number or a count of pages:
    /// each by what the layout knows its paragraph by and its number among
    /// that paragraph's page fields.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub fields: Vec<(usize, usize)>,
}

impl PlacedLine {
    /// The line moved by `dx` across and `dy` down.
    pub fn moved(&self, dx: f32, dy: f32) -> PlacedLine {
        PlacedLine {
            baseline: self.baseline + dy,
            x0: self.x0 + dx,
            x1: self.x1 + dx,
            text: self.text.clone(),
            xs: self.xs.iter().map(|x| x + dx).collect(),
            size: self.size,
            fields: self.fields.clone(),
        }
    }
}

/// A place in the document's content: a paragraph, by its index in
/// [`oco_docx::Document::paragraphs`], and how many pieces of its
/// [`content`] come before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Position {
    pub paragraph: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    /// Lines that have text, for comparing positions against a reference.
    pub lines: Vec<PlacedLine>,
    /// Where in the document the page begins. `None` when it begins with
    /// something other than a paragraph of body text, such as a table row.
    pub start: Option<Position>,
    /// The top and the bottom of the area the body text is set in: the page
    /// between its top and bottom margins. Headers and footers lie outside.
    pub body: (f32, f32),
    /// Lines of text that are on the page but cut off from view: what a
    /// table row of exact height has no room for. They are not text to
    /// read, select or find. A PDF that Word makes of the page still holds
    /// them, though, so a comparison with one has to know of them.
    pub cut: Vec<PlacedLine>,
}

/// The result of laying a document out.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
    /// Fonts the document asks for that are neither installed nor have a
    /// designated replacement. Text in them was set in a fallback font, so
    /// its line breaks are not to be trusted.
    pub missing_fonts: Vec<String>,
    /// How many anchored objects were left out because the place they are
    /// anchored in is not handled yet: a table cell.
    pub unplaced: usize,
    /// How many pictures on the pages are of a kind that cannot be drawn,
    /// such as a Windows metafile: each is shown as a frame with a cross.
    pub undrawn: usize,
}

/// One piece of a paragraph's content, as [`content`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// A character of text, or a symbol.
    Char(char),
    Tab,
    Break(BreakKind),
    /// A picture or another object that stands in the line.
    Object,
    /// Word's record that a page began at this point when it last laid the
    /// document out. It takes up no place of its own.
    PageBreakHint,
}

/// Walks the content of a paragraph that takes part in layout, in order.
/// Deleted and hidden text is left out, as it is from the pages.
///
/// Every piece but a [`Content::PageBreakHint`] is one unit of
/// [`Position::offset`], so a caller that counts them can say where in the
/// paragraph a line or a page of the layout begins.
pub fn content(docx: &Docx, paragraph: &oco_docx::Paragraph, mut visit: impl FnMut(Content)) {
    let resolver = docx.resolver();
    for run in &paragraph.runs {
        if run.revision.as_ref().is_some_and(|r| r.kind.removes_content()) {
            continue;
        }
        if resolver.run(paragraph.props.style_id.as_deref(), &run.props).vanish == Some(true) {
            continue;
        }
        for piece in &run.content {
            match piece {
                RunContent::Text(text) => text.chars().for_each(|ch| visit(Content::Char(ch))),
                RunContent::Symbol { character, .. } => visit(Content::Char(*character)),
                RunContent::Tab => visit(Content::Tab),
                RunContent::Break(kind) => visit(Content::Break(*kind)),
                RunContent::PageBreakHint => visit(Content::PageBreakHint),
                // An anchored object takes no place in the text.
                RunContent::Drawing(drawing) => {
                    if drawing.anchor.is_none() {
                        visit(Content::Object);
                    }
                }
                // A note's number counts as one place, as a picture does.
                RunContent::NoteReference { custom: false, .. } | RunContent::NoteMark => visit(Content::Object),
                RunContent::NoteReference { .. } | RunContent::NoteSeparator(_) => {}
                RunContent::PageNumber(shown)
                | RunContent::PageCount(shown)
                | RunContent::SectionPageCount(shown)
                | RunContent::FormattedPageNumber { shown, .. } => {
                    shown.chars().for_each(|ch| visit(Content::Char(ch)));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub model: Model,
}

impl Default for Options {
    fn default() -> Self {
        Options { model: Model::Word }
    }
}

const TWIP: f32 = 1.0 / 20.0;

/// A block ready to be placed on a page.
#[derive(Default)]
struct Unit {
    lines: Vec<paragraph::Line>,
    before: f32,
    after: f32,
    left: f32,
    keep_next: bool,
    keep_lines: bool,
    widow_control: bool,
    page_break_before: bool,
    contextual: bool,
    style: Option<String>,
    /// The section that ends after this unit.
    ends_section: Option<Section>,
    /// Index of the paragraph this is, in document order; `None` for a table row.
    paragraph: Option<usize>,
    /// The unit is a table row; `lines` is then empty.
    row: Option<Row>,
    /// The row is of a floating table. In a header or footer such a table
    /// takes no room; in the body it is laid out in line all the same.
    floating: bool,
    /// The space before and the space after are "automatic": 14 pt, less
    /// where the paragraph stands (see [`Builder::blocks`]).
    automatic: (bool, bool),
    /// The list the paragraph is an item of, by the id of its numbering.
    list: Option<i32>,
    /// The rows at the head of the unit's table that are repeated at the
    /// top of each page the table goes on to, if the unit is a later row.
    header: Option<std::rc::Rc<Vec<Row>>>,
    /// The objects anchored in the paragraph.
    anchored: Vec<paragraph::Anchored>,
    /// The paragraph's list label and whether it shares its bottom border
    /// with the paragraph below: what it takes to lay it out a second time,
    /// around an object, without counting it again as a list item.
    label: Option<paragraph::LabelSpec>,
    /// The paragraph above (below) has the same borders and is boxed with this one.
    joined: (bool, bool),
    /// The paragraph's box, where it has borders or shading.
    frame: Option<ParaBox>,
    /// The text boxes that stand in the paragraph's lines, as drawn: which
    /// box, its items from its own left and top, and its lines of text.
    boxes: Vec<Drawn>,
}

/// A text box as drawn: which it is ([`paragraph::InlineBox::source`]), its
/// items from its own left and top, and the lines of text among them.
type Drawn = ((usize, usize), Vec<Item>, TextLines);

/// Lines of text inside something drawn as one, from its left and top.
type TextLines = Vec<PlacedLine>;

/// An object on a page that text keeps clear of.
struct Float {
    /// Its box, grown by the distance text keeps from it.
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    /// Text stops above the object and goes on below it, rather than passing beside it.
    across: bool,
    side: WrapSide,
}

/// A line beside an object needs at least this much room; with less it goes below.
const LEAST_BESIDE: f32 = 12.0;

/// Where a line of `height` whose top would be at `y` can stand among the
/// objects on the page: its top, at `y` or lower, and the bounds it keeps
/// between if an object is beside it. (Measured in Word, docx4j's probes
/// `image-anchored` and `anchor-side-by-side`: a line is kept clear of an
/// object when any part of its height meets the object's, from the object's
/// top less its distance to its bottom plus its distance; under an object
/// that text does not pass, the next line begins right at that bottom.)
fn among(floats: &[Float], mut y: f32, height: f32, column: (f32, f32)) -> (f32, Option<(f32, f32)>) {
    loop {
        let meets = |float: &&Float| y < float.bottom - 0.01 && y + height > float.top + 0.01;
        if let Some(float) = floats.iter().filter(meets).find(|float| float.across) {
            y = float.bottom;
            continue;
        }
        let mut bounds = column;
        let mut beside: Option<f32> = None;
        for float in floats.iter().filter(meets) {
            if float.right <= bounds.0 || float.left >= bounds.1 {
                continue;
            }
            let (before, after) = ((bounds.0, float.left.max(bounds.0)), (float.right.min(bounds.1), bounds.1));
            bounds = match float.side {
                WrapSide::Left => before,
                WrapSide::Right => after,
                // On both sides Word sets text on both; here it goes on the wider one.
                _ if before.1 - before.0 >= after.1 - after.0 => before,
                _ => after,
            };
            beside = Some(beside.map_or(float.bottom, |lowest: f32| lowest.min(float.bottom)));
        }
        match beside {
            Some(lowest) if bounds.1 - bounds.0 < LEAST_BESIDE => y = lowest,
            Some(_) => return (y, Some(bounds)),
            None => return (y, None),
        }
    }
}

/// The left and top of an anchored object on its page. `column` and `body`
/// are the left and right, and the top and bottom, of the text area;
/// `paragraph` is the top of the paragraph the object is anchored in.
fn anchored_at(
    object: &paragraph::Anchored,
    page: (f32, f32),
    column: (f32, f32),
    body: (f32, f32),
    paragraph: f32,
) -> (f32, f32) {
    let place = |placement: oco_docx::Placement, area: (f32, f32), size: f32| match placement.offset {
        Offset::By(distance) => area.0 + distance as f32 / oco_docx::EMU_PER_POINT as f32,
        Offset::Align(Alignment::Start | Alignment::Inside) => area.0,
        Offset::Align(Alignment::Center) => (area.0 + area.1 - size) / 2.0,
        Offset::Align(Alignment::End | Alignment::Outside) => area.1 - size,
    };
    let anchor = &object.anchor;
    let across = match anchor.horizontal.relative_to {
        RelativeTo::Page => (0.0, page.0),
        RelativeTo::LeftMargin | RelativeTo::InsideMargin => (0.0, column.0),
        RelativeTo::RightMargin | RelativeTo::OutsideMargin => (column.1, page.0),
        _ => column,
    };
    let down = match anchor.vertical.relative_to {
        RelativeTo::Page => (0.0, page.1),
        RelativeTo::Margin => body,
        RelativeTo::TopMargin => (0.0, body.0),
        RelativeTo::BottomMargin => (body.1, page.1),
        _ => (paragraph, paragraph),
    };
    (place(anchor.horizontal, across, object.width), place(anchor.vertical, down, object.height))
}

/// A table row ready to be placed. Its cells are columns of lines side by
/// side, and where the row meets the bottom of a page each cell breaks
/// between two of its lines.
#[derive(Clone)]
struct Row {
    cells: Vec<RowCell>,
    /// Where each cell begins and the last one ends, from the text margin.
    edges: Vec<f32>,
    /// The least the row is tall (`w:trHeight`).
    min_height: f32,
    /// The row is exactly this tall, whatever is in it.
    exact_height: Option<f32>,
    cant_split: bool,
    /// Blank above and below the content of the cells.
    padding: (f32, f32),
    /// Width of the rules between and around the cells; 0 for none. The
    /// rule above a row takes up room of its own (measured in Word: a row
    /// of a table with half-point borders begins half a point lower than
    /// its content alone would).
    border: f32,
    /// The last row of its table, which has the table's bottom rule under
    /// it; that rule takes up room too.
    last: bool,
    /// The width of the rule under the row, which only the last row of a
    /// table has room of its own for.
    under: f32,
    /// What the row is taller by than its own cells ask, for the content
    /// of a cell merged down to it that needs more than its rows give.
    stretch: f32,
    /// Where the cells stand apart (`w:tblCellSpacing`): the blank between
    /// the rule under one row's cells and the rule over the next row's,
    /// which is also the blank between the table's rule and its first and
    /// last rows; the sides of each cell's own rules; the sides of the
    /// table's; and whether this is the table's first row.
    apart: Option<Apart>,
}

/// How a line of a table is drawn, and how much room it takes: the width
/// it is drawn with, the width it is given, its colour and its dashes.
#[derive(Clone, Copy, PartialEq)]
struct Stroke {
    /// The width of the line, or of each line of a double or triple one.
    width: f32,
    room: f32,
    color: Rgb,
    dash: [f32; 2],
    /// How many lines it is drawn with, and how far apart their middles are.
    lines: usize,
    pitch: f32,
}

impl Stroke {
    fn of(border: &oco_docx::Border) -> Self {
        let width = rule_width(border);
        let (lines, pitch) = rule_lines(border);
        Stroke {
            width,
            room: rule_room(border),
            color: rule_colour(border),
            dash: rule_dash(border, width),
            lines,
            pitch,
        }
    }

    /// How wide the stroke is from the outer edge of its first line to
    /// that of its last.
    fn span(self) -> f32 {
        self.width + (self.lines - 1) as f32 * self.pitch
    }

    /// The stroke along the line from (`x1`, `y1`) to (`x2`, `y2`), which
    /// runs across or upright: its lines side by side about that line.
    fn rules(self, x1: f32, y1: f32, x2: f32, y2: f32) -> impl Iterator<Item = Item> {
        let upright = (x2 - x1).abs() < (y2 - y1).abs();
        (0..self.lines).map(move |line| {
            let aside = (line as f32 - (self.lines - 1) as f32 / 2.0) * self.pitch;
            let (dx, dy) = if upright { (aside, 0.0) } else { (0.0, aside) };
            Item::Rule {
                x1: x1 + dx,
                y1: y1 + dy,
                x2: x2 + dx,
                y2: y2 + dy,
                width: self.width,
                color: self.color,
                dash: self.dash,
            }
        })
    }
}

/// Of the lines two cells that meet would each draw between them, the one
/// that is drawn: the wider, and of two as wide the darker. (WPS, a probe
/// made for this: a cell with a blue line of 1.5 pt around it has that
/// line on every side, in place of the table's black half-point lines, and
/// a cell that says it has no line above still has the line of the cell
/// over it.)
fn stronger(one: Option<&oco_docx::Border>, other: Option<&oco_docx::Border>) -> Option<Stroke> {
    let darkness =
        |border: &oco_docx::Border| -> u32 { rule_colour(border).iter().map(|part| 255 - u32::from(*part)).sum() };
    let rank = |border: &&oco_docx::Border| (border.size, darkness(border));
    [one, other].into_iter().flatten().filter(|border| border.is_visible()).max_by_key(rank).map(Stroke::of)
}

#[derive(Clone)]
struct Apart {
    gap: f32,
    cells: Vec<(f32, f32)>,
    sides: (f32, f32),
    first: bool,
}

#[derive(Clone)]
struct RowCell {
    lines: Vec<CellLine>,
    /// Space after the cell's last paragraph, which the row's height includes.
    tail: f32,
    /// The cell is one of several merged down a column. Its lines are the
    /// part of the merged cell's content that falls in this row; they do
    /// not make the row any taller.
    merged: bool,
    /// What a merged cell asks of its row all the same: the height of the
    /// paragraph the file has in it, though that is empty and not shown,
    /// or, in the first of the merged cells, of the first line.
    own: f32,
    /// There is no rule above the cell, or below it: it runs on from the
    /// cell above, or into the one below.
    open: (bool, bool),
    /// What the cell is filled with.
    fill: Option<Rgb>,
    /// The lines over the cell, at its left, under it and at its right.
    strokes: [Option<Stroke>; 4],
    /// Where the content stands in a cell taller than it: 0 at the top,
    /// 0.5 in the middle, 1 at the bottom.
    align: f32,
}

#[derive(Clone)]
struct CellLine {
    line: paragraph::Line,
    /// Blank above the line: the space between two paragraphs.
    gap: f32,
    /// Where the line's origin is, from the text margin.
    x: f32,
    /// A page may end before this line: it begins a paragraph, or leaves
    /// enough of its paragraph on either side (two lines, unless the
    /// paragraph does without that care).
    breakable: bool,
}

impl RowCell {
    /// Height of the cell's content from line `from` on.
    fn rest(&self, from: usize) -> f32 {
        let lines: f32 = self.lines[from.min(self.lines.len())..].iter().map(|l| l.gap + l.line.height).sum();
        lines + if from < self.lines.len() { self.tail } else { 0.0 }
    }
}

impl Row {
    fn height(&self) -> f32 {
        self.part_height(self.content(), true)
    }

    /// The height of the tallest content among the cells that are the
    /// row's own.
    fn content(&self) -> f32 {
        self.cells.iter().map(|cell| if cell.merged { cell.own } else { cell.rest(0) }).fold(0.0, f32::max)
    }

    /// How far down from the top of its room the content of `cell` begins,
    /// in a row `height` tall that is placed whole.
    /// From the top of the row to where the content of its cells begins:
    /// the rule, the top margin, and the blank above cells that stand apart.
    fn head(&self) -> f32 {
        self.gap() + self.border + self.padding.0
    }

    /// From where the content of the cells ends to the bottom of the row:
    /// the bottom margin and, in the last row, the rule under the table.
    fn foot(&self) -> f32 {
        self.padding.1 + if self.last { self.under + self.gap() } else { 0.0 }
    }

    fn gap(&self) -> f32 {
        self.apart.as_ref().map_or(0.0, |apart| apart.gap)
    }

    fn lowered(&self, cell: &RowCell, height: f32) -> f32 {
        let room = height - self.head() - self.foot();
        if cell.merged {
            0.0
        } else {
            cell.align * (room - cell.rest(0)).max(0.0)
        }
    }

    /// Height of the row, or of what is left of it, given the height of the
    /// content still to place.
    fn part_height(&self, content: f32, first_part: bool) -> f32 {
        // The least height of a row is that of its cells; the rule above
        // them comes on top of it. A row of exact height is that height,
        // rule and all. (Word, docx4j's probe `table-rowheight`: a row of at
        // least 30 pt with one line in it is 30.5 pt from rule to rule, and
        // one of exactly 30 pt is 30 pt.)
        let least = if first_part { self.min_height } else { 0.0 };
        let natural = self.border + (content + self.padding.0 + self.padding.1).max(least);
        self.gap()
            + self.exact_height.unwrap_or(natural + self.stretch)
            + if self.last { self.under + self.gap() } else { 0.0 }
    }

    /// Where what a row of exact height shows ends, the row ending at
    /// `bottom`: at the bottom of the row itself, above
    /// the room the last row keeps for the table's rule. What is in such a
    /// row beyond its height is not shown. (Word, docx4j's probe
    /// `table-rowheight`: of the three lines in a row of exactly 10 pt the
    /// first is seen as far down as the row reaches, and the others are
    /// not; all three are in the text of Word's PDF, and where it has them
    /// is where the lines would stand if nothing were cut off. WPS 12
    /// shows the same of a probe made for the display audit of 2026-11-14.)
    fn floor(&self, bottom: f32) -> Option<f32> {
        self.exact_height.map(|_| bottom - if self.last { self.under + self.gap() } else { 0.0 })
    }

    /// Puts the lines `ranges` names of each cell on `page`, the row's top
    /// at `top`, with the rules around the part of the row that ends at
    /// `bottom`. A row put down `whole` has its content where its cells'
    /// vertical alignment has it.
    fn place(&self, ranges: &[(usize, usize)], whole: bool, left: f32, top: f32, bottom: f32, page: &mut Page) {
        for mut item in self.fills(top, bottom, whole) {
            item.translate(left, 0.0);
            page.items.push(item);
        }
        let floor = self.floor(bottom);
        page.items.extend(floor.map(|floor| Item::clip_between(top, floor)));
        for (cell, (from, to)) in self.cells.iter().zip(ranges) {
            let lowered = if whole { self.lowered(cell, bottom - top) } else { 0.0 };
            let mut y = top + self.head() + lowered;
            for part in &cell.lines[*from..*to] {
                y += part.gap;
                // A line is seen if any of it is above where the row ends.
                if floor.is_none_or(|floor| y < floor) {
                    for item in &part.line.items {
                        let mut item = item.clone();
                        item.translate(left + part.x, y);
                        page.items.push(item);
                    }
                    record(&part.line, left + part.x, y, &mut page.lines);
                } else {
                    record(&part.line, left + part.x, y, &mut page.cut);
                }
                y += part.line.height;
            }
        }
        page.items.extend(floor.map(|_| Item::ClipEnd));
        for mut item in self.rules(top, bottom, whole) {
            item.translate(left, 0.0);
            page.items.push(item);
        }
    }

    /// The rules of the row, or of the part of it between `top` and
    /// `bottom`; `whole` if that is all of the row there is left.
    fn rules(&self, top: f32, bottom: f32, whole: bool) -> Vec<Item> {
        let Some(apart) = &self.apart else {
            // Each cell has the lines on its four sides; where it runs on
            // from the cell above or into the one below there is none. A
            // line across lies in the room kept for it, its top edge where
            // that room begins: over a row at the row's top, under the last
            // row in the room the table keeps under it. (WPS, a probe made
            // for this: lines of 0.5 and 1.5 pt over one row have their
            // top edges level.) A line across reaches over the lines
            // upright at its ends, so that the corners are closed.
            let mut items = Vec::new();
            for (cell, sides) in self.cells.iter().zip(self.edges.windows(2)) {
                let [over, left, under, right] = cell.strokes;
                let (x1, x2) = (sides[0], sides[1]);
                let half = |stroke: Option<Stroke>| stroke.map_or(0.0, |stroke| stroke.span() / 2.0);
                let (from, to) = (x1 - half(left), x2 + half(right));
                let (under_at, end) = self.under_line(cell, bottom, whole);
                items.extend(left.into_iter().flat_map(|stroke| stroke.rules(x1, top, x1, end)));
                items.extend(right.into_iter().flat_map(|stroke| stroke.rules(x2, top, x2, end)));
                if let Some(stroke) = over.filter(|_| !cell.open.0) {
                    let y = top + stroke.span() / 2.0;
                    items.extend(stroke.rules(from, y, to, y));
                }
                if let Some(stroke) = under.filter(|_| !cell.open.1) {
                    items.extend(stroke.rules(from, under_at, to, under_at));
                }
            }
            return items;
        };
        if self.border <= 0.0 {
            return Vec::new();
        }
        let open: Vec<(bool, bool)> = self.cells.iter().map(|cell| cell.open).collect();
        // Cells that stand apart have each their own four rules, and the
        // table has its own around them all.
        let rule =
            |x1, y1, x2, y2| Item::Rule { x1, y1, x2, y2, width: self.border, color: [0, 0, 0], dash: [0.0, 0.0] };
        let (left, right) = apart.sides;
        let mut items = vec![rule(left, top, left, bottom), rule(right, top, right, bottom)];
        if apart.first {
            items.push(rule(left, top, right, top));
        }
        let ends = whole && self.last;
        if ends {
            items.push(rule(left, bottom, right, bottom));
        }
        let (over, under) = (top + apart.gap, if ends { bottom - apart.gap - self.border } else { bottom });
        for (cell, sides) in apart.cells.iter().enumerate() {
            let (above, below) = open.get(cell).copied().unwrap_or((false, false));
            items.extend(boxes(&[sides.0, sides.1], &[(above, below || !whole)], over, under, self.border));
        }
        items
    }

    /// Where the middle of the line under a cell is, and how far down the
    /// cell's own lines and fill go: under the last row, as far as the line
    /// in the room kept for it; under any other row, to its bottom, the
    /// line lying in the top of the next row as that row's line over it;
    /// at the foot of a page, just above the page's part's bottom.
    fn under_line(&self, cell: &RowCell, bottom: f32, whole: bool) -> (f32, f32) {
        let width = cell.strokes[2].filter(|_| !cell.open.1).map_or(0.0, |stroke| stroke.span());
        match (whole, self.last) {
            (true, true) => (bottom - self.under + width / 2.0, bottom - self.under + width),
            (true, false) => (bottom + width / 2.0, bottom),
            (false, _) => (bottom - width / 2.0, bottom),
        }
    }

    /// The fills of the row's cells, or of the part of the row between
    /// `top` and `bottom`: each cell's own box, which the lines are drawn
    /// over. (WPS, a probe made for this: the fill of a cell reaches from
    /// the middle of the line at one side to the middle of the line at
    /// the other, and likewise down.)
    fn fills(&self, top: f32, bottom: f32, whole: bool) -> Vec<Item> {
        let boxes: Vec<(f32, f32)> = match &self.apart {
            Some(apart) => apart.cells.clone(),
            None => self.edges.windows(2).map(|sides| (sides[0], sides[1])).collect(),
        };
        let (top, bottom) = match &self.apart {
            Some(apart) => (top + apart.gap, if self.last { bottom - apart.gap - self.border } else { bottom }),
            None => (top, bottom),
        };
        let fill = |(cell, (x1, x2)): (&RowCell, &(f32, f32))| {
            let end = if self.apart.is_some() { bottom } else { self.under_line(cell, bottom, whole).1 };
            cell.fill.map(|color| Item::Rect { x: *x1, y: top, width: x2 - x1, height: end - top, color })
        };
        self.cells.iter().zip(&boxes).filter_map(fill).collect()
    }

    /// The row as one line that cannot break, for a table inside a cell.
    fn into_line(self) -> paragraph::Line {
        let height = self.height();
        let mut line = paragraph::Line { height, needed: height, ..paragraph::Line::default() };
        let mut texts = Vec::new();
        let lowered: Vec<f32> = self.cells.iter().map(|cell| self.lowered(cell, height)).collect();
        let (rules, head) = (self.rules(0.0, height, true), self.head());
        let floor = self.floor(height);
        line.items = self.fills(0.0, height, true);
        line.items.extend(floor.map(|floor| Item::clip_between(0.0, floor)));
        for (cell, lowered) in self.cells.into_iter().zip(lowered) {
            let mut y = head + lowered;
            for mut part in cell.lines {
                y += part.gap;
                if floor.is_some_and(|floor| y >= floor) {
                    y += part.line.height;
                    continue;
                }
                for item in &mut part.line.items {
                    item.translate(part.x, y);
                }
                if !part.line.text.trim().is_empty() {
                    if texts.is_empty() {
                        (line.baseline, line.x0) = (y + part.line.baseline, part.x + part.line.x0);
                    }
                    line.x1 = line.x1.max(part.x + part.line.x1);
                    if part.line.parts.is_empty() {
                        line.parts.push(part.line.placed().moved(part.x, y));
                    }
                    for placed in part.line.parts.drain(..) {
                        line.parts.push(placed.moved(part.x, y));
                    }
                    texts.push(part.line.text);
                }
                line.items.append(&mut part.line.items);
                y += part.line.height;
            }
        }
        line.items.extend(floor.map(|_| Item::ClipEnd));
        line.items.extend(rules);
        line.text = texts.join(" ");
        line
    }
}

/// The rules around the cells of a row, or of the part of one on a page.
/// `open` says of each cell whether it is without a rule above and below,
/// as a cell is that runs on from the one above or into the one below.
fn boxes(edges: &[f32], open: &[(bool, bool)], top: f32, bottom: f32, width: f32) -> Vec<Item> {
    let rule = |x1, y1, x2, y2| Item::Rule { x1, y1, x2, y2, width, color: [0, 0, 0], dash: [0.0, 0.0] };
    let mut items: Vec<Item> = edges.iter().map(|x| rule(*x, top, *x, bottom)).collect();
    for (cell, sides) in edges.windows(2).enumerate() {
        let (above, below) = open.get(cell).copied().unwrap_or((false, false));
        if !above {
            items.push(rule(sides[0], top, sides[1], top));
        }
        if !below {
            items.push(rule(sides[0], bottom, sides[1], bottom));
        }
    }
    items
}

struct Builder<'a> {
    docx: &'a Docx,
    fonts: &'a FontDb,
    resolver: Resolver<'a>,
    counters: Counters,
    options: Options,
    /// Paragraphs laid out so far, which numbers them as `Document::paragraphs` does.
    paragraphs: usize,
    missing_fonts: Vec<String>,
    /// The styles of the table being laid out, base first; empty outside tables.
    table_styles: Vec<&'a oco_docx::Style>,
    /// What the columns of the tables measured so far ask for, by the
    /// table's address: a table inside a cell is measured for the cell's
    /// column and again when it is laid out.
    demands: HashMap<usize, Demands>,
    /// How many tables the content being laid out is inside.
    depth: usize,
    /// Anchored objects met that were not placed: those in table cells.
    unplaced: usize,
    /// The numbering of the list items in text boxes, which Word counts
    /// apart from the body's (see [`Builder::text_box`]).
    box_counters: Counters,
    /// The numbering of the list items in the header and footer of the
    /// page being laid out.
    story_counters: Counters,
    /// The notes the document refers to, in the order of their first references.
    notes: Vec<NoteEntry>,
    /// What the mark of the note being laid out shows (`w:footnoteRef`).
    note_mark: Option<String>,
    /// The numbering of list items in footnotes and in endnotes, each kind
    /// numbered apart from the body and on from one note to the next.
    note_counters: [Counters; 2],
    /// Where the page fields of the text came to stand in the layout
    /// before this one: what each of them is to show.
    fields: Fields,
}

/// What the page fields outside headers and footers show, each named by
/// the address of its paragraph in the document and its number among that
/// paragraph's page fields.
type Fields = HashMap<(usize, usize), FieldPlace>;

/// The page a field stands on, as far as a field may ask.
#[derive(Debug, Clone, PartialEq)]
struct FieldPlace {
    page: i32,
    pages: i32,
    section_pages: i32,
    /// How the page's section writes page numbers.
    format: String,
}

/// A note the document refers to, as the layout numbers it.
#[derive(Clone)]
struct NoteEntry {
    endnote: bool,
    id: String,
    /// What the references to it and its own mark show.
    shown: String,
    /// For a footnote that is numbered afresh on each page: the format of
    /// its number, and the number of the first note of a page.
    each_page: Option<(String, i32)>,
}

/// A note laid out.
struct LaidNote {
    /// Its lines, each with how far down the note it begins.
    lines: Vec<(f32, paragraph::Line)>,
    height: f32,
    /// For each line, whether the note may be parted before it: between
    /// two of its paragraphs, and inside one where the paragraph's own
    /// rules allow (not where its lines keep together, and with two lines
    /// at least on either side where it guards against lone lines).
    breaks: Vec<bool>,
    /// The space after its last paragraph, which `height` includes.
    after: f32,
}

impl LaidNote {
    /// How tall the lines `from..to` are, with what is between them.
    fn part(&self, from: usize, to: usize) -> f32 {
        let at = |index: usize| self.lines.get(index).map_or(self.height, |(offset, _)| *offset);
        at(to) - at(from)
    }

    /// The next line after `from` that the note may be parted before: the
    /// number of its lines if there is none.
    fn next_break(&self, from: usize) -> usize {
        (from + 1..self.lines.len()).find(|&line| self.breaks[line]).unwrap_or(self.lines.len())
    }

    /// The space after the note's last paragraph, if the lines `..to` are all of it.
    fn tail(&self, to: usize) -> f32 {
        if to == self.lines.len() {
            self.after
        } else {
            0.0
        }
    }

    /// Up to which line the note goes into `room` from the line `from` on,
    /// as the last thing on its page: as far as it fits and may be parted.
    /// `forced` puts its first part in whatever the room, so that a note
    /// taller than a page still goes on.
    fn take(&self, from: usize, room: f32, forced: bool) -> usize {
        let mut to = from;
        while to < self.lines.len() {
            let next = self.next_break(to);
            if self.part(from, next) - self.tail(next) > room + 0.01 && !(forced && to == from) {
                break;
            }
            to = next;
        }
        to
    }
}

/// What the notes running on from the page before leave of a page for its
/// text, at the least. (An assumption: no document at hand shows what Word
/// does with a note longer than a page.)
const LEAST_TEXT: f32 = 24.0;

/// A part of a note: the note and the range of its lines.
type NotePart = (usize, usize, usize);

/// The footnotes at the foot of a page: the parts of notes there, and
/// whether the first of them runs on from the page before.
type PageNotes = (Vec<NotePart>, bool);

/// The notes the document refers to, numbered as Word numbers them: in the
/// order of their first references, from the number the document says
/// (1 unless it says otherwise), in the format it says (figures for
/// footnotes and small Roman numerals for endnotes unless it says
/// otherwise). A section may say otherwise than the document for the notes
/// referred to in it, and may have their numbers start again
/// (`w:numRestart` `eachSect`). A reference that brings a mark of its own
/// takes no number.
///
/// The number a section says to start from is added to the count of the
/// notes before, less one: where numbers run on from the section before,
/// a section that says 5 after four notes numbers its first 9. (Measured
/// in WPS on a probe made for this; Word is not known to differ.)
///
/// Footnotes that are numbered afresh on each page (`eachPage`) are
/// numbered on here as the others are: [`renumber`] gives them their
/// numbers once it is known which page each is on.
fn number_notes(docx: &Docx) -> Vec<NoteEntry> {
    let merged = |document: &oco_docx::NoteProps, section: &oco_docx::NoteProps| {
        let mut props = document.clone();
        props.apply(section);
        props
    };
    let paragraphs = docx.document.paragraphs();
    let mut notes: Vec<NoteEntry> = Vec::new();
    // How many notes of either kind have been numbered, since the numbers
    // last started again.
    let mut before = [0, 0];
    let mut from = 0;
    while from < paragraphs.len() {
        // A section ends with the paragraph that carries its properties.
        let ends = paragraphs[from..].iter().position(|paragraph| paragraph.section_break.is_some());
        let to = ends.map_or(paragraphs.len(), |at| from + at + 1);
        let section = paragraphs[to - 1].section_break.as_ref().unwrap_or(&docx.document.section);
        let kinds =
            [merged(&docx.settings.footnotes, &section.footnotes), merged(&docx.settings.endnotes, &section.endnotes)];
        for (kind, props) in kinds.iter().enumerate() {
            if props.restart.as_deref() == Some("eachSect") {
                before[kind] = 0;
            }
        }
        for paragraph in &paragraphs[from..to] {
            let shown =
                paragraph.runs.iter().filter(|run| !run.revision.as_ref().is_some_and(|r| r.kind.removes_content()));
            for piece in shown.flat_map(|run| &run.content) {
                let RunContent::NoteReference { endnote, id, custom } = piece else { continue };
                if notes.iter().any(|note| note.endnote == *endnote && note.id == *id) {
                    continue;
                }
                let kind = usize::from(*endnote);
                let props = &kinds[kind];
                let format = props.format.as_deref().unwrap_or(if *endnote { "lowerRoman" } else { "decimal" });
                let number = before[kind] + props.start.unwrap_or(1);
                let shown = if *custom { String::new() } else { format_number(format, number) };
                if !custom {
                    before[kind] += 1;
                }
                let each_page = !endnote && !custom && props.restart.as_deref() == Some("eachPage");
                let each_page = each_page.then(|| (format.to_owned(), props.start.unwrap_or(1)));
                notes.push(NoteEntry { endnote: *endnote, id: id.clone(), shown, each_page });
            }
        }
        from = to;
    }
    notes
}

/// Numbers the footnotes that start afresh on each page by the pages a
/// layout put them on (`began`: for each note, the page it begins on).
/// Says whether a number changed.
fn renumber(notes: &mut [NoteEntry], began: &[Option<usize>]) -> bool {
    let mut changed = false;
    let mut on_page: HashMap<usize, i32> = HashMap::new();
    for (note, page) in notes.iter_mut().zip(began) {
        let (Some((format, start)), Some(page)) = (&note.each_page, page) else { continue };
        let before = on_page.entry(*page).or_insert(0);
        let shown = format_number(format, start + *before);
        *before += 1;
        changed |= shown != note.shown;
        note.shown = shown;
    }
    changed
}

/// What the columns of an auto-fit table ask for, each across its whole
/// width, cell margins included.
#[derive(Debug, Clone, Default)]
struct Demands {
    /// The least a column can be: the widest thing in it that no line can
    /// be broken inside.
    min: Vec<f32>,
    /// What a column would like to be: the width its cells state, or else
    /// that of the longest paragraph in it set on one line.
    pref: Vec<f32>,
    /// A cell states the column's width (`w:tcW`).
    stated: Vec<bool>,
    /// The share of the table a cell states for the column, where one does.
    share: Vec<Option<f32>>,
}

fn context(docx: &Docx, section: &Section, width: f32, model: Model) -> paragraph::Context {
    let snaps = matches!(section.grid.kind, GridKind::Lines | GridKind::LinesAndChars) && section.grid.line_pitch > 0;
    paragraph::Context {
        width,
        grid_pitch: snaps.then_some(section.grid.line_pitch as f32 * TWIP),
        default_tab: docx.settings.default_tab_stop as f32 * TWIP,
        compress_punctuation: docx.settings.compress_punctuation,
        underline_trailing_spaces: docx.settings.underline_trailing_spaces,
        hanging_tab: !docx.settings.no_tab_hang_indent,
        // From Word 2013 on, a justified line may squeeze its spaces a little
        // to take in one more word.
        shrink_spaces: docx.settings.compatibility_mode >= 15,
        // Measured in WPS Office, which this follows for want of Word's own
        // output of Chinese text. LibreOffice does not do it.
        squeeze_punctuation: docx.settings.compress_punctuation && model == Model::Word,
        model,
    }
}

/// Bullets are often characters of the Symbol or Wingdings fonts, addressed
/// in the private-use area. Without those fonts, the usual ones are mapped to
/// their Unicode look-alikes.
fn portable_bullet(text: &str, props: &mut RunProps) -> String {
    let symbol_font = [&props.fonts.ascii, &props.fonts.h_ansi].iter().any(|font| {
        matches!(font, Some(oco_docx::FontRef::Name(name)) if ["symbol", "wingdings", "wingdings 2"].contains(&name.to_lowercase().as_str()))
    });
    if !symbol_font {
        return text.to_owned();
    }
    props.fonts.ascii = None;
    props.fonts.h_ansi = None;
    text.chars()
        .map(|ch| match ch {
            '\u{F0B7}' | '\u{B7}' => '•',
            '\u{F0A7}' | '\u{F06E}' | '\u{A7}' => '▪',
            '\u{F06C}' | 'l' => '●',
            '\u{F075}' | 'u' => '◆',
            '\u{F0D8}' => '➢',
            '\u{F0FC}' => '✓',
            'o' => '○',
            'n' => '■',
            other => other,
        })
        .collect()
}

/// Draws an anchored object on a page with its left and top at `x` and `y`:
/// the content of a text box, or else a picture. What lies behind the text
/// is drawn before it.
fn put_object(page: &mut Page, object: &paragraph::Anchored, x: f32, y: f32) {
    let mut items: Vec<Item> = if object.source.is_some() {
        object.content.clone()
    } else {
        vec![Item::Image {
            x: 0.0,
            y: 0.0,
            width: object.width,
            height: object.height,
            part: object.part.clone(),
            crop: object.crop,
            flip: object.flip,
            rotation: object.rotation,
        }]
    };
    items.iter_mut().for_each(|item| item.translate(x, y));
    match object.anchor.behind_text {
        true => {
            page.items.splice(0..0, items);
        }
        false => page.items.extend(items),
    }
    page.lines.extend(object.inner.iter().map(|placed| placed.moved(x, y)));
}

/// Puts the text boxes that stand in a paragraph's lines, already drawn, where the lines have them.
fn put_boxes(laid: &mut paragraph::Laid, drawn: &[Drawn]) {
    for inline in &laid.boxes {
        let Some((_, items, inner)) = drawn.iter().find(|(source, ..)| *source == inline.source) else { continue };
        let line = &mut laid.lines[inline.line];
        for item in items {
            let mut item = item.clone();
            item.translate(inline.x, inline.top);
            line.items.push(item);
        }
        line.inner.extend(inner.iter().map(|placed| placed.moved(inline.x, inline.top)));
    }
}

/// How wide a border is drawn. Word and WPS both draw one device unit of
/// 1/600 inch to each eighth of a point of the border's size: a border of
/// 1 pt is 0.96 pt wide, one of 3 pt 2.88 pt. (Measured in both: a border of
/// size 18 is 2.16 pt in Word's PDF, of size 24 2.88 pt in WPS's.)
fn rule_width(border: &oco_docx::Border) -> f32 {
    border.size as f32 * 0.12
}

/// The room a border takes in the layout: its size as written. (Measured in
/// WPS: a border of 3 pt kept 6 pt from the text moves what follows down by
/// 9 pt.) The line is drawn in the middle of that room. A double border is
/// two lines of that size a line's width apart, a triple one three, and
/// takes the room of them all. (WPS 12, the probe `paragraph-border-double`
/// of the display audit of 2026-11-14: a double border of 2 pt is two
/// lines of 2 pt with 2 pt between them, the first where a single one
/// would be. That what follows moves down by the whole is an assumption.)
fn rule_room(border: &oco_docx::Border) -> f32 {
    let (lines, _) = rule_lines(border);
    (2 * lines - 1) as f32 * border.size as f32 / 8.0
}

/// How many lines a border is drawn with, and how far apart the middles of
/// two of them are. The styles of a thin and a thick line together are
/// drawn as two lines alike.
fn rule_lines(border: &oco_docx::Border) -> (usize, f32) {
    let lines = match border.style.as_str() {
        "double" | "doubleWave" => 2,
        style if style.starts_with("thinThick") || style.starts_with("thickThin") => 2,
        "triple" => 3,
        _ => 1,
    };
    (lines, 2.0 * border.size as f32 / 8.0)
}

fn rule_colour(border: &oco_docx::Border) -> Rgb {
    match border.color {
        Some(oco_docx::Color::Rgb(r, g, b)) => [r, g, b],
        _ => [0, 0, 0],
    }
}

/// The dashes and gaps a border's style is drawn with; none for a solid line.
fn rule_dash(border: &oco_docx::Border, width: f32) -> [f32; 2] {
    match border.style.as_str() {
        "dotted" => [width, width],
        "dashed" | "dashSmallGap" | "dotDash" | "dotDotDash" => [3.0 * width.max(0.5), 2.0 * width.max(0.5)],
        _ => [0.0, 0.0],
    }
}

/// The box a paragraph stands in, as far as the page has to know of it: its
/// fill and the rules at its sides, which go on through the space between
/// the paragraph and its neighbours where no rule closes the box there.
#[derive(Debug, Clone)]
struct ParaBox {
    shade: Option<Rgb>,
    /// Where the fill begins and ends, from the text margin.
    fill: (f32, f32),
    /// The rules at the sides: the middle of each, its width, colour and dashes.
    sides: Vec<(f32, f32, Rgb, [f32; 2])>,
    /// No rule closes the box above (below): the space there is inside it.
    open: (bool, bool),
}

impl ParaBox {
    /// Draws the part of the box between `top` and `bottom` on a page, `x`
    /// being where the text margin is.
    fn draw(&self, page: &mut Page, x: f32, top: f32, bottom: f32) {
        if bottom - top < 0.01 {
            return;
        }
        if let Some(color) = self.shade {
            let (from, to) = self.fill;
            page.items.push(Item::Rect { x: x + from, y: top, width: to - from, height: bottom - top, color });
        }
        for &(at, width, color, dash) in &self.sides {
            page.items.push(Item::Rule { x1: x + at, y1: top, x2: x + at, y2: bottom, width, color, dash });
        }
    }
}

/// Puts a paragraph's borders and shading on its lines and gives the lines
/// the room the borders take. `joined` says whether the paragraph above and
/// the one below have the same borders, in which case they are boxed as one.
///
/// Measured in WPS (a probe of 23 paragraphs made for this, and where Word's
/// PDFs show the same they agree):
/// - a border stands outside the text by its space: the left one ends that
///   far before the leftmost text edge, which is the first line's where it
///   hangs; the right one begins that far after the right indent;
/// - the top border and its space take room above the first line, the
///   bottom border and its space below the last;
/// - top and bottom borders run from the outer edge of the left border to
///   that of the right, or between the text edges where there are none;
/// - the fill covers the lines and the borders' spaces, inside the borders;
/// - paragraphs with the same borders and indents are one box: no border
///   between them, and the space between them is inside the box.
fn boxed(laid: &mut paragraph::Laid, props: &ParaProps, width: f32, joined: (bool, bool)) -> Option<ParaBox> {
    let shown = |border: &Option<oco_docx::Border>| border.clone().filter(|b| b.is_visible());
    let borders = &props.borders;
    let (top, bottom) = (shown(&borders.top).filter(|_| !joined.0), shown(&borders.bottom).filter(|_| !joined.1));
    let (left, right) = (shown(&borders.left), shown(&borders.right));
    let between = shown(&borders.between).filter(|_| joined.0);
    let shade = match props.shade {
        Some(oco_docx::Color::Rgb(r, g, b)) => Some([r, g, b]),
        _ => None,
    };
    if shade.is_none() && top.is_none() && bottom.is_none() && left.is_none() && right.is_none() && between.is_none() {
        return None;
    }
    let (indent_left, indent_right, first) = laid.indents;
    let (text_left, text_right) = (indent_left + first.min(0.0), width - indent_right);
    let space = |border: &oco_docx::Border| border.space as f32;
    // Where the fill begins and ends, and the outer edges of the box.
    let fill = (text_left - left.as_ref().map_or(0.0, space), text_right + right.as_ref().map_or(0.0, space));
    let outer = (fill.0 - left.as_ref().map_or(0.0, rule_room), fill.1 + right.as_ref().map_or(0.0, rule_room));
    // Where the middle of each line of a border lies, from the middle of the border's room.
    let asides = |border: &oco_docx::Border| {
        let (lines, pitch) = rule_lines(border);
        (0..lines).map(move |line| (line as f32 - (lines - 1) as f32 / 2.0) * pitch)
    };
    let side = |border: &oco_docx::Border, inner: f32, outward: f32| {
        let width = rule_width(border);
        let middle = inner + outward * rule_room(border) / 2.0;
        let (color, dash) = (rule_colour(border), rule_dash(border, width));
        asides(border).map(move |aside| (middle + aside, width, color, dash)).collect::<Vec<_>>()
    };
    let mut sides = Vec::new();
    sides.extend(left.as_ref().into_iter().flat_map(|border| side(border, fill.0, -1.0)));
    sides.extend(right.as_ref().into_iter().flat_map(|border| side(border, fill.1, 1.0)));
    let across = |border: &oco_docx::Border, y: f32| {
        let width = rule_width(border);
        let (color, dash) = (rule_colour(border), rule_dash(border, width));
        asides(border)
            .map(move |aside| Item::Rule { x1: outer.0, y1: y + aside, x2: outer.1, y2: y + aside, width, color, dash })
            .collect::<Vec<_>>()
    };

    let count = laid.lines.len();
    for (number, line) in laid.lines.iter_mut().enumerate() {
        // The fill lies behind the line from `from` to `to` below the line's top.
        let (mut from, mut to) = (0.0, line.height);
        let mut rules = Vec::new();
        if number == 0 {
            // A rule over the paragraph, or between it and the one above that it is boxed
            // with: the line moves down by the rule and the space kept from it.
            let over =
                top.as_ref().map(|border| (0.0, border)).or(between.as_ref().map(|border| (space(border), border)));
            if let Some((above, border)) = over {
                let room = above + rule_room(border) + space(border);
                line.items.iter_mut().for_each(|item| item.translate(0.0, room));
                line.parts.iter_mut().chain(&mut line.inner).for_each(|part| part.baseline += room);
                line.baseline += room;
                line.height += room;
                line.needed += room;
                rules.extend(across(border, above + rule_room(border) / 2.0));
                // The fill begins under a rule that closes the box, and goes on under one inside it.
                from = if top.is_some() { rule_room(border) } else { 0.0 };
                to = line.height;
            }
        }
        if number + 1 == count {
            if let Some(border) = &bottom {
                let under = line.height + space(border);
                rules.extend(across(border, under + rule_room(border) / 2.0));
                to = under;
                line.height = under + rule_room(border);
                line.needed = line.height;
            }
        }
        // The sides run the whole height of the line, the borders above and below included.
        for &(at, width, color, dash) in &sides {
            rules.push(Item::Rule { x1: at, y1: 0.0, x2: at, y2: line.height, width, color, dash });
        }
        if let Some(color) = shade {
            let fill = Item::Rect { x: fill.0, y: from, width: fill.1 - fill.0, height: to - from, color };
            line.items.insert(0, fill);
        }
        line.items.extend(rules);
    }
    Some(ParaBox { shade, fill, sides, open: (top.is_none(), bottom.is_none()) })
}

impl<'a> Builder<'a> {
    fn new(docx: &'a Docx, fonts: &'a FontDb, options: &Options, notes: Vec<NoteEntry>, fields: &Fields) -> Self {
        Builder {
            docx,
            fonts,
            resolver: docx.resolver(),
            counters: Counters::default(),
            options: *options,
            paragraphs: 0,
            missing_fonts: Vec::new(),
            table_styles: Vec::new(),
            demands: HashMap::new(),
            depth: 0,
            unplaced: 0,
            box_counters: Counters::default(),
            story_counters: Counters::default(),
            notes,
            note_mark: None,
            note_counters: [Counters::default(), Counters::default()],
            fields: fields.clone(),
        }
    }
}

impl Builder<'_> {
    /// What a paragraph is laid out from: its properties, runs and label,
    /// all resolved. Counts the paragraph if it is a list item.
    fn resolved<'s>(&mut self, source: &'s oco_docx::Paragraph) -> (ParaProps, paragraph::Paragraph<'s>) {
        self.resolve(source, None)
    }

    /// As [`Builder::resolved`]; with `known`, the label is the one given
    /// and the paragraph is not counted.
    fn resolve<'s>(
        &mut self,
        source: &'s oco_docx::Paragraph,
        known: Option<Option<paragraph::LabelSpec>>,
    ) -> (ParaProps, paragraph::Paragraph<'s>) {
        static NONE: std::sync::OnceLock<ParaProps> = std::sync::OnceLock::new();
        let props: ParaProps = self.resolver.paragraph_in(&source.props, &self.table_styles);
        let style = source.props.style_id.as_deref();
        let label = known.unwrap_or_else(|| {
            self.resolver.list_level(&props).and_then(|(num_id, ilvl, level)| {
                let label = self.counters.next(&self.docx.numbering, num_id, ilvl)?;
                let mut label_props = self.resolver.label(style, &source.mark_props, level);
                let text = portable_bullet(&label.text, &mut label_props);
                Some(paragraph::LabelSpec { text, props: label_props, suffix: level.suffix, align: level.align })
            })
        });
        let runs: Vec<(RunProps, &[RunContent])> = source
            .runs
            .iter()
            // Shown as the document reads with every tracked change accepted.
            .filter(|run| !run.revision.as_ref().is_some_and(|r| r.kind.removes_content()))
            .map(|run| (self.resolver.run_in(style, &run.props, &self.table_styles), run.content.as_slice()))
            .collect();
        let mark = self.resolver.run_in(style, &source.mark_props, &self.table_styles);
        // The caller puts the properties in; they cannot be borrowed from here.
        let placeholder = NONE.get_or_init(ParaProps::default);
        // The notes the paragraph refers to, as this layout numbers them.
        let notes = runs
            .iter()
            .flat_map(|(_, content)| content.iter())
            .filter_map(|piece| match piece {
                RunContent::NoteReference { endnote, id, custom } => {
                    let index = self.notes.iter().position(|note| note.endnote == *endnote && note.id == *id)?;
                    Some((index, (!custom).then(|| self.notes[index].shown.clone())))
                }
                _ => None,
            })
            .collect();
        let note_mark = self.note_mark.clone();
        // What the paragraph's page fields show: the numbers of the pages
        // the layout before this one put them on.
        let key = source as *const oco_docx::Paragraph as usize;
        let live = runs.iter().flat_map(|(_, content)| content.iter()).filter(|piece| is_page_field(piece));
        let fields = live
            .enumerate()
            .map(|(number, piece)| {
                let place = self.fields.get(&(key, number))?;
                let numbers = PageNumbers {
                    page: place.page,
                    pages: place.pages,
                    section_pages: place.section_pages,
                    format: &place.format,
                };
                field_text(piece, &numbers)
            })
            .collect();
        (props, paragraph::Paragraph { props: placeholder, runs, mark, label, notes, note_mark, fields, key })
    }

    /// The least and the most room these blocks need across, each laid out
    /// on its own: the widest of their paragraphs either way. Nothing is
    /// counted: list items measured here are numbered again when laid out.
    fn extent(&mut self, blocks: &[Block], section: &Section) -> paragraph::Extent {
        let (paragraphs, counters) = (self.paragraphs, self.counters.clone());
        let mut extent = paragraph::Extent::default();
        for block in blocks {
            let own = match block {
                Block::Paragraph(source) => {
                    let (props, input) = self.resolved(source);
                    let input = paragraph::Paragraph { props: &props, ..input };
                    let ctx = context(self.docx, section, f32::MAX / 4.0, self.options.model);
                    paragraph::measure(&input, self.fonts, &ctx)
                }
                Block::Table(table) => self.table_extent(table, section),
            };
            extent = paragraph::Extent {
                min: extent.min.max(own.min),
                max: extent.max.max(own.max),
                guessed: extent.guessed || own.guessed,
            };
        }
        (self.paragraphs, self.counters) = (paragraphs, counters);
        extent
    }

    /// Lays out a paragraph. `around` are the paragraphs right before and
    /// after it, where there are any: paragraphs with the same borders are
    /// boxed as one.
    fn paragraph(
        &mut self,
        source: &oco_docx::Paragraph,
        around: (Option<&oco_docx::Paragraph>, Option<&oco_docx::Paragraph>),
        section: &Section,
        width: f32,
    ) -> Unit {
        let (props, input) = self.resolved(source);
        let input = paragraph::Paragraph { props: &props, ..input };
        let label = input.label.clone();
        let ctx = context(self.docx, section, width, self.options.model);
        let mut laid = paragraph::layout(&input, self.fonts, &ctx);
        for name in std::mem::take(&mut laid.missing_fonts) {
            if !self.missing_fonts.contains(&name) {
                self.missing_fonts.push(name);
            }
        }
        let index = self.paragraphs;
        self.paragraphs += 1;

        // Text boxes are stories of their own, laid out once.
        let shown: Vec<&oco_docx::Run> =
            source.runs.iter().filter(|run| !run.revision.as_ref().is_some_and(|r| r.kind.removes_content())).collect();
        let find = |(run, index): (usize, usize)| shown.get(run).and_then(|run| run.text_boxes.get(index));
        let mut boxes: Vec<Drawn> = Vec::new();
        for inline in &laid.boxes {
            if let Some(text_box) = find(inline.source) {
                let (items, inner) = self.text_box(text_box, section, inline.width, inline.height);
                boxes.push((inline.source, items, inner));
            }
        }
        put_boxes(&mut laid, &boxes);
        for object in &mut laid.anchored {
            if let Some(text_box) = object.source.and_then(find) {
                (object.content, object.inner) = self.text_box(text_box, section, object.width, object.height);
            }
        }
        // Paragraphs one after the other with the same borders and indents
        // are boxed as one.
        let bordered = props.borders != ParaBorders::default();
        let same = |other: Option<&oco_docx::Paragraph>| {
            bordered
                && other.is_some_and(|other| {
                    let other = self.resolver.paragraph_in(&other.props, &self.table_styles);
                    other.borders == props.borders
                        && (other.indent.left, other.indent.right) == (props.indent.left, props.indent.right)
                })
        };
        let joined = (same(around.0), same(around.1));
        let frame = boxed(&mut laid, &props, width, joined);
        Unit {
            lines: laid.lines,
            before: laid.space_before,
            after: laid.space_after,
            left: 0.0,
            keep_next: props.keep_next == Some(true),
            keep_lines: props.keep_lines == Some(true),
            // On unless the document turns it off, as Chinese templates do.
            widow_control: props.widow_control != Some(false),
            page_break_before: props.page_break_before == Some(true),
            contextual: props.contextual_spacing == Some(true),
            style: props.style_id.clone(),
            ends_section: source.section_break.clone(),
            paragraph: Some(index),
            row: None,
            floating: false,
            automatic: (props.spacing.before_auto == Some(true), props.spacing.after_auto == Some(true)),
            list: self.resolver.list_level(&props).map(|(list, _, _)| list),
            header: None,
            anchored: laid.anchored,
            label,
            joined,
            frame,
            boxes,
        }
    }

    /// Lays out the content of a text box of the given size and returns
    /// what to draw, from the box's left and top, with its lines of text.
    ///
    /// The text keeps the box's insets from its edges and begins at the
    /// top unless the box says otherwise. (Measured in Word, docx4j's probe
    /// `mc-textbox-branches-numbered`: in a box with the usual insets of
    /// 7.2 and 3.6 pt, text begins 7.5 pt from the left edge and its first
    /// baseline is 14.9 pt below the top.) List items in text boxes are
    /// numbered apart from the body's: a list that has reached 3 in the
    /// body begins at 1 in a box, and goes on with 4 after it (probe
    /// `numbering-stories`).
    fn text_box(
        &mut self,
        text_box: &oco_docx::TextBox,
        section: &Section,
        width: f32,
        height: f32,
    ) -> (Vec<Item>, TextLines) {
        let emu = |length: i64| length as f32 / oco_docx::EMU_PER_POINT as f32;
        let [left, top, right, bottom] = text_box.insets.map(emu);
        let rule = text_box.outline.map_or(0.0, |(_, width)| emu(width));
        let left = left + rule / 2.0;
        let mut items = Vec::new();
        if let Some(color) = text_box.fill {
            items.push(Item::Rect { x: 0.0, y: 0.0, width, height, color });
        }
        if let Some((color, _)) = text_box.outline {
            for (x1, y1, x2, y2) in [
                (0.0, 0.0, width, 0.0),
                (width, 0.0, width, height),
                (width, height, 0.0, height),
                (0.0, height, 0.0, 0.0),
            ] {
                items.push(Item::Rule { x1, y1, x2, y2, width: rule, color, dash: [0.0, 0.0] });
            }
        }

        let (paragraphs, body) = (self.paragraphs, std::mem::take(&mut self.counters));
        self.counters = std::mem::take(&mut self.box_counters);
        let measured = std::mem::take(&mut self.demands);
        let mut units = Vec::new();
        self.blocks(&text_box.blocks, section, (width - left - right - rule / 2.0).max(1.0), &mut units);
        self.box_counters = std::mem::replace(&mut self.counters, body);
        (self.paragraphs, self.demands) = (paragraphs, measured);

        let (placed, content, _) = stack(units);
        let spare = (height - top - bottom - content).max(0.0);
        let down = top
            + match text_box.anchor {
                Alignment::Center => spare / 2.0,
                Alignment::End => spare,
                _ => 0.0,
            };
        // A box that keeps its size shows only what is inside it; one that
        // is made to fit its text (`a:spAutoFit`) is as tall as Word last
        // made it, and text that needs more here is left to be seen. (WPS
        // 12, a probe made for the display audit of 2026-11-14: of four
        // lines in a box 20 pt tall the first is whole, the second cut off
        // at the box's edge, the others not seen. Word is known to cut the
        // text of its boxes off likewise; that has not been measured here.)
        let cut = !text_box.auto_fit && down + content > height + 0.01;
        if cut {
            items.push(Item::ClipStart { x: 0.0, y: 0.0, width, height });
        }
        let mut lines = Vec::new();
        for (offset, mut line) in placed {
            if cut && down + offset >= height {
                continue;
            }
            for mut item in std::mem::take(&mut line.items) {
                item.translate(left, down + offset);
                items.push(item);
            }
            record(&line, left, down + offset, &mut lines);
        }
        if cut {
            items.push(Item::ClipEnd);
        }
        (items, lines)
    }

    /// Lays out the note with the given index at the width of `section`'s
    /// text. A note the document does not have comes out empty.
    fn note(&mut self, index: usize, section: &Section) -> LaidNote {
        let units = self.note_units(index, section);
        // Where the note may be parted, should it not fit on a page: as a
        // paragraph of the text may be between two pages.
        let mut breaks = Vec::new();
        let mut tied = true;
        for unit in &units {
            let count = if unit.row.is_some() { 1 } else { unit.lines.len() };
            for line in 0..count {
                breaks.push(match line {
                    0 => !tied,
                    _ => !unit.keep_lines && !(unit.widow_control && (line == 1 || line + 1 == count)),
                });
            }
            tied = unit.keep_next;
        }
        let after = units.last().map_or(0.0, |unit| unit.after);
        let (lines, height, _) = stack(units);
        breaks.resize(lines.len(), false);
        LaidNote { lines, height, breaks, after }
    }

    /// The note with the given index as units to place: they belong to no
    /// paragraph of the body, so a page that begins in them begins nowhere
    /// in the body's content.
    fn note_units(&mut self, index: usize, section: &Section) -> Vec<Unit> {
        let entry = &self.notes[index];
        let notes = if entry.endnote { &self.docx.endnotes } else { &self.docx.footnotes };
        let Some(note) = notes.iter().find(|note| note.id == entry.id && note.kind == oco_docx::NoteKind::Normal)
        else {
            return Vec::new();
        };
        let kind = usize::from(entry.endnote);
        let mark = self.note_mark.replace(entry.shown.clone());
        let (paragraphs, body) = (self.paragraphs, std::mem::take(&mut self.counters));
        self.counters = std::mem::take(&mut self.note_counters[kind]);
        let measured = std::mem::take(&mut self.demands);
        let mut units = Vec::new();
        self.blocks(&note.blocks, section, section.text_width() as f32 * TWIP, &mut units);
        self.note_counters[kind] = std::mem::replace(&mut self.counters, body);
        (self.paragraphs, self.demands, self.note_mark) = (paragraphs, measured, mark);
        units.iter_mut().for_each(|unit| unit.paragraph = None);
        units
    }

    /// How tall the paragraph that holds the line between the text and the
    /// footnotes is: that of the document's separator note, or of an empty
    /// paragraph where it has none. (Word draws the line in the middle of
    /// that paragraph's line: docx4j's probes `footnotes` and
    /// `footnote-space-after`.)
    fn separator(&mut self, section: &Section, continuation: bool) -> f32 {
        self.separator_of(section, continuation, false)
    }

    /// As [`Builder::separator`], of footnotes or of endnotes.
    fn separator_of(&mut self, section: &Section, continuation: bool, endnotes: bool) -> f32 {
        let kind = if continuation { oco_docx::NoteKind::ContinuationSeparator } else { oco_docx::NoteKind::Separator };
        let empty = [Block::Paragraph(oco_docx::Paragraph::default())];
        let notes = if endnotes { &self.docx.endnotes } else { &self.docx.footnotes };
        let blocks = match notes.iter().find(|note| note.kind == kind) {
            Some(note) if !note.blocks.is_empty() => note.blocks.clone(),
            _ => empty.to_vec(),
        };
        stack(self.story(&blocks, section, None)).1
    }

    /// Lays a paragraph out a second time, its lines kept within `shape`
    /// (see [`paragraph::layout_within`]). Nothing is counted again.
    fn again(
        &mut self,
        source: &oco_docx::Paragraph,
        unit: &Unit,
        section: &Section,
        width: f32,
        shape: &[(f32, f32)],
    ) -> Vec<paragraph::Line> {
        let (props, input) = self.resolve(source, Some(unit.label.clone()));
        let input = paragraph::Paragraph { props: &props, ..input };
        let ctx = context(self.docx, section, width, self.options.model);
        let mut laid = paragraph::layout_within(&input, self.fonts, &ctx, shape);
        put_boxes(&mut laid, &unit.boxes);
        boxed(&mut laid, &props, width, unit.joined);
        laid.lines
    }

    /// What each column of `table` asks for, from its cells: see [`Demands`].
    ///
    /// A cell of one column speaks for that column. A cell that spans
    /// several is heard after those: where the columns together fall short
    /// of what it asks, they grow in proportion to what they ask themselves
    /// (measured in Word, docx4j's probe `table-span`), or, where the cell
    /// states its width and some of its columns have none stated, those
    /// columns take up the difference.
    fn demands(&mut self, table: &oco_docx::Table, props: &oco_docx::TableProps, section: &Section) -> Demands {
        let key = table as *const oco_docx::Table as usize;
        if let Some(known) = self.demands.get(&key) {
            return known.clone();
        }
        let styles = self.resolver.table_styles(table.props.style_id.as_deref());
        let outer = std::mem::replace(&mut self.table_styles, styles);
        let points = |twips: Option<i32>| twips.unwrap_or(0) as f32 * TWIP;
        let count = column_count(table);
        let mut min = vec![0.0f32; count];
        let mut content = vec![0.0f32; count];
        let mut stated: Vec<Option<f32>> = vec![None; count];
        let mut share: Vec<Option<f32>> = vec![None; count];
        let mut spanning = Vec::new();
        let grid = |i: usize| table.grid.get(i).map_or(0.0, |w| (*w).max(0) as f32 * TWIP);
        let border = props.borders.unwrap_or(0).max(0) as f32 / 8.0;
        let spacing = points(props.cell_spacing).max(0.0);
        let apart = (spacing > 0.0).then_some((spacing + border / 2.0, 2.0 * spacing + border / 2.0));
        self.depth += 1;
        for row in &table.rows {
            // Columns the row leaves empty at either end are as wide as it says.
            let before = (row.props.grid_before as usize).min(count);
            let after = (row.props.grid_after as usize).min(count - before);
            for (first, span, width) in
                [(0, before, row.props.width_before), (count - after, after, row.props.width_after)]
            {
                if let Some(width) = width.filter(|_| span > 0) {
                    spanning.push((first, span, 0.0, width as f32 * TWIP, true));
                }
            }
            let mut column = before;
            for cell in &row.cells {
                let span = (cell.props.grid_span as usize).clamp(1, count.saturating_sub(column).max(1));
                if column >= count {
                    break;
                }
                // Beside its margins a cell that stands apart needs the
                // blank and the rules that part it from its neighbours.
                let around = match apart {
                    Some((beside, at_edge)) => {
                        (if column == 0 { at_edge } else { beside })
                            + (if column + span >= count { at_edge } else { beside })
                            + border
                    }
                    None => 0.0,
                };
                let margins = points(cell.props.margins.left.or(props.cell_margins.left))
                    + points(cell.props.margins.right.or(props.cell_margins.right))
                    + around;
                let extent = self.extent(&cell.blocks, section);
                let (mut least, most) = (extent.min + margins, extent.max + margins);
                let mut width = cell.props.width.map(|w| w as f32 * TWIP);
                // What is in the cell cannot all be measured: there is a
                // picture or the like in it, or, in a document Word saved,
                // text in a font that is missing. The grid is what Word
                // last made of the table with those in it, and stands in.
                let from_grid: f32 = (column..column + span).map(grid).sum();
                let unknown = cell.unmeasured || extent.guessed && self.docx.recorded.from_word();
                if unknown && from_grid > 0.0 {
                    (least, width) = (least.max(from_grid), Some(from_grid));
                }
                if span == 1 {
                    min[column] = min[column].max(least);
                    content[column] = content[column].max(most);
                    if let Some(width) = width {
                        stated[column] = Some(stated[column].map_or(width, |known: f32| known.max(width)));
                    }
                    if let Some(part) = cell.props.width_share {
                        let part = part as f32 / 5000.0;
                        share[column] = Some(share[column].map_or(part, |known: f32| known.max(part)));
                    }
                } else {
                    spanning.push((column, span, least, width.unwrap_or(most).max(least), width.is_some()));
                }
                column += span;
            }
        }
        self.depth -= 1;
        self.table_styles = outer;

        let mut pref: Vec<f32> = (0..count).map(|i| stated[i].unwrap_or(content[i]).max(min[i])).collect();
        let mut is_stated: Vec<bool> = stated.iter().map(Option::is_some).collect();
        for (first, span, least, wanted, states) in spanning {
            let columns = first..first + span;
            // What the columns lack is shared out by `weights`.
            let grow = |values: &mut [f32], target: f32, weights: Vec<f32>| {
                let lacking = target - values.iter().sum::<f32>();
                let total: f32 = weights.iter().sum();
                if lacking > 0.0 {
                    for (value, weight) in values.iter_mut().zip(&weights) {
                        *value += lacking * if total > 0.0 { weight / total } else { 1.0 / span as f32 };
                    }
                }
            };
            let by_wish: Vec<f32> = pref[columns.clone()].to_vec();
            grow(&mut min[columns.clone()], least, by_wish.clone());
            let open: Vec<usize> = columns.clone().filter(|i| !is_stated[*i]).collect();
            if states && !open.is_empty() {
                // Columns whose width nothing has stated yet make up the
                // cell's stated width, in the proportions of the grid.
                let lacking = wanted - pref[columns.clone()].iter().sum::<f32>();
                let total: f32 = open.iter().map(|i| grid(*i)).sum();
                for i in &open {
                    if lacking > 0.0 {
                        pref[*i] += lacking * if total > 0.0 { grid(*i) / total } else { 1.0 / open.len() as f32 };
                    }
                    is_stated[*i] = true;
                }
            } else {
                grow(&mut pref[columns.clone()], wanted, by_wish);
            }
            for i in columns {
                pref[i] = pref[i].max(min[i]);
            }
        }
        let demands = Demands { min, pref, stated: is_stated, share };
        self.demands.insert(key, demands.clone());
        demands
    }

    /// The widths of the columns of a table of fixed layout: those of its
    /// grid. Where the grid gives none, a column is as wide as a cell in it
    /// states, and failing that the columns share `room` equally.
    fn fixed_columns(table: &oco_docx::Table, room: f32) -> Vec<f32> {
        let count = column_count(table);
        let mut widths: Vec<f32> =
            (0..count).map(|i| table.grid.get(i).map_or(0.0, |w| (*w).max(0) as f32 * TWIP)).collect();
        if widths.iter().all(|w| *w > 0.0) {
            return widths;
        }
        for row in &table.rows {
            let mut column = 0usize;
            for cell in &row.cells {
                let span = (cell.props.grid_span as usize).max(1);
                let columns = column..(column + span).min(count);
                if let Some(width) = cell.props.width.filter(|_| columns.clone().all(|i| widths[i] <= 0.0)) {
                    for i in columns.clone() {
                        widths[i] = width as f32 * TWIP / columns.len().max(1) as f32;
                    }
                }
                column += span;
            }
        }
        let known: f32 = widths.iter().sum();
        let unknown = widths.iter().filter(|w| **w <= 0.0).count();
        for width in widths.iter_mut().filter(|w| **w <= 0.0) {
            *width = if known > 0.0 { (room - known).max(0.0) / unknown as f32 } else { room / count as f32 };
        }
        widths
    }

    /// The widths of a table's columns, each from the middle of one border
    /// to the middle of the next, where there is `width` for the table.
    ///
    /// A table of fixed layout has the columns of its grid. Any other is
    /// fitted to its content, as measured in Word (docx4j's probes
    /// `table-width`, `table-autofit`, `table-span`, `table-nested`,
    /// `table-grid-vs-tcw-*` and `table-grid-overwide-auto`; the grid such a
    /// table carries is what Word last made of it, and is not consulted):
    ///
    /// - Each column asks for the width a cell states for it, or else for
    ///   that of its longest paragraph on one line; and it cannot be less
    ///   than its longest word. Cell margins are part of both.
    /// - The table is as wide as it states, even where that is wider than
    ///   the page. One that states nothing is as wide as its columns ask,
    ///   up to the room there is: the text's width less the border, or,
    ///   before Word 2013, plus the cell margins that hang out either side.
    /// - Where the columns ask for more than the table has, those without a
    ///   stated width give way first: each gives up the same fraction of
    ///   the difference between what it asks and its least. Columns with a
    ///   stated width give way, in the same manner, only once the others
    ///   are at their least.
    ///
    /// What happens to room left over in a table that states its width is
    /// not measured: here it goes to the columns without a stated width,
    /// or to all of them, in proportion to what they ask.
    fn columns(
        &mut self,
        table: &oco_docx::Table,
        props: &oco_docx::TableProps,
        section: &Section,
        width: f32,
    ) -> Vec<f32> {
        if props.fixed_layout == Some(true) {
            return Self::fixed_columns(table, width);
        }
        let points = |twips: Option<i32>| twips.unwrap_or(0) as f32 * TWIP;
        let border = props.borders.unwrap_or(0).max(0) as f32 / 8.0;
        // What a share of the room is a share of. Before Word 2013 a table
        // in the body hung out into the page margins by its cell margins.
        let room = if self.hangs() {
            width + points(props.cell_margins.left) + points(props.cell_margins.right)
        } else {
            width - border
        };
        // A table that states no width may reach from where its indent
        // puts it to the far margin.
        let available = room - points(props.indent);
        let whole = props.width.map(|w| w as f32 * TWIP).or(props.width_share.map(|s| s as f32 / 5000.0 * room));
        let Demands { min, mut pref, mut stated, share } = self.demands(table, props, section);
        // A share of the table is a stated width once the table's is known.
        // Shares that come to more than the whole are cut from the last.
        let mut left = 1.0f32;
        for (i, part) in share.iter().enumerate() {
            if let Some(part) = part.filter(|_| !stated[i]) {
                let part = part.min(left);
                left -= part;
                pref[i] = (part * whole.unwrap_or(available)).max(min[i]);
                stated[i] = true;
            }
        }
        let asked: f32 = pref.iter().sum();
        let least: f32 = min.iter().sum();
        let target = whole.unwrap_or(asked.min(available));
        let mut widths = pref.clone();
        if least > target {
            // Not even the least fits. The table grows as far as the room
            // goes; beyond that every column is cut down alike, and words
            // are broken to fit.
            let room = least.min(target.max(available));
            widths = min.iter().map(|w| w * room / least).collect();
        } else if asked < target {
            let open: Vec<usize> = (0..widths.len()).filter(|i| !stated[*i]).collect();
            let chosen: Vec<usize> = if open.is_empty() { (0..widths.len()).collect() } else { open };
            let total: f32 = chosen.iter().map(|i| pref[*i]).sum();
            for i in &chosen {
                widths[*i] += (target - asked) * if total > 0.0 { pref[*i] / total } else { 1.0 / chosen.len() as f32 };
            }
        } else if asked > target {
            let mut excess = asked - target;
            for yielding in [false, true] {
                let group: Vec<usize> = (0..widths.len()).filter(|i| stated[*i] == yielding).collect();
                let give: f32 = group.iter().map(|i| pref[*i] - min[*i]).sum();
                if give <= 0.0 {
                    continue;
                }
                let taken = excess.min(give);
                for i in group {
                    widths[i] -= (pref[i] - min[i]) * taken / give;
                }
                excess -= taken;
            }
        }
        widths
    }

    /// The table being laid out hangs out into the page margins by its cell
    /// margins, so that its text lines up with the text around it: the way
    /// of Word before 2013, for tables that are not inside another.
    fn hangs(&self) -> bool {
        self.docx.settings.compatibility_mode < 15 && self.depth == 0
    }

    /// The least and the most room a table needs across, as content of a
    /// cell: its columns' demands, or its width if that is fixed, with the
    /// halves of its border that stand outside its first and last column.
    fn table_extent(&mut self, table: &oco_docx::Table, section: &Section) -> paragraph::Extent {
        let props = self.resolver.table(&table.props);
        let around = props.borders.unwrap_or(0).max(0) as f32 / 8.0 + props.indent.unwrap_or(0).max(0) as f32 * TWIP;
        if props.fixed_layout == Some(true) {
            let whole: f32 = Self::fixed_columns(table, 0.0).iter().sum();
            return paragraph::Extent { min: whole + around, max: whole + around, guessed: false };
        }
        let demands = self.demands(table, &props, section);
        let (least, asked): (f32, f32) = (demands.min.iter().sum(), demands.pref.iter().sum());
        match props.width {
            // A table that states its width keeps it: the column it stands
            // in cannot be narrower (measured against the grids Word left
            // in documents it saved, as with all of this).
            Some(stated) => {
                let whole = (stated as f32 * TWIP).max(least) + around;
                paragraph::Extent { min: whole, max: whole, guessed: false }
            }
            None => paragraph::Extent { min: least + around, max: asked + around, guessed: false },
        }
    }

    /// A table: one unit for each row, with the columns [`Builder::columns`] gives.
    fn table(&mut self, table: &oco_docx::Table, section: &Section, width: f32, units: &mut Vec<Unit>) {
        let props = self.resolver.table(&table.props);
        let widths = self.columns(table, &props, section, width);
        let styles = self.resolver.table_styles(table.props.style_id.as_deref());
        let outer = std::mem::replace(&mut self.table_styles, styles);
        let points = |twips: Option<i32>| twips.unwrap_or(0) as f32 * TWIP;
        let margins = props.cell_margins;
        let border = props.borders.unwrap_or(0).max(0) as f32 / 8.0;

        let table_width: f32 = widths.iter().sum();
        let mut origin = points(props.indent)
            + match props.align.as_deref() {
                Some("center") => (width - table_width) / 2.0,
                Some("right" | "end") => width - table_width,
                _ => 0.0,
            };
        // Before Word 2013 a table was moved out so that its text, not its
        // edge, stood at the margin.
        if self.hangs() {
            origin -= points(margins.left);
        }
        // Where each cell of each row begins in the grid, and whether it
        // runs on from the cell above it (`w:vMerge`).
        let mut starts: Vec<Vec<usize>> = Vec::with_capacity(table.rows.len());
        let mut continues: Vec<Vec<bool>> = Vec::with_capacity(table.rows.len());
        for (number, row) in table.rows.iter().enumerate() {
            let mut column = row.props.grid_before as usize;
            let (mut own_starts, mut own_continues) = (Vec::new(), Vec::new());
            for cell in &row.cells {
                let above = number.checked_sub(1).and_then(|over| {
                    let at = starts[over].iter().position(|start| *start == column)?;
                    table.rows[over].cells[at].props.merge
                });
                own_continues.push(cell.props.merge == Some(VerticalMerge::Continue) && above.is_some());
                own_starts.push(column);
                column += (cell.props.grid_span as usize).max(1);
            }
            starts.push(own_starts);
            continues.push(own_continues);
        }

        // Cells that stand apart (`w:tblCellSpacing`). As measured in Word
        // (docx4j's probes `table-cellspacing` and `table-hidemark`): the
        // columns keep their widths, and each cell stands inside its
        // column, its own rules twice the spacing and a rule's width from
        // the rules of the cell beside it, and twice the spacing and half a
        // rule's width from the table's own. Down the page the blank is
        // twice the spacing and a rule's width everywhere. The text keeps
        // clear of the whole of its cell's rule.
        // The line a cell asks for on one of its sides (0 above, 1 at the
        // left, 2 below, 3 at the right): its own if it says, else the
        // table's, which at the table's edge is the outer one and elsewhere
        // the one between rows or columns.
        let color_resolver = self.docx.resolver();
        let cell_edges: Vec<Vec<_>> = table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        let mut edges = cell.props.borders.clone();
                        color_resolver.border_colors(&mut edges);
                        edges
                    })
                    .collect()
            })
            .collect();
        let wish = |row: usize, index: usize, side: usize| -> Option<&oco_docx::Border> {
            let (cells, edges) = (&table.rows[row].cells, &props.edges);
            let own = &cell_edges[row][index];
            let (own, of_table) = match side {
                0 => (&own.top, if row == 0 { &edges.top } else { &edges.inside_h }),
                1 => (&own.left, if index == 0 { &edges.left } else { &edges.inside_v }),
                2 => (&own.bottom, if row + 1 == table.rows.len() { &edges.bottom } else { &edges.inside_h }),
                _ => (&own.right, if index + 1 == cells.len() { &edges.right } else { &edges.inside_v }),
            };
            own.as_ref().or(of_table.as_ref())
        };
        // The cell of a row that covers a column of the grid.
        let over = |row: usize, column: usize| -> Option<usize> {
            let at = starts.get(row)?.iter().rposition(|start| *start <= column)?;
            let span = (table.rows[row].cells[at].props.grid_span as usize).max(1);
            (column < starts[row][at] + span).then_some(at)
        };
        let strokes = |row: usize, index: usize| -> [Option<Stroke>; 4] {
            let column = starts[row][index];
            let above = row.checked_sub(1).and_then(|row| wish(row, over(row, column)?, 2));
            let below = over(row + 1, column).and_then(|at| wish(row + 1, at, 0));
            let before = index.checked_sub(1).and_then(|index| wish(row, index, 3));
            let after = (index + 1 < table.rows[row].cells.len()).then(|| wish(row, index + 1, 1)).flatten();
            [
                stronger(wish(row, index, 0), above),
                stronger(wish(row, index, 1), before),
                stronger(wish(row, index, 2), below),
                stronger(wish(row, index, 3), after),
            ]
        };
        let fill_of = |color: Option<oco_docx::Color>| match color.map(|color| color_resolver.color(color)) {
            Some(oco_docx::Color::Rgb(r, g, b)) => Some([r, g, b]),
            _ => None,
        };

        let spacing = points(props.cell_spacing).max(0.0);
        let stands_apart = spacing > 0.0;
        let (beside, at_edge) = (spacing + border / 2.0, 2.0 * spacing + border / 2.0);

        let mut rows: Vec<(Row, bool)> = Vec::with_capacity(table.rows.len());
        for (number, row) in table.rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(row.cells.len());
            let skipped = row.props.grid_before as usize;
            let start = origin + widths.iter().take(skipped).sum::<f32>();
            let mut edges = vec![start];
            let mut boxes_of_cells = Vec::with_capacity(row.cells.len());
            let (mut x, mut column) = (start, skipped);
            for (index, cell) in row.cells.iter().enumerate() {
                let span = (cell.props.grid_span as usize).max(1);
                let column_width: f32 = widths.iter().skip(column).take(span).sum();
                let (left, right) = (
                    points(cell.props.margins.left.or(margins.left)),
                    points(cell.props.margins.right.or(margins.right)),
                );
                // Where the cell itself begins and ends within its columns.
                let (before, after) = match stands_apart {
                    true => (
                        if column == 0 { at_edge } else { beside },
                        if column + span >= widths.len() { at_edge } else { beside },
                    ),
                    false => (0.0, 0.0),
                };
                let (cell_x, cell_width) = (x + before, column_width - before - after);
                boxes_of_cells.push((cell_x, cell_x + cell_width));
                // The cell's margin is measured from the middle of its border,
                // and the text never stands on the border itself.
                let inset = if stands_apart { border } else { (border / 2.0).max(border - left) };
                // A cell has to end with a paragraph. Where that paragraph
                // is empty and comes right after a table, it is there for
                // the file's sake only and takes no room. (Word, docx4j's
                // probe `table-nested-trailing`: its mark is set on the last
                // row of the table inside.)
                let empty = |paragraph: &oco_docx::Paragraph| paragraph.runs.iter().all(|run| run.content.is_empty());
                let blocks = match cell.blocks.as_slice() {
                    // A cell may say that its empty paragraph is not to
                    // make the row any taller (`w:hideMark`; Word, docx4j's
                    // probe `table-hidemark`: rows of such cells are as
                    // tall as their margins and rule).
                    all if cell.props.hide_mark
                        && all.iter().all(|block| matches!(block, Block::Paragraph(only) if empty(only))) =>
                    {
                        &cell.blocks[..0]
                    }
                    [before @ .., Block::Table(_), Block::Paragraph(last)] if empty(last) => {
                        &cell.blocks[..before.len() + 1]
                    }
                    all => all,
                };
                let mut inner = Vec::new();
                self.depth += 1;
                self.blocks(blocks, section, (cell_width - left - right).max(1.0), &mut inner);
                self.unplaced += inner.iter().map(|unit| unit.anchored.len()).sum::<usize>();
                self.depth -= 1;
                // Automatic spacing gives way to the edges of the cell:
                // above the first paragraph always, below the last unless
                // it is a list item. (Word, docx4j's probes
                // `spacing-autospacing-context` and
                // `spacing-autospacing-cell-list`.)
                if let Some(first) = inner.first_mut().filter(|unit| unit.automatic.0) {
                    first.before = 0.0;
                }
                if let Some(last) = inner.last_mut().filter(|unit| unit.automatic.1 && unit.list.is_none()) {
                    last.after = 0.0;
                }
                // What is left out are paragraphs of the document all the same.
                self.paragraphs += paragraphs_in(&cell.blocks[blocks.len()..]);

                let mut lines = Vec::new();
                let (mut previous_after, mut first) = (0.0f32, true);
                for unit in inner {
                    let mut gap = if first { unit.before } else { unit.before.max(previous_after) };
                    first = false;
                    previous_after = unit.after;
                    let (keep_lines, widow_control) = (unit.keep_lines, unit.widow_control);
                    let own = match unit.row {
                        Some(nested) => vec![nested.into_line()],
                        None => unit.lines,
                    };
                    let count = own.len();
                    for (number, line) in own.into_iter().enumerate() {
                        // The lines of a paragraph in a cell are kept from
                        // standing alone like any others. (Word, docx4j's
                        // probe `table-rowsplit`: a row whose cell has two
                        // lines goes to the next page whole where only one
                        // of them fits.)
                        let breakable =
                            number == 0 || !keep_lines && (!widow_control || number >= 2 && count - number >= 2);
                        lines.push(CellLine { line, gap, x: cell_x + left + inset, breakable });
                        gap = 0.0;
                    }
                }
                let align = match cell.props.vertical_align.as_deref() {
                    Some("center") => 0.5,
                    Some("bottom") => 1.0,
                    _ => 0.0,
                };
                let mut cell = RowCell {
                    lines,
                    tail: previous_after,
                    merged: false,
                    own: 0.0,
                    open: (false, false),
                    fill: fill_of(cell.props.shade).or(fill_of(props.shade)),
                    strokes: strokes(number, index),
                    align,
                };
                if continues[number][index] {
                    // A cell that runs on from the one above shows nothing
                    // of its own, but the paragraph the file has in it
                    // still asks for its height. (Word, docx4j's probe
                    // `tables-banding-merged-row`: a row made up of such
                    // cells only is as tall as any other.)
                    (cell.own, cell.tail) = (cell.rest(0), 0.0);
                    cell.lines.clear();
                }
                cells.push(cell);
                x += column_width;
                column += span;
                edges.push(x);
            }
            let height = points(row.props.height);
            // A row keeps with what follows it if the first paragraph of its
            // first cell does. (Word, docx4j's probe `table-row-keepnext`: it
            // is of no account whether any other paragraph of the row does.)
            let keeps = match row.cells.first().and_then(|cell| cell.blocks.first()) {
                Some(Block::Paragraph(first)) => {
                    self.resolver.paragraph_in(&first.props, &self.table_styles).keep_next == Some(true)
                }
                _ => false,
            };
            // The rule over a row is as wide as the widest line over one of its
            // cells, and takes no room where there is no line. (WPS, the same
            // probe: rows under a dotted half-point line are 14.3 pt apart, rows
            // without lines 13.8 pt, and a row under a cell's line of 1.5 pt 15.3 pt.)
            let widest = |side: usize| match stands_apart {
                true => border,
                false => {
                    cells.iter().filter_map(|cell| cell.strokes[side]).map(|stroke| stroke.room).fold(0.0, f32::max)
                }
            };
            let (over_row, under_row) = (widest(0), widest(2));
            rows.push((
                Row {
                    cells,
                    edges,
                    min_height: height,
                    exact_height: row.props.exact_height.then_some(height).filter(|h| *h > 0.0),
                    cant_split: row.props.cant_split,
                    padding: (points(margins.top), points(margins.bottom)),
                    border: over_row,
                    under: under_row,
                    last: number + 1 == table.rows.len(),
                    stretch: 0.0,
                    apart: stands_apart.then_some(Apart {
                        gap: 2.0 * spacing + border,
                        cells: boxes_of_cells,
                        sides: (origin, origin + table_width),
                        first: number == 0,
                    }),
                },
                keeps,
            ));
        }

        // A cell merged down a column: its content, which the file keeps in
        // the first of the merged cells, runs through the rows the cell
        // covers. Each line goes to the row it begins in; the rows are as
        // tall as their own cells make them, and only where the content
        // needs more than all of them give is the last made taller.
        for first in 0..rows.len() {
            for index in 0..rows[first].0.cells.len() {
                if table.rows[first].cells[index].props.merge.is_none() || continues[first][index] {
                    continue;
                }
                let column = starts[first][index];
                let mut members = vec![index];
                while let Some(at) = starts.get(first + members.len()).and_then(|below| {
                    let at = below.iter().position(|start| *start == column)?;
                    continues[first + members.len()][at].then_some(at)
                }) {
                    members.push(at);
                }
                if members.len() < 2 {
                    continue;
                }
                let last = first + members.len() - 1;
                let source = &mut rows[first].0.cells[index];
                let content = std::mem::take(&mut source.lines);
                let (tail, align) = (std::mem::take(&mut source.tail), std::mem::take(&mut source.align));
                // The first of the merged cells asks its row for one line.
                source.own = content.first().map_or(0.0, |part| part.gap + part.line.height)
                    + if content.len() == 1 { tail } else { 0.0 };
                for (offset, at) in members.iter().enumerate() {
                    let cell = &mut rows[first + offset].0.cells[*at];
                    cell.merged = true;
                    cell.open = (offset > 0, first + offset < last);
                }
                let needed: f32 = content.iter().map(|part| part.gap + part.line.height).sum::<f32>() + tail;
                let heights: Vec<f32> = rows[first..=last].iter().map(|(row, _)| row.height()).collect();
                let (above, below) = (rows[first].0.head(), rows[last].0.foot());
                let mut room = heights.iter().sum::<f32>() - above - below;
                if needed > room && rows[last].0.exact_height.is_none() {
                    rows[last].0.stretch += needed - room;
                    room = needed;
                }
                // Where the room for content begins in each of the rows:
                // under its rule and its top margin.
                let mut tops = Vec::with_capacity(heights.len());
                let mut top = 0.0;
                for (offset, height) in heights.iter().enumerate() {
                    let row = &rows[first + offset].0;
                    tops.push(top + row.head());
                    top += height;
                }
                let mut y = above + align * (room - needed).max(0.0);
                for mut part in content {
                    y += part.gap;
                    let offset = tops.iter().rposition(|top| *top <= y + 0.01).unwrap_or(0);
                    let row = &mut rows[first + offset].0;
                    let wanted = y - tops[offset];
                    let target = &mut row.cells[members[offset]];
                    let used: f32 = target.lines.iter().map(|placed| placed.gap + placed.line.height).sum();
                    part.gap = (wanted - used).max(0.0);
                    y += part.line.height;
                    target.lines.push(part);
                }
            }
        }

        // Rows at the head of the table may be marked to come again at the
        // top of every page the table goes on to.
        let heading = table.rows.iter().take_while(|row| row.props.header).count();
        let repeated = (heading > 0 && heading < rows.len())
            .then(|| std::rc::Rc::new(rows[..heading].iter().map(|(row, _)| row.clone()).collect::<Vec<_>>()));
        for (number, (row, keeps)) in rows.into_iter().enumerate() {
            units.push(Unit {
                lines: Vec::new(),
                before: 0.0,
                after: 0.0,
                left: 0.0,
                keep_next: keeps,
                keep_lines: true,
                widow_control: false,
                page_break_before: false,
                contextual: false,
                style: None,
                ends_section: None,
                paragraph: None,
                row: Some(row),
                floating: props.floating,
                automatic: (false, false),
                list: None,
                header: repeated.clone().filter(|_| number >= heading),
                anchored: Vec::new(),
                label: None,
                joined: (false, false),
                frame: None,
                boxes: Vec::new(),
            });
        }
        self.table_styles = outer;
    }

    /// Lays out the content of a header or footer, with the page number and
    /// count written in when they are known. The body's own numbering of
    /// paragraphs and of list items is left as it was.
    fn story(&mut self, blocks: &[Block], section: &Section, numbers: Option<&PageNumbers>) -> Vec<Unit> {
        let filled = numbers.map(|numbers| with_numbers(blocks, numbers));
        // The header and the footer of a page are numbered as one, apart
        // from the body. (Measured in Word, docx4j's probe
        // `numbering-stories`: three items of one list in the header are
        // 1 to 3, three more of it in the footer 4 to 6, while the body's
        // own begin at 1.)
        let (paragraphs, body) = (self.paragraphs, std::mem::take(&mut self.counters));
        self.counters = std::mem::take(&mut self.story_counters);
        // Tables measured here may be copies that are gone when this returns,
        // and their addresses may then be those of other tables.
        let measured = std::mem::take(&mut self.demands);
        let mut units = Vec::new();
        self.blocks(filled.as_deref().unwrap_or(blocks), section, section.text_width() as f32 * TWIP, &mut units);
        self.story_counters = std::mem::replace(&mut self.counters, body);
        (self.paragraphs, self.demands) = (paragraphs, measured);
        units
    }

    /// The frame of a page of `section` (the section with index `index`):
    /// which header and footer it has, and where they leave the body.
    ///
    /// Measured in Word: a header begins its distance from the top of the
    /// page and a footer ends its distance from the bottom; the body begins
    /// at the top margin, or under the header where that reaches lower, and
    /// likewise at the bottom. A page without a header has only its margin,
    /// and a margin given as a negative number holds whatever the header.
    fn frame(
        &mut self,
        section: &Section,
        index: usize,
        stories: &(StoryRefs, StoryRefs),
        number: i32,
        first: bool,
        heights: &mut HashMap<(String, usize), f32>,
    ) -> Frame {
        let pick = |refs: &StoryRefs| {
            if first && section.title_page {
                refs.first.clone()
            } else if self.docx.settings.even_and_odd_headers && number % 2 == 0 {
                refs.even.clone()
            } else {
                refs.default.clone()
            }
        };
        let (header, footer) = (pick(&stories.0), pick(&stories.1));
        // How far a header or footer reaches in from its edge of the page;
        // nothing at all where the page has none.
        let mut reach = |id: &Option<String>, distance: i32| -> f32 {
            let Some((id, blocks)) = id.as_ref().and_then(|id| Some((id, self.docx.stories.get(id)?))) else {
                return 0.0;
            };
            let height = match heights.get(&(id.clone(), index)) {
                Some(known) => *known,
                None => {
                    let measured = stack(self.story(blocks, section, None)).1;
                    heights.insert((id.clone(), index), measured);
                    measured
                }
            };
            distance as f32 * TWIP + height
        };
        let (header_reach, footer_reach) = (reach(&header, section.header), reach(&footer, section.footer));
        let points = |twips: i32| twips as f32 * TWIP;
        let top = if section.margin_top_fixed { 0.0 } else { header_reach }.max(points(section.margin_top));
        let bottom = points(section.page_height)
            - if section.margin_bottom_fixed { 0.0 } else { footer_reach }.max(points(section.margin_bottom));
        Frame { top, bottom, header, footer, number, section: index }
    }

    /// Lays out blocks that follow one another.
    ///
    /// Automatic spacing is as a browser would have it, which is where it
    /// comes from: none between two items of one list. (Measured in Word,
    /// docx4j's probe `spacing-autospacing-context`; and none above the
    /// first paragraph of a cell or below the last, which
    /// [`Builder::table`] sees to.)
    fn blocks(&mut self, blocks: &[Block], section: &Section, width: f32, units: &mut Vec<Unit>) {
        let mut previous: Option<usize> = None;
        for (index, block) in blocks.iter().enumerate() {
            match block {
                Block::Paragraph(paragraph) => {
                    let beside = |at: Option<usize>| match at.and_then(|at| blocks.get(at)) {
                        Some(Block::Paragraph(other)) => Some(other),
                        _ => None,
                    };
                    let around = (beside(index.checked_sub(1)), beside(Some(index + 1)));
                    let mut unit = self.paragraph(paragraph, around, section, width);
                    if let Some(before) = previous.map(|at| &mut units[at]) {
                        if before.list.is_some() && before.list == unit.list {
                            if before.automatic.1 {
                                before.after = 0.0;
                            }
                            if unit.automatic.0 {
                                unit.before = 0.0;
                            }
                        }
                    }
                    previous = Some(units.len());
                    units.push(unit);
                }
                Block::Table(table) => {
                    self.table(table, section, width, units);
                    previous = None;
                }
            }
        }
    }
}

/// How many paragraphs there are in `blocks`, those in tables included.
fn paragraphs_in(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(_) => 1,
            Block::Table(table) => {
                table.rows.iter().flat_map(|row| &row.cells).map(|cell| paragraphs_in(&cell.blocks)).sum()
            }
        })
        .sum()
}

/// Notes where the text of a line placed at (`x`, `y`) is: the line itself,
/// or the lines it is made of if it is the row of a table inside a cell.
fn record(line: &paragraph::Line, x: f32, y: f32, lines: &mut Vec<PlacedLine>) {
    lines.extend(line.inner.iter().map(|placed| placed.moved(x, y)));
    if line.text.trim().is_empty() {
        return;
    }
    if line.parts.is_empty() {
        lines.push(line.placed().moved(x, y));
    }
    lines.extend(line.parts.iter().map(|placed| placed.moved(x, y)));
}

/// How many columns a table has: those of its grid, or as many as its
/// longest row covers.
fn column_count(table: &oco_docx::Table) -> usize {
    let covered =
        |row: &oco_docx::TableRow| row.cells.iter().map(|cell| (cell.props.grid_span as usize).max(1)).sum::<usize>();
    table.rows.iter().map(covered).max().unwrap_or(0).max(table.grid.len()).max(1)
}

/// Text laid out on its own, outside any page: what [`story`] returns.
#[derive(Debug, Clone, Default)]
pub struct Story {
    /// What to draw, from the story's left and top.
    pub items: Vec<Item>,
    /// Its lines of text, placed as the items are.
    pub lines: Vec<PlacedLine>,
    /// How tall it is, the space after its last paragraph included.
    pub height: f32,
    /// How far to the right its text reaches at the most.
    pub reach: f32,
    pub missing_fonts: Vec<String>,
}

/// Lays out blocks in a column `width` wide, as a story of its own: the
/// text of a shape on a slide, of a cell of a sheet, of anything that is
/// not a page of a document but has text set as a document's is. `docx`
/// is what the blocks are read against, its styles, its numbering and its
/// settings; `Docx::default()` for text that has all its formatting on it.
/// Lines are broken, and paragraphs spaced, exactly as in a document's
/// body; there is no grid, and nothing is anchored or noted.
pub fn story(docx: &Docx, blocks: &[Block], width: f32, fonts: &FontDb, options: &Options) -> Story {
    let mut builder = Builder::new(docx, fonts, options, Vec::new(), &Fields::new());
    let section = Section::default();
    let mut units = Vec::new();
    builder.blocks(blocks, &section, width.max(1.0), &mut units);
    let (placed, height, _) = stack(units);
    let mut story = Story { height, missing_fonts: builder.missing_fonts, ..Story::default() };
    for (offset, mut line) in placed {
        story.items.extend(std::mem::take(&mut line.items).into_iter().map(|item| item.moved(0.0, offset)));
        record(&line, 0.0, offset, &mut story.lines);
    }
    story.reach = story.lines.iter().map(|line| line.x1).fold(0.0, f32::max);
    story
}

pub fn layout(docx: &Docx, fonts: &FontDb, options: &Options) -> Layout {
    let mut notes = number_notes(docx);
    let mut fields = Fields::new();
    let (mut laid, mut began, mut placed) = layout_pass(docx, fonts, options, notes.clone(), &fields);
    // Footnotes numbered afresh on each page take their numbers from the
    // pages they come to stand on, and the widths of the numbers may in
    // turn move a line to another page: lay out again until the two agree,
    // which they nearly always do the first time. The same goes for the
    // fields in the text that show the number of their page or a count of
    // pages: they show what the layout before found, until a layout finds
    // what it was given. (WPS 12, the probe `body-page-fields` of the
    // display audit of 2026-11-14: a `PAGE` field stored as 99 on each of
    // two pages shows 1 and 2. Word works such fields out when it prints;
    // docx4j's Word PDFs have none to measure.)
    for _ in 0..3 {
        let renumbered = renumber(&mut notes, &began);
        if !renumbered && placed == fields {
            break;
        }
        fields = placed;
        (laid, began, placed) = layout_pass(docx, fonts, options, notes.clone(), &fields);
    }
    let drawable = |part: &String| {
        docx.media.get(part).and_then(|file| file.bytes().ok()).is_some_and(|bytes| svg::media_type(&bytes).is_some())
    };
    laid.undrawn = laid
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter(|item| matches!(item, Item::Image { part: Some(part), .. } if !drawable(part)))
        .count();
    laid
}

/// Lays the document out with its notes numbered as given and its page
/// fields showing what `fields` says. Returns the pages; for each note, the
/// page its footnote begins on; and where the page fields came to stand.
fn layout_pass(
    docx: &Docx,
    fonts: &FontDb,
    options: &Options,
    notes: Vec<NoteEntry>,
    fields: &Fields,
) -> (Layout, Vec<Option<usize>>, Fields) {
    // Section properties sit on the last paragraph of each section, so the
    // page setup of a paragraph is that of the next section break at or
    // after it. Collect them in order first.
    let mut setups: Vec<&Section> =
        docx.document.paragraphs().iter().filter_map(|p| p.section_break.as_ref()).collect();
    setups.push(&docx.document.section);

    let mut builder = Builder::new(docx, fonts, options, notes, fields);
    let mut units = Vec::new();
    // Each section is laid out at its own text width. A section ends with
    // the paragraph that carries its page setup.
    let body = &docx.document.body;
    let (mut from, mut number) = (0, 0);
    for (index, block) in body.iter().enumerate() {
        let ends = matches!(block, Block::Paragraph(paragraph) if paragraph.section_break.is_some());
        if ends || index + 1 == body.len() {
            let section = setups[number.min(setups.len() - 1)];
            builder.blocks(&body[from..=index], section, section.text_width() as f32 * TWIP, &mut units);
            (from, number) = (index + 1, number + 1);
        }
    }

    // Endnotes come after the text, under a short line, in the order of
    // their first references (as Word puts them by default, at the end of
    // the document).
    let endnotes: Vec<usize> = (0..builder.notes.len()).filter(|&index| builder.notes[index].endnote).collect();
    if !endnotes.is_empty() {
        let last = setups[setups.len() - 1];
        let height = builder.separator_of(last, false, true);
        let rule = Item::Rule {
            x1: 0.0,
            y1: height / 2.0,
            x2: 144.0,
            y2: height / 2.0,
            width: 0.6,
            color: [0, 0, 0],
            dash: [0.0, 0.0],
        };
        let line = paragraph::Line { height, needed: height, items: vec![rule], ..paragraph::Line::default() };
        units.push(Unit { lines: vec![line], keep_next: true, keep_lines: true, ..Unit::default() });
        for index in endnotes {
            units.extend(builder.note_units(index, last));
        }
    }

    let mut pages: Vec<Page> = Vec::new();
    let mut section_index = 0;
    let mut section_starts = vec![0usize; setups.len()];
    let mut section_ends = vec![0usize; setups.len()];
    // A header or footer that a section does not name is the one of the
    // section before.
    let mut stories: Vec<(StoryRefs, StoryRefs)> = Vec::with_capacity(setups.len());
    for section in &setups {
        let inherit = |own: &StoryRefs, before: Option<&StoryRefs>| StoryRefs {
            default: own.default.clone().or_else(|| before.and_then(|b| b.default.clone())),
            first: own.first.clone().or_else(|| before.and_then(|b| b.first.clone())),
            even: own.even.clone().or_else(|| before.and_then(|b| b.even.clone())),
        };
        let before = stories.last();
        stories.push((
            inherit(&section.headers, before.map(|(headers, _)| headers)),
            inherit(&section.footers, before.map(|(_, footers)| footers)),
        ));
    }
    // What each page has around its body: one frame for each page.
    let mut frames: Vec<Frame> = Vec::new();
    let mut heights: HashMap<(String, usize), f32> = HashMap::new();
    // The number of the page being filled, as its header would show it.
    let mut number = 0;
    let (mut y, mut top, mut bottom): (f32, f32, f32);
    // Footnotes. The notes laid out so far; for each page, the parts of
    // notes at its foot, each a note and the range of its lines, with
    // whether the first of them runs on from the page before; and the part
    // still to go on the next page. The notes stand on the bottom of the
    // page's body, `floor`, and the room they take is taken from the text
    // as they come: `bottom` is always as far down as the text may go.
    let mut floor: f32;
    let mut laid_notes: HashMap<usize, LaidNote> = HashMap::new();
    let mut page_notes: Vec<PageNotes> = Vec::new();
    let mut carried: Vec<(usize, usize)> = Vec::new();
    // Word 2010 and later make the space after a paragraph fit on a page
    // with footnotes; a document in the mode of an earlier Word is
    // paginated without that. (`stress018.docx` of the corpus, saved by
    // Word 2007: all 27 page starts Word recorded without the rule, 24 with
    // it. `stress008.docx`, in the mode of Word 2010: 13 of 117 with the
    // rule, 5 without it; weak, since it drifts from Word for other reasons.)
    let spaced = docx.settings.compatibility_mode >= 14;
    // Opens a page of the section with the given index; `$first` says that
    // the section begins on it.
    macro_rules! turn_page {
        ($section:expr, $first:expr) => {{
            let index = ($section).min(setups.len() - 1);
            let section = setups[index];
            number = section.page_number_start.filter(|_| $first).unwrap_or(number + 1);
            let frame = builder.frame(section, index, &stories[index], number, $first, &mut heights);
            (top, bottom) = (frame.top, frame.bottom);
            pages.push(Page {
                width: section.page_width as f32 * TWIP,
                height: section.page_height as f32 * TWIP,
                body: (top, bottom),
                ..Page::default()
            });
            frames.push(frame);
            y = top;
            floor = bottom;
            page_notes.push((Vec::new(), false));
            // What did not fit at the foot of the page before goes on at the
            // foot of this one, under a line across the whole text.
            if !carried.is_empty() {
                let rule = builder.separator(section, true);
                let mut parts: Vec<NotePart> = Vec::new();
                for (note, from) in std::mem::take(&mut carried) {
                    let laid = &laid_notes[&note];
                    // In their order: once one runs on again, so do those after it.
                    let room = floor - top - LEAST_TEXT - rule - stacked(&parts, &laid_notes, false);
                    let to = if carried.is_empty() { laid.take(from, room, parts.is_empty()) } else { from };
                    if to > from {
                        parts.push((note, from, to));
                    }
                    if to < laid.lines.len() {
                        carried.push((note, to));
                    }
                }
                bottom = floor - rule - stacked(&parts, &laid_notes, true);
                *page_notes.last_mut().expect("just pushed") = (parts, true);
            }
        }};
    }
    turn_page!(0, true);
    let mut page_is_empty = true;
    // How many of the pages have had their first line placed.
    let mut started = 0;
    // Whether the current page began with an automatic break (space before
    // the first paragraph is then dropped) rather than at the document's start.
    let mut after_break = false;
    let mut previous_after = 0.0f32;
    let mut previous_style: Option<(Option<String>, bool)> = None;

    // Height of the lines of `unit` that must share a page with its first line.
    let must_stay = |unit: &Unit| -> f32 {
        if let Some(row) = &unit.row {
            // A row that may split needs room for the first line of its
            // cells, and for its least height; any other, for all of it.
            if row.cant_split || row.exact_height.is_some() {
                return row.height();
            }
            let first = row.cells.iter().filter_map(|cell| cell.lines.first()).map(|part| part.gap + part.line.height);
            return row.border + (row.padding.0 + first.fold(0.0, f32::max)).max(row.min_height);
        }
        let count = if unit.keep_lines {
            unit.lines.len()
        } else if unit.widow_control {
            // Fewer than four lines cannot be split without leaving one alone.
            if unit.lines.len() <= 3 {
                unit.lines.len()
            } else {
                2
            }
        } else {
            1
        };
        // The last of them only has to fit as far as its text reaches: extra
        // leading below it may run past the bottom margin, as it may for
        // any line that ends a page.
        let lines = &unit.lines[..count.min(unit.lines.len())];
        let last = lines.last().map_or(0.0, |line| line.needed);
        lines.iter().rev().skip(1).map(|line| line.height).sum::<f32>() + last
    };

    let sources = docx.document.paragraphs();
    // The box of the paragraph placed last, and where its text margin is.
    let mut above: Option<(ParaBox, f32)> = None;
    // The objects on the page being filled that text keeps clear of.
    let mut floats: Vec<Float> = Vec::new();
    let mut floats_page = 0;
    for index in 0..units.len() {
        let section = setups[section_index.min(setups.len() - 1)];
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        let unit = &units[index];

        // An empty paragraph that is there only to carry the break to a
        // section that goes on in the same page takes no room at all: what
        // follows stands as if it were not there. (Measured in Word,
        // docx4j's probe `section-continuous-geometry`: at each of three
        // such breaks the first paragraph of the new section follows the
        // last of the old at the distance of any two paragraphs.)
        if unit.ends_section.is_some() && !unit.page_break_before {
            let next = setups[(section_index + 1).min(setups.len() - 1)];
            let stays = next.start == SectionStart::Continuous
                && (next.page_width, next.page_height) == (section.page_width, section.page_height);
            let bare = |line: &paragraph::Line| {
                line.text.is_empty() && line.items.is_empty() && line.notes.is_empty() && !line.page_break_after
            };
            let empty = unit.row.is_none()
                && unit.anchored.is_empty()
                && unit.boxes.is_empty()
                && unit.frame.is_none()
                && matches!(unit.lines.as_slice(), [only] if bare(only));
            if stays && empty {
                section_ends[section_index] = pages.len();
                section_index += 1;
                section_starts[section_index] = pages.len() - 1;
                continue;
            }
        }
        if unit.page_break_before && !page_is_empty {
            turn_page!(section_index, false);
            page_is_empty = true;
            after_break = true;
        }
        let gap = |page_is_empty: bool, after_break: bool| -> f32 {
            if page_is_empty {
                // Top of a page: nothing above to keep away from. Word still
                // honours space before at the very start of the document,
                // and at the start of a section on a page of its own, where
                // the space after the paragraph that ended the section
                // before is taken off it: the space between the two is the
                // larger of the two, and that much of it has been given at
                // the foot of the page before. (Measured in Word, docx4j's
                // probe `spacing-section-start`: 36 pt before, after a
                // section that ends with 10 pt after, is 26 pt at the top of
                // the page; after 20 pt it is 16 pt; and 6 pt before after
                // 20 pt is nothing.)
                if after_break {
                    0.0
                } else {
                    (unit.before - previous_after).max(0.0)
                }
            } else {
                let same_style = previous_style
                    .as_ref()
                    .is_some_and(|(style, contextual)| *style == unit.style && (*contextual || unit.contextual));
                if same_style {
                    0.0
                } else {
                    previous_after.max(unit.before)
                }
            }
        };

        // Keep with next: this paragraph and what it is chained to must fit
        // together, or all of it moves to the next page.
        let mut chain = 0.0;
        let mut k = index;
        loop {
            let link = &units[k];
            if !link.keep_next || k + 1 >= units.len() {
                chain += must_stay(link);
                break;
            }
            chain += match &link.row {
                Some(row) => row.height(),
                None => link.lines.iter().map(|l| l.height).sum::<f32>() + link.after.max(units[k + 1].before),
            };
            k += 1;
        }
        // The footnotes of the lines that must stay together begin on the
        // same page; and where the last of those lines ends its paragraph
        // on a page with footnotes, the paragraph's space after must fit
        // too (see `spaced`).
        {
            let last = &units[k];
            let together = match (&last.row, last.keep_lines || last.widow_control && last.lines.len() <= 3) {
                (Some(_), _) => 0,
                (None, true) => last.lines.len(),
                (None, false) => 1 + usize::from(last.widow_control),
            };
            let lines = units[index..k].iter().flat_map(|link| &link.lines).chain(last.lines.iter().take(together));
            let (on_page, continued) = page_notes.last().expect("a page is always open");
            let fresh = fresh_notes(lines.flat_map(|line| &line.notes), &builder.notes, on_page, &[]);
            let section = setups[section_index.min(setups.len() - 1)];
            for &note in &fresh {
                laid_notes.entry(note).or_insert_with(|| builder.note(note, section));
            }
            let noted = !on_page.is_empty() || !fresh.is_empty();
            if !fresh.is_empty() {
                let rule = builder.separator(section, *continued);
                chain += bottom - (floor - rule - least_with(on_page, &fresh, &laid_notes));
            }
            if noted && spaced && last.row.is_none() && together >= last.lines.len() {
                chain += last.after;
            }
        }
        let page_height = bottom - top;
        // A page has been begun inside a table: its heading rows come first.
        let mut turned = false;
        if !page_is_empty && y + gap(false, false) + chain > bottom + 0.01 && chain <= page_height {
            turn_page!(section_index, false);
            page_is_empty = true;
            after_break = true;
            turned = true;
        }
        let space = gap(page_is_empty, after_break);
        if !page_is_empty {
            // The space between two paragraphs is the upper one's space
            // after, and then what the lower one's space before adds to
            // it; each part is inside its paragraph's box unless a border
            // closes the box there. (Measured in WPS: the fill of a shaded
            // paragraph goes on through its space after, and through as
            // much of its space before as exceeds the space after of the
            // paragraph above. Word's PDFs show the same of a border at
            // the left, which runs on through the space after.)
            let own = previous_after.min(space);
            let page = pages.last_mut().expect("a page is always open");
            if let Some((upper, x)) = above.as_ref().filter(|(upper, _)| upper.open.1) {
                upper.draw(page, *x, y, y + own);
            }
            if let Some(lower) = unit.frame.as_ref().filter(|lower| lower.open.0) {
                lower.draw(page, left + unit.left, y + own, y + space);
            }
        }
        above = None;
        y += space;
        if let Some(row) = &unit.row {
            // Where each cell has got to: the next of its lines to place.
            let mut cursors = vec![0usize; row.cells.len()];
            loop {
                if let Some(heading) = unit.header.as_ref().filter(|_| std::mem::take(&mut turned)) {
                    let page = pages.last_mut().expect("a page is always open");
                    for row in heading.iter() {
                        let all: Vec<(usize, usize)> = row.cells.iter().map(|cell| (0, cell.lines.len())).collect();
                        let height = row.height();
                        row.place(&all, true, left, y, y + height, page);
                        y += height;
                    }
                }
                let first_part = cursors.iter().all(|cursor| *cursor == 0);
                let content = row
                    .cells
                    .iter()
                    .zip(&cursors)
                    .map(|(cell, cursor)| match cell.merged {
                        true if first_part => cell.own,
                        true => 0.0,
                        false => cell.rest(*cursor),
                    })
                    .fold(0.0, f32::max);
                let height = row.part_height(content, first_part);
                // How many more lines of each cell the page has room for.
                let room = bottom - y - row.head();
                let takes: Vec<usize> = row
                    .cells
                    .iter()
                    .zip(&cursors)
                    .map(|(cell, cursor)| {
                        // The lines of a merged cell that begin in this row
                        // stay with the first part of it.
                        if cell.merged {
                            return cell.lines.len() - *cursor;
                        }
                        let mut used = 0.0;
                        let mut take = cell.lines[*cursor..]
                            .iter()
                            .take_while(|part| {
                                used += part.gap + part.line.height;
                                used <= room + 0.01
                            })
                            .count();
                        // Back to a place where the page may end.
                        while take > 0 && cell.lines.get(*cursor + take).is_some_and(|next| !next.breakable) {
                            take -= 1;
                        }
                        take
                    })
                    .collect();
                // The footnotes the rest of the row refers to go on its page.
                // A row that refers to some is not broken: it goes on whole.
                let (on_page, continued) = page_notes.last().expect("a page is always open");
                let rest = row.cells.iter().zip(&cursors).flat_map(|(cell, cursor)| {
                    cell.lines[(*cursor).min(cell.lines.len())..].iter().flat_map(|part| &part.line.notes)
                });
                let fresh = fresh_notes(rest, &builder.notes, on_page, &[]);
                for &note in &fresh {
                    laid_notes.entry(note).or_insert_with(|| builder.note(note, section));
                }
                let notes_height = match fresh.is_empty() {
                    true => 0.0,
                    false => {
                        let rule = builder.separator(section, *continued);
                        bottom - (floor - rule - least_with(on_page, &fresh, &laid_notes))
                    }
                };
                let fits = y + height + notes_height <= bottom + 0.01;
                // A row breaks only where every cell that has something left
                // gets at least a line onto this page.
                let own = |cell: &RowCell, cursor: usize| !cell.merged && cursor < cell.lines.len();
                let splits = !row.cant_split
                    && row.exact_height.is_none()
                    && row
                        .cells
                        .iter()
                        .zip(&cursors)
                        .zip(&takes)
                        .any(|((cell, cursor), take)| own(cell, *cursor) && *take > 0)
                    && row
                        .cells
                        .iter()
                        .zip(&cursors)
                        .zip(&takes)
                        .all(|((cell, cursor), take)| !own(cell, *cursor) || *take > 0);
                if !fits && (!splits || notes_height > 0.0) && !page_is_empty {
                    turn_page!(section_index, false);
                    page_is_empty = true;
                    after_break = true;
                    turned = true;
                    continue;
                }
                let whole = fits || !splits || notes_height > 0.0;
                let number_of_pages = pages.len();
                let page = pages.last_mut().expect("a page is always open");
                if started < number_of_pages {
                    started = number_of_pages;
                    page.start = None;
                }
                let ranges: Vec<(usize, usize)> = row
                    .cells
                    .iter()
                    .zip(&cursors)
                    .zip(&takes)
                    .map(|((cell, cursor), take)| (*cursor, if whole { cell.lines.len() } else { *cursor + *take }))
                    .collect();
                let part_bottom = if whole { y + height } else { bottom };
                row.place(&ranges, whole && first_part, left, y, part_bottom, page);
                // The notes of what was placed, at the foot of this page.
                let on_page = &page_notes.last().expect("a page is always open").0;
                let within = row
                    .cells
                    .iter()
                    .zip(&ranges)
                    .flat_map(|(cell, (from, to))| cell.lines[*from..*to].iter().flat_map(|part| &part.line.notes));
                let placed = fresh_notes(within, &builder.notes, on_page, &[]);
                if !placed.is_empty() {
                    for &note in &placed {
                        laid_notes.entry(note).or_insert_with(|| builder.note(note, section));
                    }
                    let (parts, continued) = page_notes.last_mut().expect("a page is always open");
                    let rule = builder.separator(section, *continued);
                    put_notes(&placed, &laid_notes, parts, &mut carried, floor - rule - part_bottom);
                    bottom = floor - rule - stacked(parts, &laid_notes, true);
                }
                for (cursor, (_, end)) in cursors.iter_mut().zip(&ranges) {
                    *cursor = *end;
                }
                page_is_empty = false;
                if whole {
                    y = part_bottom;
                    break;
                }
                turn_page!(section_index, false);
                page_is_empty = true;
                after_break = true;
                turned = true;
            }
            previous_after = 0.0;
            previous_style = None;
            continue;
        }

        if floats_page != pages.len() {
            floats.clear();
            floats_page = pages.len();
        }
        let width = section.text_width() as f32 * TWIP;
        let column = (left + unit.left, left + unit.left + width);
        // Objects anchored in the paragraph go on the page its first line
        // goes on, and that line may have to begin below them: if it then
        // does not fit, paragraph and objects go to the next page together.
        if !unit.anchored.is_empty() {
            let placed = |top_of_paragraph: f32, page: &Page, body: (f32, f32)| -> Vec<(f32, f32, Float)> {
                let boxes = unit.anchored.iter().map(|object| {
                    let (x, y) = anchored_at(object, (page.width, page.height), column, body, top_of_paragraph);
                    let distance = object.anchor.distance.map(|emu| emu as f32 / oco_docx::EMU_PER_POINT as f32);
                    let float = Float {
                        left: x - distance[2],
                        right: x + object.width + distance[3],
                        top: y - distance[0],
                        bottom: y + object.height + distance[1],
                        across: object.anchor.wrap == Wrap::TopAndBottom,
                        side: object.anchor.wrap_side,
                    };
                    (x, y, float)
                });
                boxes.collect()
            };
            let first = unit.lines.first().map_or(0.0, |line| line.needed);
            let mut own = placed(y, pages.last().expect("a page is always open"), (top, bottom));
            let wraps = |object: &paragraph::Anchored| object.anchor.wrap != Wrap::None;
            let mut all: Vec<Float> = std::mem::take(&mut floats);
            let before = all.len();
            all.extend(
                own.drain(..).zip(&unit.anchored).filter(|(_, object)| wraps(object)).map(|(placed, _)| placed.2),
            );
            let begins = among(&all, y, first, column).0;
            all.truncate(before);
            floats = all;
            if begins + first > bottom + 0.01 && !page_is_empty {
                turn_page!(section_index, false);
                page_is_empty = true;
                after_break = true;
                floats.clear();
                floats_page = pages.len();
            }
            let page = pages.last_mut().expect("a page is always open");
            for ((x, y, float), object) in placed(y, page, (top, bottom)).into_iter().zip(&unit.anchored) {
                put_object(page, object, x, y);
                if wraps(object) {
                    floats.push(float);
                }
            }
        }

        // Where an object stands beside the paragraph, its lines are broken
        // again to keep to one side of it. Which lines those are depends on
        // how they break, so this is done until it settles.
        let mut lines = std::borrow::Cow::Borrowed(&unit.lines[..]);
        let beside = floats.iter().any(|float| !float.across);
        if let Some(source) = unit.paragraph.and_then(|index| sources.get(index)).filter(|_| beside) {
            const OPEN: (f32, f32) = (f32::MIN, f32::MAX);
            let mut shape: Vec<(f32, f32)> = Vec::new();
            for _ in 0..6 {
                let mut next = Vec::new();
                let mut at = y;
                for line in lines.iter() {
                    let (settled, bounds) = among(&floats, at, line.height, column);
                    if settled + line.needed > bottom + 0.01 {
                        // The rest goes to the next page, clear of these objects.
                        break;
                    }
                    next.push(bounds.map_or(OPEN, |(from, to)| (from - column.0, to - column.0)));
                    at = settled + line.height;
                }
                while next.last() == Some(&OPEN) {
                    next.pop();
                }
                if next == shape {
                    break;
                }
                shape = next;
                lines = std::borrow::Cow::Owned(builder.again(source, unit, section, width, &shape));
            }
        }

        let count = lines.len();
        for (number, line) in lines.iter().enumerate() {
            if floats_page != pages.len() {
                floats.clear();
                floats_page = pages.len();
            }
            if !floats.is_empty() {
                y = among(&floats, y, line.height, column).0;
            }
            // A line goes on the page where the footnotes it refers to
            // begin. They take room at the foot of the page, as does the
            // line between them and the text when they are the first
            // there. What must fit with the line is each of its notes but
            // the last whole, and of the last as much as may stand apart
            // from its rest: what is left of it runs on at the foot of the
            // next page. (Word, `stress018.docx` of the corpus: the line
            // that refers to a note of thirteen lines ends a page that has
            // room for five of them, and the next page begins with the line
            // after it.) Where the document is of Word 2010 or later, the
            // space after a paragraph has to fit as well on a page with
            // footnotes. (Word, docx4j's probes `footnotes` and
            // `footnote-space-after`: a paragraph of three lines that would
            // just fit goes to the next page when its space after does not.)
            let mut fresh;
            loop {
                let (on_page, continued) = page_notes.last().expect("a page is always open");
                fresh = fresh_notes(line.notes.iter(), &builder.notes, on_page, &[]);
                // The notes of the paragraph's last line, where this is the line before it.
                let last = match number + 2 == count {
                    true => fresh_notes(lines[number + 1].notes.iter(), &builder.notes, on_page, &fresh),
                    false => Vec::new(),
                };
                for &note in fresh.iter().chain(&last) {
                    laid_notes.entry(note).or_insert_with(|| builder.note(note, section));
                }
                let rule = builder.separator(section, *continued);
                let after = |ends: bool, noted: bool| if ends && noted && spaced { unit.after } else { 0.0 };
                // As far down as the text may go once the notes of this line have begun on the page.
                let under = match fresh.is_empty() {
                    true => bottom,
                    false => floor - rule - least_with(on_page, &fresh, &laid_notes),
                };
                let noted = !on_page.is_empty() || !fresh.is_empty();
                let fits = y + line.needed + after(number + 1 == count, noted) <= under + 0.01;
                // Widow control: the last line may not go to a new page
                // alone. It stays here if it fits below this line with what
                // the notes of both take; a note of this line that is
                // parted leaves it no room.
                let strands_last = unit.widow_control && count >= 4 && number + 2 == count && fits && {
                    let next = &lines[number + 1];
                    let mut whole = on_page.clone();
                    whole.extend(fresh.iter().map(|&note| (note, 0, laid_notes[&note].lines.len())));
                    let noted = !whole.is_empty() || !last.is_empty();
                    let under = if noted { floor - rule - least_with(&whole, &last, &laid_notes) } else { bottom };
                    y + line.height + next.needed + after(true, noted) > under + 0.01
                };
                if (!fits || strands_last) && !page_is_empty {
                    turn_page!(section_index, false);
                    after_break = true;
                    page_is_empty = true;
                    continue;
                }
                break;
            }
            let number_of_pages = pages.len();
            let page = pages.last_mut().expect("a page is always open");
            // The first line to land on a page is where the page begins.
            if started < number_of_pages {
                started = number_of_pages;
                page.start = unit.paragraph.map(|paragraph| Position { paragraph, offset: line.offset });
            }
            for item in &line.items {
                let mut item = item.clone();
                item.translate(left + unit.left, y);
                page.items.push(item);
            }
            record(line, left, y, &mut page.lines);
            y += line.height;
            page_is_empty = false;
            if !fresh.is_empty() {
                let (parts, continued) = page_notes.last_mut().expect("a page is always open");
                let rule = builder.separator(section, *continued);
                put_notes(&fresh, &laid_notes, parts, &mut carried, floor - rule - y);
                bottom = floor - rule - stacked(parts, &laid_notes, true);
            }
            if line.page_break_after {
                turn_page!(section_index, false);
                page_is_empty = true;
                after_break = true;
            }
        }
        previous_after = unit.after;
        previous_style = Some((unit.style.clone(), unit.contextual));
        above = unit.frame.clone().map(|frame| (frame, left + unit.left));

        if unit.ends_section.is_some() {
            section_ends[section_index] = pages.len();
            section_index += 1;
            let next = setups[section_index.min(setups.len() - 1)];
            // A section that goes on in the same page cannot do so on paper
            // of another size or turned the other way: it begins a new
            // page then. (Measured in Word, docx4j's probe
            // `page-blank-landscape`.)
            let same_paper = (next.page_width, next.page_height) == (section.page_width, section.page_height);
            if next.start != SectionStart::Continuous || !same_paper {
                // A section that is to begin on an odd page, or on an even
                // one, has an empty page put before it where the next page
                // is of the other kind. The empty page takes a number and
                // has no header or footer. (WPS 12, the probes
                // `section-oddPage` and `section-evenPage` of the display
                // audit of 2026-11-14: after one page, a section for odd
                // pages begins on the third, one for even pages on the
                // second.) Where the section numbers its pages afresh from
                // a number of the right kind, the empty page comes if the
                // page before is of that kind too: two right-hand pages do
                // not follow each other. (Not measured.)
                let wanted = match next.start {
                    SectionStart::OddPage => Some(1),
                    SectionStart::EvenPage => Some(0),
                    _ => None,
                };
                let empty_first = wanted.is_some_and(|wanted| match next.page_number_start {
                    None => (number + 1).rem_euclid(2) != wanted,
                    Some(first) => first.rem_euclid(2) == wanted && number.rem_euclid(2) == wanted,
                });
                if empty_first {
                    number += 1;
                    pages.push(Page {
                        width: next.page_width as f32 * TWIP,
                        height: next.page_height as f32 * TWIP,
                        ..Page::default()
                    });
                    let index = section_index.min(setups.len() - 1);
                    frames.push(Frame { top: 0.0, bottom: 0.0, header: None, footer: None, number, section: index });
                    page_notes.push((Vec::new(), false));
                    started = pages.len();
                }
                turn_page!(section_index, true);
                page_is_empty = true;
                // The first paragraph of a section keeps its space before,
                // less the space after the paragraph that ended the last.
                after_break = false;
            }
            // A continuous section starts on the existing page, so that
            // shared page belongs to both sections' SECTIONPAGES counts.
            section_starts[section_index] = pages.len() - 1;
        }
    }
    // Notes that run on past the last page of text get pages of their own.
    while !carried.is_empty() {
        let before = carried.clone();
        turn_page!(section_index, false);
        if carried == before {
            break;
        }
    }
    // The pages that hold only the rest of a note belong to the last section.
    section_ends[section_index] = pages.len();
    // Where the text of the last page ended is of no further use.
    let _ = (y, bottom, floor);

    // The footnotes at the foot of each page, under the line that parts
    // them from the text: a short one, or one across the text where a note
    // runs on from the page before. (Word, docx4j's probes `footnotes` and
    // `footnote-space-after`: the notes end at the bottom of the body; the
    // line is 144 pt long and 0.6 pt wide, in the middle of the line of the
    // paragraph that holds it.)
    for ((page, frame), (parts, continued)) in pages.iter_mut().zip(&frames).zip(&page_notes) {
        if parts.is_empty() {
            continue;
        }
        let section = setups[frame.section];
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        let separator = builder.separator(section, *continued);
        let height = separator + stacked(parts, &laid_notes, true);
        let mut y = frame.bottom - height;
        let length = if *continued { section.text_width() as f32 * TWIP } else { 144.0 };
        let middle = y + separator / 2.0;
        page.items.push(Item::Rule {
            x1: left,
            y1: middle,
            x2: left + length,
            y2: middle,
            width: 0.6,
            color: [0, 0, 0],
            dash: [0.0, 0.0],
        });
        y += separator;
        for &(note, from, to) in parts {
            let laid = &laid_notes[&note];
            let start = laid.lines.get(from).map_or(laid.height, |(offset, _)| *offset);
            for (offset, line) in &laid.lines[from..to] {
                for item in &line.items {
                    let mut item = item.clone();
                    item.translate(left, y + offset - start);
                    page.items.push(item);
                }
                record(line, left, y + offset - start, &mut page.lines);
            }
            y += laid.part(from, to);
        }
    }

    // Headers and footers go on last, when the number of pages is known.
    let count = pages.len();
    // Where the page fields of the text have come to stand.
    let mut placed = Fields::new();
    for (page, frame) in pages.iter_mut().zip(&frames) {
        let section = setups[frame.section];
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        // The page's number is written the way its section numbers pages;
        // the count of pages is always in figures (as WPS shows it: a page
        // numbered ii of a document of ten pages is "ii" of "10").
        let numbers = PageNumbers {
            page: frame.number,
            pages: count.min(i32::MAX as usize) as i32,
            section_pages: section_ends[frame.section]
                .saturating_sub(section_starts[frame.section])
                .max(1)
                .min(i32::MAX as usize) as i32,
            format: section.page_number_format.as_deref().unwrap_or("decimal"),
        };
        // A field that is parted between two pages is of the first.
        for field in page.lines.iter().flat_map(|line| &line.fields) {
            placed.entry(*field).or_insert_with(|| FieldPlace {
                page: numbers.page,
                pages: numbers.pages,
                section_pages: numbers.section_pages,
                format: numbers.format.to_owned(),
            });
        }
        let mut lines = Vec::new();
        builder.story_counters = Counters::default();
        for (story, footer) in [(&frame.header, false), (&frame.footer, true)] {
            let Some(blocks) = story.as_deref().and_then(|id| docx.stories.get(id)) else { continue };
            let (placed, height, anchored) = stack(builder.story(blocks, section, Some(&numbers)));
            let origin =
                if footer { page.height - section.footer as f32 * TWIP - height } else { section.header as f32 * TWIP };
            for (offset, mut line) in placed {
                for mut item in std::mem::take(&mut line.items) {
                    item.translate(left, origin + offset);
                    page.items.push(item);
                }
                record(&line, left, origin + offset, &mut lines);
            }
            // Objects anchored in a header or footer are placed as in the
            // body; its text takes no notice of them.
            let column = (left, left + section.text_width() as f32 * TWIP);
            for (offset, object) in anchored {
                let (x, y) = anchored_at(&object, (page.width, page.height), column, page.body, origin + offset);
                put_object(page, &object, x, y);
            }
        }
        page.lines.append(&mut lines);
        page.lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
    }
    let mut began = vec![None; builder.notes.len()];
    for (page, (parts, _)) in page_notes.iter().enumerate() {
        for &(note, from, _) in parts {
            if from == 0 {
                began[note] = Some(page);
            }
        }
    }
    let laid = Layout { pages, missing_fonts: builder.missing_fonts, unplaced: builder.unplaced, undrawn: 0 };
    (laid, began, placed)
}

/// What a page has around its body.
struct Frame {
    /// Where the body begins and ends: at the margins, or beyond a header
    /// or footer that reaches further.
    top: f32,
    bottom: f32,
    /// The header and footer of the page, by relationship id.
    header: Option<String>,
    footer: Option<String>,
    /// The page's number, as a `PAGE` field shows it.
    number: i32,
    /// Index of the page's section.
    section: usize,
}

/// Lines stacked from the top down with the spacing of their paragraphs:
/// each with its distance from the top, and the height of the whole.
///
/// A floating table takes no room: what follows it begins where it began.
/// (In Word it stands where its properties say, which are not read; here it
/// stays where the flow had come to.)
/// Stacks the units of a header or footer: its lines with how far down
/// each begins, its height, and the objects anchored in it with the top of
/// the paragraph each is anchored in.
type Stacked = (Vec<(f32, paragraph::Line)>, f32, Vec<(f32, paragraph::Anchored)>);

/// The footnotes among `notes` that are new to a page: not on it yet
/// (`parts`), not among `known`, and each once.
fn fresh_notes<'a>(
    notes: impl Iterator<Item = &'a usize>,
    entries: &[NoteEntry],
    parts: &[NotePart],
    known: &[usize],
) -> Vec<usize> {
    let mut fresh: Vec<usize> = Vec::new();
    for &note in notes {
        let seen = parts.iter().any(|part| part.0 == note) || known.contains(&note) || fresh.contains(&note);
        if !entries[note].endnote && !seen {
            fresh.push(note);
        }
    }
    fresh
}

/// How tall parts of notes are, one under the other. With `hangs`, the
/// space after the last paragraph of a note that ends the page's notes
/// does not count: it hangs below the body, as the space after the last
/// paragraph of the text does. (Word 365, `review-for-word-word15-accepted.docx`
/// of the corpus: under a note whose paragraph has 10 pt after it, a line
/// goes on the page that would not if that space stood on the body's bottom.)
fn stacked(parts: &[NotePart], laid: &HashMap<usize, LaidNote>, hangs: bool) -> f32 {
    let total: f32 = parts.iter().map(|&(note, from, to)| laid[&note].part(from, to)).sum();
    let tail = parts.last().map_or(0.0, |&(note, _, to)| laid[&note].tail(to));
    total - if hangs { tail } else { 0.0 }
}

/// How tall the notes at the foot of a page are at the least once the
/// notes `fresh` have begun under those there (`parts`). Each note begins
/// on the page of its reference: all of `fresh` but the last are there
/// whole, and of the last at least as much as may stand apart from its rest.
fn least_with(parts: &[NotePart], fresh: &[usize], laid: &HashMap<usize, LaidNote>) -> f32 {
    let mut all = parts.to_vec();
    for (index, &note) in fresh.iter().enumerate() {
        let whole = index + 1 < fresh.len();
        all.push((note, 0, if whole { laid[&note].lines.len() } else { laid[&note].next_break(0) }));
    }
    stacked(&all, laid, true)
}

/// Puts the notes `fresh` at the foot of a page, under those there
/// (`parts`), where the notes may take `room` in all: each whole where it
/// fits, else as much of it as fits and the rest carried to the next page,
/// as are the notes after it. A page that has no note on it yet takes the
/// first part of one whatever the room.
fn put_notes(
    fresh: &[usize],
    laid: &HashMap<usize, LaidNote>,
    parts: &mut Vec<NotePart>,
    carried: &mut Vec<(usize, usize)>,
    room: f32,
) {
    for &note in fresh {
        let lines = laid[&note].lines.len();
        let left = room - stacked(parts, laid, false);
        let to = if carried.is_empty() { laid[&note].take(0, left, parts.is_empty()) } else { 0 };
        if to > 0 {
            parts.push((note, 0, to));
        }
        if to < lines {
            carried.push((note, to));
        }
    }
}

fn stack(units: Vec<Unit>) -> Stacked {
    let mut placed = Vec::new();
    let mut anchored = Vec::new();
    let (mut y, mut previous_after, mut first) = (0.0f32, 0.0f32, true);
    // Where the flow goes on once the floating table being placed is over.
    let mut flow = None;
    for unit in units {
        if unit.floating && flow.is_none() {
            flow = Some((y, previous_after, first));
        }
        y += if first { unit.before } else { unit.before.max(previous_after) };
        first = false;
        previous_after = unit.after;
        anchored.extend(unit.anchored.into_iter().map(|object| (y, object)));
        let ends_table = unit.row.as_ref().is_some_and(|row| row.last);
        let lines = match unit.row {
            Some(row) => vec![row.into_line()],
            None => unit.lines,
        };
        for line in lines {
            let height = line.height;
            placed.push((y, line));
            y += height;
        }
        if ends_table {
            if let Some(resume) = flow.take() {
                (y, previous_after, first) = resume;
            }
        }
    }
    (placed, y + previous_after, anchored)
}

/// A copy of `blocks` with the page number and page count written out
/// where the fields for them are.
struct PageNumbers<'a> {
    page: i32,
    pages: i32,
    section_pages: i32,
    format: &'a str,
}

/// Whether a piece of a run is a field that shows a page number or a count of pages.
fn is_page_field(piece: &RunContent) -> bool {
    matches!(
        piece,
        RunContent::PageNumber(_)
            | RunContent::PageCount(_)
            | RunContent::SectionPageCount(_)
            | RunContent::FormattedPageNumber { .. }
    )
}

/// What a page field shows on a page with these numbers; `None` for
/// anything that is not a page field.
fn field_text(piece: &RunContent, numbers: &PageNumbers) -> Option<String> {
    Some(match piece {
        RunContent::PageNumber(_) => format_number(numbers.format, numbers.page),
        RunContent::PageCount(_) => numbers.pages.to_string(),
        RunContent::SectionPageCount(_) => numbers.section_pages.to_string(),
        RunContent::FormattedPageNumber { kind, format, .. } => {
            let value = match kind {
                oco_docx::PageFieldKind::Page => numbers.page,
                oco_docx::PageFieldKind::Pages => numbers.pages,
                oco_docx::PageFieldKind::SectionPages => numbers.section_pages,
            };
            if format == "arabicDash" {
                format!("- {value} -")
            } else if matches!(format.as_str(), "cardinalText" | "ordinalText") {
                // The field switches default to lowercase; list
                // numbering uses initial capitals for these formats.
                format_number(format, value).to_lowercase()
            } else {
                format_number(format, value)
            }
        }
        _ => return None,
    })
}

fn with_numbers(blocks: &[Block], numbers: &PageNumbers) -> Vec<Block> {
    let mut blocks = blocks.to_vec();
    fn fill(blocks: &mut [Block], numbers: &PageNumbers) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    for text_box in paragraph.runs.iter_mut().flat_map(|run| &mut run.text_boxes) {
                        fill(&mut text_box.blocks, numbers);
                    }
                    for piece in paragraph.runs.iter_mut().flat_map(|run| &mut run.content) {
                        if let Some(text) = field_text(piece, numbers) {
                            *piece = RunContent::Text(text);
                        }
                    }
                }
                Block::Table(table) => {
                    for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                        fill(&mut cell.blocks, numbers);
                    }
                }
            }
        }
    }
    fill(&mut blocks, numbers);
    blocks
}
