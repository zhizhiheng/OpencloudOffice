//! Theme fonts (`word/theme/theme1.xml`) and the settings needed to pick them.
//!
//! Word's default template does not name fonts directly: run properties say
//! e.g. `w:eastAsiaTheme="minorEastAsia"`, and the theme maps that slot to a
//! typeface. For East Asian and complex-script slots the theme usually lists
//! one typeface per script, and `w:themeFontLang` in the settings part says
//! which script the document uses.

use std::collections::HashMap;

use serde::Serialize;

use crate::xml::{elements, w_attr, w_name};

#[derive(Debug, Clone, Default, Serialize)]
struct FontSet {
    latin: Option<String>,
    east_asia: Option<String>,
    complex: Option<String>,
    /// ISO 15924 script code (`Hans`, `Jpan`, ...) -> typeface.
    by_script: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ThemeFonts {
    /// Heading fonts.
    major: FontSet,
    /// Body fonts.
    minor: FontSet,
}

/// `w:themeFontLang` from the settings part.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ThemeFontLang {
    pub latin: Option<String>,
    pub east_asia: Option<String>,
    pub bidi: Option<String>,
}

impl ThemeFonts {
    pub(crate) fn parse(xml: &str) -> Result<Self, roxmltree::Error> {
        let doc = roxmltree::Document::parse(xml)?;
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
    pub(crate) fn parse_settings(xml: &str) -> Result<Self, roxmltree::Error> {
        let doc = roxmltree::Document::parse(xml)?;
        let lang = elements(doc.root_element()).find(|n| w_name(*n) == Some("themeFontLang"));
        Ok(lang
            .map(|n| {
                let get = |name| w_attr(n, name).map(str::to_owned);
                ThemeFontLang { latin: get("val"), east_asia: get("eastAsia"), bidi: get("bidi") }
            })
            .unwrap_or_default())
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
