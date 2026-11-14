//! Draws a laid-out page as SVG.
//!
//! Glyphs are emitted as outlines taken from the font files, so the result
//! needs no fonts to display and looks the same in every viewer. Each
//! distinct glyph is defined once and reused.

use std::collections::BTreeMap;
use std::fmt::Write;

use oco_docx::Media;

use crate::fonts::FontDb;
use crate::sfnt::OutlineBuilder;
use crate::{Item, Page, Rgb};

/// Collects a glyph outline as SVG path data, in font units with y up.
struct PathData(String);

impl OutlineBuilder for PathData {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{} {}", x.round(), y.round());
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{} {}", x.round(), y.round());
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{} {} {} {}", x1.round(), y1.round(), x.round(), y.round());
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ =
            write!(self.0, "C{} {} {} {} {} {}", x1.round(), y1.round(), x2.round(), y2.round(), x.round(), y.round());
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// Trims a coordinate to two decimals, which is finer than any screen shows.
fn n(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What kind of image a file is, if it is a kind that viewers of SVG draw.
/// Windows metafiles (EMF, WMF) and TIFF are not.
fn media_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'B', b'M', ..] => Some("image/bmp"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => {
            let start = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes).trim_ascii_start();
            (start.starts_with(b"<svg") || start.starts_with(b"<?xml")).then_some("image/svg+xml")
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = chunk.iter().fold(0u32, |word, &byte| word << 8 | u32::from(byte)) << (8 * (3 - chunk.len()));
        for digit in 0..4 {
            if digit <= chunk.len() {
                out.push(DIGITS[(word >> (18 - 6 * digit) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Draws a page. `media` is where the pictures on it are found
/// ([`oco_docx::Docx::media`]); they are put into the SVG itself.
pub fn render(page: &Page, fonts: &FontDb, media: &BTreeMap<String, Media>) -> String {
    let mut definitions: BTreeMap<(usize, u16), String> = BTreeMap::new();
    let mut body = String::new();
    for item in &page.items {
        match *item {
            Item::Glyph { font, glyph, x, y, size, x_scale, color, fake_bold, fake_italic } => {
                let face = fonts.get(font);
                definitions.entry((font.0, glyph)).or_insert_with(|| {
                    let mut path = PathData(String::new());
                    face.outline(glyph, &mut path);
                    path.0
                });
                let scale = size / face.units_per_em;
                let slant = if fake_italic { " skewX(-12)" } else { "" };
                // A bold face that does not exist is imitated by thickening the outline.
                let stroke = if fake_bold {
                    format!(r#" stroke="{}" stroke-width="{}""#, hex(color), n(face.units_per_em * 0.035))
                } else {
                    String::new()
                };
                let fill = if color == [0, 0, 0] { String::new() } else { format!(r#" fill="{}""#, hex(color)) };
                let _ = writeln!(
                    body,
                    r##"<use href="#g{}_{}" transform="translate({} {}){slant} scale({} {})"{fill}{stroke}/>"##,
                    font.0,
                    glyph,
                    n(x),
                    n(y),
                    scale * x_scale,
                    -scale
                );
            }
            Item::Rect { x, y, width, height, color } => {
                let _ = writeln!(
                    body,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                    n(x),
                    n(y),
                    n(width),
                    n(height),
                    hex(color)
                );
            }
            Item::Rule { x1, y1, x2, y2, width, color, dash } => {
                let dashes = if dash[0] > 0.0 {
                    format!(r#" stroke-dasharray="{} {}""#, n(dash[0]), n(dash[1]))
                } else {
                    String::new()
                };
                let _ = writeln!(
                    body,
                    r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}"{dashes}/>"#,
                    n(x1),
                    n(y1),
                    n(x2),
                    n(y2),
                    hex(color),
                    n(width)
                );
            }
            Item::Wave { x1, x2, y, amplitude, width, color } => {
                let step = amplitude * 2.0;
                let mut path = format!("M{} {}", n(x1), n(y));
                let (mut x, mut up) = (x1, true);
                while x < x2 {
                    let next = (x + step).min(x2);
                    let peak = if up { y - amplitude } else { y + amplitude };
                    let _ = write!(path, "Q{} {} {} {}", n((x + next) / 2.0), n(peak), n(next), n(y));
                    x = next;
                    up = !up;
                }
                let _ = writeln!(
                    body,
                    r#"<path d="{path}" fill="none" stroke="{}" stroke-width="{}"/>"#,
                    hex(color),
                    n(width)
                );
            }
            Item::Circle { cx, cy, r, color, filled } => {
                let paint = if filled {
                    format!(r#"fill="{}""#, hex(color))
                } else {
                    format!(r#"fill="none" stroke="{}" stroke-width="{}""#, hex(color), n(r * 0.3))
                };
                let _ = writeln!(body, r#"<circle cx="{}" cy="{}" r="{}" {paint}/>"#, n(cx), n(cy), n(r));
            }
            Item::Image { x, y, width, height, ref part, crop, flip } => {
                let file = part.as_ref().and_then(|part| media.get(part)).and_then(|file| file.bytes().ok());
                let Some((kind, file)) = file.as_deref().and_then(|bytes| Some((media_type(bytes)?, bytes))) else {
                    // Nothing to show: a frame with a cross marks the room the object takes.
                    let (right, bottom) = (n(x + width), n(y + height));
                    let _ = writeln!(
                        body,
                        r##"<path d="M{0} {1}H{right}V{bottom}H{0}ZM{0} {1}L{right} {bottom}M{right} {1}L{0} {bottom}" fill="#f4f4f4" stroke="#b0b0b0" stroke-width="0.5"/>"##,
                        n(x),
                        n(y)
                    );
                    continue;
                };
                let [left, top, right, bottom] = crop;
                if flip != [false; 2] {
                    // Reflect the viewport and its content together: cropping
                    // happens in image coordinates, before the picture flips.
                    let (sx, sy) = (if flip[0] { -1 } else { 1 }, if flip[1] { -1 } else { 1 });
                    let (tx, ty) =
                        (if flip[0] { 2.0 * x + width } else { 0.0 }, if flip[1] { 2.0 * y + height } else { 0.0 });
                    let _ = writeln!(body, r#"<g transform="matrix({sx} 0 0 {sy} {} {})">"#, n(tx), n(ty));
                }
                let (across, down) = ((1.0 - left - right).max(0.001), (1.0 - top - bottom).max(0.001));
                let source = format!(r#"preserveAspectRatio="none" href="data:{kind};base64,{}""#, base64(file));
                if crop == [0.0; 4] {
                    let _ = writeln!(
                        body,
                        r#"<image x="{}" y="{}" width="{}" height="{}" {source}/>"#,
                        n(x),
                        n(y),
                        n(width),
                        n(height)
                    );
                } else {
                    // The box shows a part of the image: the whole is laid under it and cut by an inner viewport.
                    let (whole_width, whole_height) = (width / across, height / down);
                    let _ = writeln!(
                        body,
                        r#"<svg x="{}" y="{}" width="{}" height="{}"><image x="{}" y="{}" width="{}" height="{}" {source}/></svg>"#,
                        n(x),
                        n(y),
                        n(width),
                        n(height),
                        n(0.0 - left * whole_width),
                        n(0.0 - top * whole_height),
                        n(whole_width),
                        n(whole_height)
                    );
                }
                if flip != [false; 2] {
                    body.push_str("</g>\n");
                }
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}pt" height="{}pt">"#,
        n(page.width),
        n(page.height),
        n(page.width),
        n(page.height)
    );
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<defs>\n");
    for ((font, glyph), path) in &definitions {
        let _ = writeln!(out, r#"<path id="g{font}_{glyph}" d="{path}"/>"#);
    }
    out.push_str("</defs>\n");
    out.push_str(&body);
    text_layer(page, &mut out);
    out.push_str("</svg>\n");
    out
}

/// The text of the page, invisible, over what is drawn: each line with
/// every character at the place its glyph is drawn, so that a viewer can
/// select it, copy it and find it as it finds the text of any web page.
fn text_layer(page: &Page, out: &mut String) {
    if page.lines.is_empty() {
        return;
    }
    out.push_str(r#"<g fill-opacity="0" font-family="sans-serif" xml:space="preserve" style="white-space:pre">"#);
    out.push('\n');
    for line in &page.lines {
        let (mut xs, mut text) = (String::new(), String::new());
        for (ch, x) in line.text.chars().zip(&line.xs) {
            let ch = match ch {
                '\t' => ' ',
                '&' => {
                    text.push_str("&amp;");
                    '\0'
                }
                '<' => {
                    text.push_str("&lt;");
                    '\0'
                }
                '>' => {
                    text.push_str("&gt;");
                    '\0'
                }
                // What XML cannot hold is left out, with its place.
                c if c < ' ' || ('\u{FFFE}'..='\u{FFFF}').contains(&c) => continue,
                c => c,
            };
            if ch != '\0' {
                text.push(ch);
            }
            if !xs.is_empty() {
                xs.push(' ');
            }
            xs.push_str(&n(*x));
            // An addressable character is a UTF-16 code unit: one beyond the
            // basic plane takes two places.
            if ch.len_utf16() == 2 {
                xs.push(' ');
                xs.push_str(&n(*x));
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        let size = if line.size > 0.0 { line.size } else { 10.0 };
        let _ = writeln!(out, r#"<text x="{xs}" y="{}" font-size="{}">{text}</text>"#, n(line.baseline), n(size));
    }
    out.push_str("</g>\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(&[0xFB, 0xFF, 0xFE, 0x00]), "+//+AA==");
    }

    #[test]
    fn tells_the_images_a_viewer_can_draw() {
        assert_eq!(media_type(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(media_type(b"\xFF\xD8\xFF\xE0"), Some("image/jpeg"));
        assert_eq!(media_type(b"GIF89a"), Some("image/gif"));
        assert_eq!(media_type(b"\xEF\xBB\xBF <svg xmlns='...'/>"), Some("image/svg+xml"));
        // An enhanced metafile: nothing a browser draws.
        assert_eq!(media_type(b"\x01\x00\x00\x00\x6c\x00\x00\x00"), None);
        assert_eq!(media_type(b""), None);
    }

    #[test]
    fn draws_a_picture_from_the_documents_media_and_a_frame_where_there_is_none() {
        let mut writer = oco_docx::zip::Writer::new();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let wp = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
        let a = "http://schemas.openxmlformats.org/drawingml/2006/main";
        let pic = "http://schemas.openxmlformats.org/drawingml/2006/picture";
        let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        writer.add(
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{w}"><w:body><w:p><w:r><w:drawing><wp:inline xmlns:wp="{wp}"><wp:extent cx="12700" cy="12700"/>
                   <a:graphic xmlns:a="{a}"><a:graphicData><pic:pic xmlns:pic="{pic}"><pic:blipFill><a:blip xmlns:r="{r}" r:embed="rId1"/>
                   </pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#
            )
            .as_bytes(),
        );
        writer.add(
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                   <Relationship Id="rId1" Type="{r}/image" Target="media/image1.png"/></Relationships>"#
            )
            .as_bytes(),
        );
        writer.add("word/media/image1.png", b"\x89PNG and so on");
        let docx = oco_docx::Docx::from_bytes(&writer.finish()).unwrap();
        let picture = |part: Option<&str>, crop| Item::Image {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            part: part.map(str::to_owned),
            crop,
            flip: [false; 2],
        };
        let draw = |item| {
            let page = Page { width: 100.0, height: 100.0, items: vec![item], ..Page::default() };
            render(&page, &FontDb::default(), &docx.media)
        };

        let whole = draw(picture(Some("word/media/image1.png"), [0.0; 4]));
        let data = format!("href=\"data:image/png;base64,{}\"", base64(b"\x89PNG and so on"));
        assert!(whole.contains(r#"<image x="10" y="20" width="30" height="40" preserveAspectRatio="none""#), "{whole}");
        assert!(whole.contains(&data), "{whole}");

        // A quarter cut away at the left and half at the bottom: the whole image is 40 by 80, moved left by 10.
        let cropped = draw(picture(Some("word/media/image1.png"), [0.25, 0.0, 0.0, 0.5]));
        assert!(
            cropped
                .contains(r#"<svg x="10" y="20" width="30" height="40"><image x="-10" y="0" width="40" height="80""#),
            "{cropped}"
        );

        let missing = draw(picture(None, [0.0; 4]));
        assert!(!missing.contains("<image") && missing.contains(r#"<path d="M10 20H40V60H10Z"#), "{missing}");
        let mut reflected = picture(Some("word/media/image1.png"), [0.25, 0.0, 0.0, 0.5]);
        if let Item::Image { flip, .. } = &mut reflected {
            *flip = [true, true];
        }
        let reflected = draw(reflected);
        assert!(reflected.contains(r#"<g transform="matrix(-1 0 0 -1 50 80)">"#), "{reflected}");
        assert!(
            reflected.contains(r#"<image x="-10" y="0" width="40" height="80""#),
            "crop stays in source coordinates"
        );
    }
}
