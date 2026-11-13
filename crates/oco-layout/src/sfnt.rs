//! Reads font files: TrueType and OpenType, alone or in a collection.
//!
//! Only what layout and drawing need is read: which glyph stands for a
//! character, how far it advances, its outline, the font's line metrics and
//! its names. Hinting, variations, kerning and the layout tables are not.
//!
//! Font files are untrusted input. Every read is checked, nothing here
//! panics on a malformed file, and nesting (composite glyphs, charstring
//! subroutines) is bounded.

mod cff;
mod cmap;
mod glyf;

use std::ops::Range;
use std::sync::Arc;

/// Receives a glyph outline, in font units with y pointing up.
pub trait OutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32);
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    fn close(&mut self);
}

/// A cursor over big-endian data. Reads past the end fail and leave the
/// cursor where it was; skipping past the end is allowed and makes every
/// later read fail.
#[derive(Clone, Copy)]
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }

    fn new_at(data: &'a [u8], at: usize) -> Option<Self> {
        (at <= data.len()).then_some(Reader { data, at })
    }

    fn at_end(&self) -> bool {
        self.at >= self.data.len()
    }

    fn skip(&mut self, len: usize) {
        self.at = self.at.saturating_add(len);
    }

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(bytes)
    }

    fn tail(&self) -> Option<&'a [u8]> {
        self.data.get(self.at..)
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }

    fn i8(&mut self) -> Option<i8> {
        self.u8().map(|b| b as i8)
    }

    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|n| n as i16)
    }

    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|n| n as i32)
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn i16_at(data: &[u8], at: usize) -> Option<i16> {
    u16_at(data, at).map(|n| n as i16)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

const TRUETYPE: u32 = 0x0001_0000;
const TRUETYPE_APPLE: u32 = 0x7472_7565; // 'true'
const OPENTYPE: u32 = 0x4F54_544F; // 'OTTO'
const COLLECTION: u32 = 0x7474_6366; // 'ttcf'

/// How many faces a font file holds: what a collection says, 1 for a single font.
pub fn faces_in(data: &[u8]) -> u32 {
    let mut reader = Reader::new(data);
    if reader.u32() != Some(COLLECTION) {
        return 1;
    }
    reader.skip(4); // version
                    // No more than there is room to list, whatever a damaged file claims.
    let room = data.len().saturating_sub(12) / 4;
    reader.u32().map_or(1, |faces| faces.min(u32::try_from(room).unwrap_or(u32::MAX)))
}

/// `hhea`: the line metrics every font has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hhea {
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
}

/// `OS/2`: the metrics Windows uses, and the style flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Os2 {
    /// Lines are to be spaced by the typographic metrics, not the Windows ones.
    pub use_typographic_metrics: bool,
    pub typographic_ascender: i16,
    pub typographic_descender: i16,
    pub typographic_line_gap: i16,
    pub windows_ascender: i16,
    /// Negative below the baseline, like the other descenders.
    pub windows_descender: i16,
    pub bold: bool,
    pub italic: bool,
    pub strikeout_position: i16,
    pub strikeout_thickness: i16,
}

/// `post`: the underline, and whether the design slants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Post {
    pub slanted: bool,
    pub underline_position: i16,
    pub underline_thickness: i16,
}

enum Outlines {
    None,
    TrueType(glyf::Glyf),
    Cff(Box<cff::Cff>),
}

/// One face of a font file.
pub struct Face {
    data: Arc<[u8]>,
    units_per_em: u16,
    hhea: Hhea,
    os2: Option<Os2>,
    post: Option<Post>,
    /// Where `hmtx` starts, how many glyphs it holds full metrics for and
    /// how many glyphs it covers in all.
    advances: Option<(usize, u16, u16)>,
    cmap: Vec<cmap::Subtable>,
    name: Option<Range<usize>>,
    outlines: Outlines,
}

impl Face {
    /// Opens face `index` of a font file. `None` if the file is not a font,
    /// has no such face, or lacks the tables no font can do without.
    pub fn parse(data: Arc<[u8]>, index: u32) -> Option<Face> {
        let tables = Tables::locate(&data, index)?;
        let table = |tag: &[u8; 4]| tables.get(&data, tag);

        let head = table(b"head").filter(|head| head.len() >= 54)?;
        let units_per_em = u16_at(head, 18)?;
        let long_offsets = match u16_at(head, 50)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        if !(16..=16384).contains(&units_per_em) {
            return None;
        }

        let hhea_data = table(b"hhea").filter(|hhea| hhea.len() >= 36)?;
        let hhea =
            Hhea { ascender: i16_at(hhea_data, 4)?, descender: i16_at(hhea_data, 6)?, line_gap: i16_at(hhea_data, 8)? };
        let full_metrics = u16_at(hhea_data, 34)?;

        let maxp = table(b"maxp")?;
        if !matches!(u32_at(maxp, 0)?, 0x0000_5000 | 0x0001_0000) {
            return None;
        }
        let glyphs = u16_at(maxp, 4).filter(|count| *count != 0)?;

        // Fewer full metrics than glyphs means the last advance repeats.
        let advances = tables.range(&data, b"hmtx").and_then(|hmtx| {
            let fits = full_metrics != 0 && hmtx.len() >= usize::from(full_metrics) * 4;
            fits.then_some((hmtx.start, full_metrics, full_metrics.max(glyphs)))
        });

        let outlines = match (tables.range(&data, b"loca"), tables.range(&data, b"glyf")) {
            (Some(loca), Some(glyf)) => Outlines::TrueType(glyf::Glyf::new(glyphs, long_offsets, loca, glyf)),
            _ => match tables.range(&data, b"CFF ").and_then(|range| cff::Cff::parse(&data, range)) {
                Some(cff) => Outlines::Cff(Box::new(cff)),
                None => Outlines::None,
            },
        };

        Some(Face {
            units_per_em,
            hhea,
            os2: table(b"OS/2").and_then(parse_os2),
            post: table(b"post").and_then(parse_post),
            advances,
            cmap: tables.range(&data, b"cmap").map(|range| cmap::subtables(&data, range)).unwrap_or_default(),
            name: tables.range(&data, b"name"),
            outlines,
            data,
        })
    }

    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    pub fn hhea(&self) -> Hhea {
        self.hhea
    }

    pub fn os2(&self) -> Option<Os2> {
        self.os2
    }

    pub fn post(&self) -> Option<Post> {
        self.post
    }

    pub fn is_bold(&self) -> bool {
        self.os2.is_some_and(|os2| os2.bold)
    }

    /// Marked italic, or drawn at a slant without being marked.
    pub fn is_italic(&self) -> bool {
        self.os2.is_some_and(|os2| os2.italic) || self.post.is_some_and(|post| post.slanted)
    }

    /// The glyph the font maps `ch` to. A font may map a character to glyph
    /// 0, its "missing" glyph, which is still an answer.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        self.cmap.iter().find_map(|subtable| subtable.glyph_index(&self.data, u32::from(ch)))
    }

    /// Advance width of a glyph in font units.
    pub fn advance(&self, glyph: u16) -> Option<u16> {
        let (start, full_metrics, covered) = self.advances?;
        if glyph >= covered {
            return None;
        }
        let record = usize::from(glyph.min(full_metrics - 1));
        u16_at(&self.data, start + record * 4)
    }

    /// Draws a glyph. An outline that turns out to be malformed part-way is
    /// drawn as far as it was good.
    pub fn outline(&self, glyph: u16, builder: &mut dyn OutlineBuilder) {
        match &self.outlines {
            Outlines::None => {}
            Outlines::TrueType(glyf) => glyf.outline(&self.data, glyph, builder),
            Outlines::Cff(cff) => cff.outline(&self.data, glyph, builder),
        }
    }

    /// The font's family names (name ids 1 and 16) in every language it
    /// gives them in, in file order. Only names stored as Unicode are read.
    pub fn family_names(&self) -> Vec<String> {
        self.names().filter(|(id, _)| matches!(id, 1 | 16)).map(|(_, text)| text).collect()
    }

    fn names(&self) -> impl Iterator<Item = (u16, String)> + '_ {
        let table = self.name.clone().and_then(|range| self.data.get(range)).unwrap_or_default();
        // Versions 0 and 1 begin alike: a count, where the strings start, the records.
        let count = u16_at(table, 2).filter(|_| matches!(u16_at(table, 0), Some(0 | 1))).unwrap_or(0);
        let records = table.get(6..6 + usize::from(count) * 12).unwrap_or_default();
        // Strings start where the header says, but never inside the records.
        let storage = u16_at(table, 4).map_or(0, usize::from).max(6 + records.len());
        let storage = table.get(storage..);
        records.chunks_exact(12).filter_map(move |record| {
            let field = |at: usize| u16::from_be_bytes([record[at], record[at + 1]]);
            let (platform, encoding, id) = (field(0), field(2), field(6));
            // Unicode, or Windows with a symbol or Unicode BMP encoding: UTF-16.
            if !(platform == 0 || (platform == 3 && encoding <= 1)) {
                return None;
            }
            let start = usize::from(field(10));
            let bytes = storage?.get(start..start + usize::from(field(8)))?;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|unit| u16::from_be_bytes([unit[0], unit[1]])).collect();
            Some((id, String::from_utf16(&units).ok()?))
        })
    }
}

/// The table directory of one face.
struct Tables {
    /// Where the records start, and how many there are.
    records: usize,
    count: u16,
}

impl Tables {
    fn locate(data: &[u8], index: u32) -> Option<Tables> {
        let mut reader = Reader::new(data);
        let mut magic = reader.u32()?;
        if magic == COLLECTION {
            reader.skip(4); // version
            let faces = reader.u32()?;
            let offsets = reader.bytes(usize::try_from(faces).ok()?.checked_mul(4)?)?;
            let offset = u32_at(offsets, usize::try_from(index).ok()?.checked_mul(4)?)?;
            // A face lies after the list of faces, never inside it.
            let offset = usize::try_from(offset).ok().filter(|offset| *offset >= reader.at)?;
            reader = Reader::new_at(data, offset)?;
            magic = reader.u32()?;
        } else if index != 0 {
            return None;
        }
        if !matches!(magic, TRUETYPE | TRUETYPE_APPLE | OPENTYPE) {
            return None;
        }
        let count = reader.u16()?;
        reader.skip(6); // searchRange, entrySelector, rangeShift
        let records = reader.at;
        reader.bytes(usize::from(count) * 16)?;
        Some(Tables { records, count })
    }

    /// Where a table lies in the file. When a tag is listed twice the last
    /// one counts, even if it points outside the file.
    fn range(&self, data: &[u8], tag: &[u8; 4]) -> Option<Range<usize>> {
        let records = data.get(self.records..self.records + usize::from(self.count) * 16)?;
        let record = records.chunks_exact(16).rev().find(|record| &record[..4] == tag)?;
        let start = usize::try_from(u32_at(record, 8)?).ok()?;
        let end = start.checked_add(usize::try_from(u32_at(record, 12)?).ok()?)?;
        (end <= data.len()).then_some(start..end)
    }

    fn get<'a>(&self, data: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.range(data, tag).and_then(|range| data.get(range))
    }
}

fn parse_os2(data: &[u8]) -> Option<Os2> {
    let version = u16_at(data, 0)?;
    let length = match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        5 => 100,
        _ => return None,
    };
    if data.len() < length {
        return None;
    }
    let selection = u16_at(data, 62)?;
    Some(Os2 {
        use_typographic_metrics: version >= 4 && selection & (1 << 7) != 0,
        typographic_ascender: i16_at(data, 68)?,
        typographic_descender: i16_at(data, 70)?,
        typographic_line_gap: i16_at(data, 72)?,
        windows_ascender: i16_at(data, 74)?,
        windows_descender: i16_at(data, 76)?.wrapping_neg(),
        bold: selection & (1 << 5) != 0,
        italic: selection & 1 != 0,
        strikeout_thickness: i16_at(data, 26)?,
        strikeout_position: i16_at(data, 28)?,
    })
}

fn parse_post(data: &[u8]) -> Option<Post> {
    if data.len() < 32 {
        return None;
    }
    let version = u32_at(data, 0)?;
    if !matches!(version, 0x0001_0000 | 0x0002_0000 | 0x0002_5000 | 0x0003_0000 | 0x0004_0000) {
        return None;
    }
    // Version 2 goes on to name the glyphs; a table cut short there is not trusted.
    if version == 0x0002_0000 {
        let names = usize::from(u16_at(data, 32)?);
        data.get(34..34 + names * 2)?;
    }
    Some(Post {
        slanted: u32_at(data, 4)? != 0,
        underline_position: i16_at(data, 8)?,
        underline_thickness: i16_at(data, 10)?,
    })
}

#[cfg(test)]
mod tests;
