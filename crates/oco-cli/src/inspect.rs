//! `oco inspect`: print one document's content with its effective formatting.

use std::process::ExitCode;

use oco_docx::{
    Block, Color, Docx, FontRef, Highlight, ParaProps, Paragraph, Resolver, RevisionKind, RunProps, Underline,
    VertAlign,
};
use serde_json::{json, Value};

pub fn run(path: &str, as_json: bool) -> ExitCode {
    let docx = match Docx::open(path) {
        Ok(docx) => docx,
        Err(error) => {
            eprintln!("oco: {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    if as_json {
        let mut value = json!({
            "body": blocks_json(&docx.document.body, &docx.resolver()),
            "unsupported": docx.coverage.unsupported,
        });
        strip_nulls(&mut value);
        println!("{}", serde_json::to_string(&value).expect("JSON values always serialise"));
    } else {
        let mut counter = 0;
        print_blocks(&docx.document.body, &docx, 0, &mut counter);
        print_coverage(&docx);
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------- text output

fn print_blocks(blocks: &[Block], docx: &Docx, indent: usize, counter: &mut usize) {
    let pad = "  ".repeat(indent);
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                *counter += 1;
                print_paragraph(paragraph, docx, &pad, *counter);
            }
            Block::Table(table) => {
                let columns = table.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
                println!("{pad}TABLE {} rows x {columns} columns", table.rows.len());
                for (r, row) in table.rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        println!("{pad}  cell [{r},{c}]");
                        print_blocks(&cell.blocks, docx, indent + 2, counter);
                    }
                }
            }
        }
    }
}

fn print_paragraph(paragraph: &Paragraph, docx: &Docx, pad: &str, number: usize) {
    let resolver = docx.resolver();
    let props = resolver.paragraph(&paragraph.props);
    println!("{pad}#{number} P {}", describe_paragraph(&props, docx));
    for run in &paragraph.runs {
        let props = resolver.run(paragraph.props.style_id.as_deref(), &run.props);
        let revision = match &run.revision {
            Some(revision) => {
                let verb = match revision.kind {
                    RevisionKind::Inserted => "inserted",
                    RevisionKind::Deleted => "deleted",
                    RevisionKind::MovedFrom => "moved-from",
                    RevisionKind::MovedTo => "moved-to",
                };
                format!(" [{verb} by {}]", revision.author.as_deref().unwrap_or("?"))
            }
            None => String::new(),
        };
        println!("{pad}    {:?}{revision}  {}", run.text(), describe_run(&props));
    }
}

fn describe_paragraph(props: &ParaProps, docx: &Docx) -> String {
    let mut parts = Vec::new();
    if let Some(id) = &props.style_id {
        let name = docx.styles.get(id).and_then(|s| s.name.as_deref());
        parts.push(match name {
            Some(name) if name != id => format!("style={id} ({name:?})"),
            _ => format!("style={id}"),
        });
    }
    let mut field = |label: &str, value: Option<String>| {
        if let Some(value) = value {
            parts.push(format!("{label}={value}"));
        }
    };
    field("align", props.justification.clone());
    field("outline", props.outline_level.map(|v| v.to_string()));
    field("before", props.spacing.before.map(twips));
    field("after", props.spacing.after.map(twips));
    field(
        "line",
        props.spacing.line.map(|line| match props.spacing.line_rule.as_deref() {
            Some("exact") => format!("exactly {}", twips(line)),
            Some("atLeast") => format!("at least {}", twips(line)),
            _ => format!("{:.2}x", f64::from(line) / 240.0),
        }),
    );
    field("left", props.indent.left.map(twips));
    field("right", props.indent.right.map(twips));
    field("first-line", props.indent.first_line_chars.map(chars).or(props.indent.first_line.map(twips)));
    field("hanging", props.indent.hanging_chars.map(chars).or(props.indent.hanging.map(twips)));
    field("list", props.numbering.as_ref().and_then(|n| n.num_id).map(|id| id.to_string()));
    parts.join(" ")
}

fn describe_run(props: &RunProps) -> String {
    let mut parts = Vec::new();
    let font = |slot: &Option<FontRef>| match slot {
        Some(FontRef::Name(name)) => name.clone(),
        Some(FontRef::Theme(theme)) => format!("<theme {theme}>"),
        None => "-".to_owned(),
    };
    if props.fonts.ascii.is_some() || props.fonts.east_asia.is_some() {
        parts.push(format!("font={} / {}", font(&props.fonts.ascii), font(&props.fonts.east_asia)));
    }
    if let Some(size) = props.size {
        parts.push(format!("{}pt", f64::from(size) / 2.0));
    }
    let flags = [
        (props.bold, "bold"),
        (props.italic, "italic"),
        (props.strike, "strike"),
        (props.caps, "caps"),
        (props.small_caps, "small-caps"),
        (props.vanish, "hidden"),
    ];
    parts.extend(flags.iter().filter(|(on, _)| *on == Some(true)).map(|(_, name)| (*name).to_owned()));
    if let Some(underline) = props.underline.filter(|u| *u != Underline::None) {
        parts.push(format!("underline={}", underline.as_str()));
    }
    if let Some(color) = props.color.filter(|c| *c != Color::Auto) {
        parts.push(format!("color={color}"));
    }
    if let Some(highlight) = props.highlight.filter(|h| *h != Highlight::None) {
        parts.push(format!("highlight={}", highlight.as_str()));
    }
    if let Some(align) = props.vert_align.filter(|v| *v != VertAlign::Baseline) {
        parts.push(align.as_str().to_owned());
    }
    if let Some(style) = &props.style_id {
        parts.push(format!("char-style={style}"));
    }
    parts.join(" ")
}

/// Twips (1/20 pt) as points.
fn twips(value: i32) -> String {
    format!("{}pt", f64::from(value) / 20.0)
}

/// Hundredths of a character as characters.
fn chars(value: i32) -> String {
    format!("{}ch", f64::from(value) / 100.0)
}

fn print_coverage(docx: &Docx) {
    let ranked = docx.coverage.ranked();
    if ranked.is_empty() {
        println!("\nAll markup in this document is modelled.");
        return;
    }
    println!("\nNot modelled yet ({} kinds):", ranked.len());
    for (name, count) in ranked {
        println!("{count:>7}  {name}");
    }
}

// ---------------------------------------------------------------- JSON output

fn blocks_json(blocks: &[Block], resolver: &Resolver) -> Value {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(paragraph) => {
                let runs: Vec<Value> = paragraph
                    .runs
                    .iter()
                    .map(|run| {
                        json!({
                            "text": run.text(),
                            "props": resolver.run(paragraph.props.style_id.as_deref(), &run.props),
                            "revision": run.revision,
                        })
                    })
                    .collect();
                json!({ "type": "paragraph", "props": resolver.paragraph(&paragraph.props), "runs": runs })
            }
            Block::Table(table) => {
                let rows: Vec<Value> = table
                    .rows
                    .iter()
                    .map(|row| row.cells.iter().map(|cell| blocks_json(&cell.blocks, resolver)).collect())
                    .collect();
                json!({ "type": "table", "rows": rows })
            }
        })
        .collect()
}

/// Drops unspecified properties so the output shows only what is set.
fn strip_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            map.values_mut().for_each(strip_nulls);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}
