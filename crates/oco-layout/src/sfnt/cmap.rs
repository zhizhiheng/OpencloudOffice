//! `cmap`: which glyph stands for a character.
//!
//! A font carries several mappings for different platforms. Those that map
//! Unicode are kept, in file order; a character is looked up in each in
//! turn until one knows it.

use std::ops::Range;

use super::{u16_at, u32_at};

pub(super) struct Subtable {
    /// From the subtable's first byte to the end of the `cmap` table: some
    /// formats point past their own declared length.
    data: Range<usize>,
    format: Format,
}

enum Format {
    /// Format 0: one byte per character code below 256.
    Bytes,
    /// Format 4: segments of the Basic Multilingual Plane.
    Segments { count: u16 },
    /// Format 6: one dense run of 16-bit codes.
    Trimmed { first: u16, count: u16 },
    /// Format 10: one dense run of 32-bit codes.
    TrimmedWide { first: u32, count: u32 },
    /// Format 12: ranges of consecutive codes mapping to consecutive glyphs.
    Groups { count: u32 },
    /// Format 13: ranges of codes that all map to one glyph.
    Shared { count: u32 },
}

/// The Unicode subtables of a `cmap` table lying at `table` in the file.
pub(super) fn subtables(file: &[u8], table: Range<usize>) -> Vec<Subtable> {
    let Some(cmap) = file.get(table.clone()) else { return Vec::new() };
    let count = u16_at(cmap, 2).unwrap_or(0);
    let Some(records) = cmap.get(4..4 + usize::from(count) * 8) else { return Vec::new() };
    let mut found = Vec::new();
    for record in records.chunks_exact(8) {
        let platform = u16::from_be_bytes([record[0], record[1]]);
        let encoding = u16::from_be_bytes([record[2], record[3]]);
        let Some(data) = u32_at(record, 4).and_then(|offset| cmap.get(usize::try_from(offset).ok()?..)) else {
            continue;
        };
        let Some(format) = parse(data) else { continue };
        // Unicode platform; Windows with the BMP encoding; or Windows with
        // the full repertoire, which only the 32-bit range formats can hold.
        let unicode = match (platform, encoding) {
            (0, _) | (3, 1) => true,
            (3, 10) => matches!(format, Some(Format::Groups { .. } | Format::Shared { .. })),
            _ => false,
        };
        if let (true, Some(format)) = (unicode, format) {
            found.push(Subtable { data: table.end - data.len()..table.end, format });
        }
    }
    found
}

/// `Some(None)` for a well-formed subtable in a format that maps no plain
/// characters (variation sequences, the legacy mixed formats).
fn parse(data: &[u8]) -> Option<Option<Format>> {
    // Whether `count` items of `size` bytes fit from `start` on.
    let fits = |start: usize, count: usize, size: usize| {
        count.checked_mul(size).and_then(|bytes| bytes.checked_add(start)).is_some_and(|end| end <= data.len())
    };
    let wide = |count: u32| usize::try_from(count).unwrap_or(usize::MAX);
    let format = match u16_at(data, 0)? {
        0 => fits(6, 256, 1).then_some(Format::Bytes),
        4 => {
            let count = u16_at(data, 6)? / 2;
            // Four arrays of `count` 16-bit values, and a pad after the first.
            (count >= 1 && fits(16, usize::from(count) * 4, 2)).then_some(Format::Segments { count })
        }
        6 => {
            let (first, count) = (u16_at(data, 6)?, u16_at(data, 8)?);
            fits(10, usize::from(count), 2).then_some(Format::Trimmed { first, count })
        }
        10 => {
            let (first, count) = (u32_at(data, 12)?, u32_at(data, 16)?);
            fits(20, wide(count), 2).then_some(Format::TrimmedWide { first, count })
        }
        12 => {
            let count = u32_at(data, 12)?;
            fits(16, wide(count), 12).then_some(Format::Groups { count })
        }
        13 => {
            let count = u32_at(data, 12)?;
            fits(16, wide(count), 12).then_some(Format::Shared { count })
        }
        2 | 8 | 14 => return Some(None),
        _ => return None,
    };
    format.map(Some)
}

impl Subtable {
    pub(super) fn glyph_index(&self, file: &[u8], code: u32) -> Option<u16> {
        let data = file.get(self.data.clone())?;
        match self.format {
            Format::Bytes => {
                let glyph = *data.get(6usize.checked_add(usize::try_from(code).ok()?)?).filter(|_| code < 256)?;
                (glyph != 0).then_some(u16::from(glyph))
            }
            Format::Segments { count } => segments(data, count, u16::try_from(code).ok()?),
            Format::Trimmed { first, count } => {
                let index = u16::try_from(code).ok()?.checked_sub(first).filter(|index| *index < count)?;
                u16_at(data, 10 + usize::from(index) * 2)
            }
            Format::TrimmedWide { first, count } => {
                let index = code.checked_sub(first).filter(|index| *index < count)?;
                u16_at(data, 20 + usize::try_from(index).ok()? * 2)
            }
            Format::Groups { count } => {
                let group = |index: u32| -> Option<(u32, u32, u32)> {
                    let at = 16 + usize::try_from(index).ok()? * 12;
                    Some((u32_at(data, at)?, u32_at(data, at + 4)?, u32_at(data, at + 8)?))
                };
                // Narrows to the last group that does not start after the code.
                let (mut base, mut size) = (0, count);
                if size == 0 {
                    return None;
                }
                while size > 1 {
                    let half = size / 2;
                    if group(base + half)?.0 <= code {
                        base += half;
                    }
                    size -= half;
                }
                let (first, last, glyph) = group(base)?;
                if !(first..=last).contains(&code) {
                    return None;
                }
                u16::try_from(glyph.checked_add(code)?.checked_sub(first)?).ok()
            }
            Format::Shared { count } => {
                for index in 0..usize::try_from(count).ok()? {
                    let at = 16 + index * 12;
                    if (u32_at(data, at)?..=u32_at(data, at + 4)?).contains(&code) {
                        return u16::try_from(u32_at(data, at + 8)?).ok();
                    }
                }
                None
            }
        }
    }
}

/// Format 4. The arrays follow one another: last code of each segment, a
/// pad, first code, delta, and an offset into the glyph array that follows.
fn segments(data: &[u8], count: u16, code: u16) -> Option<u16> {
    let size = usize::from(count) * 2;
    let (ends, starts, deltas, offsets) = (14, 16 + size, 16 + size * 2, 16 + size * 3);
    let (mut low, mut high) = (0u16, count);
    while high > low {
        let index = (low + high) / 2;
        let at = usize::from(index) * 2;
        if u16_at(data, ends + at)? < code {
            low = index + 1;
            continue;
        }
        let first = u16_at(data, starts + at)?;
        if first > code {
            high = index;
            continue;
        }
        let delta = u16_at(data, deltas + at)?;
        return match u16_at(data, offsets + at)? {
            0 => Some(code.wrapping_add(delta)),
            0xFFFF => None,
            // The offset counts from its own position in the offsets array,
            // in 16-bit arithmetic.
            offset => {
                let position = (offsets + at) as u16;
                let position = position.wrapping_add((code - first).checked_mul(2)?).wrapping_add(offset);
                match u16_at(data, usize::from(position))? {
                    0 => None,
                    // A sum that overflows as a signed number maps nothing.
                    glyph => u16::try_from((glyph as i16).wrapping_add(delta as i16)).ok(),
                }
            }
        };
    }
    None
}
