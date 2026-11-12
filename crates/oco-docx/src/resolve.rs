//! Computes effective formatting by layering the style hierarchy
//! (ECMA-376 Part 1 §17.7.2):
//!
//! 1. document defaults
//! 2. paragraph style, base style first
//! 3. character style, base style first (runs only)
//! 4. direct formatting
//!
//! Table styles and numbering-level formatting also take part in the real
//! hierarchy; they are not modelled yet.

use crate::props::{FontRef, ParaProps, RunProps};
use crate::styles::{Style, StyleKind, StyleSheet};
use crate::theme::{ThemeFontLang, ThemeFonts};

pub struct Resolver<'a> {
    pub(crate) styles: &'a StyleSheet,
    pub(crate) theme: Option<&'a ThemeFonts>,
    pub(crate) theme_lang: &'a ThemeFontLang,
}

impl Resolver<'_> {
    /// Effective paragraph formatting for a paragraph with `direct` properties.
    /// `style_id` in the result is the style actually used, which is the
    /// default paragraph style when the paragraph names none.
    pub fn paragraph(&self, direct: &ParaProps) -> ParaProps {
        let chain = self
            .styles
            .chain(direct.style_id.as_deref(), StyleKind::Paragraph);
        let mut out = self.styles.default_para.clone();
        for style in &chain {
            out.apply(&style.para);
        }
        out.apply(direct);
        out.style_id = chain.last().map(|s| s.id.clone());
        out
    }

    /// Effective run formatting for a run with `direct` properties inside a
    /// paragraph that uses `paragraph_style`.
    pub fn run(&self, paragraph_style: Option<&str>, direct: &RunProps) -> RunProps {
        let defaults = &self.styles.default_run;
        let from_paragraph = flatten(&self.styles.chain(paragraph_style, StyleKind::Paragraph));
        let from_character = flatten(
            &self
                .styles
                .chain(direct.style_id.as_deref(), StyleKind::Character),
        );

        let mut out = defaults.clone();
        out.apply(&from_paragraph);
        out.apply(&from_character);
        out.apply(direct);

        let levels = [&from_paragraph, &from_character];
        out.bold = toggle(|p| p.bold, defaults, &levels, direct);
        out.italic = toggle(|p| p.italic, defaults, &levels, direct);
        out.caps = toggle(|p| p.caps, defaults, &levels, direct);
        out.small_caps = toggle(|p| p.small_caps, defaults, &levels, direct);
        out.strike = toggle(|p| p.strike, defaults, &levels, direct);
        out.vanish = toggle(|p| p.vanish, defaults, &levels, direct);

        for slot in [
            &mut out.fonts.ascii,
            &mut out.fonts.h_ansi,
            &mut out.fonts.east_asia,
            &mut out.fonts.cs,
        ] {
            self.resolve_theme_font(slot);
        }
        out
    }

    /// Replaces a theme reference with the typeface it names. A reference the
    /// theme cannot satisfy is left in place so callers can see what was asked.
    fn resolve_theme_font(&self, slot: &mut Option<FontRef>) {
        let Some(FontRef::Theme(name)) = slot else {
            return;
        };
        if let Some(typeface) = self.theme.and_then(|t| t.resolve(name, self.theme_lang)) {
            *slot = Some(FontRef::Name(typeface.to_owned()));
        }
    }
}

/// Collapses one style's inheritance chain into a single set of properties.
/// Within a chain a derived style simply overrides its base.
fn flatten(chain: &[&Style]) -> RunProps {
    let mut props = RunProps::default();
    for style in chain {
        props.apply(&style.run);
    }
    props
}

/// Toggle properties (§17.7.3) do not simply override across style levels.
///
/// - Direct formatting sets the state outright.
/// - Otherwise, if the document defaults turn the property on, it is on.
/// - Otherwise each style level that turns it on *flips* the state, so bold
///   text in a character style comes out non-bold inside a bold paragraph
///   style. This is what makes "emphasis" visible inside a bold heading.
fn toggle(
    get: impl Fn(&RunProps) -> Option<bool>,
    defaults: &RunProps,
    style_levels: &[&RunProps],
    direct: &RunProps,
) -> Option<bool> {
    if let Some(value) = get(direct) {
        return Some(value);
    }
    if get(defaults) == Some(true) {
        return Some(true);
    }
    let specified: Vec<bool> = style_levels.iter().filter_map(|level| get(level)).collect();
    if specified.is_empty() {
        return get(defaults);
    }
    Some(specified.into_iter().fold(false, |state, on| state ^ on))
}
