//! Run and paragraph formatting properties (`w:rPr`, `w:pPr`).
//!
//! Every field is optional: `None` means "not specified at this level", which
//! is what lets one level of the style hierarchy be layered over another.

use roxmltree::Node;
use serde::Serialize;

use crate::coverage::Coverage;
use crate::values::{Color, Highlight, Underline, VertAlign};
use crate::xml::{elements, half_points, int, on_off, on_off_attr, twips, val, w_attr, w_name};

/// Copies every field that `$over` specifies onto `$base`.
macro_rules! overlay {
    ($base:expr, $over:expr; $($field:ident),+ $(,)?) => {
        $( if $over.$field.is_some() { $base.$field = $over.$field.clone(); } )+
    };
}

/// A font named directly, or by reference to a slot of the document theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRef {
    Name(String),
    /// A theme slot such as `minorEastAsia`; see [`crate::theme::ThemeFonts::resolve`].
    Theme(String),
}

/// `w:rFonts`: one font per script category. Word picks the slot per character.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Fonts {
    /// Basic Latin (U+0000..U+007F).
    pub ascii: Option<FontRef>,
    /// Other "high ANSI" characters.
    pub h_ansi: Option<FontRef>,
    /// East Asian characters, including Chinese.
    pub east_asia: Option<FontRef>,
    /// Complex scripts (Arabic, Hebrew, ...).
    pub cs: Option<FontRef>,
    /// Which slot ambiguous characters should use.
    pub hint: Option<String>,
}

impl Fonts {
    pub fn apply(&mut self, over: &Fonts) {
        overlay!(self, over; ascii, h_ansi, east_asia, cs, hint);
    }

    fn parse(node: Node) -> Self {
        // Within one element a theme reference wins over an explicit name.
        let slot = |name: &str, theme: &str| {
            w_attr(node, theme)
                .map(|t| FontRef::Theme(t.to_owned()))
                .or_else(|| w_attr(node, name).map(|n| FontRef::Name(n.to_owned())))
        };
        Fonts {
            ascii: slot("ascii", "asciiTheme"),
            h_ansi: slot("hAnsi", "hAnsiTheme"),
            east_asia: slot("eastAsia", "eastAsiaTheme"),
            cs: slot("cs", "cstheme"),
            hint: w_attr(node, "hint").map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunProps {
    /// Character style (`w:rStyle`).
    pub style_id: Option<String>,
    // Toggle properties: see `Resolver::run` for how these combine across styles.
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub strike: Option<bool>,
    /// Hidden text.
    pub vanish: Option<bool>,
    pub underline: Option<Underline>,
    /// Font size in half-points (`w:sz`).
    pub size: Option<u32>,
    pub color: Option<Color>,
    pub highlight: Option<Highlight>,
    pub vert_align: Option<VertAlign>,
    pub fonts: Fonts,
}

impl RunProps {
    /// Layers `over` on top of `self`: whatever `over` specifies replaces the
    /// current value. Toggle properties need extra care across style levels,
    /// which the resolver handles.
    pub fn apply(&mut self, over: &RunProps) {
        overlay!(self, over;
            style_id, bold, italic, caps, small_caps, strike, vanish,
            underline, size, color, highlight, vert_align);
        self.fonts.apply(&over.fonts);
    }

    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut p = RunProps::default();
        p.read(node, coverage);
        p
    }

    /// Reads a `w:rPr` on top of what is already here. A run with two
    /// `w:rPr` elements is invalid, and Word merges them rather than keeping
    /// only one.
    pub(crate) fn read(&mut self, node: Node, coverage: &mut Coverage) {
        let p = self;
        for child in elements(node) {
            match w_name(child) {
                Some("rStyle") => p.style_id = val(child).map(str::to_owned),
                Some("b") => p.bold = Some(on_off(child)),
                Some("i") => p.italic = Some(on_off(child)),
                Some("caps") => p.caps = Some(on_off(child)),
                Some("smallCaps") => p.small_caps = Some(on_off(child)),
                Some("strike") => p.strike = Some(on_off(child)),
                Some("vanish") => p.vanish = Some(on_off(child)),
                // For these four an invalid value means "not specified here",
                // so it must not overwrite a value read earlier.
                Some("u") => p.underline = Underline::parse(val(child)).or(p.underline),
                Some("color") => p.color = val(child).and_then(Color::parse).or(p.color),
                Some("highlight") => p.highlight = val(child).and_then(Highlight::from_exact).or(p.highlight),
                Some("vertAlign") => p.vert_align = val(child).and_then(VertAlign::from_exact).or(p.vert_align),
                Some("sz") => p.size = half_points(val(child)),
                Some("rFonts") => p.fonts.apply(&Fonts::parse(child)),
                _ => coverage.miss(Some("w:rPr"), child),
            }
        }
    }
}

/// `w:ind`. Lengths are in twips (1/20 pt); the `*_chars` variants are in
/// hundredths of a character width and take precedence when present. They are
/// how Chinese documents express "indent the first line by two characters".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Indent {
    pub left: Option<i32>,
    pub right: Option<i32>,
    pub first_line: Option<i32>,
    pub hanging: Option<i32>,
    pub left_chars: Option<i32>,
    pub right_chars: Option<i32>,
    pub first_line_chars: Option<i32>,
    pub hanging_chars: Option<i32>,
}

impl Indent {
    pub fn apply(&mut self, over: &Indent) {
        overlay!(self, over; left, right, left_chars, right_chars);
        // First-line and hanging indents are mutually exclusive: specifying
        // either one at a higher level replaces both.
        if over.first_line.is_some() || over.hanging.is_some() {
            self.first_line = over.first_line;
            self.hanging = over.hanging;
        }
        if over.first_line_chars.is_some() || over.hanging_chars.is_some() {
            self.first_line_chars = over.first_line_chars;
            self.hanging_chars = over.hanging_chars;
        }
    }

    fn parse(node: Node) -> Self {
        let get = |name: &str| twips(w_attr(node, name));
        let chars = |name: &str| int(w_attr(node, name));
        Indent {
            // `start`/`end` are the bidi-neutral spellings of `left`/`right`.
            left: get("left").or_else(|| get("start")),
            right: get("right").or_else(|| get("end")),
            first_line: get("firstLine"),
            hanging: get("hanging"),
            left_chars: chars("leftChars").or_else(|| chars("startChars")),
            right_chars: chars("rightChars").or_else(|| chars("endChars")),
            first_line_chars: chars("firstLineChars"),
            hanging_chars: chars("hangingChars"),
        }
    }
}

/// `w:spacing` on a paragraph. `before`/`after` are in twips; `line` is in
/// twips or in 240ths of a line, depending on `line_rule`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Spacing {
    pub before: Option<i32>,
    pub after: Option<i32>,
    pub line: Option<i32>,
    /// `auto` (multiple of single spacing), `exact` or `atLeast`.
    pub line_rule: Option<String>,
    /// Hundredths of a line; takes precedence over `before` when present.
    pub before_lines: Option<i32>,
    pub after_lines: Option<i32>,
    /// "Auto" spacing, as HTML paragraphs have: when on, the application
    /// picks the gap and `before`/`after` are ignored.
    pub before_auto: Option<bool>,
    pub after_auto: Option<bool>,
}

impl Spacing {
    pub fn apply(&mut self, over: &Spacing) {
        overlay!(self, over; before, after, before_lines, after_lines, before_auto, after_auto);
        // The rule belongs to the line height it was written with: a level
        // that states `w:line` alone means "auto", whatever rule a lower
        // level had. A level that states neither changes neither.
        if over.line.is_some() {
            self.line = over.line;
            self.line_rule = over.line_rule.clone();
        } else if over.line_rule.is_some() {
            self.line_rule = over.line_rule.clone();
        }
    }

    fn parse(node: Node) -> Self {
        let get = |name: &str| twips(w_attr(node, name));
        let lines = |name: &str| int(w_attr(node, name));
        Spacing {
            before: get("before"),
            after: get("after"),
            line: get("line"),
            line_rule: w_attr(node, "lineRule").map(str::to_owned),
            before_lines: lines("beforeLines"),
            after_lines: lines("afterLines"),
            before_auto: on_off_attr(node, "beforeAutospacing"),
            after_auto: on_off_attr(node, "afterAutospacing"),
        }
    }
}

/// `w:numPr`: membership of a numbered or bulleted list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NumRef {
    pub num_id: Option<i32>,
    pub level: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParaProps {
    /// Paragraph style (`w:pStyle`).
    pub style_id: Option<String>,
    /// Alignment (`w:jc`): `left`, `center`, `right`, `both`, `distribute`, ...
    pub justification: Option<String>,
    pub indent: Indent,
    pub spacing: Spacing,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
    /// Heading level for the document outline, 0-based.
    pub outline_level: Option<i32>,
    pub numbering: Option<NumRef>,
}

impl ParaProps {
    pub fn apply(&mut self, over: &ParaProps) {
        overlay!(self, over;
            style_id, justification, keep_next, keep_lines, page_break_before,
            widow_control, outline_level, numbering);
        self.indent.apply(&over.indent);
        self.spacing.apply(&over.spacing);
    }

    pub(crate) fn parse(node: Node, coverage: &mut Coverage) -> Self {
        let mut p = ParaProps::default();
        p.read(node, coverage);
        p
    }

    /// Reads a `w:pPr` on top of what is already here; see [`RunProps::read`].
    pub(crate) fn read(&mut self, node: Node, coverage: &mut Coverage) {
        let p = self;
        for child in elements(node) {
            match w_name(child) {
                Some("pStyle") => p.style_id = val(child).map(str::to_owned),
                Some("jc") => p.justification = val(child).map(str::to_owned),
                Some("ind") => p.indent.apply(&Indent::parse(child)),
                Some("spacing") => p.spacing.apply(&Spacing::parse(child)),
                Some("keepNext") => p.keep_next = Some(on_off(child)),
                Some("keepLines") => p.keep_lines = Some(on_off(child)),
                Some("pageBreakBefore") => p.page_break_before = Some(on_off(child)),
                Some("widowControl") => p.widow_control = Some(on_off(child)),
                Some("outlineLvl") => p.outline_level = int(val(child)),
                Some("numPr") => {
                    let get = |name| elements(child).find(|n| w_name(*n) == Some(name)).and_then(|n| int(val(n)));
                    p.numbering = Some(NumRef { num_id: get("numId"), level: get("ilvl") });
                }
                // The paragraph mark's run properties; the caller reads them.
                Some("rPr") => {}
                _ => coverage.miss(Some("w:pPr"), child),
            }
        }
    }
}
