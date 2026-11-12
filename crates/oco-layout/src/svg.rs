//! Draws a laid-out page as SVG.
//!
//! Glyphs are emitted as outlines taken from the font files, so the result
//! needs no fonts to display and looks the same in every viewer. Each
//! distinct glyph is defined once and reused.

use std::collections::BTreeMap;
use std::fmt::Write;

use owned_ttf_parser::OutlineBuilder;

use crate::fonts::FontDb;
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

pub fn render(page: &Page, fonts: &FontDb) -> String {
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
    out.push_str("</svg>\n");
    out
}
