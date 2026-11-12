//! Run and paragraph formatting properties (`w:rPr`, `w:pPr`).
//!
//! Every field is optional: `None` means "not specified at this level", which
//! is what lets one level of the style hierarchy be layered over another.

use roxmltree::Node;
use serde::Serialize;

use crate::coverage::Coverage;
use crate::xml::{elements, int, on_off, val, w_attr, w_name};

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
    /// Underline pattern (`single`, `double`, `none`, ...).
    pub underline: Option<String>,
    /// Font size in half-points (`w:sz`).
    pub size: Option<u32>,
    /// `RRGGBB` or `auto`.
    pub color: Option<String>,
    pub highlight: Option<String>,
    /// `superscript`, `subscript` or `baseline`.
    pub vert_align: Option<String>,
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
        for child in elements(node) {
            let text = || val(child).map(str::to_owned);
            match w_name(child) {
                Some("rStyle") => p.style_id = text(),
                Some("b") => p.bold = Some(on_off(child)),
                Some("i") => p.italic = Some(on_off(child)),
                Some("caps") => p.caps = Some(on_off(child)),
                Some("smallCaps") => p.small_caps = Some(on_off(child)),
                Some("strike") => p.strike = Some(on_off(child)),
                Some("vanish") => p.vanish = Some(on_off(child)),
                Some("u") => p.underline = text(),
                Some("sz") => p.size = int(val(child)).and_then(|v| u32::try_from(v).ok()),
                Some("color") => p.color = text(),
                Some("highlight") => p.highlight = text(),
                Some("vertAlign") => p.vert_align = text(),
                Some("rFonts") => p.fonts = Fonts::parse(child),
                _ => coverage.miss(Some("w:rPr"), child),
            }
        }
        p
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
        let get = |name: &str| int(w_attr(node, name));
        Indent {
            // `start`/`end` are the bidi-neutral spellings of `left`/`right`.
            left: get("left").or_else(|| get("start")),
            right: get("right").or_else(|| get("end")),
            first_line: get("firstLine"),
            hanging: get("hanging"),
            left_chars: get("leftChars").or_else(|| get("startChars")),
            right_chars: get("rightChars").or_else(|| get("endChars")),
            first_line_chars: get("firstLineChars"),
            hanging_chars: get("hangingChars"),
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
}

impl Spacing {
    pub fn apply(&mut self, over: &Spacing) {
        overlay!(self, over; before, after, line, line_rule, before_lines, after_lines);
    }

    fn parse(node: Node) -> Self {
        let get = |name: &str| int(w_attr(node, name));
        Spacing {
            before: get("before"),
            after: get("after"),
            line: get("line"),
            line_rule: w_attr(node, "lineRule").map(str::to_owned),
            before_lines: get("beforeLines"),
            after_lines: get("afterLines"),
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
        for child in elements(node) {
            match w_name(child) {
                Some("pStyle") => p.style_id = val(child).map(str::to_owned),
                Some("jc") => p.justification = val(child).map(str::to_owned),
                Some("ind") => p.indent = Indent::parse(child),
                Some("spacing") => p.spacing = Spacing::parse(child),
                Some("keepNext") => p.keep_next = Some(on_off(child)),
                Some("keepLines") => p.keep_lines = Some(on_off(child)),
                Some("pageBreakBefore") => p.page_break_before = Some(on_off(child)),
                Some("widowControl") => p.widow_control = Some(on_off(child)),
                Some("outlineLvl") => p.outline_level = int(val(child)),
                Some("numPr") => {
                    let get = |name| {
                        elements(child)
                            .find(|n| w_name(*n) == Some(name))
                            .and_then(|n| int(val(n)))
                    };
                    p.numbering = Some(NumRef {
                        num_id: get("numId"),
                        level: get("ilvl"),
                    });
                }
                // The paragraph mark's run properties; the caller reads them.
                Some("rPr") => {}
                _ => coverage.miss(Some("w:pPr"), child),
            }
        }
        p
    }
}
