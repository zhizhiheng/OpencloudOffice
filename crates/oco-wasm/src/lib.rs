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

/// Opens a `.docx` file from its bytes and returns a handle to it, or 0 with
/// the reason in the result buffer. The bytes are only read during the call.
///
/// # Safety
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn oco_open(ptr: *const u8, len: usize) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len);
    with(|state| match Docx::from_bytes(bytes) {
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
        if layout.unplaced > 0 {
            // Pictures anchored in table cells are read but not placed yet.
            match kinds.iter_mut().find(|(kind, _)| *kind == "pictures") {
                Some((_, count)) => *count += layout.unplaced,
                None => kinds.insert(0, ("pictures", layout.unplaced)),
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
        ("notes", &["footnoteReference", "endnoteReference"]),
        ("equations", &["m:oMath"]),
        ("columns", &["w:sectPr/w:cols"]),
        ("frames", &["w:pPr/w:framePr"]),
        ("floating_tables", &["w:tblPr/w:tblpPr"]),
        ("comments", &["commentReference"]),
    ];
    let count = |needles: &[&str]| -> usize {
        docx.coverage
            .ranked()
            .iter()
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

        assert_eq!(unsafe { oco_open(b"junk".as_ptr(), 4) }, 0);
        assert!(result().contains("zip"), "{}", result());
    }
}
