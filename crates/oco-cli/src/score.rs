//! `oco score`: measure layout against what Word itself recorded.
//!
//! A document saved by Word carries Word's own account of its layout:
//!
//! - the number of pages, in `docProps/app.xml`;
//! - a `w:lastRenderedPageBreak` at each place a page began.
//!
//! Laying such a document out and comparing gives a fidelity score with Word
//! as the reference and no copy of Word in the loop. The record can be
//! stale (the file may have been edited by another tool since Word wrote
//! it), so a document is only trusted when the character count Word
//! recorded fits the text that is actually there.
//!
//! Of the counts Word records, only the one of characters with spaces is
//! used. The others are often estimates derived from it (words as a fixed
//! share of the characters, one line and one paragraph whatever the text),
//! and the line count disagrees by half in documents whose every page start
//! agrees.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use oco_docx::{Block, BreakKind, Docx, Section, SectionStart};
use oco_layout::{content, Content, FontDb, Options};
use serde::Serialize;

pub struct Arguments {
    roots: Vec<PathBuf>,
    json: Option<PathBuf>,
    jobs: usize,
    /// Show each recorded page start of every document next to ours.
    explain: bool,
}

impl Arguments {
    pub fn parse(args: &[&str]) -> Option<Self> {
        let mut parsed = Arguments {
            roots: Vec::new(),
            json: None,
            explain: false,
            jobs: std::thread::available_parallelism().map_or(1, usize::from),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match *arg {
                "--json" => parsed.json = Some(PathBuf::from(args.next()?)),
                "--explain" => parsed.explain = true,
                "--jobs" => parsed.jobs = args.next()?.parse().ok().filter(|n| *n > 0)?,
                flag if flag.starts_with("--") => return None,
                root => parsed.roots.push(PathBuf::from(root)),
            }
        }
        (!parsed.roots.is_empty()).then_some(parsed)
    }
}

/// One document's layout next to Word's record of it.
#[derive(Serialize)]
struct Scored {
    path: String,
    word_pages: u32,
    pages: usize,
    /// Characters, spaces included: as Word recorded, and as found.
    word_characters: Option<u32>,
    characters: usize,
    /// The two fit, so Word's record describes this content and not an
    /// earlier version of it.
    fresh: bool,
    /// Page starts Word recorded that its layout chose, and how many of them
    /// are page starts of ours: at the same character, or in the same paragraph.
    page_starts: usize,
    page_starts_equal: usize,
    page_starts_same_paragraph: usize,
    /// Page starts Word recorded right after an explicit break. Finding
    /// those says nothing about layout, so they are counted apart.
    forced_page_starts: usize,
    forced_page_starts_equal: usize,
    /// Nothing in the document is beyond what the engine lays out: no
    /// tables, pictures, text boxes, notes, columns, frames or section changes.
    plain: bool,
    missing_fonts: Vec<String>,
}

/// A page start that Word recorded.
struct Hint {
    paragraph: usize,
    /// How much of the document's content comes before it. Counting across
    /// the whole document makes a start at the end of one paragraph and one
    /// at the beginning of the next the same place, as it is for the reader:
    /// Word has nowhere to record a page that begins with an empty paragraph
    /// but in the text after it.
    place: usize,
    /// It follows an explicit break with no content in between.
    forced: bool,
}

/// What the file says about where Word broke its pages.
struct Record {
    hints: Vec<Hint>,
    /// Content before each paragraph, counted as [`Hint::place`] is.
    before: Vec<usize>,
    /// Characters of text, spaces included.
    characters: usize,
    /// The content itself, one character for each place, to quote from.
    text: Vec<char>,
}

impl Record {
    /// Whether the number of characters Word recorded fits the text that is
    /// there. Word's figure is the text, plus in many files one for each
    /// paragraph; anything outside that range means the text has changed since.
    fn fits(&self, recorded: Option<u32>) -> bool {
        recorded.is_some_and(|recorded| {
            let recorded = recorded as usize;
            recorded + 2 >= self.characters && recorded <= self.characters + self.before.len() + 2
        })
    }
}

fn record(docx: &Docx) -> Record {
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();
    // Whether a section break begins a page is for the section after it to say.
    let mut sections: Vec<&Section> = paragraphs.iter().filter_map(|p| p.section_break.as_ref()).collect();
    sections.push(&docx.document.section);
    let mut section = 0;

    let mut record =
        Record { hints: Vec::new(), before: Vec::with_capacity(paragraphs.len()), characters: 0, text: Vec::new() };
    let mut place = 0;
    // An explicit break has been passed and no content has followed it yet.
    let mut after_break = false;
    for (index, paragraph) in paragraphs.iter().enumerate() {
        record.before.push(place);
        after_break |= resolver.paragraph(&paragraph.props).page_break_before == Some(true);
        content(docx, paragraph, |piece| {
            match piece {
                Content::PageBreakHint => {
                    record.hints.push(Hint { paragraph: index, place, forced: after_break });
                    return;
                }
                Content::Break(BreakKind::Page | BreakKind::Column) => after_break = true,
                Content::Char(_) => {
                    after_break = false;
                    record.characters += 1;
                }
                Content::Tab | Content::Break(BreakKind::Line) => after_break = false,
            }
            record.text.push(match piece {
                Content::Char(ch) => ch,
                _ => ' ',
            });
            place += 1;
        });
        if paragraph.section_break.is_some() {
            section += 1;
            after_break |= sections[section.min(sections.len() - 1)].start != SectionStart::Continuous;
        }
    }
    record
}

/// Sets each page start Word recorded in the document beside the nearest
/// of ours, to show where and in which direction the two layouts part.
fn explain(path: &Path, fonts: &FontDb) -> Option<String> {
    let docx = Docx::open(path).ok()?;
    let layout = oco_layout::layout(&docx, fonts, &Options::default());
    let record = record(&docx);
    let ours: Vec<usize> = layout
        .pages
        .iter()
        .map(|page| page.start.map_or(usize::MAX, |start| record.before[start.paragraph] + start.offset))
        .collect();

    let mut out = String::new();
    let pages = docx.recorded.pages.map_or("?".to_owned(), |pages| pages.to_string());
    let _ = writeln!(out, "{}: Word {} pages, ours {}", path.display(), pages, layout.pages.len());
    for (number, hint) in record.hints.iter().enumerate() {
        let quote: String = record.text[hint.place.min(record.text.len())..].iter().take(28).collect();
        let nearest = ours
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, place)| **place != usize::MAX)
            .min_by_key(|(_, place)| place.abs_diff(hint.place));
        let verdict = match nearest {
            Some((_, place)) if *place == hint.place => "same".to_owned(),
            Some((page, place)) => format!(
                "ours: page {} begins {} characters {}",
                page + 1,
                place.abs_diff(hint.place),
                if *place < hint.place { "earlier" } else { "later" }
            ),
            None => "ours: no such page".to_owned(),
        };
        let _ = writeln!(
            out,
            "  Word page {:<3} {} \u{b6}{} {:28?}  {verdict}",
            number + 2,
            if hint.forced { "after a break" } else { "             " },
            hint.paragraph + 1,
            quote
        );
    }
    Some(out)
}

fn score(path: &Path, fonts: &FontDb) -> Option<Scored> {
    let docx = Docx::open(path).ok()?;
    if !docx.recorded.from_word() {
        return None;
    }
    let word_pages = docx.recorded.pages.filter(|pages| *pages > 0)?;
    let layout = oco_layout::layout(&docx, fonts, &Options::default());
    let record = record(&docx);

    let fresh = record.fits(docx.recorded.characters_with_spaces);

    let starts: Vec<_> = layout.pages.iter().skip(1).filter_map(|page| page.start).collect();
    let places: HashSet<usize> = starts.iter().map(|start| record.before[start.paragraph] + start.offset).collect();
    let paragraphs: HashSet<usize> = starts.iter().map(|start| start.paragraph).collect();
    let equal = |hint: &Hint| places.contains(&hint.place);
    let (forced, chosen): (Vec<&Hint>, Vec<&Hint>) = record.hints.iter().partition(|hint| hint.forced);

    let unsupported =
        |needles: &[&str]| docx.coverage.unsupported.keys().any(|key| needles.iter().any(|n| key.contains(n)));
    let rich = docx.document.body.iter().any(|block| matches!(block, Block::Table(_)))
        || unsupported(&[
            "w:drawing",
            "w:pict",
            "w:object",
            "mc:AlternateContent",
            "footnoteReference",
            "endnoteReference",
            "w:sectPr/w:cols",
            "w:pPr/w:framePr",
            "m:oMath",
        ])
        || docx.document.paragraphs().iter().any(|p| p.section_break.is_some());

    Some(Scored {
        path: path.display().to_string(),
        word_pages,
        pages: layout.pages.len(),
        word_characters: docx.recorded.characters_with_spaces,
        characters: record.characters,
        fresh,
        page_starts: chosen.len(),
        page_starts_equal: chosen.iter().filter(|hint| equal(hint)).count(),
        page_starts_same_paragraph: chosen
            .iter()
            .filter(|hint| equal(hint) || paragraphs.contains(&hint.paragraph))
            .count(),
        forced_page_starts: forced.len(),
        forced_page_starts_equal: forced.iter().filter(|hint| equal(hint)).count(),
        plain: !rich,
        missing_fonts: layout.missing_fonts,
    })
}

/// Agreement figures over one group of documents.
///
/// Page counts are judged on documents of two pages or more only: nearly
/// every one-page document comes out at one page, which says nothing.
#[derive(Serialize, Default)]
struct Group {
    documents: usize,
    multipage: usize,
    multipage_pages_equal: usize,
    multipage_pages_within_one: usize,
    /// Page starts that Word's layout chose, across the group, and how many
    /// ours reproduces: at the same character, or at least in the same paragraph.
    page_starts: usize,
    page_starts_equal: usize,
    page_starts_same_paragraph: usize,
    /// Page starts right after an explicit break, and how many are ours too.
    forced_page_starts: usize,
    forced_page_starts_equal: usize,
}

impl Group {
    fn add(&mut self, doc: &Scored) {
        self.documents += 1;
        if doc.word_pages >= 2 {
            let difference = (doc.pages as i64 - i64::from(doc.word_pages)).abs();
            self.multipage += 1;
            self.multipage_pages_equal += usize::from(difference == 0);
            self.multipage_pages_within_one += usize::from(difference <= 1);
        }
        self.page_starts += doc.page_starts;
        self.page_starts_equal += doc.page_starts_equal;
        self.page_starts_same_paragraph += doc.page_starts_same_paragraph;
        self.forced_page_starts += doc.forced_page_starts;
        self.forced_page_starts_equal += doc.forced_page_starts_equal;
    }
}

#[derive(Serialize)]
struct Report {
    /// Where the fonts came from. The figures only compare with those of
    /// another run when both had the same fonts.
    font_dirs: &'static [&'static str],
    /// Documents saved by Word that record a page count.
    candidates: usize,
    /// Group name -> figures. See `GROUPS` for what each holds.
    groups: BTreeMap<&'static str, Group>,
    documents: Vec<Scored>,
}

/// The groups reported. Each narrows the one before, ending with the
/// documents the engine has no excuse for getting wrong.
const GROUPS: [(&str, &str); 4] = [
    ("all", "saved by Word"),
    ("fresh", "... and Word's character count fits the text"),
    ("plain", "... and only body text"),
    ("covered", "... and every font available"),
];

fn member(doc: &Scored, group: &str) -> bool {
    match group {
        "all" => true,
        "fresh" => doc.fresh,
        "plain" => doc.fresh && doc.plain,
        "covered" => doc.fresh && doc.plain && doc.missing_fonts.is_empty(),
        _ => false,
    }
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let mut files = Vec::new();
    for root in &arguments.roots {
        collect(root, &mut files);
    }
    files.sort();
    let mut fonts = FontDb::default();
    crate::render::DEFAULT_FONT_DIRS.iter().for_each(|dir| fonts.load_dir(Path::new(dir)));
    if fonts.is_empty() {
        eprintln!("oco: no fonts found; run tools/fonts/fetch.sh");
        return ExitCode::FAILURE;
    }

    if arguments.explain {
        for path in &files {
            if let Some(text) = explain(path, &fonts) {
                let _ = std::io::stdout().write_all(text.as_bytes());
            }
        }
        return ExitCode::SUCCESS;
    }

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let next = AtomicUsize::new(0);
    let scored = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..arguments.jobs.min(files.len().max(1)) {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Ok(Some(doc)) = panic::catch_unwind(AssertUnwindSafe(|| score(path, &fonts))) {
                        scored.lock().unwrap().push(doc);
                    }
                }
            });
        }
    });
    panic::set_hook(default_hook);

    let mut documents = scored.into_inner().unwrap();
    documents.sort_by(|a, b| a.path.cmp(&b.path));
    let mut groups: BTreeMap<&'static str, Group> = BTreeMap::new();
    for (name, _) in GROUPS {
        let group = groups.entry(name).or_default();
        documents.iter().filter(|doc| member(doc, name)).for_each(|doc| group.add(doc));
    }
    let report = Report { font_dirs: crate::render::DEFAULT_FONT_DIRS, candidates: documents.len(), groups, documents };

    let _ = std::io::stdout().write_all(summary(&report).as_bytes());
    if let Some(path) = &arguments.json {
        let json = serde_json::to_string(&report).expect("report always serialises");
        if let Err(error) = fs::write(path, json) {
            eprintln!("oco: {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if path.file_name().is_some_and(|name| name == ".git") {
            return;
        }
        for entry in fs::read_dir(path).into_iter().flatten().flatten() {
            collect(&entry.path(), files);
        }
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["docx", "docm"].contains(&e.to_ascii_lowercase().as_str()))
    {
        files.push(path.to_owned());
    }
}

fn summary(report: &Report) -> String {
    let share = |n: usize, of: usize| {
        if of == 0 {
            "-".to_owned()
        } else {
            format!("{n}/{of} {:5.1}%", 100.0 * n as f64 / of as f64)
        }
    };
    let mut out = String::new();
    macro_rules! line {
        ($($arg:tt)*) => { let _ = writeln!(out, $($arg)*); };
    }
    line!("Layout against Word's own record: {} documents saved by Word with a page count\n", report.candidates);
    line!("{:50} {:>5}  {:>18}  {:>18}  {:>18}", "", "docs", "same page count", "page starts", "page starts");
    line!("{:50} {:>5}  {:>18}  {:>18}  {:>18}", "", "", "(2+ pages)", "same place", "same paragraph");
    for (name, description) in GROUPS {
        let group = &report.groups[name];
        line!(
            "{:50} {:>5}  {:>18}  {:>18}  {:>18}",
            description,
            group.documents,
            share(group.multipage_pages_equal, group.multipage),
            share(group.page_starts_equal, group.page_starts),
            share(group.page_starts_same_paragraph, group.page_starts)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};

    use super::*;

    fn docx(body: &str, app: &str) -> Docx {
        let word = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let parts = [
            ("word/document.xml", format!("<w:document {word}><w:body>{body}</w:body></w:document>")),
            ("docProps/app.xml", format!("<Properties>{app}</Properties>")),
        ];
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        Docx::from_bytes(&writer.finish().unwrap().into_inner()).unwrap()
    }

    fn hints(body: &str) -> Vec<(usize, usize, bool)> {
        record(&docx(body, "")).hints.iter().map(|hint| (hint.paragraph, hint.place, hint.forced)).collect()
    }

    const HINT: &str = "<w:lastRenderedPageBreak/>";

    #[test]
    fn a_recorded_page_start_is_placed_by_the_content_before_it() {
        let body = format!(
            "<w:p><w:r><w:t>abc</w:t></w:r></w:p><w:p/>\
             <w:p><w:r><w:t>de</w:t><w:tab/>{HINT}<w:t>f</w:t></w:r></w:p>\
             <w:p><w:r>{HINT}<w:t>g</w:t></w:r></w:p>"
        );
        let record = record(&docx(&body, ""));
        // The mark itself takes up no place.
        assert_eq!(record.before, [0, 3, 3, 7]);
        assert_eq!(record.characters, 7);
        let places: Vec<_> = record.hints.iter().map(|hint| (hint.paragraph, hint.place, hint.forced)).collect();
        assert_eq!(places, [(2, 6, false), (3, 7, false)]);
    }

    #[test]
    fn page_starts_after_an_explicit_break_are_told_apart() {
        let page_break = r#"<w:br w:type="page"/>"#;
        // Straight after a page break, in the same paragraph or the next,
        // with nothing or only empty paragraphs in between.
        assert_eq!(hints(&format!("<w:p><w:r><w:t>a</w:t>{page_break}{HINT}<w:t>b</w:t></w:r></w:p>")), [(0, 2, true)]);
        assert_eq!(
            hints(&format!(
                "<w:p><w:r><w:t>a</w:t>{page_break}</w:r></w:p><w:p/><w:p><w:r>{HINT}<w:t>b</w:t></w:r></w:p>"
            )),
            [(2, 2, true)]
        );
        // Once text has followed the break, the next page is the layout's doing.
        assert_eq!(
            hints(&format!("<w:p><w:r><w:t>a</w:t>{page_break}<w:t>b</w:t>{HINT}<w:t>c</w:t></w:r></w:p>")),
            [(0, 3, false)]
        );
        // So it is once a tab or a line break has: those are content too.
        for content in ["<w:tab/>", "<w:br/>"] {
            assert_eq!(
                hints(&format!("<w:p><w:r>{page_break}{content}{HINT}<w:t>b</w:t></w:r></w:p>")),
                [(0, 2, false)],
                "{content}"
            );
        }
        // A paragraph that asks for a new page.
        assert_eq!(
            hints(&format!(
                "<w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r>{HINT}<w:t>b</w:t></w:r></w:p>"
            )),
            [(1, 1, true)]
        );
    }

    #[test]
    fn a_section_break_forces_a_page_unless_the_next_section_is_continuous() {
        let body = |kind: &str| {
            format!(
                "<w:p><w:pPr><w:sectPr/></w:pPr><w:r><w:t>a</w:t></w:r></w:p>\
                 <w:p><w:r>{HINT}<w:t>b</w:t></w:r></w:p><w:sectPr>{kind}</w:sectPr>"
            )
        };
        assert_eq!(hints(&body("")), [(1, 1, true)]);
        assert_eq!(hints(&body(r#"<w:type w:val="continuous"/>"#)), [(1, 1, false)]);
    }

    #[test]
    fn the_record_is_trusted_only_when_words_character_count_fits_the_text() {
        // Seven characters in two paragraphs: Word records seven, or one more per paragraph.
        let record = record(&docx("<w:p><w:r><w:t>ab cd</w:t></w:r></w:p><w:p><w:r><w:t>ef</w:t></w:r></w:p>", ""));
        assert!(record.fits(Some(7)) && record.fits(Some(9)));
        assert!(!record.fits(Some(4)) && !record.fits(Some(40)) && !record.fits(None));
    }
}
