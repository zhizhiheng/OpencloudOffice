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
        let props: ParaProps = self.resolver.paragraph(&source.props);
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
            .map(|run| (self.resolver.run(style, &run.props), run.content.as_slice()))
            .collect();
        let input =
            paragraph::Paragraph { props: &props, runs, mark: self.resolver.run(style, &source.mark_props), label };
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
        }
    }

    /// A table as a plain grid: equal columns, each row as tall as its
    /// tallest cell, every cell boxed. A placeholder until table properties
    /// are modelled; it keeps the content readable and in place.
    fn table(&mut self, table: &oco_docx::Table, section: &Section, width: f32, units: &mut Vec<Unit>) {
        const PADDING: f32 = 5.4;
        let columns = table.rows.iter().map(|row| row.cells.len()).max().unwrap_or(1).max(1);
        let column = width / columns as f32;
        for row in &table.rows {
            let mut items = Vec::new();
            let mut text = Vec::new();
            let mut height = 0.0f32;
            for (index, cell) in row.cells.iter().enumerate() {
                let mut inner = Vec::new();
                self.blocks(&cell.blocks, section, column - 2.0 * PADDING, &mut inner);
                let mut y = 0.0;
                let mut previous_after = 0.0f32;
                for unit in inner {
                    y += unit.before.max(previous_after);
                    previous_after = unit.after;
                    for mut line in unit.lines {
                        let dx = index as f32 * column + PADDING + unit.left;
                        for item in &mut line.items {
                            item.translate(dx, y);
                        }
                        text.push((line.text, dx + line.x0, dx + line.x1, y + line.baseline));
                        items.append(&mut line.items);
                        y += line.height;
                    }
                }
                height = height.max(y);
            }
            let black = [0, 0, 0];
            let rule = |x1, y1, x2, y2| Item::Rule { x1, y1, x2, y2, width: 0.5, color: black, dash: [0.0, 0.0] };
            for edge in 0..=row.cells.len() {
                let x = edge as f32 * column;
                items.push(rule(x, 0.0, x, height));
            }
            let right = row.cells.len() as f32 * column;
            items.push(rule(0.0, 0.0, right, 0.0));
            items.push(rule(0.0, height, right, height));
            // The whole row travels as one unbreakable line.
            let mut line =
                paragraph::Line { height, baseline: 0.0, needed: height, items, ..paragraph::Line::default() };
            line.text = text.iter().map(|(t, ..)| t.as_str()).collect::<Vec<_>>().join(" | ");
            line.x0 = text.iter().map(|(_, x0, ..)| *x0).fold(f32::MAX, f32::min).min(right);
            line.x1 = right;
            line.baseline = text.first().map_or(0.0, |(.., baseline)| *baseline);
            units.push(Unit {
                lines: vec![line],
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
            });
        }
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
