//! The main document part (`word/document.xml`): block and inline structure.
//!
//! Properties stored here are the *direct* formatting found in the file. Use
//! [`crate::Resolver`] to compute what a paragraph or run actually looks like
//! once styles and document defaults are applied.

use roxmltree::Node;
use serde::Serialize;

use crate::coverage::Coverage;
use crate::props::{ParaProps, RunProps};
use crate::xml::{
    elements, is_mc_alternate_content, mc_fallback, preserves_space, text_of, w_attr, w_child,
    w_name,
};

/// Tables nested deeper than this are dropped rather than risking the stack on
/// a hostile file. Word itself stops far earlier.
const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Document {
    pub body: Vec<Block>,
}

// Paragraphs are by far the common case, so boxing them to shrink the rarer
// `Table` variant would cost an allocation per paragraph for nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Paragraph {
    pub props: ParaProps,
    /// Formatting of the paragraph mark. It sets the height of an empty paragraph.
    pub mark_props: RunProps,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Run {
    pub props: RunProps,
    pub content: Vec<RunContent>,
    /// Set when the run sits inside a tracked change.
    pub revision: Option<Revision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunContent {
    Text(String),
    Tab,
    Break(BreakKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    Line,
    Page,
    Column,
}

/// A tracked change (`w:ins`, `w:del`, `w:moveFrom`, `w:moveTo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: Option<String>,
    /// ISO 8601 timestamp as written in the file.
    pub date: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    Inserted,
    Deleted,
    MovedFrom,
    MovedTo,
}

impl RevisionKind {
    /// Whether the content disappears once the change is accepted.
    pub fn removes_content(self) -> bool {
        matches!(self, RevisionKind::Deleted | RevisionKind::MovedFrom)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Table {
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TableCell {
    pub blocks: Vec<Block>,
}

impl Run {
    /// The run's text, with tabs and breaks as `\t` and `\n`.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .map(|c| match c {
                RunContent::Text(t) => t.as_str(),
                RunContent::Tab => "\t",
                RunContent::Break(_) => "\n",
            })
            .collect()
    }
}

impl Paragraph {
    /// The text as it reads with all tracked changes accepted.
    pub fn text(&self) -> String {
        self.runs
            .iter()
            .filter(|r| {
                !r.revision
                    .as_ref()
                    .is_some_and(|rev| rev.kind.removes_content())
            })
            .map(Run::text)
            .collect()
    }
}

impl Document {
    pub(crate) fn parse(xml: &str, coverage: &mut Coverage) -> Result<Self, roxmltree::Error> {
        let doc = roxmltree::Document::parse(xml)?;
        let body = w_child(doc.root_element(), "body")
            .map(|body| parse_blocks(body, 0, coverage))
            .unwrap_or_default();
        Ok(Document { body })
    }

    /// Every paragraph in document order, including those inside tables.
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
            for block in blocks {
                match block {
                    Block::Paragraph(p) => out.push(p),
                    Block::Table(t) => {
                        for cell in t.rows.iter().flat_map(|r| &r.cells) {
                            walk(&cell.blocks, out);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.body, &mut out);
        out
    }
}

/// Markers that carry no content of their own. Skipped without being reported.
fn is_ignorable(name: &str) -> bool {
    matches!(
        name,
        "bookmarkStart"
            | "bookmarkEnd"
            | "proofErr"
            | "permStart"
            | "permEnd"
            | "lastRenderedPageBreak"
            | "moveFromRangeStart"
            | "moveFromRangeEnd"
            | "moveToRangeStart"
            | "moveToRangeEnd"
    )
}

fn parse_blocks(parent: Node, depth: usize, coverage: &mut Coverage) -> Vec<Block> {
    let mut blocks = Vec::new();
    for child in elements(parent) {
        match w_name(child) {
            Some("p") => blocks.push(Block::Paragraph(parse_paragraph(child, coverage))),
            Some("tbl") if depth < MAX_NESTING => {
                blocks.push(Block::Table(parse_table(child, depth + 1, coverage)));
            }
            // Wrappers whose content is ordinary block content.
            Some("sdt") => {
                if let Some(content) = w_child(child, "sdtContent") {
                    blocks.extend(parse_blocks(content, depth, coverage));
                }
            }
            Some("customXml") => blocks.extend(parse_blocks(child, depth, coverage)),
            // Cell properties, met when `parent` is a table cell.
            Some("tcPr") => coverage.miss(Some("w:tc"), child),
            Some(name) if is_ignorable(name) => {}
            None if is_mc_alternate_content(child) => {
                if let Some(fallback) = mc_fallback(child) {
                    blocks.extend(parse_blocks(fallback, depth, coverage));
                }
            }
            _ => coverage.miss(None, child),
        }
    }
    blocks
}

fn parse_table(node: Node, depth: usize, coverage: &mut Coverage) -> Table {
    let mut table = Table::default();
    for child in elements(node) {
        match w_name(child) {
            Some("tr") => {
                let mut row = TableRow::default();
                for cell in elements(child) {
                    match w_name(cell) {
                        Some("tc") => row.cells.push(TableCell {
                            blocks: parse_blocks(cell, depth, coverage),
                        }),
                        Some(name) if is_ignorable(name) => {}
                        _ => coverage.miss(Some("w:tr"), cell),
                    }
                }
                table.rows.push(row);
            }
            Some(name) if is_ignorable(name) => {}
            _ => coverage.miss(Some("w:tbl"), child),
        }
    }
    table
}

fn parse_paragraph(node: Node, coverage: &mut Coverage) -> Paragraph {
    let mut paragraph = Paragraph::default();
    if let Some(ppr) = w_child(node, "pPr") {
        paragraph.props = ParaProps::parse(ppr, coverage);
        if let Some(rpr) = w_child(ppr, "rPr") {
            paragraph.mark_props = RunProps::parse(rpr, coverage);
        }
    }
    parse_inlines(node, None, &mut paragraph.runs, coverage);
    paragraph
}

fn parse_inlines(
    parent: Node,
    revision: Option<&Revision>,
    runs: &mut Vec<Run>,
    coverage: &mut Coverage,
) {
    for child in elements(parent) {
        let tracked = |kind| Revision {
            kind,
            author: w_attr(child, "author").map(str::to_owned),
            date: w_attr(child, "date").map(str::to_owned),
            id: w_attr(child, "id").map(str::to_owned),
        };
        match w_name(child) {
            Some("r") => runs.push(parse_run(child, revision, coverage)),
            Some("pPr") => {}
            Some("ins") => parse_inlines(
                child,
                Some(&tracked(RevisionKind::Inserted)),
                runs,
                coverage,
            ),
            Some("del") => {
                parse_inlines(child, Some(&tracked(RevisionKind::Deleted)), runs, coverage)
            }
            Some("moveFrom") => parse_inlines(
                child,
                Some(&tracked(RevisionKind::MovedFrom)),
                runs,
                coverage,
            ),
            Some("moveTo") => {
                parse_inlines(child, Some(&tracked(RevisionKind::MovedTo)), runs, coverage)
            }
            // Wrappers whose content is ordinary inline content.
            Some("hyperlink" | "smartTag" | "customXml" | "fldSimple" | "dir" | "bdo") => {
                parse_inlines(child, revision, runs, coverage);
            }
            Some("sdt") => {
                if let Some(content) = w_child(child, "sdtContent") {
                    parse_inlines(content, revision, runs, coverage);
                }
            }
            Some(name) if is_ignorable(name) => {}
            None if is_mc_alternate_content(child) => {
                if let Some(fallback) = mc_fallback(child) {
                    parse_inlines(fallback, revision, runs, coverage);
                }
            }
            _ => coverage.miss(Some("w:p"), child),
        }
    }
}

fn parse_run(node: Node, revision: Option<&Revision>, coverage: &mut Coverage) -> Run {
    let mut run = Run {
        revision: revision.cloned(),
        ..Run::default()
    };
    for child in elements(node) {
        match w_name(child) {
            Some("rPr") => run.props = RunProps::parse(child, coverage),
            // `w:delText` is the text of a run inside a deletion.
            Some("t" | "delText") => {
                let text = text_of(child);
                let text = if preserves_space(child) {
                    text
                } else {
                    text.trim().to_owned()
                };
                push_text(&mut run.content, &text);
            }
            Some("tab") => run.content.push(RunContent::Tab),
            Some("cr") => run.content.push(RunContent::Break(BreakKind::Line)),
            Some("br") => run
                .content
                .push(RunContent::Break(match w_attr(child, "type") {
                    Some("page") => BreakKind::Page,
                    Some("column") => BreakKind::Column,
                    _ => BreakKind::Line,
                })),
            Some("noBreakHyphen") => push_text(&mut run.content, "\u{2011}"),
            Some("softHyphen") => push_text(&mut run.content, "\u{00AD}"),
            Some(name) if is_ignorable(name) => {}
            _ => coverage.miss(Some("w:r"), child),
        }
    }
    run
}

fn push_text(content: &mut Vec<RunContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    match content.last_mut() {
        Some(RunContent::Text(last)) => last.push_str(text),
        _ => content.push(RunContent::Text(text.to_owned())),
    }
}
