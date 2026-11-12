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
use crate::table::{CellMargins, TableProps};
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
        self.paragraph_in(direct, &[])
    }

    /// The styles a table named `id` takes its formatting from, base first.
    /// A table that names none uses the default table style.
    pub fn table_styles(&self, id: Option<&str>) -> Vec<&'a Style> {
        self.styles.chain(id, StyleKind::Table)
    }

    /// Effective properties of a table: its styles', then its own.
    ///
    /// The style named `Normal Table` is built into Word: whatever the file
    /// says of it, its cells have margins of 0.075 inch left and right and
    /// none above and below. A table that names no style has that style,
    /// and so has one whose style is based on it. A table whose styles state
    /// no margins and do not lead back to it has none. (Measured in Word:
    /// docx4j's probe `styles-table-default`.)
    pub fn table(&self, direct: &TableProps) -> TableProps {
        const BUILT_IN: CellMargins = CellMargins { top: Some(0), left: Some(108), bottom: Some(0), right: Some(108) };
        let chain = self.table_styles(direct.style_id.as_deref());
        let mut out = TableProps::default();
        if chain.is_empty() && direct.style_id.is_none() {
            out.cell_margins = BUILT_IN;
        }
        for style in chain {
            let mut props = style.table.clone();
            if style.name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case("Normal Table")) {
                props.cell_margins = BUILT_IN;
            }
            out.apply(&props);
        }
        out.apply(direct);
        let margins = &mut out.cell_margins;
        (margins.left, margins.right) = (margins.left.or(Some(0)), margins.right.or(Some(0)));
        (margins.top, margins.bottom) = (margins.top.or(Some(0)), margins.bottom.or(Some(0)));
        out
    }

    /// As [`Resolver::paragraph`], for a paragraph inside a table whose
    /// styles are `table` (see [`Resolver::table_styles`]). A table style's
    /// paragraph formatting comes after the document defaults and before the
    /// paragraph's own style.
    pub fn paragraph_in(&self, direct: &ParaProps, table: &[&Style]) -> ParaProps {
        let chain = self.styles.chain(direct.style_id.as_deref(), StyleKind::Paragraph);
        let mut out = self.styles.default_para.clone();
        for style in table {
            out.apply(&style.para);
        }
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
                // The level is the one stated, by the paragraph or by its
                // style, and the first when neither states one. That a level
                // names the style with `w:pStyle` does not put the style's
                // paragraphs at that level (measured in Word, against
                // ECMA-376 17.9.24).
                let ilvl = stated_level.and_then(|l| usize::try_from(l).ok()).unwrap_or(0).min(8);
                let numbered = direct_list.is_some() || self.owns_level(&chain, num_id, ilvl);
                out.numbering = None;
                if let Some(level) = self.numbering.level(num_id, ilvl) {
                    // The level's indents beat the style's; direct formatting beats both.
                    // They apply even to a paragraph the level does not number.
                    out.apply(&level.para);
                    out.apply(direct);
                    out.style_id = chain.last().map(|s| s.id.clone());
                    out.numbering = numbered.then_some(NumRef { num_id: Some(num_id), level: Some(ilvl as i32) });
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

    /// Whether level `ilvl` of list `num_id` numbers a paragraph that got the
    /// list from its style, where `chain` is the paragraph's style chain.
    ///
    /// Measured in Word (docx4j's probes `numbering-linked-level-other-style`,
    /// `numbering-unlinked-level-claim` and `numbering-shared-abstract-styles`):
    /// a level belongs to one paragraph style. That is the style the level
    /// names with `w:pStyle`, or else the first style in the styles part
    /// whose own `w:numPr` names the level, through any instance of the
    /// list's definition. Paragraphs of that style, or of a style based on
    /// it, are numbered. Paragraphs of any other style that carries the list
    /// get the level's indents but no label, and are not counted.
    fn owns_level(&self, chain: &[&Style], num_id: i32, ilvl: usize) -> bool {
        let owner = self.numbering.level_style(num_id, ilvl).or_else(|| {
            let claims = |style: &&Style| {
                style.kind == StyleKind::Paragraph
                    && style.para.numbering.as_ref().is_some_and(|own| {
                        own.num_id.is_some_and(|id| self.numbering.same_definition(id, num_id))
                            && own.level.map_or(0, |level| level.max(0) as usize) == ilvl
                    })
            };
            self.styles.in_order().find(claims).map(|style| style.id.as_str())
        });
        owner.is_none_or(|owner| chain.iter().any(|style| style.id == owner))
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
        self.run_in(paragraph_style, direct, &[])
    }

    /// As [`Resolver::run`], for a run inside a table whose styles are `table`.
    pub fn run_in(&self, paragraph_style: Option<&str>, direct: &RunProps, table: &[&Style]) -> RunProps {
        let defaults = &self.styles.default_run;
        let from_table = flatten(table);
        let from_paragraph = flatten(&self.styles.chain(paragraph_style, StyleKind::Paragraph));
        let character_chain = self.styles.chain(direct.style_id.as_deref(), StyleKind::Character);
        let mut from_character = flatten(&character_chain);
        // Word does not draw a highlight that comes from a character style,
        // although it does draw one from a paragraph style.
        from_character.highlight = None;

        let mut out = defaults.clone();
        out.apply(&from_table);
        out.apply(&from_paragraph);
        out.apply(&from_character);
        out.apply(direct);

        let levels = [&from_table, &from_paragraph, &from_character];
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
        let typeface = match self.theme {
            Some(theme) => theme.resolve(name, self.theme_lang),
            // A document with no theme part gets the fonts of the Office
            // theme for its Latin theme references (measured in Word:
            // docx4j's probe `theme-fonts-no-theme-part`).
            None if name.ends_with("Ascii") || name.ends_with("HAnsi") => {
                Some(if name.starts_with("major") { "Cambria" } else { "Calibri" })
            }
            None => None,
        };
        if let Some(typeface) = typeface {
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
