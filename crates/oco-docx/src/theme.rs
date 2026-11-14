//! Theme fonts (`word/theme/theme1.xml`) and the settings needed to pick them.
//!
//! Word's default template does not name fonts directly: run properties say
//! e.g. `w:eastAsiaTheme="minorEastAsia"`, and the theme maps that slot to a
//! typeface. For East Asian and complex-script slots the theme usually lists
//! one typeface per script, and `w:themeFontLang` in the settings part says
//! which script the document uses.

use std::collections::HashMap;

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::xml::{elements, w_attr, w_name};

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct FontSet {
    latin: Option<String>,
    east_asia: Option<String>,
    complex: Option<String>,
    /// ISO 15924 script code (`Hans`, `Jpan`, ...) -> typeface.
    by_script: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ThemeFonts {
    /// Heading fonts.
    major: FontSet,
    /// Body fonts.
    minor: FontSet,
}

/// `w:themeFontLang` from the settings part.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ThemeFontLang {
    pub latin: Option<String>,
    pub east_asia: Option<String>,
    pub bidi: Option<String>,
}

impl ThemeFonts {
    pub(crate) fn parse(xml: &str) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut theme = ThemeFonts::default();
        // DrawingML also has Transitional and Strict namespaces; inside a theme
        // part the local names are unambiguous, so match on those.
        let scheme = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "fontScheme");
        for set_node in scheme.into_iter().flat_map(elements) {
            let set = match set_node.tag_name().name() {
                "majorFont" => &mut theme.major,
                "minorFont" => &mut theme.minor,
                _ => continue,
            };
            for font in elements(set_node) {
                let typeface = font.attribute("typeface").filter(|t| !t.is_empty()).map(str::to_owned);
                match font.tag_name().name() {
                    "latin" => set.latin = typeface,
                    "ea" => set.east_asia = typeface,
                    "cs" => set.complex = typeface,
                    "font" => {
                        if let (Some(script), Some(typeface)) = (font.attribute("script"), typeface) {
                            set.by_script.insert(script.to_owned(), typeface);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(theme)
    }

    /// Typeface for a theme slot such as `majorHAnsi` or `minorEastAsia`.
    pub fn resolve(&self, slot: &str, lang: &ThemeFontLang) -> Option<&str> {
        let (set, category) = slot
            .strip_prefix("major")
            .map(|rest| (&self.major, rest))
            .or_else(|| slot.strip_prefix("minor").map(|rest| (&self.minor, rest)))?;
        let for_lang = |lang: &Option<String>| {
            let script = script_for_lang(lang.as_deref()?)?;
            set.by_script.get(script).map(String::as_str)
        };
        match category {
            "Ascii" | "HAnsi" => set.latin.as_deref(),
            "EastAsia" => set.east_asia.as_deref().or_else(|| for_lang(&lang.east_asia)),
            "Bidi" => set.complex.as_deref().or_else(|| for_lang(&lang.bidi)),
            _ => None,
        }
    }
}

impl ThemeFontLang {
    fn parse(node: crate::xmltree::Node) -> Self {
        let get = |name| w_attr(node, name).map(str::to_owned);
        ThemeFontLang { latin: get("val"), east_asia: get("eastAsia"), bidi: get("bidi") }
    }
}

/// Document-wide settings (`word/settings.xml`) that affect how text is laid out.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Settings {
    pub theme_lang: ThemeFontLang,
    /// Spacing of the default tab stops, in twips.
    pub default_tab_stop: i32,
    /// Squeeze adjacent full-width punctuation (`compressPunctuation`), the
    /// default in Chinese editions of Word.
    pub compress_punctuation: bool,
    /// Underline spaces at the end of a line (`ulTrailSpace`), which is what
    /// makes the blanks in "签字：______" visible.
    pub underline_trailing_spaces: bool,
    /// Do not treat a hanging indent as a tab stop (`noTabHangInd`).
    pub no_tab_hang_indent: bool,
    /// Even pages have headers and footers of their own (`evenAndOddHeaders`).
    pub even_and_odd_headers: bool,
    /// The Word version whose layout rules the document asks for; 15 is
    /// current, and a document that does not say gets 12.
    pub compatibility_mode: i32,
    /// How notes are numbered in the whole document, unless a section says otherwise.
    pub footnotes: crate::section::NoteProps,
    pub endnotes: crate::section::NoteProps,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_lang: ThemeFontLang::default(),
            default_tab_stop: 720,
            compress_punctuation: false,
            underline_trailing_spaces: false,
            no_tab_hang_indent: false,
            even_and_odd_headers: false,
            compatibility_mode: 12,
            footnotes: Default::default(),
            endnotes: Default::default(),
        }
    }
}

impl Settings {
    pub(crate) fn parse(xml: &str) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut settings = Settings::default();
        for node in elements(doc.root_element()) {
            match w_name(node) {
                Some("themeFontLang") => settings.theme_lang = ThemeFontLang::parse(node),
                Some("defaultTabStop") => {
                    settings.default_tab_stop = crate::xml::twips(crate::xml::val(node)).unwrap_or(720).max(1);
                }
                Some("evenAndOddHeaders") => settings.even_and_odd_headers = crate::xml::on_off(node),
                Some("footnotePr") => settings.footnotes = crate::section::NoteProps::parse(node),
                Some("endnotePr") => settings.endnotes = crate::section::NoteProps::parse(node),
                Some("characterSpacingControl") => {
                    settings.compress_punctuation =
                        crate::xml::val(node).is_some_and(|v| v.starts_with("compressPunctuation"));
                }
                Some("compat") => {
                    for flag in elements(node) {
                        match w_name(flag) {
                            Some("ulTrailSpace") => settings.underline_trailing_spaces = crate::xml::on_off(flag),
                            Some("noTabHangInd") => settings.no_tab_hang_indent = crate::xml::on_off(flag),
                            Some("compatSetting") if w_attr(flag, "name") == Some("compatibilityMode") => {
                                settings.compatibility_mode = crate::xml::int(crate::xml::val(flag)).unwrap_or(12);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(settings)
    }
}

/// Maps a BCP 47 language tag to the script code themes key their fonts by.
fn script_for_lang(lang: &str) -> Option<&'static str> {
    let lang = lang.to_ascii_lowercase();
    let primary = lang.split('-').next().unwrap_or_default();
    Some(match primary {
        "zh" => match lang.as_str() {
            "zh-tw" | "zh-hk" | "zh-mo" | "zh-hant" => "Hant",
            _ => "Hans",
        },
        "ja" => "Jpan",
        "ko" => "Hang",
        "ar" | "fa" | "ur" => "Arab",
        "he" => "Hebr",
        "th" => "Thai",
        _ => return None,
    })
}
