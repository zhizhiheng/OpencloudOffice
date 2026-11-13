//! Lays a parsed document out into pages.
//!
//! [`layout`] takes a [`Docx`] and a set of fonts and returns pages of
//! drawing instructions: glyphs at positions, plus the rules, backgrounds and
//! marks that character effects need. [`svg::render`] turns a page into a
//! self-contained SVG. Nothing here depends on a platform's text system, so
//! the same input gives the same pages everywhere.
//!
//! This is a research prototype. It covers body text: fonts and sizes,
//! character effects, indents, alignment, line and paragraph spacing, the
//! document grid, tabs, list numbering, page breaks and a rough grid for
//! tables. Headers, footers, footnotes, images, columns and floating objects
//! are not laid out.

pub mod fonts;
mod paragraph;
pub mod svg;
pub mod text;

use std::collections::HashMap;

use oco_docx::{
    format_number, Block, BreakKind, Counters, Docx, GridKind, ParaProps, Resolver, RunContent, RunProps, Section,
    SectionStart, StoryRefs, VerticalMerge,
};
#[cfg(feature = "serde")]
use serde::Serialize;

pub use fonts::{FontDb, FontId, Model};

pub type Rgb = [u8; 3];

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
}

impl Item {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Item::Glyph { x, y, .. } | Item::Rect { x, y, .. } => {
                *x += dx;
                *y += dy;
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

/// Where a line of text ended up, for comparing against a reference.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct PlacedLine {
    pub baseline: f32,
    pub x0: f32,
    pub x1: f32,
    pub text: String,
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
}

/// The result of laying a document out.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
    /// Fonts the document asks for that are neither installed nor have a
    /// designated replacement. Text in them was set in a fallback font, so
    /// its line breaks are not to be trusted.
    pub missing_fonts: Vec<String>,
}

/// One piece of a paragraph's content, as [`content`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// A character of text, or a symbol.
    Char(char),
    Tab,
    Break(BreakKind),
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
                // Not laid out yet.
                RunContent::Drawing(_) => {}
                RunContent::PageNumber(shown) | RunContent::PageCount(shown) => {
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
        self.padding.1 + if self.last { self.border + self.gap() } else { 0.0 }
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
            + if self.last { self.border + self.gap() } else { 0.0 }
    }

    /// Puts the lines `ranges` names of each cell on `page`, the row's top
    /// at `top`, with the rules around the part of the row that ends at
    /// `bottom`. A row put down `whole` has its content where its cells'
    /// vertical alignment has it.
    fn place(&self, ranges: &[(usize, usize)], whole: bool, left: f32, top: f32, bottom: f32, page: &mut Page) {
        for (cell, (from, to)) in self.cells.iter().zip(ranges) {
            let lowered = if whole { self.lowered(cell, bottom - top) } else { 0.0 };
            let mut y = top + self.head() + lowered;
            for part in &cell.lines[*from..*to] {
                y += part.gap;
                for item in &part.line.items {
                    let mut item = item.clone();
                    item.translate(left + part.x, y);
                    page.items.push(item);
                }
                record(&part.line, left + part.x, y, &mut page.lines);
                y += part.line.height;
            }
        }
        for mut item in self.rules(top, bottom, whole) {
            item.translate(left, 0.0);
            page.items.push(item);
        }
    }

    /// The rules of the row, or of the part of it between `top` and
    /// `bottom`; `whole` if that is all of the row there is left.
    fn rules(&self, top: f32, bottom: f32, whole: bool) -> Vec<Item> {
        if self.border <= 0.0 {
            return Vec::new();
        }
        let open: Vec<(bool, bool)> = self.cells.iter().map(|cell| cell.open).collect();
        let Some(apart) = &self.apart else {
            return boxes(&self.edges, &open, top, bottom, self.border);
        };
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

    /// The row as one line that cannot break, for a table inside a cell.
    fn into_line(self) -> paragraph::Line {
        let height = self.height();
        let mut line = paragraph::Line { height, needed: height, ..paragraph::Line::default() };
        let mut texts = Vec::new();
        let lowered: Vec<f32> = self.cells.iter().map(|cell| self.lowered(cell, height)).collect();
        let (rules, head) = (self.rules(0.0, height, true), self.head());
        for (cell, lowered) in self.cells.into_iter().zip(lowered) {
            let mut y = head + lowered;
            for mut part in cell.lines {
                y += part.gap;
                for item in &mut part.line.items {
                    item.translate(part.x, y);
                }
                if !part.line.text.trim().is_empty() {
                    if texts.is_empty() {
                        (line.baseline, line.x0) = (y + part.line.baseline, part.x + part.line.x0);
                    }
                    line.x1 = line.x1.max(part.x + part.line.x1);
                    if part.line.parts.is_empty() {
                        let own = (y + part.line.baseline, part.x + part.line.x0, part.x + part.line.x1);
                        line.parts.push((own.0, own.1, own.2, part.line.text.clone()));
                    }
                    for (baseline, x0, x1, text) in part.line.parts.drain(..) {
                        line.parts.push((y + baseline, part.x + x0, part.x + x1, text));
                    }
                    texts.push(part.line.text);
                }
                line.items.append(&mut part.line.items);
                y += part.line.height;
            }
        }
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

impl Builder<'_> {
    /// What a paragraph is laid out from: its properties, runs and label,
    /// all resolved. Counts the paragraph if it is a list item.
    fn resolved<'s>(&mut self, source: &'s oco_docx::Paragraph) -> (ParaProps, paragraph::Paragraph<'s>) {
        static NONE: std::sync::OnceLock<ParaProps> = std::sync::OnceLock::new();
        let props: ParaProps = self.resolver.paragraph_in(&source.props, &self.table_styles);
        let style = source.props.style_id.as_deref();
        let label = self.resolver.list_level(&props).and_then(|(num_id, ilvl, level)| {
            let label = self.counters.next(&self.docx.numbering, num_id, ilvl)?;
            let mut label_props = self.resolver.label(style, &source.mark_props, level);
            let text = portable_bullet(&label.text, &mut label_props);
            Some(paragraph::LabelSpec { text, props: label_props, suffix: level.suffix, align: level.align })
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
        (props, paragraph::Paragraph { props: placeholder, runs, mark, label })
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

    /// Lays out a paragraph. `next` is the paragraph right after it, if
    /// there is one: paragraphs with the same borders share them.
    fn paragraph(
        &mut self,
        source: &oco_docx::Paragraph,
        next: Option<&oco_docx::Paragraph>,
        section: &Section,
        width: f32,
    ) -> Unit {
        let (props, input) = self.resolved(source);
        let input = paragraph::Paragraph { props: &props, ..input };
        let ctx = context(self.docx, section, width, self.options.model);
        let mut laid = paragraph::layout(&input, self.fonts, &ctx);
        for name in std::mem::take(&mut laid.missing_fonts) {
            if !self.missing_fonts.contains(&name) {
                self.missing_fonts.push(name);
            }
        }
        let index = self.paragraphs;
        self.paragraphs += 1;

        // The border under a paragraph is drawn its space below the last
        // line, and the paragraph is taller by that space and by the width
        // of the border: what follows begins under the border. Measured in
        // WPS, in the body (a 3 pt border 6 pt away moves the next paragraph
        // down 9 pt) and under a header that reaches below the top margin.
        //
        // Paragraphs one after the other with the same borders and indents
        // are bordered as one, so only the last of them has the border.
        if let Some(border) = props.borders.bottom.as_ref().filter(|b| b.is_visible()) {
            let joined = next.is_some_and(|next| {
                let below = self.resolver.paragraph_in(&next.props, &self.table_styles);
                below.borders == props.borders
                    && (below.indent.left, below.indent.right) == (props.indent.left, props.indent.right)
            });
            if let Some(line) = laid.lines.last_mut().filter(|_| !joined) {
                let color = match border.color {
                    Some(oco_docx::Color::Rgb(r, g, b)) => [r, g, b],
                    _ => [0, 0, 0],
                };
                let (rule, space) = (border.size as f32 / 8.0, border.space as f32);
                let at = line.height + space + rule / 2.0;
                line.items.push(Item::Rule {
                    x1: props.indent.left.unwrap_or(0) as f32 * TWIP,
                    y1: at,
                    x2: width - props.indent.right.unwrap_or(0) as f32 * TWIP,
                    y2: at,
                    width: rule,
                    color,
                    dash: [0.0, 0.0],
                });
                line.height += space + rule;
                line.needed += space + rule;
            }
        }
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
        }
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
                let mut cell =
                    RowCell { lines, tail: previous_after, merged: false, own: 0.0, open: (false, false), align };
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
            rows.push((
                Row {
                    cells,
                    edges,
                    min_height: height,
                    exact_height: row.props.exact_height.then_some(height).filter(|h| *h > 0.0),
                    cant_split: row.props.cant_split,
                    padding: (points(margins.top), points(margins.bottom)),
                    border,
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
            });
        }
        self.table_styles = outer;
    }

    /// Lays out the content of a header or footer, with the page number and
    /// count written in when they are known. The body's own numbering of
    /// paragraphs and of list items is left as it was.
    fn story(&mut self, blocks: &[Block], section: &Section, numbers: Option<&(String, String)>) -> Vec<Unit> {
        let filled = numbers.map(|numbers| with_numbers(blocks, numbers));
        let (paragraphs, counters) = (self.paragraphs, std::mem::take(&mut self.counters));
        // Tables measured here may be copies that are gone when this returns,
        // and their addresses may then be those of other tables.
        let measured = std::mem::take(&mut self.demands);
        let mut units = Vec::new();
        self.blocks(filled.as_deref().unwrap_or(blocks), section, section.text_width() as f32 * TWIP, &mut units);
        (self.paragraphs, self.counters, self.demands) = (paragraphs, counters, measured);
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
                    let next = match blocks.get(index + 1) {
                        Some(Block::Paragraph(next)) => Some(next),
                        _ => None,
                    };
                    let mut unit = self.paragraph(paragraph, next, section, width);
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
    if line.text.trim().is_empty() {
        return;
    }
    if line.parts.is_empty() {
        lines.push(PlacedLine {
            baseline: y + line.baseline,
            x0: x + line.x0,
            x1: x + line.x1,
            text: line.text.clone(),
        });
    }
    for (baseline, x0, x1, text) in &line.parts {
        lines.push(PlacedLine { baseline: y + baseline, x0: x + x0, x1: x + x1, text: text.clone() });
    }
}

/// How many columns a table has: those of its grid, or as many as its
/// longest row covers.
fn column_count(table: &oco_docx::Table) -> usize {
    let covered =
        |row: &oco_docx::TableRow| row.cells.iter().map(|cell| (cell.props.grid_span as usize).max(1)).sum::<usize>();
    table.rows.iter().map(covered).max().unwrap_or(0).max(table.grid.len()).max(1)
}

pub fn layout(docx: &Docx, fonts: &FontDb, options: &Options) -> Layout {
    // Section properties sit on the last paragraph of each section, so the
    // page setup of a paragraph is that of the next section break at or
    // after it. Collect them in order first.
    let mut setups: Vec<&Section> =
        docx.document.paragraphs().iter().filter_map(|p| p.section_break.as_ref()).collect();
    setups.push(&docx.document.section);

    let mut builder = Builder {
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
    };
    let mut units = Vec::new();
    // Line breaks depend on the text width, which this prototype takes from
    // the first section throughout.
    let first = setups[0];
    let width = first.text_width() as f32 * TWIP;
    builder.blocks(&docx.document.body, first, width, &mut units);

    let mut pages: Vec<Page> = Vec::new();
    let mut section_index = 0;
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

    for index in 0..units.len() {
        let section = setups[section_index.min(setups.len() - 1)];
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        let unit = &units[index];

        if unit.page_break_before && !page_is_empty {
            turn_page!(section_index, false);
            page_is_empty = true;
            after_break = true;
        }
        let gap = |page_is_empty: bool, after_break: bool| -> f32 {
            if page_is_empty {
                // Top of a page: nothing above to keep away from. Word still
                // honours space before at the very start of the document.
                if after_break {
                    0.0
                } else {
                    unit.before
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
        let page_height = bottom - top;
        // A page has been begun inside a table: its heading rows come first.
        let mut turned = false;
        if !page_is_empty && y + gap(false, false) + chain > bottom + 0.01 && chain <= page_height {
            turn_page!(section_index, false);
            page_is_empty = true;
            after_break = true;
            turned = true;
        }
        y += gap(page_is_empty, after_break);
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
                let fits = y + height <= bottom + 0.01;
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
                if !fits && !splits && !page_is_empty {
                    turn_page!(section_index, false);
                    page_is_empty = true;
                    after_break = true;
                    turned = true;
                    continue;
                }
                let whole = fits || !splits;
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

        let count = unit.lines.len();
        for (number, line) in unit.lines.iter().enumerate() {
            let fits = |y: f32, line: &paragraph::Line| y + line.needed <= bottom + 0.01;
            // Widow control: the last line may not go to a new page alone.
            let strands_last = unit.widow_control
                && count >= 4
                && number + 2 == count
                && fits(y, line)
                && !fits(y + line.height, &unit.lines[number + 1]);
            if (!fits(y, line) || strands_last) && !page_is_empty {
                turn_page!(section_index, false);
                after_break = true;
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
            if line.page_break_after {
                turn_page!(section_index, false);
                page_is_empty = true;
                after_break = true;
            }
        }
        previous_after = unit.after;
        previous_style = Some((unit.style.clone(), unit.contextual));

        if unit.ends_section.is_some() {
            section_index += 1;
            let next = setups[section_index.min(setups.len() - 1)];
            if next.start != SectionStart::Continuous {
                turn_page!(section_index, true);
                page_is_empty = true;
                // The first paragraph of a section keeps its space before.
                after_break = false;
                previous_after = 0.0;
            }
        }
    }
    // Headers and footers go on last, when the number of pages is known.
    let count = pages.len();
    for (page, frame) in pages.iter_mut().zip(&frames) {
        let section = setups[frame.section];
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        // The page's number is written the way its section numbers pages;
        // the count of pages is always in figures (as WPS shows it: a page
        // numbered ii of a document of ten pages is "ii" of "10").
        let number = format_number(section.page_number_format.as_deref().unwrap_or("decimal"), frame.number);
        let numbers = (number, count.to_string());
        let mut lines = Vec::new();
        for (story, footer) in [(&frame.header, false), (&frame.footer, true)] {
            let Some(blocks) = story.as_deref().and_then(|id| docx.stories.get(id)) else { continue };
            let (placed, height) = stack(builder.story(blocks, section, Some(&numbers)));
            let origin =
                if footer { page.height - section.footer as f32 * TWIP - height } else { section.header as f32 * TWIP };
            for (offset, mut line) in placed {
                for mut item in std::mem::take(&mut line.items) {
                    item.translate(left, origin + offset);
                    page.items.push(item);
                }
                record(&line, left, origin + offset, &mut lines);
            }
        }
        page.lines.append(&mut lines);
        page.lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
    }
    Layout { pages, missing_fonts: builder.missing_fonts }
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
fn stack(units: Vec<Unit>) -> (Vec<(f32, paragraph::Line)>, f32) {
    let mut placed = Vec::new();
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
    (placed, y + previous_after)
}

/// A copy of `blocks` with the page number and page count written out
/// where the fields for them are.
fn with_numbers(blocks: &[Block], numbers: &(String, String)) -> Vec<Block> {
    let mut blocks = blocks.to_vec();
    fn fill(blocks: &mut [Block], numbers: &(String, String)) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    for piece in paragraph.runs.iter_mut().flat_map(|run| &mut run.content) {
                        match piece {
                            RunContent::PageNumber(_) => *piece = RunContent::Text(numbers.0.clone()),
                            RunContent::PageCount(_) => *piece = RunContent::Text(numbers.1.clone()),
                            _ => {}
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
