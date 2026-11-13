//! Font files, font selection and font metrics.
//!
//! Documents name fonts that are usually not available: the fonts Chinese
//! documents rely on (宋体, 黑体, 楷体, 仿宋, 微软雅黑) may not be shipped with a
//! product. Each is replaced by an open font of the same class. Replacing the
//! glyphs is harmless; replacing the *metrics* is not, because line height
//! comes from the font. Word sets a line of 宋体 at 1.3 times the font size,
//! while the open replacement's own metrics would give 1.87 times, which with
//! a document grid doubles the line spacing. So a substitution can carry the
//! metrics of the font it stands in for.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::sfnt::{self, Face, OutlineBuilder};

/// Whose layout rules to follow where they differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Word's: line height from the original font's metrics even where a
    /// substitute is drawn, the East Asian 1.3 factor, adjacent punctuation
    /// squeezed. This is what a product should use.
    Word,
    /// LibreOffice's: the substitute font's own metrics, no punctuation
    /// squeezing, quotes and dashes following the script around them. Only
    /// for checking the engine's mechanics against LibreOffice's output.
    LibreOffice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub usize);

pub struct Font {
    face: Face,
    families: Vec<String>,
    pub bold: bool,
    pub italic: bool,
    pub units_per_em: f32,
    /// Has Chinese, Japanese or Korean glyphs.
    pub east_asian: bool,
    box_word: LineBox,
    box_plain: LineBox,
    /// Underline and strike-through: (offset above the baseline, thickness), in em.
    pub underline: (f32, f32),
    pub strikeout: (f32, f32),
}

/// Height a font contributes to a line, in em: above and below the baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineBox {
    pub ascent: f32,
    pub descent: f32,
}

impl LineBox {
    /// Word gives East Asian fonts 30% more line height than their metrics
    /// say, half above and half below (measured: 宋体 10 pt sets at 12.97 pt).
    fn east_asian(self) -> Self {
        let extra = 0.15 * (self.ascent + self.descent);
        LineBox { ascent: self.ascent + extra, descent: self.descent + extra }
    }
}

impl Font {
    fn new(face: Face) -> Self {
        let upm = f32::from(face.units_per_em());
        let em = |units: i16| f32::from(units) / upm;
        let hhea = face.hhea();
        let hhea_box = LineBox { ascent: em(hhea.ascender) + em(hhea.line_gap).max(0.0), descent: -em(hhea.descender) };
        // Word's rule: the Windows ascent and descent, plus whatever the
        // `hhea` table asks for beyond them as external leading above the
        // line; or the typographic metrics when the font says to use those.
        let box_word = match face.os2() {
            Some(os2) if os2.use_typographic_metrics => LineBox {
                ascent: em(os2.typographic_ascender) + em(os2.typographic_line_gap).max(0.0),
                descent: -em(os2.typographic_descender),
            },
            Some(os2) => {
                let (ascent, descent) = (em(os2.windows_ascender), em(os2.windows_descender).abs());
                let leading = (hhea_box.ascent + hhea_box.descent - ascent - descent).max(0.0);
                LineBox { ascent: ascent + leading, descent }
            }
            None => hhea_box,
        };
        let mut families = Vec::new();
        for name in face.family_names() {
            let name = name.to_lowercase();
            if !families.contains(&name) {
                families.push(name);
            }
        }
        let line = |metrics: Option<(i16, i16)>, fallback: (f32, f32)| {
            metrics.map_or(fallback, |(position, thickness)| (em(position), em(thickness).max(0.02)))
        };
        Font {
            bold: face.is_bold(),
            italic: face.is_italic(),
            units_per_em: upm,
            east_asian: ['中', 'あ', '한'].iter().any(|ch| face.glyph_index(*ch).is_some()),
            box_word,
            box_plain: hhea_box,
            underline: line(face.post().map(|post| (post.underline_position, post.underline_thickness)), (-0.1, 0.05)),
            strikeout: line(face.os2().map(|os2| (os2.strikeout_position, os2.strikeout_thickness)), (0.3, 0.05)),
            families,
            face,
        }
    }

    pub fn glyph(&self, ch: char) -> Option<u16> {
        self.face.glyph_index(ch)
    }

    /// Advance width of a glyph in em.
    pub fn advance(&self, glyph: u16) -> f32 {
        self.face.advance(glyph).map_or(0.0, |advance| f32::from(advance) / self.units_per_em)
    }

    pub fn outline(&self, glyph: u16, builder: &mut dyn OutlineBuilder) {
        self.face.outline(glyph, builder);
    }

    pub fn family(&self) -> &str {
        self.families.first().map_or("", String::as_str)
    }
}

/// The metrics a missing font is known to have. Its replacement is drawn,
/// but lines are spaced and Latin letters advanced as the original would.
#[derive(Debug, Clone, Copy)]
struct Original {
    line_box: LineBox,
    /// ASCII letters and digits are half an em wide, as in the fonts of the
    /// 宋体 family, where Latin text is monospaced.
    half_width_latin: bool,
}

/// 宋体 and its relatives are drawn on a 256-unit em with ascent 220 and
/// descent 36. For 宋体 the resulting line height was checked against Word's
/// output; the others are taken to share it.
const SONG_BOX: Original =
    Original { line_box: LineBox { ascent: 220.0 / 256.0, descent: 36.0 / 256.0 }, half_width_latin: true };
/// 微软雅黑, from its published metrics (not checked against Word here).
const YAHEI_BOX: Original =
    Original { line_box: LineBox { ascent: 2167.0 / 2048.0, descent: 536.0 / 2048.0 }, half_width_latin: false };
/// Assumed for any other Chinese font that is not installed: nearly all are
/// drawn on a box one em tall.
const GENERIC_CJK_BOX: Original =
    Original { line_box: LineBox { ascent: 0.86, descent: 0.14 }, half_width_latin: false };

/// Requested family (lower case) -> replacement families in order of
/// preference, and the original's metrics where known.
const SUBSTITUTES: &[(&[&str], &[&str], Option<Original>)] = &[
    (&["宋体", "simsun", "新宋体", "nsimsun"], &["source han serif cn", "source han serif sc"], Some(SONG_BOX)),
    // No open 仿宋 is bundled yet; 宋 is the nearest in proportions.
    (
        &["仿宋", "fangsong", "仿宋_gb2312", "fangsong_gb2312"],
        &["source han serif cn", "source han serif sc"],
        Some(SONG_BOX),
    ),
    (&["黑体", "simhei"], &["source han sans cn", "source han sans sc"], Some(SONG_BOX)),
    (&["楷体", "kaiti", "楷体_gb2312", "kaiti_gb2312"], &["lxgw wenkai", "source han serif cn"], Some(SONG_BOX)),
    (
        &["微软雅黑", "microsoft yahei", "microsoft yahei ui"],
        &["source han sans cn", "source han sans sc"],
        Some(YAHEI_BOX),
    ),
    (
        &["等线", "dengxian", "华文黑体", "stheiti", "华文细黑", "stxihei"],
        &["source han sans cn"],
        Some(GENERIC_CJK_BOX),
    ),
    (
        &[
            "华文中宋",
            "stzhongsong",
            "华文宋体",
            "stsong",
            "方正小标宋简体",
            "方正小标宋_gbk",
            "fzxiaobiaosong-b05s",
            "华文仿宋",
            "stfangsong",
        ],
        &["source han serif cn"],
        Some(GENERIC_CJK_BOX),
    ),
    (&["华文楷体", "stkaiti", "方正楷体简体"], &["lxgw wenkai", "source han serif cn"], Some(GENERIC_CJK_BOX)),
    // These Latin replacements were designed with the same advance widths
    // and line metrics as the originals, so nothing needs overriding.
    (&["times new roman", "times"], &["liberation serif"], None),
    (&["arial", "helvetica"], &["liberation sans"], None),
    (&["courier new", "courier"], &["liberation mono"], None),
    (&["calibri"], &["carlito"], None),
    (&["cambria"], &["caladea"], None),
];

/// Fonts that are not shipped and have no replacement of the same widths,
/// but whose line is known. Their text is measured in another font and so
/// breaks differently, and the font still counts as missing; but its lines
/// are as tall as Word makes them, and so is an empty paragraph in it.
///
/// Aptos, the default of Word since 2023: 2500 units of a 2048-unit em from
/// one baseline to the next, 1925 of them above the baseline. (Measured in
/// Word, docx4j's probe `line-box-typo-metrics`: lines of 11 pt are 13.43 pt
/// apart, and the first baseline of a page is 10.34 pt down.)
const LINE_ONLY: &[(&[&str], LineBox)] = &[(
    &["aptos", "aptos display", "aptos narrow", "aptos light", "aptos semibold", "aptos black", "aptos extrabold"],
    LineBox { ascent: 1925.0 / 2048.0, descent: 575.0 / 2048.0 },
)];

const LATIN_FALLBACK: &[&str] = &["liberation serif", "dejavu serif"];
const CJK_FALLBACK: &[&str] = &["source han serif cn", "source han sans cn", "lxgw wenkai", "wenquanyi zen hei"];

/// The outcome of choosing a font for some text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Choice {
    pub font: FontId,
    /// The font has no bold (or italic) face; the effect is imitated.
    pub fake_bold: bool,
    pub fake_italic: bool,
    pub line_box: LineBox,
    /// The font's own ascent and descent, before the enlargement Word gives
    /// the lines of East Asian fonts. Where a line's height is fixed, this
    /// is what places the text within it.
    pub text_box: LineBox,
    pub half_width_latin: bool,
    /// The font asked for, or its designated replacement, was available.
    /// When false the text is set in an unrelated fallback font, so its
    /// widths, and with them the line breaks, cannot be expected to match.
    pub known_family: bool,
    /// The font asked for is missing and another is drawn, but the line is
    /// as tall as the missing font's would be: see `LINE_ONLY`.
    pub line_known: bool,
}

#[derive(Default)]
pub struct FontDb {
    fonts: Vec<Font>,
    by_family: HashMap<String, Vec<FontId>>,
}

impl FontDb {
    /// Loads every font file under `dir`, recursively. Unreadable files are skipped.
    pub fn load_dir(&mut self, dir: &Path) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                if path.file_name().is_some_and(|n| !n.to_string_lossy().starts_with('.')) {
                    self.load_dir(&path);
                }
                continue;
            }
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
            if !matches!(extension.as_str(), "ttf" | "otf" | "ttc") {
                continue;
            }
            let Ok(data) = fs::read(&path) else { continue };
            self.add(data);
        }
    }

    /// Adds the faces of a font file held in memory (a collection has
    /// several) and returns how many there were: none if it is not a font.
    pub fn add(&mut self, data: Vec<u8>) -> usize {
        let data: Arc<[u8]> = data.into();
        let before = self.fonts.len();
        for index in 0..sfnt::faces_in(&data) {
            if let Some(face) = Face::parse(Arc::clone(&data), index) {
                self.register(Font::new(face));
            }
        }
        self.fonts.len() - before
    }

    fn register(&mut self, font: Font) {
        let id = FontId(self.fonts.len());
        for family in &font.families {
            self.by_family.entry(family.clone()).or_default().push(id);
        }
        self.fonts.push(font);
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    pub fn get(&self, id: FontId) -> &Font {
        &self.fonts[id.0]
    }

    /// The face of `family` closest to the wanted style.
    fn face(&self, family: &str, bold: bool, italic: bool) -> Option<FontId> {
        let candidates = self.by_family.get(family)?;
        candidates.iter().copied().min_by_key(|id| {
            let font = self.get(*id);
            u8::from(font.bold != bold) * 2 + u8::from(font.italic != italic)
        })
    }

    /// Chooses the font to draw `ch` with when the document asks for
    /// `family` (`None` if it names no font for this kind of character).
    pub fn choose(&self, family: Option<&str>, bold: bool, italic: bool, ch: char, model: Model) -> Option<Choice> {
        let mut wanted = family.map(|f| f.split(';').next().unwrap_or(f).trim().to_lowercase());
        // Some documents name a face rather than a family ("Arial Bold").
        // When no such family exists, read the style off the end of the name.
        let (mut bold, mut italic) = (bold, italic);
        if let Some(name) = wanted.as_mut().filter(|name| !self.by_family.contains_key(name.as_str())) {
            for (suffix, is_bold, is_italic) in [
                (" bold italic", true, true),
                (" bold", true, false),
                (" italic", false, true),
                (" oblique", false, true),
                (" regular", false, false),
            ] {
                if let Some(base) = name.strip_suffix(suffix).filter(|base| !base.is_empty()) {
                    *name = base.to_owned();
                    bold |= is_bold;
                    italic |= is_italic;
                    break;
                }
            }
        }
        let cjk = crate::text::is_cjk(ch);
        let mut original = None;
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(wanted) = wanted.as_deref() {
            candidates.push(wanted);
            if !self.by_family.contains_key(wanted) {
                match SUBSTITUTES.iter().find(|(names, _, _)| names.contains(&wanted)) {
                    Some((_, replacements, known)) => {
                        candidates.extend(*replacements);
                        original = *known;
                    }
                    // An unknown font with a Chinese name is almost certainly a Chinese font.
                    None if wanted.chars().any(crate::text::is_cjk) => original = Some(GENERIC_CJK_BOX),
                    None => {}
                }
            }
        }
        candidates.extend(if cjk { CJK_FALLBACK } else { LATIN_FALLBACK });
        candidates.extend(if cjk { LATIN_FALLBACK } else { CJK_FALLBACK });

        let has_glyph = |id: &FontId| self.get(*id).glyph(ch).is_some();
        let font = candidates
            .iter()
            .filter_map(|family| self.face(family, bold, italic))
            .find(has_glyph)
            // Last resort: any font at all that has the character, else the first font.
            .or_else(|| (0..self.fonts.len()).map(FontId).find(has_glyph))
            .or_else(|| (!self.fonts.is_empty()).then_some(FontId(0)))?;

        let actual = self.get(font);
        // The original's metrics apply only while its own replacement is in
        // use, not when the character had to come from a fallback font.
        let from_replacement = wanted.as_deref().is_some_and(|w| {
            SUBSTITUTES.iter().any(|(names, replacements, _)| {
                names.contains(&w) && actual.families.iter().any(|f| replacements.contains(&f.as_str()))
            })
        }) || (original.is_some() && actual.east_asian);
        let asked_by_name = wanted.as_deref().is_some_and(|w| actual.families.iter().any(|f| f == w));
        let original = match original.filter(|_| from_replacement) {
            Some(known) => Some(known),
            // A character the named font lacks (a circled digit in a Latin
            // font, say) falls back to a Chinese font. Word would fall back
            // to 宋体 or one like it, so the line is spaced for that, not for
            // the much taller box of the font actually drawn.
            None if actual.east_asian && !asked_by_name => Some(GENERIC_CJK_BOX),
            None => None,
        }
        .filter(|_| model == Model::Word);
        // The font's own box, and the box a line of it takes up.
        let mut line_known = false;
        let (text_box, line_box) = match model {
            Model::LibreOffice => (actual.box_plain, actual.box_plain),
            Model::Word => {
                let line_only = wanted
                    .as_deref()
                    .filter(|w| !self.by_family.contains_key(*w) && !actual.east_asian)
                    .and_then(|w| LINE_ONLY.iter().find(|(names, _)| names.contains(&w)))
                    .map(|(_, line)| *line);
                line_known = original.is_none() && line_only.is_some();
                let text_box = original.map_or(line_only.unwrap_or(actual.box_word), |o| o.line_box);
                (text_box, if actual.east_asian { text_box.east_asian() } else { text_box })
            }
        };
        let known_family = wanted.as_deref().is_none_or(|w| {
            self.by_family.contains_key(w)
                || SUBSTITUTES.iter().any(|(names, replacements, _)| {
                    names.contains(&w) && replacements.iter().any(|r| self.by_family.contains_key(*r))
                })
        });
        Some(Choice {
            font,
            known_family,
            fake_bold: bold && !actual.bold,
            fake_italic: italic && !actual.italic,
            line_box,
            text_box,
            half_width_latin: original.is_some_and(|o| o.half_width_latin),
            line_known,
        })
    }
}
