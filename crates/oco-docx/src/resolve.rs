//! Computes effective formatting by layering the style hierarchy
//! (ECMA-376 Part 1 §17.7.2):
//!
//! 1. document defaults
//! 2. paragraph style, base style first
//! 3. the list level, for a numbered paragraph (indents and tab stops)
//! 4. character style, base style first (runs only)
//! 5. direct formatting
//!
//! Table styles also take part in the real hierarchy; they are not modelled yet.

use crate::numbering::{Level, Numbering};
use crate::props::{FontRef, Indent, NumRef, ParaProps, RunProps};
use crate::styles::{Style, StyleKind, StyleSheet};
use crate::theme::{ThemeFontLang, ThemeFonts};

pub struct Resolver<'a> {
    pub(crate) styles: &'a StyleSheet,
    pub(crate) theme: Option<&'a ThemeFonts>,
    pub(crate) theme_lang: &'a ThemeFontLang,
    pub(crate) numbering: &'a Numbering,
}

impl<'a> Resolver<'a> {
    /// Effective paragraph formatting for a paragraph with `direct` properties.
    ///
    /// `style_id` in the result is the style actually used, which is the
    /// default paragraph style when the paragraph names none. `numbering` is
    /// `Some` only for a paragraph that really is a list item, with both the
    /// list and the level filled in.
    pub fn paragraph(&self, direct: &ParaProps) -> ParaProps {
        let chain = self.styles.chain(direct.style_id.as_deref(), StyleKind::Paragraph);
        let mut out = self.styles.default_para.clone();
        for style in &chain {
            out.apply(&style.para);
        }
        let from_styles = out.numbering.clone();
        out.apply(direct);
        out.style_id = chain.last().map(|s| s.id.clone());

        let list = out.numbering.take().and_then(|n| Some((n.num_id.filter(|id| *id != 0)?, n.level)));
        let direct_list = direct.numbering.as_ref().and_then(|n| n.num_id);
        match list {
            Some((num_id, stated_level)) => {
                // A list named by a style uses the level that belongs to that
                // style; this is how numbered headings find their depth.
                let from_style = if direct_list.is_none() {
                    chain.iter().rev().find_map(|style| self.numbering.level_for_style(num_id, &style.id))
                } else {
                    None
                };
                let ilvl = from_style.or(stated_level.and_then(|l| usize::try_from(l).ok())).unwrap_or(0).min(8);
                if let Some(level) = self.numbering.level(num_id, ilvl) {
                    // The level's indents beat the style's; direct formatting beats both.
                    out.apply(&level.para);
                    out.apply(direct);
                    out.style_id = chain.last().map(|s| s.id.clone());
                    out.numbering = Some(NumRef { num_id: Some(num_id), level: Some(ilvl as i32) });
                }
            }
            // `numId` 0 takes a paragraph out of the list its style puts it
            // in, and with the label go the indents that came with it.
            None if direct_list == Some(0) && from_styles.is_some() => {
                out.indent = Indent::default();
                out.indent.apply(&direct.indent);
                out.numbering = None;
            }
            None => out.numbering = None,
        }
        out
    }

    /// The list level a resolved paragraph belongs to, if it is a list item.
    pub fn list_level(&self, resolved: &ParaProps) -> Option<(i32, usize, &'a Level)> {
        let list = resolved.numbering.as_ref()?;
        let (num_id, ilvl) = (list.num_id?, usize::try_from(list.level?).ok()?);
        Some((num_id, ilvl, self.numbering.level(num_id, ilvl)?))
    }

    /// Formatting of a list label: the paragraph mark's formatting with the
    /// level's own character properties on top.
    pub fn label(&self, paragraph_style: Option<&str>, mark: &RunProps, level: &Level) -> RunProps {
        let mut direct = mark.clone();
        direct.apply(&level.run);
        self.run(paragraph_style, &direct)
    }

    /// Effective run formatting for a run with `direct` properties inside a
    /// paragraph that uses `paragraph_style`.
    pub fn run(&self, paragraph_style: Option<&str>, direct: &RunProps) -> RunProps {
        let defaults = &self.styles.default_run;
        let from_paragraph = flatten(&self.styles.chain(paragraph_style, StyleKind::Paragraph));
        let character_chain = self.styles.chain(direct.style_id.as_deref(), StyleKind::Character);
        let mut from_character = flatten(&character_chain);
        // Word does not draw a highlight that comes from a character style,
        // although it does draw one from a paragraph style.
        from_character.highlight = None;

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
        out.outline = toggle(|p| p.outline, defaults, &levels, direct);
        out.shadow = toggle(|p| p.shadow, defaults, &levels, direct);
        out.emboss = toggle(|p| p.emboss, defaults, &levels, direct);
        out.imprint = toggle(|p| p.imprint, defaults, &levels, direct);
        out.bold_cs = toggle(|p| p.bold_cs, defaults, &levels, direct);
        out.italic_cs = toggle(|p| p.italic_cs, defaults, &levels, direct);

        for slot in [&mut out.fonts.ascii, &mut out.fonts.h_ansi, &mut out.fonts.east_asia, &mut out.fonts.cs] {
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
/// - Otherwise each style level that turns the property on *flips* the
///   state, so bold text in a character style comes out non-bold inside a
///   bold paragraph style. This is what makes "emphasis" visible inside a
///   bold heading. A level that states "off" leaves the state alone.
/// - The document defaults apply only when no style level says anything.
///
/// The last point is Word's behaviour, read off its own PDF output. As we
/// read the standard, a property the defaults turn on should stay on whatever
/// the styles say; Word nevertheless draws regular text for bold defaults
/// under a paragraph style with `<w:b w:val="0"/>`, and Word is what counts.
fn toggle(
    get: impl Fn(&RunProps) -> Option<bool>,
    defaults: &RunProps,
    style_levels: &[&RunProps],
    direct: &RunProps,
) -> Option<bool> {
    if let Some(value) = get(direct) {
        return Some(value);
    }
    let specified: Vec<bool> = style_levels.iter().filter_map(|level| get(level)).collect();
    if specified.is_empty() {
        return get(defaults);
    }
    Some(specified.into_iter().fold(false, |state, on| state ^ on))
}
