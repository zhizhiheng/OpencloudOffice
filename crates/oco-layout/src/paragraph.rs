//! Lays out one paragraph: characters to glyphs, glyphs to lines.
//!
//! The steps are (1) turn the runs into a flat list of *atoms*, one per
//! character, each with its font, width and line-box contribution; (2) adjust
//! widths where neighbours interact (punctuation squeezing, the gap between
//! Chinese and Western text); (3) break the atoms into lines, greedily;
//! (4) place each line's atoms horizontally and work out its height.

use oco_docx::{
    BreakKind, Color, Emphasis, FontRef, Highlight, LabelAlign, ParaProps, RunContent, RunProps, Suffix, TabKind,
    TabStop, Underline, VertAlign,
};

use crate::fonts::{Choice, FontDb, Model};
use crate::text::{self, Slot};
use crate::{Item, Rgb};

/// What a paragraph needs to know about its surroundings.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// Width between the page margins, in points.
    pub width: f32,
    /// Line pitch of the document grid when lines snap to it.
    pub grid_pitch: Option<f32>,
    pub default_tab: f32,
    pub compress_punctuation: bool,
    pub underline_trailing_spaces: bool,
    /// A hanging indent acts as a tab stop (Word's default).
    pub hanging_tab: bool,
    /// A justified line may compress its spaces to fit one more word.
    pub shrink_spaces: bool,
    /// A line may squeeze its full-width punctuation to fit one more character.
    pub squeeze_punctuation: bool,
    pub model: Model,
}

/// A list label to set at the start of the paragraph.
pub struct LabelSpec {
    pub text: String,
    pub props: RunProps,
    pub suffix: Suffix,
    pub align: LabelAlign,
}

pub struct Paragraph<'a> {
    pub props: &'a ParaProps,
    /// Resolved properties of each run, with its content.
    pub runs: Vec<(RunProps, &'a [RunContent])>,
    /// Resolved properties of the paragraph mark, which sizes an empty paragraph.
    pub mark: RunProps,
    pub label: Option<LabelSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct Line {
    /// Distance from this line's top to the next line's top.
    pub height: f32,
    /// Distance from the line's top to the baseline.
    pub baseline: f32,
    /// Height the line needs to be shown at all. Smaller than `height` when
    /// extra leading follows the text, which may hang off the page bottom.
    pub needed: f32,
    /// Drawing instructions, relative to the left margin and the line's top.
    pub items: Vec<Item>,
    pub text: String,
    /// Where the first and last visible glyphs sit, from the left margin.
    pub x0: f32,
    pub x1: f32,
    /// The line ends with an explicit page break.
    pub page_break_after: bool,
    /// How many pieces of the paragraph's [`crate::content`] come before this line.
    pub offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Laid {
    pub lines: Vec<Line>,
    pub space_before: f32,
    pub space_after: f32,
    /// Fonts the paragraph asks for that are neither installed nor have a
    /// designated replacement.
    pub missing_fonts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Char,
    Tab,
    LineBreak,
    PageBreak,
}

#[derive(Debug, Clone)]
struct Style {
    color: Rgb,
    underline: Option<(Underline, Rgb)>,
    strike: bool,
    double_strike: bool,
    background: Option<Rgb>,
    border: bool,
    /// The mark and whether it goes below the character.
    emphasis: Option<(Emphasis, bool)>,
    /// Baseline shift in points, positive upwards.
    rise: f32,
    /// The run's nominal size, which positions underlines and strike-throughs.
    size: f32,
}

#[derive(Debug, Clone)]
struct Atom {
    ch: char,
    kind: Kind,
    style: usize,
    choice: Choice,
    glyph: u16,
    /// Size the glyph is drawn at.
    size: f32,
    advance: f32,
    /// Where the glyph is drawn within its advance; negative for an opening
    /// bracket whose blank left half has been squeezed out.
    shift: f32,
    x_scale: f32,
    ascent: f32,
    descent: f32,
    label: bool,
}

const TWIP: f32 = 1.0 / 20.0;

fn rgb(color: Option<Color>) -> Rgb {
    match color {
        Some(Color::Rgb(r, g, b)) => [r, g, b],
        _ => [0, 0, 0],
    }
}

fn highlight_rgb(highlight: Highlight) -> Option<Rgb> {
    Some(match highlight {
        Highlight::None => return None,
        Highlight::Black => [0, 0, 0],
        Highlight::Blue => [0, 0, 255],
        Highlight::Cyan => [0, 255, 255],
        Highlight::Green => [0, 255, 0],
        Highlight::Magenta => [255, 0, 255],
        Highlight::Red => [255, 0, 0],
        Highlight::Yellow => [255, 255, 0],
        Highlight::White => [255, 255, 255],
        Highlight::DarkBlue => [0, 0, 128],
        Highlight::DarkCyan => [0, 128, 128],
        Highlight::DarkGreen => [0, 128, 0],
        Highlight::DarkMagenta => [128, 0, 128],
        Highlight::DarkRed => [128, 0, 0],
        Highlight::DarkYellow => [128, 128, 0],
        Highlight::DarkGray => [128, 128, 128],
        Highlight::LightGray => [192, 192, 192],
    })
}

fn family(font: &Option<FontRef>) -> Option<&str> {
    match font {
        Some(FontRef::Name(name)) => Some(name.as_str()),
        _ => None,
    }
}

struct Builder<'a> {
    fonts: &'a FontDb,
    context: &'a Context,
    styles: Vec<Style>,
    atoms: Vec<Atom>,
    /// The last letter seen was Chinese rather than Western.
    after_cjk: bool,
    missing_fonts: Vec<String>,
}

impl Builder<'_> {
    fn push_run(&mut self, props: &RunProps, content: &[RunContent], label: bool) {
        if props.vanish == Some(true) {
            return;
        }
        let base = props.size.map_or(10.0, |half| half as f32 / 2.0);
        let (factor, rise) = match props.vert_align {
            // Measured in Word: two thirds the size, raised a third of the full size.
            Some(VertAlign::Superscript) => (0.65, 0.33 * base),
            Some(VertAlign::Subscript) => (0.65, -0.16 * base),
            _ => (1.0, 0.0),
        };
        let color = rgb(props.color);
        let chinese = props.fonts.hint.as_deref() == Some("eastAsia");
        let style = Style {
            color,
            underline: props
                .underline
                .filter(|u| *u != Underline::None)
                .map(|u| (u, props.underline_color.filter(|c| *c != Color::Auto).map_or(color, |c| rgb(Some(c))))),
            strike: props.strike == Some(true),
            double_strike: props.double_strike == Some(true),
            background: props
                .highlight
                .and_then(highlight_rgb)
                .or(props.shade.filter(|c| *c != Color::Auto).map(|c| rgb(Some(c)))),
            border: props.border.as_ref().is_some_and(|b| b.is_visible()),
            // `dot` is the 着重号: below the character in Chinese text, above
            // it otherwise. `underDot` is always below.
            emphasis: props.emphasis.filter(|e| *e != Emphasis::None).map(|e| (e, e == Emphasis::UnderDot || chinese)),
            rise: rise + props.position.map_or(0.0, |half| half as f32 / 2.0),
            size: base,
        };
        self.styles.push(style);
        let style = self.styles.len() - 1;

        let bold = props.bold == Some(true);
        let italic = props.italic == Some(true);
        let spacing = props.char_spacing.map_or(0.0, |t| t as f32 * TWIP);
        let x_scale = props.scale.filter(|s| *s > 0).map_or(1.0, |s| s as f32 / 100.0);
        let push = |builder: &mut Self, ch: char, kind: Kind, font_override: Option<&str>| {
            // Capitals: `caps` shows everything in upper case; small caps
            // shows lower-case letters as smaller capitals.
            let upper = ch.to_uppercase().next().filter(|_| ch.to_uppercase().count() == 1).unwrap_or(ch);
            let (ch, small) = match () {
                _ if props.caps == Some(true) => (upper, false),
                _ if props.small_caps == Some(true) && ch.is_lowercase() => (upper, true),
                _ => (ch, false),
            };
            let probe = if kind == Kind::Char { ch } else { ' ' };
            if text::is_cjk(probe) {
                builder.after_cjk = true;
            } else if probe.is_alphanumeric() {
                builder.after_cjk = false;
            }
            // Word decides quotes, dashes and the like by the run's hint;
            // LibreOffice lets them follow the text before them.
            let east_asian = match builder.context.model {
                Model::Word => chinese,
                Model::LibreOffice => builder.after_cjk,
            };
            let slot = text::slot_for(probe, east_asian, props.complex == Some(true));
            let wanted = font_override.or(match slot {
                Slot::Ascii => family(&props.fonts.ascii),
                Slot::HighAnsi => family(&props.fonts.h_ansi).or(family(&props.fonts.ascii)),
                Slot::EastAsia => family(&props.fonts.east_asia),
                Slot::Complex => family(&props.fonts.cs),
            });
            // With no font named anywhere Word uses Times New Roman; for
            // Chinese text the system's default, which is 宋体.
            let wanted = wanted.or(Some(if slot == Slot::EastAsia { "宋体" } else { "Times New Roman" }));
            let Some(choice) = builder.fonts.choose(wanted, bold, italic, probe, builder.context.model) else { return };
            if !choice.known_family {
                if let Some(name) = wanted.filter(|name| !builder.missing_fonts.iter().any(|known| known == name)) {
                    builder.missing_fonts.push(name.to_owned());
                }
            }
            let font = builder.fonts.get(choice.font);
            let size = base * factor * if small { 0.8 } else { 1.0 };
            let glyph = font.glyph(probe).unwrap_or(0);
            let natural = if choice.half_width_latin && probe.is_ascii() { 0.5 } else { font.advance(glyph) };
            let advance = match kind {
                Kind::Char => natural * size * x_scale + spacing,
                _ => 0.0,
            };
            // A half-width override condenses or widens the glyph to fit its cell.
            let fit = if choice.half_width_latin && probe.is_ascii() && font.advance(glyph) > 0.0 {
                0.5 / font.advance(glyph)
            } else {
                1.0
            };
            builder.atoms.push(Atom {
                ch,
                kind,
                style,
                choice,
                glyph,
                size,
                advance,
                shift: 0.0,
                x_scale: x_scale * fit,
                // A raised or lowered run counts with its unshifted, full-size box.
                ascent: choice.line_box.ascent * base,
                descent: choice.line_box.descent * base,
                label,
            });
        };
        for piece in content {
            match piece {
                RunContent::Text(text) => text.chars().for_each(|ch| push(self, ch, Kind::Char, None)),
                RunContent::Tab => push(self, '\t', Kind::Tab, None),
                RunContent::Break(BreakKind::Line) => push(self, '\n', Kind::LineBreak, None),
                RunContent::Break(_) => push(self, '\n', Kind::PageBreak, None),
                RunContent::Symbol { font, character } => push(self, *character, Kind::Char, font.as_deref()),
                RunContent::PageBreakHint => {}
                // What Word last showed; a header or footer has the real
                // number put in before it is laid out.
                RunContent::PageNumber(shown) | RunContent::PageCount(shown) => {
                    shown.chars().for_each(|ch| push(self, ch, Kind::Char, None));
                }
            }
        }
    }

    /// Squeezes full-width punctuation that meets other punctuation, and
    /// opens a small gap where Chinese meets Western text or digits.
    fn adjust_neighbours(&mut self, auto_space: bool) {
        for i in 0..self.atoms.len().saturating_sub(1) {
            let (a, b) = (self.atoms[i].clone(), self.atoms[i + 1].clone());
            if a.kind != Kind::Char || b.kind != Kind::Char {
                continue;
            }
            let full = |atom: &Atom| atom.advance > 0.9 * atom.size;
            if self.context.compress_punctuation && self.context.model == Model::Word {
                let b_is_punctuation =
                    (text::is_closing_punctuation(b.ch) || text::is_opening_punctuation(b.ch)) && full(&b);
                if text::is_closing_punctuation(a.ch) && full(&a) && b_is_punctuation {
                    // The blank right half of a closing mark goes.
                    self.atoms[i].advance -= 0.5 * a.size;
                } else if text::is_opening_punctuation(a.ch) && text::is_opening_punctuation(b.ch) && full(&b) {
                    // The blank left half of the second opening mark goes.
                    self.atoms[i + 1].advance -= 0.5 * b.size;
                    self.atoms[i + 1].shift = -0.5 * b.size;
                }
            }
            if auto_space && !a.label && !b.label {
                let western = |ch: char| ch.is_ascii_alphanumeric();
                let ideograph = |ch: char| {
                    text::is_cjk(ch)
                        && !text::is_closing_punctuation(ch)
                        && !text::is_opening_punctuation(ch)
                        && ch != '\u{3000}'
                };
                if (ideograph(a.ch) && western(b.ch)) || (western(a.ch) && ideograph(b.ch)) {
                    self.atoms[i].advance += 0.25 * a.size.min(b.size);
                }
            }
        }
    }
}

/// How far past the measure, in em, a line may run when it has full-width
/// punctuation to squeeze, and the most one mark gives up. Measured in WPS
/// Office: a line takes one more character when that needs no more than
/// half an em, the excess comes equally out of each mark that has blank to
/// give, and no mark is seen narrower than 9/16 em.
const SQUEEZE_ROOM: f32 = 0.5;
const SQUEEZE_MOST: f32 = 0.4375;

/// The room a character needs as the last of its line: a full-width closing
/// mark shows only its inked left half there.
fn at_line_end(atom: &Atom) -> f32 {
    if text::is_closing_punctuation(atom.ch) && atom.advance > 0.9 * atom.size {
        atom.advance - 0.5 * atom.size
    } else {
        atom.advance
    }
}

/// Whether the atom at `index` of a line that starts at `start` is a
/// full-width punctuation mark with blank to give up: a closing mark before
/// ordinary text, or an opening mark after it. Marks that touch other marks
/// have been closed up already, and an opening mark that starts the line
/// stays whole.
fn squeezable(atoms: &[Atom], start: usize, index: usize) -> bool {
    let atom = &atoms[index];
    let full = |a: &Atom| a.kind == Kind::Char && a.advance > 0.9 * a.size;
    let plain = |a: Option<&Atom>| {
        a.is_some_and(|a| {
            a.kind == Kind::Char
                && !a.label
                && !text::is_closing_punctuation(a.ch)
                && !text::is_opening_punctuation(a.ch)
                && a.ch != ' '
                && a.ch != '\u{3000}'
        })
    };
    if !full(atom) || atom.label {
        return false;
    }
    if text::is_closing_punctuation(atom.ch) {
        plain(atoms.get(index + 1))
    } else if text::is_opening_punctuation(atom.ch) {
        index > start && plain(atoms.get(index - 1))
    } else {
        false
    }
}

/// Converts character-unit and twip indents to points. A character unit is
/// the width of one character at the paragraph's font size.
fn indents(props: &ParaProps, char_width: f32) -> (f32, f32, f32) {
    let ind = &props.indent;
    let pick = |chars: Option<i32>, twips: Option<i32>| match chars {
        Some(chars) => chars as f32 / 100.0 * char_width,
        None => twips.unwrap_or(0) as f32 * TWIP,
    };
    let left = pick(ind.left_chars, ind.left);
    let right = pick(ind.right_chars, ind.right);
    let first = if ind.hanging.is_some() || ind.hanging_chars.is_some() {
        -pick(ind.hanging_chars, ind.hanging)
    } else {
        pick(ind.first_line_chars, ind.first_line)
    };
    (left, right, first)
}

/// The tab stop a tab at `x` runs to: the next custom stop, the hanging
/// indent, or the next default stop. All positions are from the left margin.
fn next_stop(x: f32, stops: &[TabStop], hanging_at: Option<f32>, default: f32) -> (f32, TabKind, Option<String>) {
    let custom = stops
        .iter()
        .map(|stop| (stop.position as f32 * TWIP, stop.kind, stop.leader.clone()))
        .find(|(position, kind, _)| *position > x + 0.01 && *kind != TabKind::Bar);
    let hanging = hanging_at.filter(|position| *position > x + 0.01).map(|position| (position, TabKind::Left, None));
    match (custom, hanging) {
        (Some(c), Some(h)) => {
            if c.0 <= h.0 {
                c
            } else {
                h
            }
        }
        (Some(c), None) => c,
        (None, Some(h)) => h,
        (None, None) => {
            // Default stops form a grid that starts after the last custom stop.
            let after = stops.last().map_or(0.0, |s| s.position as f32 * TWIP).max(0.0);
            let from = x.max(after);
            (((from + 0.01) / default).floor() * default + default, TabKind::Left, None)
        }
    }
}

pub fn layout(paragraph: &Paragraph, fonts: &FontDb, context: &Context) -> Laid {
    let props = paragraph.props;
    let mut builder =
        Builder { fonts, context, styles: Vec::new(), atoms: Vec::new(), after_cjk: false, missing_fonts: Vec::new() };

    if let Some(label) = &paragraph.label {
        let mut text = label.text.clone();
        match label.suffix {
            Suffix::Tab => {}
            Suffix::Space => text.push(' '),
            Suffix::Nothing => {}
        }
        let content = [RunContent::Text(text)];
        builder.push_run(&label.props, &content, true);
        if label.suffix == Suffix::Tab {
            builder.push_run(&label.props, &[RunContent::Tab], true);
        }
    }
    for (run, content) in &paragraph.runs {
        builder.push_run(run, content, false);
    }
    let auto_space = props.auto_space_de != Some(false);
    builder.adjust_neighbours(auto_space);
    let Builder { styles, mut atoms, missing_fonts, .. } = builder;

    // An empty paragraph is as tall as one line of its paragraph mark.
    let mark_size = paragraph.mark.size.map_or(10.0, |half| half as f32 / 2.0);
    let mark_choice = {
        // The paragraph mark is a Western character unless it is marked as
        // East Asian text, so an empty paragraph is a line of the Western
        // font. (Measured in Word: with Calibri and an East Asian default
        // font at 11 pt it is 13.4 pt tall, Calibri's line; and the page
        // breaks Word records in documents full of empty paragraphs bear
        // it out.) Where it is East Asian, a font in that slot that is in
        // fact Western still gives its own line: asking it for a Chinese
        // character would bring in a taller fallback font.
        let mark = &paragraph.mark.fonts;
        let western_mark = context.model == Model::Word && mark.hint.as_deref() != Some("eastAsia");
        let named = match family(&mark.ascii).filter(|_| western_mark) {
            Some(ascii) => Some(ascii),
            None => family(&mark.east_asia).or(family(&mark.ascii)),
        };
        let western = named
            .and_then(|name| fonts.choose(Some(name), false, false, 'x', context.model))
            .filter(|choice| choice.known_family && !fonts.get(choice.font).east_asian);
        western.or_else(|| {
            let sample = if named.is_some() { '中' } else { 'x' };
            fonts.choose(named.or(Some("宋体")), false, false, sample, context.model)
        })
    };
    let mark_box = mark_choice
        .map_or((mark_size, mark_size * 0.3), |c| (c.line_box.ascent * mark_size, c.line_box.descent * mark_size));
    let mark_text_box = mark_choice
        .map_or((mark_size, mark_size * 0.3), |c| (c.text_box.ascent * mark_size, c.text_box.descent * mark_size));

    // Character units follow the paragraph's own font size.
    let char_width =
        atoms.iter().find(|a| !a.label && a.kind == Kind::Char).map_or(mark_size, |a| styles[a.style].size);
    let (left, right, first) = indents(props, char_width);
    let limit = context.width - right;
    let hanging_at = (first < 0.0 && context.hanging_tab).then_some(left);
    let kinsoku = props.kinsoku != Some(false);
    let hang_punctuation = props.overflow_punct != Some(false);
    let align = props.justification.as_deref().unwrap_or("left");

    // ------------------------------------------------------------ line breaking
    struct Break {
        start: usize,
        end: usize,
        /// Ended by an explicit break or the end of the paragraph, so not justified.
        forced: bool,
        page_break: bool,
        /// The line is over-full and its spaces are to be compressed.
        shrink: bool,
        /// The line is over-full and its punctuation is to be squeezed.
        squeeze: bool,
    }
    let mut breaks: Vec<Break> = Vec::new();
    let mut start = 0;
    let mut is_first = true;
    while start < atoms.len() {
        let origin = if is_first { left + first } else { left };
        // A right-aligned label ends at the origin, so it starts before it;
        // the tab after it must be measured from where the label really ends.
        let mut x = origin;
        if is_first {
            if let Some(label) = &paragraph.label {
                let width: f32 = atoms.iter().take_while(|a| a.label && a.kind == Kind::Char).map(|a| a.advance).sum();
                match label.align {
                    LabelAlign::Right => x -= width,
                    LabelAlign::Center => x -= width / 2.0,
                    LabelAlign::Left => {}
                }
            }
        }
        let mut i = start;
        let mut last_opportunity: Option<usize> = None;
        let mut forced = false;
        let mut page_break = false;
        let mut shrink = false;
        let mut squeeze = false;
        let mut end = atoms.len();
        while i < atoms.len() {
            // Copied out because a tab's width is written back below.
            let atom = Atom { ..atoms[i].clone() };
            match atom.kind {
                Kind::LineBreak | Kind::PageBreak => {
                    page_break = atom.kind == Kind::PageBreak;
                    forced = true;
                    end = i + 1;
                    break;
                }
                Kind::Tab => {
                    let (stop, kind, _) = next_stop(x, &props.tabs, hanging_at, context.default_tab);
                    // Text after a right or centre stop is set against the stop.
                    let following: f32 =
                        atoms[i + 1..].iter().take_while(|a| a.kind == Kind::Char).map(|a| a.advance).sum();
                    let target = match kind {
                        TabKind::Right => (stop - following).max(x),
                        TabKind::Center => (stop - following / 2.0).max(x),
                        _ => stop,
                    };
                    if target > limit + 0.01 && i > start {
                        end = i;
                        break;
                    }
                    atoms[i].advance = target - x;
                    x = target;
                }
                Kind::Char => {
                    // A full-width closing mark at the end of a line needs
                    // room only for its inked left half.
                    let closes_line = text::is_closing_punctuation(atom.ch)
                        && atom.advance > 0.9 * atom.size
                        && x + atom.advance / 2.0 <= limit + 0.01
                        && x + atom.advance > limit + 0.01
                        && !atoms.get(i + 1).is_some_and(|n| n.kind == Kind::Char && text::cannot_start_line(n.ch));
                    if closes_line && i > start {
                        end = i + 1;
                        break;
                    }
                    let overflows = x + atom.advance > limit + 0.01;
                    if overflows && i > start {
                        if atom.ch == ' ' || atom.ch == '\u{3000}' {
                            // Spaces at the end of a line hang past the margin.
                        } else if hang_punctuation
                            && text::may_hang(atom.ch)
                            && x + atom.advance - limit <= atom.size
                            && !atoms[i - 1].label
                        {
                            // A closing mark may hang, then the line ends,
                            // unless yet another mark that cannot start a line follows.
                            let next_also_stuck =
                                atoms.get(i + 1).is_some_and(|n| n.kind == Kind::Char && text::cannot_start_line(n.ch));
                            if !next_also_stuck {
                                end = i + 1;
                                break;
                            }
                            end = last_opportunity.map_or(i, |o| o + 1);
                            break;
                        } else if !squeeze
                            && context.squeeze_punctuation
                            && x + at_line_end(&atom) - limit <= SQUEEZE_ROOM * atom.size
                            && (start..i).any(|k| squeezable(&atoms, start, k))
                            && atoms.get(i + 1).is_none_or(|next| {
                                next.kind != Kind::Char || text::can_break_between(atom.ch, next.ch, kinsoku)
                            })
                        {
                            // Full-width punctuation gives up part of its
                            // blank half so that the line takes this
                            // character too; the line then ends here.
                            squeeze = true;
                            end = i + 1;
                            break;
                        } else {
                            end = last_opportunity.map_or(i, |o| o + 1);
                            // Word (2013 and later) will squeeze the spaces of a
                            // justified line by up to about a quarter to take in
                            // the next word, when that distorts the line less
                            // than stretching it without the word would.
                            if context.shrink_spaces && align == "both" && end > start {
                                let word_end = (i..atoms.len())
                                    .find(|k| match atoms.get(k + 1) {
                                        Some(next) => {
                                            atoms[*k].kind != Kind::Char
                                                || next.kind != Kind::Char
                                                || text::can_break_between(atoms[*k].ch, next.ch, kinsoku)
                                        }
                                        None => true,
                                    })
                                    .unwrap_or(i);
                                let is_space = |a: &Atom| a.kind == Kind::Char && a.ch == ' ';
                                let mut word: f32 = atoms[i..=word_end]
                                    .iter()
                                    .filter(|a| !is_space(a) && a.kind == Kind::Char)
                                    .map(|a| a.advance)
                                    .sum();
                                // A final comma or full stop can hang, so it needs no room.
                                if let Some(last) = atoms[i..=word_end].iter().rev().find(|a| !is_space(a)) {
                                    if hang_punctuation && text::may_hang(last.ch) {
                                        word -= last.advance;
                                    }
                                }
                                let all_chars = atoms[i..=word_end].iter().all(|a| a.kind == Kind::Char);
                                let from = atoms[start..i]
                                    .iter()
                                    .rposition(|a| a.kind == Kind::Tab || a.label)
                                    .map_or(start, |p| start + p + 1);
                                let spaces: f32 =
                                    atoms[from..i].iter().filter(|a| is_space(a)).map(|a| a.advance).sum();
                                let excess = x + word - limit;
                                let trailing: f32 =
                                    atoms[start..end].iter().rev().take_while(|a| is_space(a)).map(|a| a.advance).sum();
                                let width_without: f32 =
                                    atoms[start..end].iter().map(|a| a.advance).sum::<f32>() - trailing;
                                let stretch = limit - origin - width_without;
                                if all_chars && excess > 0.0 && excess <= 0.24 * spaces && excess < stretch / 2.0 {
                                    end = word_end + 1;
                                    shrink = true;
                                }
                            }
                            break;
                        }
                    }
                    x += atom.advance;
                }
            }
            if let Some(next) = atoms.get(i + 1) {
                let allowed = match (atom.kind, next.kind) {
                    (Kind::Tab, _) => true,
                    (Kind::Char, Kind::Char) => !atom.label && text::can_break_between(atom.ch, next.ch, kinsoku),
                    _ => false,
                };
                if allowed {
                    last_opportunity = Some(i);
                }
            }
            i += 1;
        }
        if end == atoms.len() {
            forced = true;
        }
        // Never loop without progress, whatever the widths were.
        let end = end.max(start + 1);
        breaks.push(Break { start, end, forced, page_break, shrink, squeeze });
        start = end;
        is_first = false;
    }
    if atoms.is_empty() {
        breaks.push(Break { start: 0, end: 0, forced: true, page_break: false, shrink: false, squeeze: false });
    }

    // --------------------------------------------------------------- placement
    let line_value = props.spacing.line.map(|l| l as f32);
    let rule = props.spacing.line_rule.as_deref().unwrap_or("auto");
    let snaps = props.snap_to_grid != Some(false) && rule != "exact";
    let grid = context.grid_pitch.filter(|_| snaps);

    let mut lines = Vec::with_capacity(breaks.len());
    for (number, br) in breaks.iter().enumerate() {
        let range = &atoms[br.start..br.end];
        let origin = if number == 0 { left + first } else { left };
        // Trailing spaces and the break itself take no part in alignment.
        // ... unless they are underlined blanks to be filled in ("编号：____"),
        // which are part of the text when the document says to draw them.
        let drawn_blank = |a: &Atom| context.underline_trailing_spaces && styles[a.style].underline.is_some();
        let visible = range
            .iter()
            .rposition(|a| a.kind == Kind::Char && ((a.ch != ' ' && a.ch != '\u{3000}') || drawn_blank(a)))
            .map_or(0, |p| p + 1);
        let natural: f32 = range[..visible].iter().map(|a| a.advance).sum();
        // A closing mark at the very end shows only its inked left half.
        let hanging_trim = range[..visible]
            .last()
            .filter(|a| text::is_closing_punctuation(a.ch) && a.advance > 0.9 * a.size)
            .map_or(0.0, |a| 0.5 * a.size);
        let slack = limit - origin - (natural - hanging_trim);

        let last_tab = range[..visible].iter().rposition(|a| a.kind == Kind::Tab || a.label);
        let stretch_from = last_tab.map_or(0, |p| p + 1);
        let stretchable = |index: usize| -> bool {
            // Gaps that take up slack when a line is justified: after spaces,
            // and between any two characters one of which is Chinese.
            if index < stretch_from || index + 1 >= visible {
                return false;
            }
            let (a, b) = (&range[index], &range[index + 1]);
            a.ch == ' ' || text::is_cjk(a.ch) || text::is_cjk(b.ch)
        };
        let justify = match align {
            "both" => !br.forced,
            "distribute" => true,
            _ => false,
        };
        let gaps = (0..visible)
            .filter(|i| if align == "distribute" { *i >= stretch_from && *i + 1 < visible } else { stretchable(*i) })
            .count();
        let per_gap = if justify && gaps > 0 && slack > 0.0 { slack / gaps as f32 } else { 0.0 };
        // An over-full line takes the excess out of its spaces alone.
        let shrinkable =
            (stretch_from..visible).filter(|i| range[*i].ch == ' ' && range[*i].kind == Kind::Char).count();
        let per_space = if br.shrink && shrinkable > 0 && slack < 0.0 { slack / shrinkable as f32 } else { 0.0 };
        // ... or, in Chinese text, out of its punctuation, equally from each mark.
        let marks: Vec<usize> = (0..visible).filter(|k| squeezable(range, 0, *k)).collect();
        let per_mark = if br.squeeze && !marks.is_empty() && slack < 0.0 {
            (-slack / marks.len() as f32).min(SQUEEZE_MOST * range[marks[0]].size)
        } else {
            0.0
        };
        let offset = match align {
            "center" => (slack / 2.0).max(0.0),
            "right" | "end" => slack.max(0.0),
            _ => 0.0,
        };

        // Vertical: the tallest ascent and the deepest descent on the line.
        let (mut ascent, mut descent) = (0.0f32, 0.0f32);
        for atom in range.iter().filter(|a| a.kind == Kind::Char || range.len() == 1) {
            ascent = ascent.max(atom.ascent);
            descent = descent.max(atom.descent);
        }
        if ascent == 0.0 {
            (ascent, descent) = mark_box;
        }
        // Word works in 1/600 inch; a single line is a whole number of them.
        let single = ((ascent + descent) * 600.0 / 72.0).round() * 72.0 / 600.0;
        let (height, baseline, needed) = match (rule, line_value, grid) {
            ("exact", Some(line), _) => {
                // The text sits in a line of fixed height by the proportions
                // of the fonts themselves, not of the enlarged box that
                // East Asian fonts get when a line takes its height from
                // them (measured in WPS Office: 12 pt Chinese text in a
                // 20 pt line; for other fonts the two are the same, as
                // measured in Word).
                let height = line * TWIP;
                let (mut above, mut below) = (0.0f32, 0.0f32);
                for atom in range.iter().filter(|a| a.kind == Kind::Char || range.len() == 1) {
                    let (line_box, text_box) = (atom.choice.line_box, atom.choice.text_box);
                    above = above.max(atom.ascent * text_box.ascent / line_box.ascent);
                    below = below.max(atom.descent * text_box.descent / line_box.descent);
                }
                if above == 0.0 {
                    (above, below) = mark_text_box;
                }
                (height, height * above / (above + below), height)
            }
            (_, _, Some(pitch)) => {
                // On a grid a line takes as many grid rows as its text needs,
                // and the text sits in the middle of them.
                let rows = (single / pitch - 0.001).ceil().max(1.0);
                let base = rows * pitch;
                let centred = (base - single) / 2.0 + ascent;
                match (rule, line_value) {
                    ("atLeast", Some(line)) => {
                        let height = base.max(((line * TWIP) / pitch - 0.001).ceil() * pitch);
                        (height, height - base + centred, height)
                    }
                    // A multiple counts grid rows, not the rows the text
                    // takes: one and a half lines is a row and a half,
                    // or the text's own rows where that is more
                    // (measured in WPS Office: 12 pt text that needs two
                    // 15.6 pt rows is 31.2 pt apart at 1.5 lines, not
                    // 46.8). The extra leading goes below, as off the grid.
                    // LibreOffice multiplies the text's rows.
                    (_, Some(line)) if line >= 240.0 => {
                        let height = match context.model {
                            Model::Word => base.max(pitch * line / 240.0),
                            Model::LibreOffice => base * line / 240.0,
                        };
                        (height, centred, base)
                    }
                    (_, Some(line)) => (base * line / 240.0, centred.min(base * line / 240.0), base * line / 240.0),
                    _ => (base, centred, base),
                }
            }
            ("atLeast", Some(line), None) => {
                // Any shortfall goes above the text.
                let height = single.max(line * TWIP);
                (height, height - descent, height)
            }
            (_, Some(line), None) => {
                // A multiple of single spacing. Word leaves the text where
                // single spacing has it and puts all the extra below
                // (measured: after a single-spaced line, the first line of a
                // paragraph at 1.08, 1.15, 1.5 or 2 lines comes one single
                // pitch later). LibreOffice raises the ascent in proportion.
                let height = single * line / 240.0;
                let baseline = match context.model {
                    Model::Word if height >= single => ascent,
                    _ => ascent.min(height - descent).max(ascent * height / single),
                };
                (height, baseline, single.min(height))
            }
            (_, None, None) => (single, ascent, single),
        };

        let mut line = Line { height, baseline, needed, page_break_after: br.page_break, ..Line::default() };
        line.offset = atoms[..br.start].iter().filter(|a| !a.label).count();
        let mut x = origin + offset;
        // A right-aligned or centred label is set against the line's origin.
        if number == 0 {
            if let Some(label) = &paragraph.label {
                let width: f32 = range.iter().take_while(|a| a.label && a.kind == Kind::Char).map(|a| a.advance).sum();
                match label.align {
                    LabelAlign::Right => x -= width,
                    LabelAlign::Center => x -= width / 2.0,
                    LabelAlign::Left => {}
                }
            }
        }
        let mut first_ink: Option<f32> = None;
        let mut last_ink = x;
        // Decorations are drawn per run of atoms sharing a style.
        let mut span_start = x;
        let mut span_style: Option<usize> = None;
        let mut decorations: Vec<(usize, f32, f32)> = Vec::new();
        for (index, atom) in range.iter().enumerate() {
            if span_style != Some(atom.style) {
                if let Some(style) = span_style {
                    decorations.push((style, span_start, x));
                }
                span_style = Some(atom.style);
                span_start = x;
            }
            let style = &styles[atom.style];
            let in_text = index < visible;
            // A squeezed closing mark loses blank on its right, an opening
            // mark on its left.
            let squeezed = if per_mark > 0.0 && marks.contains(&index) { per_mark } else { 0.0 };
            if squeezed > 0.0 && text::is_opening_punctuation(atom.ch) {
                x -= squeezed;
            }
            match atom.kind {
                Kind::Char => {
                    if atom.ch != ' ' && atom.ch != '\u{3000}' {
                        line.items.push(Item::Glyph {
                            font: atom.choice.font,
                            glyph: atom.glyph,
                            x: x + atom.shift,
                            y: baseline - style.rise,
                            size: atom.size,
                            x_scale: atom.x_scale,
                            color: style.color,
                            fake_bold: atom.choice.fake_bold,
                            fake_italic: atom.choice.fake_italic,
                        });
                        first_ink.get_or_insert(x);
                        last_ink = x + atom.advance;
                        if let Some((mark, below)) = style.emphasis {
                            let radius = 0.06 * atom.size;
                            let cy = if below { baseline + 0.22 * style.size } else { baseline - 0.95 * style.size };
                            line.items.push(Item::Circle {
                                cx: x + atom.shift + (atom.advance - atom.shift) / 2.0,
                                cy,
                                r: if mark == Emphasis::Circle { radius * 1.3 } else { radius },
                                color: style.color,
                                filled: mark != Emphasis::Circle,
                            });
                        }
                    }
                    line.text.push(atom.ch);
                    let stretches = if align == "distribute" {
                        index >= stretch_from && index + 1 < visible
                    } else {
                        stretchable(index)
                    };
                    x += atom.advance + if in_text && justify && stretches { per_gap } else { 0.0 };
                    if squeezed > 0.0 && text::is_closing_punctuation(atom.ch) {
                        x -= squeezed;
                    }
                    if in_text && index >= stretch_from && atom.ch == ' ' {
                        x += per_space;
                    }
                }
                Kind::Tab => {
                    // Find the leader of the stop this tab ran to.
                    let leader = props.tabs.iter().find(|stop| {
                        let position = stop.position as f32 * TWIP;
                        position >= x + atom.advance - 0.5
                            && position
                                <= x + atom.advance
                                    + range[index + 1..]
                                        .iter()
                                        .take_while(|a| a.kind == Kind::Char)
                                        .map(|a| a.advance)
                                        .sum::<f32>()
                                    + 0.5
                    });
                    if let Some(leader) = leader.and_then(|stop| stop.leader.as_deref()) {
                        let y = baseline - if leader == "underscore" { -1.0 } else { 0.0 };
                        let dash = match leader {
                            "dot" | "middleDot" => [0.6, 2.4],
                            "hyphen" => [2.4, 2.4],
                            _ => [0.0, 0.0],
                        };
                        if atom.advance > 4.0 {
                            line.items.push(Item::Rule {
                                x1: x + 1.5,
                                y1: y,
                                x2: x + atom.advance - 1.5,
                                y2: y,
                                width: 0.6,
                                color: style.color,
                                dash,
                            });
                        }
                    }
                    line.text.push('\t');
                    x += atom.advance;
                }
                Kind::LineBreak | Kind::PageBreak => {}
            }
        }
        if let Some(style) = span_style {
            decorations.push((style, span_start, x));
        }
        // Trailing spaces are underlined only when the document asks for it.
        let text_end = if context.underline_trailing_spaces { x } else { last_ink.max(origin) };
        let mut behind = Vec::new();
        for (style_index, from, to) in decorations {
            let style = &styles[style_index];
            let to = to.min(text_end.max(from));
            if to - from < 0.01 {
                continue;
            }
            let size = style.size;
            if let Some(color) = style.background {
                behind.push(Item::Rect {
                    x: from,
                    y: baseline - 0.88 * size,
                    width: to - from,
                    height: 1.1 * size,
                    color,
                });
            }
            if style.border {
                let (top, bottom) = (baseline - 0.9 * size, baseline + 0.22 * size);
                for (x1, y1, x2, y2) in
                    [(from, top, to, top), (from, bottom, to, bottom), (from, top, from, bottom), (to, top, to, bottom)]
                {
                    line.items.push(Item::Rule { x1, y1, x2, y2, width: 0.5, color: style.color, dash: [0.0, 0.0] });
                }
            }
            if let Some((kind, color)) = style.underline {
                let y = baseline - style.rise + 0.12 * size;
                let thin = (0.05 * size).max(0.5);
                let rule = |y: f32, width: f32, dash: [f32; 2]| Item::Rule {
                    x1: from,
                    y1: y,
                    x2: to,
                    y2: y,
                    width,
                    color,
                    dash,
                };
                match kind {
                    Underline::Double => {
                        line.items.push(rule(y - thin * 0.4, thin * 0.7, [0.0, 0.0]));
                        line.items.push(rule(y + thin * 1.4, thin * 0.7, [0.0, 0.0]));
                    }
                    Underline::Thick => line.items.push(rule(y + thin / 2.0, thin * 2.0, [0.0, 0.0])),
                    Underline::Dotted | Underline::DottedHeavy => line.items.push(rule(y, thin, [thin, thin * 1.5])),
                    Underline::Dash | Underline::DashedHeavy | Underline::DashLong | Underline::DashLongHeavy => {
                        line.items.push(rule(y, thin, [thin * 4.0, thin * 2.0]));
                    }
                    Underline::DotDash
                    | Underline::DashDotHeavy
                    | Underline::DotDotDash
                    | Underline::DashDotDotHeavy => {
                        line.items.push(rule(y, thin, [thin * 4.0, thin * 1.5]));
                    }
                    Underline::Wave | Underline::WavyHeavy | Underline::WavyDouble => {
                        line.items.push(Item::Wave {
                            x1: from,
                            x2: to,
                            y: y + thin,
                            amplitude: thin * 1.2,
                            width: thin * 0.8,
                            color,
                        });
                    }
                    // `words` skips the spaces; drawn through for now.
                    _ => line.items.push(rule(y, thin, [0.0, 0.0])),
                }
            }
            if style.strike || style.double_strike {
                let y = baseline - style.rise - 0.28 * size;
                let thin = (0.05 * size).max(0.5);
                let offsets: &[f32] = if style.double_strike { &[-thin, thin] } else { &[0.0] };
                for offset in offsets {
                    line.items.push(Item::Rule {
                        x1: from,
                        y1: y + offset,
                        x2: to,
                        y2: y + offset,
                        width: thin,
                        color: style.color,
                        dash: [0.0, 0.0],
                    });
                }
            }
        }
        // Backgrounds go under the glyphs.
        behind.append(&mut line.items);
        line.items = behind;
        line.x0 = first_ink.unwrap_or(origin + offset);
        line.x1 = last_ink;
        lines.push(line);
    }

    // Spacing in "lines" is counted in grid rows when there is a grid.
    let line_unit = context.grid_pitch.unwrap_or(12.0);
    let space = |lines: Option<i32>, twips: Option<i32>, auto: Option<bool>| match (auto, lines) {
        (Some(true), _) => 14.0,
        (_, Some(lines)) => lines as f32 / 100.0 * line_unit,
        _ => twips.unwrap_or(0) as f32 * TWIP,
    };
    Laid {
        missing_fonts,
        lines,
        space_before: space(props.spacing.before_lines, props.spacing.before, props.spacing.before_auto),
        space_after: space(props.spacing.after_lines, props.spacing.after, props.spacing.after_auto),
    }
}
