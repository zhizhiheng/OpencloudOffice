//! The style definitions part (`word/styles.xml`).

use std::collections::{HashMap, HashSet};

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::coverage::Coverage;
use crate::props::{ParaProps, RunProps};
use crate::table::TableProps;
use crate::xml::{elements, val, w_attr, w_child, w_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Style {
    pub id: String,
    /// Display name, e.g. `heading 1`. Ids are locale-dependent; names are not.
    pub name: Option<String>,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    pub is_default: bool,
    pub para: ParaProps,
    pub run: RunProps,
    /// `w:tblPr` of a table style.
    pub table: TableProps,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct StyleSheet {
    /// `w:docDefaults`: the base of every paragraph's and run's formatting.
    pub default_para: ParaProps,
    pub default_run: RunProps,
    styles: HashMap<String, Style>,
    /// Style ids in the order the part defines them, which Word consults
    /// when two styles lay claim to one list level.
    order: Vec<String>,
    default_paragraph_style: Option<String>,
    default_character_style: Option<String>,
    default_table_style: Option<String>,
}

impl StyleSheet {
    pub(crate) fn parse(xml: &str, coverage: &mut Coverage) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut sheet = StyleSheet::default();
        for node in elements(doc.root_element()) {
            match w_name(node) {
                Some("docDefaults") => {
                    let props_of = |wrapper: &str, inner: &str| w_child(node, wrapper).and_then(|n| w_child(n, inner));
                    if let Some(rpr) = props_of("rPrDefault", "rPr") {
                        sheet.default_run = RunProps::parse(rpr, coverage);
                    }
                    if let Some(ppr) = props_of("pPrDefault", "pPr") {
                        sheet.default_para = ParaProps::parse(ppr, coverage);
                    }
                }
                Some("style") => sheet.add(node, coverage),
                // The list of built-in styles Word may show; carries no formatting.
                Some("latentStyles") => {}
                _ => coverage.miss(Some("w:styles"), node),
            }
        }
        Ok(sheet)
    }

    fn add(&mut self, node: crate::xmltree::Node, coverage: &mut Coverage) {
        let Some(id) = w_attr(node, "styleId") else {
            return;
        };
        let kind = match w_attr(node, "type") {
            Some("character") => StyleKind::Character,
            Some("table") => StyleKind::Table,
            Some("numbering") => StyleKind::Numbering,
            _ => StyleKind::Paragraph,
        };
        let mut style = Style {
            id: id.to_owned(),
            name: None,
            kind,
            based_on: None,
            is_default: matches!(w_attr(node, "default"), Some("1" | "true" | "on")),
            para: ParaProps::default(),
            run: RunProps::default(),
            table: TableProps::default(),
        };
        for child in elements(node) {
            match w_name(child) {
                Some("name") => style.name = val(child).map(str::to_owned),
                Some("basedOn") => style.based_on = val(child).map(str::to_owned),
                Some("pPr") => style.para = ParaProps::parse(child, coverage),
                Some("rPr") => style.run = RunProps::parse(child, coverage),
                Some("tblPr") => style.table = TableProps::parse(child, coverage),
                // Row, cell and conditional formatting of table styles is
                // not modelled yet; report it.
                Some("trPr" | "tcPr" | "tblStylePr") => coverage.miss(Some("w:style"), child),
                // Everything else is UI metadata (next, link, uiPriority, qFormat, ...).
                _ => {}
            }
        }
        if style.is_default {
            match kind {
                StyleKind::Paragraph => self.default_paragraph_style = Some(style.id.clone()),
                StyleKind::Character => self.default_character_style = Some(style.id.clone()),
                StyleKind::Table => self.default_table_style = Some(style.id.clone()),
                _ => {}
            }
        }
        if !self.styles.contains_key(&style.id) {
            self.order.push(style.id.clone());
        }
        self.styles.insert(style.id.clone(), style);
    }

    /// Every style, in the order the styles part defines them.
    pub fn in_order(&self) -> impl Iterator<Item = &Style> {
        self.order.iter().filter_map(|id| self.styles.get(id))
    }

    pub fn get(&self, id: &str) -> Option<&Style> {
        self.styles.get(id)
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }

    /// The inheritance chain for `id`, base style first. A missing or unknown
    /// id (or one naming a style of another kind) falls back to the default
    /// style of `kind`, as Word does.
    pub fn chain(&self, id: Option<&str>, kind: StyleKind) -> Vec<&Style> {
        let default = match kind {
            StyleKind::Paragraph => self.default_paragraph_style.as_deref(),
            StyleKind::Character => self.default_character_style.as_deref(),
            StyleKind::Table => self.default_table_style.as_deref(),
            _ => None,
        };
        let of_kind = |id: &str| self.styles.get(id).filter(|s| s.kind == kind);
        let mut current = id.and_then(of_kind).or_else(|| default.and_then(of_kind));

        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        while let Some(style) = current {
            // `basedOn` cycles are invalid but must not hang the reader.
            if !seen.insert(style.id.as_str()) {
                break;
            }
            chain.push(style);
            current = style.based_on.as_deref().and_then(of_kind);
        }
        chain.reverse();
        chain
    }
}
