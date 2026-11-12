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

use oco_docx::{
    Block, BreakKind, Counters, Docx, GridKind, ParaProps, Resolver, RunContent, RunProps, Section, SectionStart,
};
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
#[derive(Debug, Clone, Serialize)]
pub struct PlacedLine {
    pub baseline: f32,
    pub x0: f32,
    pub x1: f32,
    pub text: String,
}

/// A place in the document's content: a paragraph, by its index in
/// [`oco_docx::Document::paragraphs`], and how many pieces of its
/// [`content`] come before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
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
    /// Extra drawing below the last line: a bottom border.
    bottom_rule: Option<(f32, f32, Rgb, f32, f32)>,
    /// The section that ends after this unit.
    ends_section: Option<Section>,
    /// Index of the paragraph this is, in document order; `None` for a table row.
    paragraph: Option<usize>,
    /// The unit is a table row; `lines` is then empty.
    row: Option<Row>,
}

/// A table row ready to be placed. Its cells are columns of lines side by
/// side, and where the row meets the bottom of a page each cell breaks
/// between two of its lines.
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
}

struct RowCell {
    lines: Vec<CellLine>,
    /// Space after the cell's last paragraph, which the row's height includes.
    tail: f32,
}

struct CellLine {
    line: paragraph::Line,
    /// Blank above the line: the space between two paragraphs.
    gap: f32,
    /// Where the line's origin is, from the text margin.
    x: f32,
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
        let content = self.cells.iter().map(|cell| cell.rest(0)).fold(0.0, f32::max);
        self.part_height(content, true)
    }

    /// Height of the row, or of what is left of it, given the height of the
    /// content still to place.
    fn part_height(&self, content: f32, first_part: bool) -> f32 {
        let natural = self.border + content + self.padding.0 + self.padding.1;
        let least = if first_part { self.min_height } else { 0.0 };
        self.exact_height.unwrap_or(natural.max(least)) + if self.last { self.border } else { 0.0 }
    }

    /// The row as one line that cannot break, for a table inside a cell.
    fn into_line(self) -> paragraph::Line {
        let height = self.height();
        let mut line = paragraph::Line { height, needed: height, ..paragraph::Line::default() };
        let mut texts = Vec::new();
        for cell in self.cells {
            let mut y = self.border + self.padding.0;
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
                    texts.push(part.line.text);
                }
                line.items.append(&mut part.line.items);
                y += part.line.height;
            }
        }
        if self.border > 0.0 {
            line.items.extend(boxes(&self.edges, 0.0, height, self.border));
        }
        line.text = texts.join(" ");
        line
    }
}

/// The rules around the cells of a row, or of the part of one on a page.
fn boxes(edges: &[f32], top: f32, bottom: f32, width: f32) -> Vec<Item> {
    let rule = |x1, y1, x2, y2| Item::Rule { x1, y1, x2, y2, width, color: [0, 0, 0], dash: [0.0, 0.0] };
    let (Some(left), Some(right)) = (edges.first(), edges.last()) else { return Vec::new() };
    let mut items: Vec<Item> = edges.iter().map(|x| rule(*x, top, *x, bottom)).collect();
    items.push(rule(*left, top, *right, top));
    items.push(rule(*left, bottom, *right, bottom));
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
    fn paragraph(&mut self, source: &oco_docx::Paragraph, section: &Section, width: f32) -> Unit {
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
        let input = paragraph::Paragraph { props: &props, runs, mark, label };
        let ctx = context(self.docx, section, width, self.options.model);
        let laid = paragraph::layout(&input, self.fonts, &ctx);
        for name in laid.missing_fonts {
            if !self.missing_fonts.contains(&name) {
                self.missing_fonts.push(name);
            }
        }
        let index = self.paragraphs;
        self.paragraphs += 1;

        let bottom_rule = props.borders.bottom.as_ref().filter(|b| b.is_visible()).map(|border| {
            let color = match border.color {
                Some(oco_docx::Color::Rgb(r, g, b)) => [r, g, b],
                _ => [0, 0, 0],
            };
            let left = props.indent.left.unwrap_or(0) as f32 * TWIP;
            let right = width - props.indent.right.unwrap_or(0) as f32 * TWIP;
            (left, right, color, border.size as f32 / 8.0, border.space as f32)
        });
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
            bottom_rule,
            ends_section: source.section_break.clone(),
            paragraph: Some(index),
            row: None,
        }
    }

    /// A table: one unit for each row. Columns are as wide as the table's
    /// grid says (or, without one, as the cells say, or equal); they are not
    /// fitted to their content as Word fits the columns of an auto-fit table.
    fn table(&mut self, table: &oco_docx::Table, section: &Section, width: f32, units: &mut Vec<Unit>) {
        let props = self.resolver.table(&table.props);
        let styles = self.resolver.table_styles(table.props.style_id.as_deref());
        let outer = std::mem::replace(&mut self.table_styles, styles);
        let points = |twips: Option<i32>| twips.unwrap_or(0) as f32 * TWIP;
        let margins = props.cell_margins;
        let border = props.borders.unwrap_or(0).max(0) as f32 / 8.0;

        let columns = table.rows.iter().map(|row| row.cells.len()).max().unwrap_or(1).max(1);
        let grid: Vec<f32> = table.grid.iter().map(|w| *w as f32 * TWIP).collect();
        let stated = |row: &oco_docx::TableRow| row.cells.iter().map(|c| points(c.props.width)).sum::<f32>();
        let table_width = if grid.iter().sum::<f32>() > 0.0 {
            grid.iter().sum()
        } else {
            table.rows.iter().map(stated).fold(0.0, f32::max).max(0.0)
        };
        let table_width = if table_width > 0.0 { table_width } else { width };
        let mut origin = points(props.indent)
            + match props.align.as_deref() {
                Some("center") => (width - table_width) / 2.0,
                Some("right" | "end") => width - table_width,
                _ => 0.0,
            };
        // Before Word 2013 a table was moved out so that its text, not its
        // edge, stood at the margin.
        if self.docx.settings.compatibility_mode < 15 {
            origin -= points(margins.left);
        }

        for (number, row) in table.rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(row.cells.len());
            let mut edges = vec![origin];
            let (mut x, mut column) = (origin, 0usize);
            for cell in &row.cells {
                let span = cell.props.grid_span as usize;
                let from_grid: f32 = grid.iter().skip(column).take(span).sum();
                let cell_width = if from_grid > 0.0 {
                    from_grid
                } else if cell.props.width.is_some() {
                    points(cell.props.width)
                } else {
                    table_width / columns as f32
                };
                let (left, right) = (
                    points(cell.props.margins.left.or(margins.left)),
                    points(cell.props.margins.right.or(margins.right)),
                );
                // The cell's margin is measured from the middle of its border,
                // and the text never stands on the border itself.
                let inset = (border / 2.0).max(border - left);
                let mut inner = Vec::new();
                self.blocks(&cell.blocks, section, (cell_width - left - right).max(1.0), &mut inner);

                let mut lines = Vec::new();
                let (mut previous_after, mut first) = (0.0f32, true);
                for unit in inner {
                    let mut gap = if first { unit.before } else { unit.before.max(previous_after) };
                    first = false;
                    previous_after = unit.after;
                    let own = match unit.row {
                        Some(nested) => vec![nested.into_line()],
                        None => unit.lines,
                    };
                    for line in own {
                        lines.push(CellLine { line, gap, x: x + left + inset });
                        gap = 0.0;
                    }
                }
                cells.push(RowCell { lines, tail: previous_after });
                x += cell_width;
                column += span;
                edges.push(x);
            }
            let height = points(row.props.height);
            units.push(Unit {
                lines: Vec::new(),
                before: 0.0,
                after: 0.0,
                left: 0.0,
                keep_next: false,
                keep_lines: true,
                widow_control: false,
                page_break_before: false,
                contextual: false,
                style: None,
                bottom_rule: None,
                ends_section: None,
                paragraph: None,
                row: Some(Row {
                    cells,
                    edges,
                    min_height: height,
                    exact_height: row.props.exact_height.then_some(height).filter(|h| *h > 0.0),
                    cant_split: row.props.cant_split,
                    padding: (points(margins.top), points(margins.bottom)),
                    border,
                    last: number + 1 == table.rows.len(),
                }),
            });
        }
        self.table_styles = outer;
    }

    fn blocks(&mut self, blocks: &[Block], section: &Section, width: f32, units: &mut Vec<Unit>) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => units.push(self.paragraph(paragraph, section, width)),
                Block::Table(table) => self.table(table, section, width, units),
            }
        }
    }
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
    };
    let mut units = Vec::new();
    // Line breaks depend on the text width, which this prototype takes from
    // the first section throughout.
    let first = setups[0];
    let width = first.text_width() as f32 * TWIP;
    builder.blocks(&docx.document.body, first, width, &mut units);

    let mut pages: Vec<Page> = Vec::new();
    let mut section_index = 0;
    let new_page = |section: &Section| Page {
        width: section.page_width as f32 * TWIP,
        height: section.page_height as f32 * TWIP,
        body: (section.margin_top as f32 * TWIP, (section.page_height - section.margin_bottom) as f32 * TWIP),
        ..Page::default()
    };
    pages.push(new_page(setups[0]));
    let mut y = setups[0].margin_top as f32 * TWIP;
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
        let top = section.margin_top as f32 * TWIP;
        let bottom = (section.page_height - section.margin_bottom) as f32 * TWIP;
        let left = (section.margin_left + section.gutter) as f32 * TWIP;
        let unit = &units[index];

        if unit.page_break_before && !page_is_empty {
            pages.push(new_page(section));
            y = top;
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
            chain += link.lines.iter().map(|l| l.height).sum::<f32>() + link.after.max(units[k + 1].before);
            k += 1;
        }
        let page_height = bottom - top;
        if !page_is_empty && y + gap(false, false) + chain > bottom + 0.01 && chain <= page_height {
            pages.push(new_page(section));
            y = top;
            page_is_empty = true;
            after_break = true;
        }
        y += gap(page_is_empty, after_break);
        if let Some(row) = &unit.row {
            // Where each cell has got to: the next of its lines to place.
            let mut cursors = vec![0usize; row.cells.len()];
            loop {
                let first_part = cursors.iter().all(|cursor| *cursor == 0);
                let content =
                    row.cells.iter().zip(&cursors).map(|(cell, cursor)| cell.rest(*cursor)).fold(0.0, f32::max);
                let height = row.part_height(content, first_part);
                // How many more lines of each cell the page has room for.
                let room = bottom - y - row.border - row.padding.0;
                let takes: Vec<usize> = row
                    .cells
                    .iter()
                    .zip(&cursors)
                    .map(|(cell, cursor)| {
                        let mut used = 0.0;
                        cell.lines[*cursor..]
                            .iter()
                            .take_while(|part| {
                                used += part.gap + part.line.height;
                                used <= room + 0.01
                            })
                            .count()
                    })
                    .collect();
                let fits = y + height <= bottom + 0.01;
                // A row breaks only where every cell that has something left
                // gets at least a line onto this page.
                let splits = !row.cant_split
                    && row.exact_height.is_none()
                    && takes.iter().any(|take| *take > 0)
                    && row
                        .cells
                        .iter()
                        .zip(&cursors)
                        .zip(&takes)
                        .all(|((cell, cursor), take)| *cursor >= cell.lines.len() || *take > 0);
                if !fits && !splits && !page_is_empty {
                    pages.push(new_page(section));
                    y = top;
                    page_is_empty = true;
                    after_break = true;
                    continue;
                }
                let whole = fits || !splits;
                let number_of_pages = pages.len();
                let page = pages.last_mut().expect("a page is always open");
                if started < number_of_pages {
                    started = number_of_pages;
                    page.start = None;
                }
                for ((cell, cursor), take) in row.cells.iter().zip(&mut cursors).zip(&takes) {
                    let end = if whole { cell.lines.len() } else { *cursor + *take };
                    let mut line_y = y + row.border + row.padding.0;
                    for part in &cell.lines[*cursor..end] {
                        line_y += part.gap;
                        for item in &part.line.items {
                            let mut item = item.clone();
                            item.translate(left + part.x, line_y);
                            page.items.push(item);
                        }
                        if !part.line.text.trim().is_empty() {
                            page.lines.push(PlacedLine {
                                baseline: line_y + part.line.baseline,
                                x0: left + part.x + part.line.x0,
                                x1: left + part.x + part.line.x1,
                                text: part.line.text.clone(),
                            });
                        }
                        line_y += part.line.height;
                    }
                    *cursor = end;
                }
                let part_bottom = if whole { y + height } else { bottom };
                if row.border > 0.0 {
                    for mut item in boxes(&row.edges, y, part_bottom, row.border) {
                        item.translate(left, 0.0);
                        page.items.push(item);
                    }
                }
                page_is_empty = false;
                if whole {
                    y = part_bottom;
                    break;
                }
                pages.push(new_page(section));
                y = top;
                page_is_empty = true;
                after_break = true;
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
                pages.push(new_page(section));
                y = top;
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
            if !line.text.trim().is_empty() {
                page.lines.push(PlacedLine {
                    baseline: y + line.baseline,
                    x0: left + line.x0,
                    x1: left + line.x1,
                    text: line.text.clone(),
                });
            }
            y += line.height;
            page_is_empty = false;
            if line.page_break_after {
                pages.push(new_page(section));
                y = top;
                page_is_empty = true;
                after_break = true;
            }
        }
        if let Some((x1, x2, color, width, space)) = unit.bottom_rule {
            let page = pages.last_mut().expect("a page is always open");
            let at = y + space + width / 2.0;
            page.items.push(Item::Rule {
                x1: left + x1,
                y1: at,
                x2: left + x2,
                y2: at,
                width,
                color,
                dash: [0.0, 0.0],
            });
        }
        previous_after = unit.after;
        previous_style = Some((unit.style.clone(), unit.contextual));

        if unit.ends_section.is_some() {
            section_index += 1;
            let next = setups[section_index.min(setups.len() - 1)];
            if next.start != SectionStart::Continuous {
                pages.push(new_page(next));
                y = next.margin_top as f32 * TWIP;
                page_is_empty = true;
                // The first paragraph of a section keeps its space before.
                after_break = false;
                previous_after = 0.0;
            }
        }
    }
    Layout { pages, missing_fonts: builder.missing_fonts }
}
