//! The main document part (`word/document.xml`): block and inline structure.
//!
//! Properties stored here are the *direct* formatting found in the file. Use
//! [`crate::Resolver`] to compute what a paragraph or run actually looks like
//! once styles and document defaults are applied.

use std::collections::HashSet;

use crate::xmltree::Node;
#[cfg(feature = "serde")]
use serde::Serialize;

use crate::coverage::Coverage;
use crate::drawing::{Drawing, TextBox};
use crate::props::{ParaProps, RunProps};
use crate::section::Section;
use crate::table::{CellProps, RowProps, TableProps};
use crate::xml::{elements, mc_branch, preserves_space, text_of, w_attr, w_name};

/// Tables nested deeper than this are dropped rather than risking the stack on
/// a hostile file. Word itself stops far earlier.
const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Document {
    pub body: Vec<Block>,
    /// Page setup of the last section, or of the whole document when it has
    /// no section breaks.
    pub section: Section,
}

// Paragraphs are by far the common case, so boxing them to shrink the rarer
// `Table` variant would cost an allocation per paragraph for nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Paragraph {
    pub props: ParaProps,
    /// Formatting of the paragraph mark. It sets the height of an empty paragraph.
    pub mark_props: RunProps,
    pub runs: Vec<Run>,
    /// Set on the last paragraph of a section other than the final one: the
    /// page setup of the section that ends here.
    pub section_break: Option<Section>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Run {
    pub props: RunProps,
    pub content: Vec<RunContent>,
    /// Set when the run sits inside a tracked change.
    pub revision: Option<Revision>,
    /// The text boxes among the run's drawings ([`Drawing::text_box`]).
    pub text_boxes: Vec<TextBox>,
    /// Where the parts of complex fields fall in `content`, while the
    /// paragraph is being read; empty afterwards.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) field_marks: Vec<(usize, FieldMark)>,
}

/// A footnote or an endnote (`w:footnote`, `w:endnote`).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Note {
    /// What the references to the note name it by.
    pub id: String,
    pub kind: NoteKind,
    pub blocks: Vec<Block>,
}

/// What a note is for. Only `Normal` notes are referred to from the text;
/// the others hold the lines drawn between the text and the notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum NoteKind {
    Normal,
    Separator,
    ContinuationSeparator,
    ContinuationNotice,
}

/// A part of a complex field (`w:fldChar`, `w:instrText`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldMark {
    Begin,
    Code(String),
    Separate,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RunContent {
    Text(String),
    Tab,
    Break(BreakKind),
    /// `w:drawing`: a picture or another object, in the line or anchored to it.
    Drawing(Box<Drawing>),
    /// `w:footnoteReference`, `w:endnoteReference`: where a note is
    /// referred to; it shows the note's number, unless the run that follows
    /// supplies a mark of its own (`custom`).
    NoteReference {
        endnote: bool,
        id: String,
        custom: bool,
    },
    /// `w:footnoteRef`, `w:endnoteRef`: in a note, the note's own number.
    NoteMark,
    /// `w:separator`, `w:continuationSeparator`: the line between the text
    /// and the notes under it; `true` for the one where a note runs on from
    /// the page before.
    NoteSeparator(bool),
    /// `w:sym`: one character drawn in `font` whatever the run's own font is.
    Symbol {
        font: Option<String>,
        character: char,
    },
    /// `w:lastRenderedPageBreak`: not content, but Word's record that a page
    /// began at this point when it last laid the document out. It is ground
    /// truth for pagination that comes with the file.
    PageBreakHint,
    /// A `PAGE` field: the number of the page the run is on. Holds the text
    /// Word last showed for it, which is all a reader that does not lay the
    /// document out has to go on.
    PageNumber(String),
    /// A `NUMPAGES` field: how many pages the document has. Holds the text
    /// Word last showed for it.
    PageCount(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum BreakKind {
    Line,
    Page,
    Column,
}

/// A tracked change (`w:ins`, `w:del`, `w:moveFrom`, `w:moveTo`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: Option<String>,
    /// ISO 8601 timestamp as written in the file.
    pub date: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
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

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Table {
    pub props: TableProps,
    /// `w:tblGrid`: the width of each column, in twips.
    pub grid: Vec<i32>,
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TableRow {
    pub props: RowProps,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TableCell {
    pub props: CellProps,
    pub blocks: Vec<Block>,
    /// The cell holds something that takes room and is not read: a
    /// picture, a shape, an embedded object or a formula. How wide its
    /// content is cannot then be told from `blocks`.
    pub unmeasured: bool,
}

impl Run {
    /// The run's text, with tabs and breaks as `\t` and `\n`.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .map(|c| match c {
                RunContent::Text(t) => t.clone(),
                RunContent::Tab => "\t".to_owned(),
                RunContent::Break(_) => "\n".to_owned(),
                RunContent::Symbol { character, .. } => character.to_string(),
                RunContent::PageBreakHint
                | RunContent::Drawing(_)
                | RunContent::NoteReference { .. }
                | RunContent::NoteMark
                | RunContent::NoteSeparator(_) => String::new(),
                RunContent::PageNumber(shown) | RunContent::PageCount(shown) => shown.clone(),
            })
            .collect()
    }
}

impl Paragraph {
    /// The text as it reads with all tracked changes accepted.
    pub fn text(&self) -> String {
        self.runs
            .iter()
            .filter(|r| !r.revision.as_ref().is_some_and(|rev| rev.kind.removes_content()))
            .map(Run::text)
            .collect()
    }
}

impl Document {
    /// The content of a header or footer part (`w:hdr`, `w:ftr`).
    pub(crate) fn parse_story(xml: &str, coverage: &mut Coverage) -> Result<Vec<Block>, crate::xmltree::Error> {
        let tree = crate::xmltree::Document::parse(xml)?;
        let mut blocks = Vec::new();
        parse_blocks(tree.root_element(), 0, &mut blocks, coverage);
        Ok(blocks)
    }

    /// The notes of a footnotes or endnotes part (`w:footnotes`, `w:endnotes`).
    pub(crate) fn parse_notes(xml: &str, coverage: &mut Coverage) -> Result<Vec<Note>, crate::xmltree::Error> {
        let tree = crate::xmltree::Document::parse(xml)?;
        let mut notes = Vec::new();
        for node in elements(tree.root_element()).filter(|n| matches!(w_name(*n), Some("footnote" | "endnote"))) {
            let kind = match w_attr(node, "type") {
                Some("separator") => NoteKind::Separator,
                Some("continuationSeparator") => NoteKind::ContinuationSeparator,
                Some("continuationNotice") => NoteKind::ContinuationNotice,
                _ => NoteKind::Normal,
            };
            let mut blocks = Vec::new();
            parse_blocks(node, 0, &mut blocks, coverage);
            notes.push(Note { id: w_attr(node, "id").unwrap_or_default().to_owned(), kind, blocks });
        }
        Ok(notes)
    }

    pub(crate) fn parse(xml: &str, coverage: &mut Coverage) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut document = Document::default();
        read_root(doc.root_element(), &mut document, coverage);
        Ok(document)
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
            | "moveFromRangeStart"
            | "moveFromRangeEnd"
            | "moveToRangeStart"
            | "moveToRangeEnd"
    )
}

/// Calls `f` for each child element, after resolving compatibility markup:
/// an `mc:AlternateContent` is replaced by the children of the branch a
/// consumer of our vintage takes.
fn for_each_child<'a, 'i>(parent: Node<'a, 'i>, f: &mut dyn FnMut(Node<'a, 'i>)) {
    for child in elements(parent) {
        match mc_branch(child) {
            Some(branch) => {
                if let Some(branch) = branch {
                    for_each_child(branch, f);
                }
            }
            None => f(child),
        }
    }
}

/// Reads an element this context does not recognise.
///
/// Word does not skip an unknown element in the WordprocessingML namespace:
/// it reads straight through it, as if its children stood in its place. So
/// `read` is given the element as a parent, into scratch storage; if that
/// turns up content the element was a wrapper and the content is kept.
/// Otherwise, and for elements of any other namespace, the element is
/// reported as unmodelled.
fn read_unknown<T>(
    node: Node,
    context: Option<&str>,
    out: &mut Vec<T>,
    coverage: &mut Coverage,
    read: impl FnOnce(&mut Vec<T>, &mut Coverage),
) {
    if w_name(node).is_some() {
        let mut found = Vec::new();
        let mut seen = Coverage::default();
        read(&mut found, &mut seen);
        if !found.is_empty() {
            out.append(&mut found);
            coverage.merge(seen);
            return;
        }
    }
    coverage.miss(context, node);
}

/// Reads the root element of the main part. Normally that is `w:document`
/// holding one `w:body`; Word also copes with compatibility markup at the
/// root, with several documents or bodies (it joins them), and with body
/// content standing where the document element should be.
fn read_root(node: Node, document: &mut Document, coverage: &mut Coverage) {
    match mc_branch(node) {
        Some(branch) => {
            for child in branch.into_iter().flat_map(elements) {
                read_root(child, document, coverage);
            }
        }
        None => match w_name(node) {
            Some("document") => for_each_child(node, &mut |child| {
                if w_name(child) == Some("body") {
                    read_body(child, document, coverage);
                }
            }),
            Some("body") => read_body(node, document, coverage),
            Some(_) => parse_block(node, false, 0, &mut document.body, coverage),
            None => {}
        },
    }
}

fn read_body(body: Node, document: &mut Document, coverage: &mut Coverage) {
    parse_blocks(body, 0, &mut document.body, coverage);
    // The body's own `w:sectPr` describes the final section.
    if let Some(section) = elements(body).filter(|n| w_name(*n) == Some("sectPr")).last() {
        document.section = Section::parse(section, coverage);
    }
}

fn parse_blocks(parent: Node, depth: usize, blocks: &mut Vec<Block>, coverage: &mut Coverage) {
    let in_cell = w_name(parent) == Some("tc");
    for_each_child(parent, &mut |child| parse_block(child, in_cell, depth, blocks, coverage));
}

fn parse_block(child: Node, in_cell: bool, depth: usize, blocks: &mut Vec<Block>, coverage: &mut Coverage) {
    match w_name(child) {
        Some("p") => blocks.push(Block::Paragraph(parse_paragraph(child, coverage))),
        Some("tbl") if depth < MAX_NESTING => {
            blocks.push(Block::Table(parse_table(child, depth + 1, coverage)));
        }
        Some("tbl") => coverage.note("w:tbl (nested too deeply, dropped)"),
        // Wrappers whose content is ordinary block content.
        Some("sdt") => for_each_child(child, &mut |part| {
            if w_name(part) == Some("sdtContent") {
                parse_blocks(part, depth, blocks, coverage);
            }
        }),
        Some("customXml") => parse_blocks(child, depth, blocks, coverage),
        // Not modelled yet. Named so that they are not mistaken for wrappers.
        // The final section's properties; `read_body` reads them.
        Some("sectPr") if !in_cell => {}
        // Read where the cell is.
        Some("tcPr") if in_cell => {}
        Some("sectPr" | "tcPr" | "altChunk" | "customXmlPr") => coverage.miss(in_cell.then_some("w:tc"), child),
        Some(name) if is_ignorable(name) => {}
        _ => read_unknown(child, None, blocks, coverage, |found, seen| {
            parse_blocks(child, depth, found, seen);
        }),
    }
}

fn parse_table(node: Node, depth: usize, coverage: &mut Coverage) -> Table {
    let mut table = Table::default();
    for child in elements(node) {
        match w_name(child) {
            Some("tblPr") => table.props = TableProps::parse(child, coverage),
            Some("tblGrid") => table.grid = crate::table::parse_grid(child),
            _ => {}
        }
    }
    parse_rows(node, depth, &mut table.rows, coverage);
    table
}

fn parse_rows(parent: Node, depth: usize, rows: &mut Vec<TableRow>, coverage: &mut Coverage) {
    for_each_child(parent, &mut |child| match w_name(child) {
        Some("tr") => {
            let mut row = TableRow::default();
            if let Some(props) = elements(child).find(|n| w_name(*n) == Some("trPr")) {
                row.props = RowProps::parse(props, coverage);
            }
            parse_cells(child, depth, &mut row.cells, coverage);
            rows.push(row);
        }
        // A content control or custom XML element can wrap whole rows.
        Some("sdt") => for_each_child(child, &mut |part| {
            if w_name(part) == Some("sdtContent") {
                parse_rows(part, depth, rows, coverage);
            }
        }),
        Some("customXml") => parse_rows(child, depth, rows, coverage),
        // Read by `parse_table`.
        Some("tblPr" | "tblGrid") => {}
        Some("tblPrEx" | "customXmlPr") => coverage.miss(Some("w:tbl"), child),
        Some(name) if is_ignorable(name) => {}
        _ => read_unknown(child, Some("w:tbl"), rows, coverage, |found, seen| {
            parse_rows(child, depth, found, seen);
        }),
    });
}

fn parse_cells(parent: Node, depth: usize, cells: &mut Vec<TableCell>, coverage: &mut Coverage) {
    for_each_child(parent, &mut |child| match w_name(child) {
        Some("tc") => {
            let mut cell = TableCell::default();
            if let Some(props) = elements(child).find(|n| w_name(*n) == Some("tcPr")) {
                cell.props = CellProps::parse(props, coverage);
            }
            let objects = coverage.objects();
            parse_blocks(child, depth, &mut cell.blocks, coverage);
            cell.unmeasured = coverage.objects() > objects;
            cells.push(cell);
        }
        // ... and so can whole cells, which is how Word stores a form field
        // that occupies a table cell.
        Some("sdt") => for_each_child(child, &mut |part| {
            if w_name(part) == Some("sdtContent") {
                parse_cells(part, depth, cells, coverage);
            }
        }),
        Some("customXml") => parse_cells(child, depth, cells, coverage),
        // Read where the row is.
        Some("trPr") => {}
        Some("tblPrEx" | "customXmlPr") => coverage.miss(Some("w:tr"), child),
        Some(name) if is_ignorable(name) => {}
        _ => read_unknown(child, Some("w:tr"), cells, coverage, |found, seen| {
            parse_cells(child, depth, found, seen);
        }),
    });
}

fn parse_paragraph(node: Node, coverage: &mut Coverage) -> Paragraph {
    let mut paragraph = Paragraph::default();
    // Repeated `w:pPr` elements are invalid; Word merges them.
    for ppr in elements(node).filter(|n| w_name(*n) == Some("pPr")) {
        paragraph.props.read(ppr, coverage);
        for child in elements(ppr) {
            match w_name(child) {
                Some("rPr") => paragraph.mark_props.read(child, coverage),
                Some("sectPr") => paragraph.section_break = Some(Section::parse(child, coverage)),
                _ => {}
            }
        }
    }
    parse_inlines(node, None, &mut paragraph.runs, coverage);
    resolve_fields(&mut paragraph.runs);
    paragraph
}

/// A field whose value depends on where the layout puts it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LiveField {
    Page,
    Pages,
}

fn live_field(instruction: &str) -> Option<LiveField> {
    match instruction.split_whitespace().next()?.to_ascii_uppercase().as_str() {
        "PAGE" => Some(LiveField::Page),
        "NUMPAGES" | "SECTIONPAGES" => Some(LiveField::Pages),
        _ => None,
    }
}

fn live_content(field: LiveField, shown: String) -> RunContent {
    match field {
        LiveField::Page => RunContent::PageNumber(shown),
        LiveField::Pages => RunContent::PageCount(shown),
    }
}

/// Turns the complex fields of a paragraph that show a page number or a
/// page count into [`RunContent::PageNumber`] and [`RunContent::PageCount`],
/// in place of the result Word stored. Other fields keep their stored
/// result, which is what a reader shows for them. The field marks are used up.
fn resolve_fields(runs: &mut [Run]) {
    struct Open {
        code: String,
        in_result: bool,
        /// Content of the result: (run, index in its content).
        result: Vec<(usize, usize)>,
    }
    if runs.iter().all(|run| run.field_marks.is_empty()) {
        return;
    }
    let mut open: Vec<Open> = Vec::new();
    // What happens to the content at (run, index): it goes, or something
    // takes its place; and what is put in before it.
    let mut dropped: HashSet<(usize, usize)> = HashSet::new();
    let mut inserted: Vec<((usize, usize), LiveField, String)> = Vec::new();
    for (run_index, run) in runs.iter().enumerate() {
        let mut marks = run.field_marks.iter().peekable();
        for position in 0..=run.content.len() {
            while let Some((_, mark)) = marks.next_if(|(at, _)| *at <= position) {
                match mark {
                    FieldMark::Begin => open.push(Open { code: String::new(), in_result: false, result: Vec::new() }),
                    FieldMark::Code(text) => {
                        if let Some(field) = open.last_mut().filter(|field| !field.in_result) {
                            field.code.push_str(text);
                        }
                    }
                    FieldMark::Separate => {
                        if let Some(field) = open.last_mut() {
                            field.in_result = true;
                        }
                    }
                    FieldMark::End => {
                        // Only a field that is not inside another: a page
                        // number inside an IF, say, is the IF's business.
                        let Some(field) = open.pop().filter(|_| open.is_empty()) else { continue };
                        let Some(live) = live_field(&field.code) else { continue };
                        let shown = field
                            .result
                            .iter()
                            .filter_map(|(run, index)| match &runs[*run].content[*index] {
                                RunContent::Text(text) => Some(text.as_str()),
                                _ => None,
                            })
                            .collect();
                        inserted.push((field.result.first().copied().unwrap_or((run_index, position)), live, shown));
                        dropped.extend(field.result);
                    }
                }
            }
            if position == run.content.len() {
                break;
            }
            if open.iter().any(|field| !field.in_result) {
                // Inside an instruction: not part of what the document shows.
                dropped.insert((run_index, position));
            } else if let [field] = open.as_mut_slice() {
                field.result.push((run_index, position));
            }
        }
    }
    for (run_index, run) in runs.iter_mut().enumerate() {
        run.field_marks.clear();
        let old = std::mem::take(&mut run.content);
        let places = old.len();
        let mut old = old.into_iter();
        for position in 0..=places {
            for (_, live, shown) in inserted.iter().filter(|(at, ..)| *at == (run_index, position)) {
                run.content.push(live_content(*live, shown.clone()));
            }
            if let Some(piece) = old.next().filter(|_| !dropped.contains(&(run_index, position))) {
                run.content.push(piece);
            }
        }
    }
}

/// Elements that belong inside a run. Word also honours them when they sit
/// directly in a paragraph, as some generators write them.
fn is_run_content(name: &str) -> bool {
    matches!(
        name,
        "t" | "delText" | "tab" | "br" | "cr" | "sym" | "noBreakHyphen" | "softHyphen" | "lastRenderedPageBreak"
    )
}

fn parse_inlines(parent: Node, revision: Option<&Revision>, runs: &mut Vec<Run>, coverage: &mut Coverage) {
    for_each_child(parent, &mut |child| {
        let tracked = |kind| Revision {
            kind,
            author: w_attr(child, "author").map(str::to_owned),
            date: w_attr(child, "date").map(str::to_owned),
            id: w_attr(child, "id").map(str::to_owned),
        };
        match w_name(child) {
            Some("r") => runs.push(parse_run(child, revision, coverage)),
            Some("pPr") => {}
            Some("ins") => parse_inlines(child, Some(&tracked(RevisionKind::Inserted)), runs, coverage),
            Some("del") => parse_inlines(child, Some(&tracked(RevisionKind::Deleted)), runs, coverage),
            Some("moveFrom") => parse_inlines(child, Some(&tracked(RevisionKind::MovedFrom)), runs, coverage),
            Some("moveTo") => parse_inlines(child, Some(&tracked(RevisionKind::MovedTo)), runs, coverage),
            // A field written in one piece: its instruction and, inside, the
            // result Word stored.
            Some("fldSimple") => {
                let mut result = Vec::new();
                parse_inlines(child, revision, &mut result, coverage);
                match w_attr(child, "instr").and_then(live_field) {
                    Some(live) => {
                        let shown: String = result.iter().map(Run::text).collect();
                        let mut run = result.into_iter().next().unwrap_or_default();
                        run.revision = revision.cloned();
                        run.content = vec![live_content(live, shown)];
                        runs.push(run);
                    }
                    None => runs.append(&mut result),
                }
            }
            // Wrappers whose content is ordinary inline content.
            Some("hyperlink" | "smartTag" | "customXml" | "dir" | "bdo") => {
                parse_inlines(child, revision, runs, coverage);
            }
            Some("sdt") => for_each_child(child, &mut |part| {
                if w_name(part) == Some("sdtContent") {
                    parse_inlines(part, revision, runs, coverage);
                }
            }),
            Some(name) if is_run_content(name) => {
                let mut run = Run { revision: revision.cloned(), ..Run::default() };
                read_run_content(child, name, &mut run.content, coverage);
                runs.push(run);
            }
            // Not modelled yet. Named so that they are not mistaken for wrappers.
            Some(
                "commentRangeStart"
                | "commentRangeEnd"
                | "customXmlPr"
                | "smartTagPr"
                | "subDoc"
                | "oMath"
                | "oMathPara"
                | "customXmlInsRangeStart"
                | "customXmlInsRangeEnd"
                | "customXmlDelRangeStart"
                | "customXmlDelRangeEnd"
                | "customXmlMoveFromRangeStart"
                | "customXmlMoveFromRangeEnd"
                | "customXmlMoveToRangeStart"
                | "customXmlMoveToRangeEnd",
            ) => coverage.miss(Some("w:p"), child),
            Some(name) if is_ignorable(name) => {}
            _ => read_unknown(child, Some("w:p"), runs, coverage, |found, seen| {
                parse_inlines(child, revision, found, seen);
            }),
        }
    });
}

fn parse_run(node: Node, revision: Option<&Revision>, coverage: &mut Coverage) -> Run {
    let mut run = Run { revision: revision.cloned(), ..Run::default() };
    parse_run_children(node, &mut run, coverage);
    run
}

fn parse_run_children(node: Node, run: &mut Run, coverage: &mut Coverage) {
    for_each_child(node, &mut |child| match w_name(child) {
        Some("rPr") => run.props.read(child, coverage),
        Some(name) if is_run_content(name) => read_run_content(child, name, &mut run.content, coverage),
        Some(name) if is_ignorable(name) => {}
        // The parts of a complex field; `resolve_fields` makes sense of them.
        Some("fldChar") => {
            let mark = match w_attr(child, "fldCharType") {
                Some("begin") => Some(FieldMark::Begin),
                Some("separate") => Some(FieldMark::Separate),
                Some("end") => Some(FieldMark::End),
                _ => None,
            };
            run.field_marks.extend(mark.map(|mark| (run.content.len(), mark)));
        }
        Some("instrText") => run.field_marks.push((run.content.len(), FieldMark::Code(text_of(child)))),
        Some("drawing") => {
            let mut drawing = crate::drawing::read(child);
            // A text box holds content of its own. One inside another is
            // not read: Word does not make them, and nesting must end.
            if let (Some(drawing), Some((content, mut text_box))) = (&mut drawing, crate::drawing::text_box(child)) {
                if coverage.boxes_open == 0 {
                    coverage.boxes_open += 1;
                    parse_blocks(content, 0, &mut text_box.blocks, coverage);
                    coverage.boxes_open -= 1;
                    drawing.text_box = Some(run.text_boxes.len());
                    run.text_boxes.push(text_box);
                }
            }
            // A picture or a text box is laid out and drawn. Any other
            // object is read, and given its room, but not shown yet.
            if !drawing.as_ref().is_some_and(|d| d.picture.is_some() || d.text_box.is_some()) {
                coverage.miss(Some("w:r"), child);
            }
            run.content.extend(drawing.map(|drawing| RunContent::Drawing(Box::new(drawing))));
        }
        Some(kind @ ("footnoteReference" | "endnoteReference")) => {
            run.content.push(RunContent::NoteReference {
                endnote: kind == "endnoteReference",
                id: w_attr(child, "id").unwrap_or_default().to_owned(),
                custom: crate::xml::on_off_attr(child, "customMarkFollows").unwrap_or(false),
            });
        }
        Some("footnoteRef" | "endnoteRef") => run.content.push(RunContent::NoteMark),
        Some("separator") => run.content.push(RunContent::NoteSeparator(false)),
        Some("continuationSeparator") => run.content.push(RunContent::NoteSeparator(true)),
        // Not modelled yet. Named so that they are not mistaken for wrappers.
        Some(
            "pict" | "object" | "delInstrText" | "commentReference" | "annotationRef" | "ruby" | "ptab" | "pgNum"
            | "dayShort" | "dayLong" | "monthShort" | "monthLong" | "yearShort" | "yearLong" | "contentPart",
        ) => coverage.miss(Some("w:r"), child),
        _ => {
            let before = run.content.len();
            let mut seen = Coverage::default();
            if w_name(child).is_some() {
                parse_run_children(child, run, &mut seen);
            }
            if run.content.len() > before {
                coverage.merge(seen);
            } else {
                coverage.miss(Some("w:r"), child);
            }
        }
    });
}

fn read_run_content(node: Node, name: &str, content: &mut Vec<RunContent>, coverage: &mut Coverage) {
    match name {
        // `w:delText` is the text of a run inside a deletion.
        "t" | "delText" => read_text(node, content),
        "tab" => content.push(RunContent::Tab),
        "cr" => content.push(RunContent::Break(BreakKind::Line)),
        "br" => content.push(RunContent::Break(break_kind(node))),
        "lastRenderedPageBreak" => content.push(RunContent::PageBreakHint),
        "noBreakHyphen" => push_text(content, "\u{2011}"),
        "softHyphen" => push_text(content, "\u{00AD}"),
        // A character addressed by code point in a named (usually symbol)
        // font, e.g. a Wingdings tick.
        "sym" => match w_attr(node, "char").and_then(|c| u32::from_str_radix(c, 16).ok()).and_then(char::from_u32) {
            Some(character) => {
                content.push(RunContent::Symbol { font: w_attr(node, "font").map(str::to_owned), character });
            }
            None => coverage.miss(Some("w:r"), node),
        },
        _ => unreachable!("is_run_content and read_run_content list the same names"),
    }
}

/// Text of a `w:t`. It should hold only characters; Word also honours a
/// break or another text element nested inside it, and drops anything else.
fn read_text(node: Node, content: &mut Vec<RunContent>) {
    if !node.children().any(|child| child.is_element()) {
        let text = text_of(node);
        return push_text(content, if preserves_space(node) { &text } else { text.trim() });
    }
    for child in node.children() {
        match w_name(child) {
            _ if child.is_text() => push_text(content, child.text().unwrap_or_default()),
            Some("br") => content.push(RunContent::Break(break_kind(child))),
            Some("t" | "delText") => read_text(child, content),
            _ => {}
        }
    }
}

fn break_kind(node: Node) -> BreakKind {
    match w_attr(node, "type") {
        Some("page") => BreakKind::Page,
        Some("column") => BreakKind::Column,
        _ => BreakKind::Line,
    }
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
