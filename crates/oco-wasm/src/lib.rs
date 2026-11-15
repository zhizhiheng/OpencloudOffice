//! The core behind a plain C interface, for hosts that load it as a
//! WebAssembly module: a browser page, a web view inside an app, a server.
//!
//! The module imports nothing. The host passes bytes by asking for room with
//! [`oco_alloc`] and writing into the module's memory. Whatever a call has to
//! say in text (a page as SVG, a report as JSON, the reason a file did not
//! open) it leaves in one result buffer, to be read through
//! [`oco_result_ptr`] and [`oco_result_len`] before the next call.
//!
//! `web/oco.js` wraps all of this for JavaScript.

use std::cell::RefCell;
use std::fmt::Write as _;

use oco_docx::Docx;
use oco_layout::{svg, FontDb, Layout, Options};

struct Document {
    docx: Docx,
    /// Laid out when first asked for, and again after the fonts change.
    layout: Option<Layout>,
}

#[derive(Default)]
struct State {
    fonts: FontDb,
    /// Open documents; a handle is an index into this plus one.
    documents: Vec<Option<Document>>,
    result: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    /// What the core said when it gave up, for the host to show after the trap.
    static PANIC: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            PANIC.with(|message| *message.borrow_mut() = info.to_string().into_bytes());
        }));
    });
    STATE.with(|state| f(&mut state.borrow_mut()))
}

impl State {
    fn say(&mut self, text: impl Into<String>) {
        self.result = text.into().into_bytes();
    }

    /// The document behind a handle, laid out, and the fonts it is set in.
    fn laid_out(&mut self, handle: u32) -> Option<(&Docx, &Layout, &FontDb)> {
        let fonts = &self.fonts;
        let document = self.documents.get_mut((handle as usize).checked_sub(1)?)?.as_mut()?;
        let layout =
            document.layout.get_or_insert_with(|| oco_layout::layout(&document.docx, fonts, &Options::default()));
        Some((&document.docx, layout, fonts))
    }
}

/// Room for `len` bytes that the host is about to pass in.
#[no_mangle]
pub extern "C" fn oco_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Gives back room that [`oco_alloc`] handed out.
///
/// # Safety
/// `ptr` and `len` must be those of one earlier [`oco_alloc`] call whose
/// buffer has not been freed or handed to [`oco_add_font`].
#[no_mangle]
pub unsafe extern "C" fn oco_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len));
}

/// Adds a font file (TrueType, OpenType or a collection) to the fonts that
/// documents are set in, and returns how many faces it held: 0 when the
/// bytes are not a font. Documents already open are laid out afresh.
///
/// The buffer becomes the core's: fonts are large and kept for good, so they
/// are not copied. The host must not free it.
///
/// # Safety
/// `ptr` and `len` must be those of one earlier [`oco_alloc`] call, the
/// buffer filled and not used again by the host.
#[no_mangle]
pub unsafe extern "C" fn oco_add_font(ptr: *mut u8, len: usize) -> u32 {
    let data = Vec::from_raw_parts(ptr, len, len);
    with(|state| {
        let faces = state.fonts.add(data);
        if faces > 0 {
            state.documents.iter_mut().flatten().for_each(|document| document.layout = None);
        }
        faces as u32
    })
}

/// Opens a document from its bytes and returns a handle to it, or 0 with
/// the reason in the result buffer. The bytes are only read during the
/// call. What kind of document it is, a Word document (`.docx` or the older
/// `.doc`) or Markdown, is told from the bytes; text is taken for Markdown.
///
/// # Safety
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn oco_open(ptr: *const u8, len: usize) -> u32 {
    open(std::slice::from_raw_parts(ptr, len), None)
}

/// As [`oco_open`], with the file's name (UTF-8), which settles what the
/// bytes alone leave open: text named `.docx` is a damaged Word document,
/// not Markdown.
///
/// # Safety
/// `ptr` must point at `len` readable bytes and `name` at `name_len`.
#[no_mangle]
pub unsafe extern "C" fn oco_open_named(ptr: *const u8, len: usize, name: *const u8, name_len: usize) -> u32 {
    let name = String::from_utf8_lossy(std::slice::from_raw_parts(name, name_len));
    open(std::slice::from_raw_parts(ptr, len), Some(&name))
}

fn open(bytes: &[u8], name: Option<&str>) -> u32 {
    with(|state| match oco_formats::open(bytes, name) {
        Ok(docx) => {
            let document = Some(Document { docx, layout: None });
            match state.documents.iter().position(Option::is_none) {
                Some(free) => {
                    state.documents[free] = document;
                    free as u32 + 1
                }
                None => {
                    state.documents.push(document);
                    state.documents.len() as u32
                }
            }
        }
        Err(error) => {
            state.say(error.to_string());
            0
        }
    })
}

/// Forgets a document. Its handle may be given out again.
#[no_mangle]
pub extern "C" fn oco_close(handle: u32) {
    with(|state| {
        if let Some(slot) = (handle as usize).checked_sub(1).and_then(|index| state.documents.get_mut(index)) {
            *slot = None;
        }
    })
}

/// The number of pages; 0 for a handle that is not open.
#[no_mangle]
pub extern "C" fn oco_page_count(handle: u32) -> u32 {
    with(|state| state.laid_out(handle).map_or(0, |(_, layout, _)| layout.pages.len() as u32))
}

fn page_size(handle: u32, page: u32) -> (f32, f32) {
    with(|state| {
        let page = state.laid_out(handle).and_then(|(_, layout, _)| layout.pages.get(page as usize));
        page.map_or((0.0, 0.0), |page| (page.width, page.height))
    })
}

/// The width of a page in points, counting pages from 0; 0 when there is no such page.
#[no_mangle]
pub extern "C" fn oco_page_width(handle: u32, page: u32) -> f32 {
    page_size(handle, page).0
}

/// The height of a page in points.
#[no_mangle]
pub extern "C" fn oco_page_height(handle: u32, page: u32) -> f32 {
    page_size(handle, page).1
}

/// Draws a page as an SVG document into the result buffer. Returns 1, or 0 when there is no such page.
#[no_mangle]
pub extern "C" fn oco_page_svg(handle: u32, page: u32) -> u32 {
    with(|state| {
        let drawn = state
            .laid_out(handle)
            .and_then(|(docx, layout, fonts)| Some(svg::render(layout.pages.get(page as usize)?, fonts, &docx.media)));
        let Some(drawn) = drawn else {
            return 0;
        };
        state.say(drawn);
        1
    })
}

/// Describes a document as JSON in the result buffer: its page count, and
/// what in it the core could not show the way Word does. Returns 1, or 0 for
/// a handle that is not open.
///
/// ```json
/// { "pages": 3,
///   "missingFonts": ["Aptos"],
///   "notShown": [{"kind": "pictures", "count": 2}],
///   "warnings": ["the styles part is malformed and was ignored: ..."] }
/// ```
#[no_mangle]
pub extern "C" fn oco_info(handle: u32) -> u32 {
    with(|state| {
        let Some((docx, layout, _)) = state.laid_out(handle) else {
            return 0;
        };
        let mut json = format!("{{\"pages\":{},\"missingFonts\":[", layout.pages.len());
        list(&mut json, layout.missing_fonts.iter(), |json, font| string(json, font));
        json.push_str("],\"notShown\":[");
        let mut kinds = not_shown(docx);
        // Pictures anchored in table cells are read but not placed yet, and
        // pictures of a kind that cannot be drawn are shown as empty frames.
        let more = layout.unplaced + layout.undrawn;
        if more > 0 {
            match kinds.iter_mut().find(|(kind, _)| *kind == "pictures") {
                Some((_, count)) => *count += more,
                None => kinds.insert(0, ("pictures", more)),
            }
        }
        list(&mut json, kinds.into_iter(), |json, (kind, count)| {
            let _ = write!(json, "{{\"kind\":\"{kind}\",\"count\":{count}}}");
        });
        json.push_str("],\"warnings\":[");
        list(&mut json, docx.warnings.iter(), |json, warning| string(json, warning));
        json.push_str("]}");
        state.say(json);
        1
    })
}

/// Writes the text of a page into the result buffer, a line to a line, in
/// reading order. Returns 1, or 0 when there is no such page.
#[no_mangle]
pub extern "C" fn oco_page_text(handle: u32, page: u32) -> u32 {
    with(|state| {
        let text = state
            .laid_out(handle)
            .and_then(|(_, layout, _)| layout.pages.get(page as usize))
            .map(|page| page.lines.iter().map(|line| line.text.replace('\t', " ")).collect::<Vec<_>>().join("\n"));
        match text {
            Some(text) => {
                state.say(text);
                1
            }
            None => 0,
        }
    })
}

/// Finds a text (UTF-8, `len` bytes at `ptr`) in a document and writes
/// where it is into the result buffer as JSON: for each place, the page
/// and the boxes it covers there, in points from the page's top left, one
/// for each line it is on. A match may run from one line to the next.
/// Letters are matched without regard to case. Returns how many places
/// there are, at most `limit`.
///
/// ```json
/// [{"page": 0, "boxes": [[72.0, 101.5, 48.0, 14.4]]}]
/// ```
///
/// # Safety
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn oco_find(handle: u32, ptr: *const u8, len: usize, limit: u32) -> u32 {
    let query = String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned();
    with(|state| {
        let Some((_, layout, _)) = state.laid_out(handle) else {
            state.say("[]");
            return 0;
        };
        let found = find(&layout.pages, &query, limit as usize);
        let mut json = String::from("[");
        list(&mut json, found.iter(), |json, (page, boxes)| {
            let _ = write!(json, "{{\"page\":{page},\"boxes\":[");
            list(json, boxes.iter(), |json, [x, y, width, height]| {
                let _ = write!(json, "[{x:.2},{y:.2},{width:.2},{height:.2}]");
            });
            json.push_str("]}");
        });
        json.push(']');
        let count = found.len() as u32;
        state.say(json);
        count
    })
}

/// The places `query` is found on `pages`, without regard to case, at most
/// `limit`: each a page and the boxes it covers there.
fn find(pages: &[oco_layout::Page], query: &str, limit: usize) -> Vec<(usize, Vec<[f32; 4]>)> {
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut found = Vec::new();
    if wanted.is_empty() || wanted.iter().all(|c| c.is_whitespace()) {
        return found;
    }
    for (number, page) in pages.iter().enumerate() {
        // The page's text as one run of characters, each with the line it is on and its place there.
        let mut text: Vec<(char, usize, usize)> = Vec::new();
        for (line_number, line) in page.lines.iter().enumerate() {
            for (at, ch) in line.text.chars().enumerate() {
                let ch = if ch == '\t' { ' ' } else { ch };
                text.extend(ch.to_lowercase().map(|lower| (lower, line_number, at)));
            }
        }
        let mut start = 0;
        while start + wanted.len() <= text.len() {
            if text[start..start + wanted.len()].iter().map(|(c, ..)| *c).eq(wanted.iter().copied()) {
                let mut boxes: Vec<[f32; 4]> = Vec::new();
                let covered = &text[start..start + wanted.len()];
                let mut from = 0;
                while from < covered.len() {
                    let line_number = covered[from].1;
                    let to = from + covered[from..].iter().take_while(|(_, line, _)| *line == line_number).count();
                    let line = &page.lines[line_number];
                    let (first, last) = (covered[from].2, covered[to - 1].2);
                    let left = line.xs.get(first).copied().unwrap_or(line.x0);
                    let right = line.xs.get(last + 1).copied().unwrap_or(line.x1).max(left);
                    let size = if line.size > 0.0 { line.size } else { 10.0 };
                    boxes.push([left, line.baseline - 0.9 * size, right - left, 1.2 * size]);
                    from = to;
                }
                found.push((number, boxes));
                if found.len() >= limit {
                    return found;
                }
                start += wanted.len();
            } else {
                start += 1;
            }
        }
    }
    found
}

/// Where the text of the last result starts in the module's memory.
#[no_mangle]
pub extern "C" fn oco_result_ptr() -> *const u8 {
    with(|state| state.result.as_ptr())
}

/// The length in bytes of the last result, which is UTF-8.
#[no_mangle]
pub extern "C" fn oco_result_len() -> usize {
    with(|state| state.result.len())
}

/// After a call trapped: where the core's last words start. The module is
/// not to be used further; load it anew.
#[no_mangle]
pub extern "C" fn oco_panic_ptr() -> *const u8 {
    PANIC.with(|message| message.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn oco_panic_len() -> usize {
    PANIC.with(|message| message.borrow().len())
}

/// Kinds of content the layout does not show yet, each with how often the
/// document has it. The names are part of the interface; hosts put their own
/// words to them.
fn not_shown(docx: &Docx) -> Vec<(&'static str, usize)> {
    const KINDS: &[(&str, &[&str])] = &[
        ("pictures", &["w:drawing", "w:pict"]),
        ("text_boxes", &["mc:AlternateContent"]),
        ("embedded_objects", &["w:object"]),
        ("equations", &["m:oMath"]),
        ("columns", &["w:sectPr/w:cols"]),
        ("frames", &["w:pPr/w:framePr"]),
        ("floating_tables", &["w:tblPr/w:tblpPr"]),
        ("comments", &["commentReference"]),
        ("vertical_text", &["w:tcPr/w:textDirection", "w:pPr/w:textDirection", "w:sectPr/w:textDirection"]),
        ("page_borders", &["w:sectPr/w:pgBorders"]),
        ("line_numbers", &["w:sectPr/w:lnNumType"]),
        ("ruby", &["w:r/w:ruby"]),
    ];
    let count = |needles: &[&str]| -> usize {
        docx.coverage
            .ranked()
            .iter()
            // A name may be followed by a word of explanation in brackets.
            .map(|(name, count)| (name.split(" (").next().unwrap_or(name), count))
            .filter(|(name, _)| needles.iter().any(|n| name.ends_with(n)))
            .map(|(_, n)| n)
            .sum()
    };
    KINDS.iter().map(|&(kind, needles)| (kind, count(needles))).filter(|&(_, count)| count > 0).collect()
}

fn list<T>(json: &mut String, items: impl Iterator<Item = T>, mut one: impl FnMut(&mut String, T)) {
    for (index, item) in items.enumerate() {
        if index > 0 {
            json.push(',');
        }
        one(json, item);
    }
}

/// Appends `text` as a JSON string.
fn string(json: &mut String, text: &str) {
    json.push('"');
    for c in text.chars() {
        match c {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            c if c < '\u{20}' => {
                let _ = write!(json, "\\u{:04x}", c as u32);
            }
            c => json.push(c),
        }
    }
    json.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_text_across_lines_and_without_regard_to_case() {
        let line = |baseline: f32, text: &str| oco_layout::PlacedLine {
            baseline,
            x0: 72.0,
            x1: 72.0 + 6.0 * text.chars().count() as f32,
            text: text.to_owned(),
            xs: (0..text.chars().count()).map(|at| 72.0 + 6.0 * at as f32).collect(),
            size: 12.0,
            fields: Vec::new(),
        };
        let page =
            oco_layout::Page { lines: vec![line(100.0, "Hello wor"), line(114.0, "ld, 世界")], ..Default::default() };
        let pages = vec![oco_layout::Page::default(), page];
        let found = find(&pages, "world", 10);
        assert_eq!(found.len(), 1);
        let (number, boxes) = &found[0];
        assert_eq!(*number, 1);
        assert_eq!(boxes.len(), 2, "on two lines: {boxes:?}");
        // "wor": from the seventh character to the end of the first line.
        assert_eq!(boxes[0][0], 72.0 + 36.0);
        assert!((boxes[0][2] - 18.0).abs() < 0.01 && (boxes[0][1] - (100.0 - 10.8)).abs() < 0.01);
        // "ld": the first two characters of the second line.
        assert_eq!((boxes[1][0], boxes[1][2]), (72.0, 12.0));
        assert_eq!(find(&pages, "HELLO", 10).len(), 1);
        assert_eq!(find(&pages, "世界", 10)[0].1[0][0], 72.0 + 24.0);
        assert_eq!(find(&pages, "o", 1).len(), 1, "no more than the limit");
        assert!(find(&pages, " ", 10).is_empty() && find(&pages, "", 10).is_empty());
    }

    #[test]
    fn writes_json_strings() {
        let mut json = String::new();
        string(&mut json, "a \"b\" \\ \n \u{1} 中");
        assert_eq!(json, "\"a \\\"b\\\" \\\\ \\n \\u0001 中\"");
    }

    #[test]
    fn opens_reports_and_closes_through_the_interface() {
        let mut writer = oco_docx::zip::Writer::new();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let body = format!(
            "<w:document xmlns:w=\"{w}\"><w:body><w:p><w:r><w:t>x</w:t><w:drawing/></w:r></w:p></w:body></w:document>"
        );
        writer.add("word/document.xml", body.as_bytes());
        let bytes = writer.finish();
        let result = || with(|state| String::from_utf8(state.result.clone()).unwrap());

        let handle = unsafe { oco_open(bytes.as_ptr(), bytes.len()) };
        assert_eq!(handle, 1);
        assert_eq!(oco_info(handle), 1);
        assert!(result().contains("\"notShown\":[{\"kind\":\"pictures\",\"count\":1}]"), "{}", result());
        assert_eq!(oco_page_svg(handle, 99), 0);
        oco_close(handle);
        assert_eq!(oco_page_count(handle), 0);
        // The handle of a closed document is given out again.
        assert_eq!(unsafe { oco_open(bytes.as_ptr(), bytes.len()) }, 1);

        // What is known not to be shown is said, whatever the reader adds to its name.
        let columns = format!(
            "<w:document xmlns:w=\"{w}\"><w:body><w:p/><w:sectPr><w:cols w:num=\"2\"/></w:sectPr></w:body></w:document>"
        );
        let mut writer = oco_docx::zip::Writer::new();
        writer.add("word/document.xml", columns.as_bytes());
        let columns = writer.finish();
        let second = unsafe { oco_open(columns.as_ptr(), columns.len()) };
        assert_eq!(oco_info(second), 1);
        assert!(result().contains("\"notShown\":[{\"kind\":\"columns\",\"count\":1}]"), "{}", result());
        oco_close(second);

        // Text set upright and a border around the page are said too; text
        // that runs the usual way is not.
        let cell =
            |direction: &str| format!(r#"<w:tc><w:tcPr><w:textDirection w:val="{direction}"/></w:tcPr><w:p/></w:tc>"#);
        let unusual = format!(
            r#"<w:document xmlns:w="{w}"><w:body><w:tbl><w:tr>{}{}</w:tr></w:tbl><w:p/>
               <w:sectPr><w:pgBorders/><w:textDirection w:val="lrTb"/></w:sectPr></w:body></w:document>"#,
            cell("tbRl"),
            cell("lrTb")
        );
        let mut writer = oco_docx::zip::Writer::new();
        writer.add("word/document.xml", unusual.as_bytes());
        let unusual = writer.finish();
        let third = unsafe { oco_open(unusual.as_ptr(), unusual.len()) };
        assert_eq!(oco_info(third), 1);
        let said = result();
        let kinds = r#""notShown":[{"kind":"vertical_text","count":1},{"kind":"page_borders","count":1}]"#;
        assert!(said.contains(kinds), "{said}");
        oco_close(third);

        // Bytes that are no kind of document, and text that is called a Word document.
        assert_eq!(unsafe { oco_open(b"\0junk".as_ptr(), 5) }, 0);
        assert!(result().contains("not a kind of document"), "{}", result());
        assert_eq!(unsafe { oco_open_named(b"junk".as_ptr(), 4, b"a.docx".as_ptr(), 6) }, 0);
        assert!(result().contains("zip"), "{}", result());
        // Text is Markdown.
        let markdown = "# 标题";
        assert_eq!(unsafe { oco_open(markdown.as_ptr(), markdown.len()) }, 2);
    }
}
