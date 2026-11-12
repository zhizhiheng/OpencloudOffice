//! Run and paragraph formatting properties (`w:rPr`, `w:pPr`).
//!
//! Every field is optional: `None` means "not specified at this level", which
//! is what lets one level of the style hierarchy be layered over another.

use roxmltree::Node;
use serde::Serialize;

use crate::coverage::Coverage;
use crate::values::{Color, Emphasis, Highlight, Underline, VertAlign};
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
    // The remaining toggles are rare; they are read so that the model is
    // complete, and drawn as plain text for now.
    pub outline: Option<bool>,
    pub shadow: Option<bool>,
    pub emboss: Option<bool>,
    pub imprint: Option<bool>,
    /// Bold and italic for complex-script text (Arabic, Hebrew, ...).
    pub bold_cs: Option<bool>,
    pub italic_cs: Option<bool>,
    pub double_strike: Option<bool>,
    pub underline: Option<Underline>,
    /// `None` means the underline takes the text colour.
    pub underline_color: Option<Color>,
    /// Font size in half-points (`w:sz`).
    pub size: Option<u32>,
    /// Font size for complex-script text, in half-points (`w:szCs`).
    pub size_cs: Option<u32>,
    pub color: Option<Color>,
    pub highlight: Option<Highlight>,
    /// Background fill behind the text (`w:shd`); `Auto` means none.
    pub shade: Option<Color>,
    pub border: Option<Border>,
    pub vert_align: Option<VertAlign>,
    pub emphasis: Option<Emphasis>,
    /// Extra space after each character in twips; negative condenses.
    pub char_spacing: Option<i32>,
    /// Horizontal scaling of the glyphs in percent (`w:w`).
    pub scale: Option<i32>,
    /// Raise (positive) or lower the text, in half-points.
    pub position: Option<i32>,
    /// Kern pairs at and above this size in half-points; 0 turns kerning off.
    pub kern: Option<u32>,
    /// The run is complex-script or right-to-left text and uses the `cs` font.
    pub complex: Option<bool>,
    pub fonts: Fonts,
}

/// One edge of a border, or the box around a run (`w:bdr`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Border {
    /// Line style: `single`, `double`, `dashed`, ... or `nil`/`none` for no line.
    pub style: String,
    /// Line width in eighths of a point.
    pub size: i32,
    /// Gap between the line and the text, in points.
    pub space: i32,
    pub color: Option<Color>,
}

impl Border {
    fn parse(node: Node) -> Self {
        Border {
            style: val(node).unwrap_or("single").to_owned(),
            size: int(w_attr(node, "sz")).unwrap_or(4),
            space: int(w_attr(node, "space")).unwrap_or(0),
            color: w_attr(node, "color").and_then(Color::parse),
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.style.as_str(), "nil" | "none")
    }
}

/// The fill of a `w:shd`. A pattern other than a plain fill is not modelled.
fn shade(node: Node) -> Option<Color> {
    w_attr(node, "fill").and_then(Color::parse)
}

impl RunProps {
    /// Layers `over` on top of `self`: whatever `over` specifies replaces the
    /// current value. Toggle properties need extra care across style levels,
    /// which the resolver handles.
    pub fn apply(&mut self, over: &RunProps) {
        overlay!(self, over;
            style_id, bold, italic, caps, small_caps, strike, vanish, outline, shadow, emboss, imprint,
            bold_cs, italic_cs, double_strike, underline, underline_color, size, size_cs, color, highlight,
            shade, border, vert_align, emphasis, char_spacing, scale, position, kern, complex);
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
                Some("outline") => p.outline = Some(on_off(child)),
                Some("shadow") => p.shadow = Some(on_off(child)),
                Some("emboss") => p.emboss = Some(on_off(child)),
                Some("imprint") => p.imprint = Some(on_off(child)),
                Some("bCs") => p.bold_cs = Some(on_off(child)),
                Some("iCs") => p.italic_cs = Some(on_off(child)),
                Some("dstrike") => p.double_strike = Some(on_off(child)),
                Some("cs" | "rtl") => p.complex = Some(on_off(child) || p.complex == Some(true)),
                // For these an invalid value means "not specified here", so
                // it must not overwrite a value read earlier.
                Some("u") => {
                    p.underline = Underline::parse(val(child)).or(p.underline);
                    p.underline_color = w_attr(child, "color").and_then(Color::parse).or(p.underline_color);
                }
                Some("em") => p.emphasis = val(child).and_then(Emphasis::from_exact).or(p.emphasis),
                Some("szCs") => p.size_cs = half_points(val(child)),
                Some("spacing") => p.char_spacing = twips(val(child)),
                Some("w") => p.scale = int(val(child)),
                Some("position") => p.position = int(val(child)),
                Some("kern") => p.kern = half_points(val(child)),
                Some("shd") => p.shade = shade(child).or(p.shade),
                Some("bdr") => p.border = Some(Border::parse(child)),
                // Proofing language and editing aids: no effect on how text looks.
                Some("lang" | "noProof" | "webHidden" | "specVanish" | "effect" | "oMath") => {}
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    Left,
    Center,
    Right,
    Decimal,
    Bar,
    /// Removes a stop inherited from a style at the same position.
    Clear,
}

/// A custom tab stop (`w:tab` inside `w:tabs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TabStop {
    pub kind: TabKind,
    /// Distance from the left margin in twips.
    pub position: i32,
    /// Fill character leading up to the stop: `dot`, `hyphen`, `underscore`, ...
    pub leader: Option<String>,
}

impl TabStop {
    fn parse(node: Node) -> Option<Self> {
        let kind = match val(node)? {
            "center" => TabKind::Center,
            "right" | "end" => TabKind::Right,
            "decimal" => TabKind::Decimal,
            "bar" => TabKind::Bar,
            "clear" => TabKind::Clear,
            // `num` is the legacy list tab; it behaves as a left tab.
            _ => TabKind::Left,
        };
        Some(TabStop {
            kind,
            position: twips(w_attr(node, "pos"))?,
            leader: w_attr(node, "leader").filter(|l| *l != "none").map(str::to_owned),
        })
    }
}

/// Borders around a paragraph (`w:pBdr`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParaBorders {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    /// Between consecutive paragraphs with identical borders.
    pub between: Option<Border>,
}

impl ParaBorders {
    pub fn apply(&mut self, over: &ParaBorders) {
        overlay!(self, over; top, left, bottom, right, between);
    }
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
    /// Custom tab stops, sorted by position.
    pub tabs: Vec<TabStop>,
    pub borders: ParaBorders,
    /// Background fill of the paragraph; `Auto` means none.
    pub shade: Option<Color>,
    /// Align lines to the section's document grid. On unless turned off.
    pub snap_to_grid: Option<bool>,
    /// Drop the space between this paragraph and a neighbour of the same style.
    pub contextual_spacing: Option<bool>,
    /// Keep punctuation that may not start or end a line off those positions.
    pub kinsoku: Option<bool>,
    /// Let closing punctuation hang into the right margin.
    pub overflow_punct: Option<bool>,
    /// Compress punctuation at the start of a line.
    pub top_line_punct: Option<bool>,
    /// Add a little space between East Asian and Latin text / digits.
    pub auto_space_de: Option<bool>,
    pub auto_space_dn: Option<bool>,
    /// Allow a Latin word to break anywhere when it must.
    pub word_wrap: Option<bool>,
    pub bidi: Option<bool>,
}

impl ParaProps {
    pub fn apply(&mut self, over: &ParaProps) {
        overlay!(self, over;
            style_id, justification, keep_next, keep_lines, page_break_before, widow_control, outline_level,
            shade, snap_to_grid, contextual_spacing, kinsoku, overflow_punct, top_line_punct, auto_space_de,
            auto_space_dn, word_wrap, bidi);
        self.indent.apply(&over.indent);
        self.spacing.apply(&over.spacing);
        self.borders.apply(&over.borders);
        // The list and the level within it are inherited separately: a style
        // may fix the list while the paragraph picks the level.
        if let Some(over) = &over.numbering {
            let mine = self.numbering.get_or_insert_with(NumRef::default);
            overlay!(mine, over; num_id, level);
        }
        for stop in &over.tabs {
            self.tabs.retain(|existing| existing.position != stop.position);
            if stop.kind != TabKind::Clear {
                self.tabs.push(stop.clone());
            }
        }
        self.tabs.sort_by_key(|stop| stop.position);
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
                // Stops are kept as written, `clear` included, so that a
                // style layered on later can still remove an inherited stop.
                Some("tabs") => {
                    p.tabs.extend(elements(child).filter(|n| w_name(*n) == Some("tab")).filter_map(TabStop::parse))
                }
                Some("pBdr") => {
                    for edge in elements(child) {
                        let slot = match w_name(edge) {
                            Some("top") => &mut p.borders.top,
                            Some("left" | "start") => &mut p.borders.left,
                            Some("bottom") => &mut p.borders.bottom,
                            Some("right" | "end") => &mut p.borders.right,
                            Some("between") => &mut p.borders.between,
                            _ => continue,
                        };
                        *slot = Some(Border::parse(edge));
                    }
                }
                Some("shd") => p.shade = shade(child).or(p.shade),
                Some("snapToGrid") => p.snap_to_grid = Some(on_off(child)),
                Some("contextualSpacing") => p.contextual_spacing = Some(on_off(child)),
                Some("kinsoku") => p.kinsoku = Some(on_off(child)),
                Some("overflowPunct") => p.overflow_punct = Some(on_off(child)),
                Some("topLinePunct") => p.top_line_punct = Some(on_off(child)),
                Some("autoSpaceDE") => p.auto_space_de = Some(on_off(child)),
                Some("autoSpaceDN") => p.auto_space_dn = Some(on_off(child)),
                Some("wordWrap") => p.word_wrap = Some(on_off(child)),
                Some("bidi") => p.bidi = Some(on_off(child)),
                // The paragraph mark's run properties and a section break;
                // the caller reads both.
                Some("rPr" | "sectPr") => {}
                // Line numbering and hyphenation switches, and editing aids.
                Some("suppressLineNumbers" | "suppressAutoHyphens" | "adjustRightInd" | "divId" | "cnfStyle") => {}
                _ => coverage.miss(Some("w:pPr"), child),
            }
        }
    }
}
