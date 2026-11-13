//! `CFF `: PostScript outlines, made of lines and cubic curves.
//!
//! A glyph is a small program (a Type 2 charstring) for a stack machine:
//! numbers are pushed, an operator consumes them and draws. Programs call
//! shared subroutines, global ones and ones local to the glyph's font
//! dictionary; a CID-keyed font, as the large East Asian fonts are, has
//! many such dictionaries and a table saying which glyph uses which.

use std::ops::Range;

use super::{OutlineBuilder, Reader};

/// Operands a charstring operator, or a dictionary operator, may take.
const MAX_OPERANDS: usize = 48;
/// How deep subroutine calls may nest.
const MAX_DEPTH: u8 = 10;
/// How many operators and operands one glyph may execute. Real glyphs need
/// a few thousand; without a bound, subroutines calling subroutines could
/// multiply without end.
const MAX_STEPS: u32 = 1 << 20;

pub(super) struct Cff {
    /// The table within the font file; every offset below is relative to it.
    table: Range<usize>,
    global_subrs: Index,
    char_strings: Index,
    charset: Charset,
    kind: Kind,
}

enum Kind {
    /// One private dictionary for all glyphs.
    Plain { local_subrs: Index },
    /// A font dictionary per group of glyphs; the subroutines of each.
    CidKeyed { select: FdSelect, local_subrs: Vec<Option<Index>> },
}

/// A list of byte strings: a count, the size of an offset, one more offset
/// than there are strings, then the strings. Offsets count from 1.
#[derive(Clone, Copy, Default)]
struct Index {
    offsets: usize,
    /// Number of offsets, one more than the strings held.
    entries: u32,
    offset_size: u8,
    data: usize,
    data_len: usize,
}

impl Index {
    fn parse(reader: &mut Reader) -> Option<Index> {
        let count = reader.u16()?;
        if count == 0 {
            return Some(Index::default());
        }
        let offset_size = reader.u8().filter(|size| (1..=4).contains(size))?;
        let entries = u32::from(count) + 1;
        let offsets = reader.at;
        reader.bytes(entries as usize * usize::from(offset_size))?;
        let mut index = Index { offsets, entries, offset_size, data: reader.at, data_len: 0 };
        match index.offset(reader.data, entries - 1) {
            Some(last) => {
                reader.bytes(last)?;
                index.data_len = last;
                Some(index)
            }
            None => Some(Index::default()),
        }
    }

    /// Steps over an index without keeping it.
    fn skip(reader: &mut Reader) -> Option<()> {
        let count = reader.u16()?;
        if count == 0 {
            return Some(());
        }
        let offset_size = reader.u8().filter(|size| (1..=4).contains(size))?;
        let last = reader.bytes((usize::from(count) + 1) * usize::from(offset_size))?;
        let last = &last[last.len() - usize::from(offset_size)..];
        let last = last.iter().fold(0usize, |value, byte| value << 8 | usize::from(*byte));
        reader.skip(last.saturating_sub(1));
        Some(())
    }

    fn len(&self) -> u32 {
        self.entries.saturating_sub(1)
    }

    fn offset(&self, table: &[u8], entry: u32) -> Option<usize> {
        if entry >= self.entries {
            return None;
        }
        let size = usize::from(self.offset_size);
        let bytes = table.get(self.offsets + entry as usize * size..)?.get(..size)?;
        bytes.iter().fold(0usize, |value, byte| value << 8 | usize::from(*byte)).checked_sub(1)
    }

    fn get<'a>(&self, table: &'a [u8], index: u32) -> Option<&'a [u8]> {
        let (start, end) = (self.offset(table, index)?, self.offset(table, index.checked_add(1)?)?);
        table.get(self.data..self.data + self.data_len)?.get(start..end)
    }
}

/// Reads a dictionary: operands followed by the operator that takes them.
struct Dict<'a> {
    data: &'a [u8],
    at: usize,
    operands: usize,
}

impl<'a> Dict<'a> {
    fn new(data: &'a [u8]) -> Self {
        Dict { data, at: 0, operands: 0 }
    }

    /// Advances to the next operator. Two-byte operators are 1200 and up.
    fn next(&mut self) -> Option<u16> {
        let mut reader = Reader::new_at(self.data, self.at)?;
        self.operands = self.at;
        while !reader.at_end() {
            let byte = reader.u8()?;
            if is_operator(byte) {
                let operator = if byte == 12 { 1200 + u16::from(reader.u8()?) } else { u16::from(byte) };
                self.at = reader.at;
                return Some(operator);
            }
            // Step over a number without reading it.
            match byte {
                28 => reader.skip(2),
                29 => reader.skip(4),
                30 => {
                    while !reader.at_end() {
                        let pair = reader.u8()?;
                        if pair >> 4 == 0xF || pair & 0xF == 0xF {
                            break;
                        }
                    }
                }
                247..=254 => reader.skip(1),
                _ => {}
            }
        }
        None
    }

    /// The operands of the operator `next` returned.
    fn operands(&self) -> Option<Vec<f64>> {
        let mut reader = Reader::new_at(self.data, self.operands)?;
        let mut operands = Vec::new();
        while !reader.at_end() && operands.len() < MAX_OPERANDS {
            let byte = reader.u8()?;
            if is_operator(byte) {
                break;
            }
            operands.push(match byte {
                28 => f64::from(reader.i16()?),
                29 => f64::from(reader.i32()?),
                30 => real(&mut reader)?,
                32..=246 => f64::from(i32::from(byte) - 139),
                247..=250 => f64::from((i32::from(byte) - 247) * 256 + i32::from(reader.u8()?) + 108),
                251..=254 => f64::from(-(i32::from(byte) - 251) * 256 - i32::from(reader.u8()?) - 108),
                _ => return None,
            });
        }
        Some(operands)
    }

    fn offset(&self) -> Option<usize> {
        match self.operands()?.as_slice() {
            [offset] => usize::try_from(*offset as i32).ok(),
            _ => None,
        }
    }

    /// A size and an offset, as a range.
    fn range(&self) -> Option<Range<usize>> {
        match self.operands()?.as_slice() {
            [len, start] => {
                let (len, start) = (usize::try_from(*len as i32).ok()?, usize::try_from(*start as i32).ok()?);
                Some(start..start.checked_add(len)?)
            }
            _ => None,
        }
    }
}

fn is_operator(byte: u8) -> bool {
    matches!(byte, 0..=27 | 31 | 255)
}

/// A real number in a dictionary: decimal digits packed two to a byte.
fn real(reader: &mut Reader) -> Option<f64> {
    let mut text = String::new();
    'digits: loop {
        let pair = reader.u8()?;
        for nibble in [pair >> 4, pair & 0xF] {
            match nibble {
                0..=9 => text.push(char::from(b'0' + nibble)),
                10 => text.push('.'),
                11 => text.push('E'),
                12 => text.push_str("E-"),
                14 => text.push('-'),
                15 => break 'digits,
                _ => return None,
            }
            if text.len() > 64 {
                return None;
            }
        }
    }
    text.parse().ok()
}

const CHARSET: u16 = 15;
const CHAR_STRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const FONT_MATRIX: u16 = 1207;
const ROS: u16 = 1230;
const FD_ARRAY: u16 = 1236;
const FD_SELECT: u16 = 1237;

/// Where a private dictionary's local subroutines are: the offset counts
/// from the start of the dictionary.
fn local_subrs(table: &[u8], private: Range<usize>) -> Option<Option<Index>> {
    let mut dict = Dict::new(table.get(private.clone())?);
    let mut offset = None;
    while let Some(operator) = dict.next() {
        if operator == SUBRS {
            offset = dict.offset();
        }
    }
    let Some(start) = offset.and_then(|offset| private.start.checked_add(offset)) else { return Some(None) };
    Some(Some(Index::parse(&mut Reader::new_at(table, start)?)?))
}

impl Cff {
    pub(super) fn parse(file: &[u8], range: Range<usize>) -> Option<Cff> {
        let table = file.get(range.clone())?;
        let mut reader = Reader::new(table);
        let major = reader.u8()?;
        reader.skip(1); // minor version
        let header = reader.u8()?;
        reader.skip(1); // offset size
        if major != 1 {
            return None;
        }
        reader.skip(usize::from(header).saturating_sub(4));
        Index::skip(&mut reader)?; // font names

        let top = Index::parse(&mut reader)?;
        let mut dict = Dict::new(top.get(table, 0)?);
        let (mut charset, mut char_strings, mut private) = (None, 0, None);
        let (mut cid_keyed, mut fd_array, mut fd_select) = (false, None, None);
        while let Some(operator) = dict.next() {
            match operator {
                CHARSET => charset = dict.offset(),
                CHAR_STRINGS => char_strings = dict.offset()?,
                PRIVATE => private = dict.range(),
                FONT_MATRIX => drop(dict.operands()?),
                ROS => cid_keyed = true,
                FD_ARRAY => fd_array = dict.offset(),
                FD_SELECT => fd_select = dict.offset(),
                _ => {}
            }
        }
        if char_strings == 0 {
            return None;
        }
        Index::parse(&mut reader)?; // strings
        let global_subrs = Index::parse(&mut reader)?;
        let char_strings = Index::parse(&mut Reader::new_at(table, char_strings)?)?;
        let glyphs = u16::try_from(char_strings.len()).ok().filter(|count| *count != 0)?;

        let charset = match charset {
            None | Some(0) => Charset::IsoAdobe,
            Some(1 | 2) => Charset::Expert,
            Some(offset) => Charset::parse(glyphs, &mut Reader::new_at(table, offset)?)?,
        };
        let kind = if cid_keyed {
            let (Some(_), Some(fd_array), Some(fd_select)) = (charset.custom(), fd_array, fd_select) else {
                return None;
            };
            let dicts = Index::parse(&mut Reader::new_at(table, fd_array)?)?;
            let select = FdSelect::parse(glyphs, &mut Reader::new_at(table, fd_select)?)?;
            let subrs_of = |fd: u32| -> Option<Index> {
                let mut dict = Dict::new(dicts.get(table, fd)?);
                let mut private = None;
                while let Some(operator) = dict.next() {
                    if operator == PRIVATE {
                        private = Some(dict.range());
                        break;
                    }
                }
                local_subrs(table, private??).flatten()
            };
            Kind::CidKeyed { select, local_subrs: (0..dicts.len().min(256)).map(subrs_of).collect() }
        } else {
            let local_subrs = match private {
                Some(private) => local_subrs(table, private)?.unwrap_or_default(),
                None => Index::default(),
            };
            Kind::Plain { local_subrs }
        };
        Some(Cff { table: range, global_subrs, char_strings, charset, kind })
    }

    pub(super) fn outline(&self, file: &[u8], glyph: u16, builder: &mut dyn OutlineBuilder) {
        let Some(table) = file.get(self.table.clone()) else { return };
        let Some(program) = self.char_strings.get(table, u32::from(glyph)) else { return };
        let mut run = Run {
            cff: self,
            table,
            glyph,
            local_subrs: match &self.kind {
                Kind::Plain { local_subrs } => Some(Some(*local_subrs)),
                Kind::CidKeyed { .. } => None,
            },
            builder,
            stack: [0.0; MAX_OPERANDS],
            len: 0,
            x: 0.0,
            y: 0.0,
            width_seen: false,
            stems: 0,
            moved: false,
            open: false,
            ended: false,
            accented: false,
            steps_left: MAX_STEPS,
        };
        let _ = run.execute(program, 0);
    }
}

/// Which glyph has which name (or, in a CID-keyed font, which CID). Only
/// needed for glyphs composed of two others, which name their parts.
enum Charset {
    IsoAdobe,
    Expert,
    /// Format 0: the id of every glyph after the first.
    Each {
        ids: usize,
        count: u16,
    },
    /// Formats 1 and 2: runs of consecutive ids; the run length is one or two bytes.
    Runs {
        runs: usize,
        count: u16,
        wide: bool,
    },
}

impl Charset {
    fn parse(glyphs: u16, reader: &mut Reader) -> Option<Charset> {
        let format = reader.u8()?;
        let start = reader.at;
        match format {
            0 => {
                reader.bytes(usize::from(glyphs - 1) * 2)?;
                Some(Charset::Each { ids: start, count: glyphs - 1 })
            }
            1 | 2 => {
                // The number of runs is not stored: they go on until every glyph is covered.
                let wide = format == 2;
                let (mut left, mut count) = (glyphs - 1, 0u16);
                while left > 0 {
                    reader.skip(2);
                    let more = if wide { reader.u16()? } else { u16::from(reader.u8()?) };
                    left = left.checked_sub(more.checked_add(1)?)?;
                    count += 1;
                }
                Some(Charset::Runs { runs: start, count, wide })
            }
            _ => None,
        }
    }

    /// The charset is the font's own rather than a predefined one.
    fn custom(&self) -> Option<()> {
        matches!(self, Charset::Each { .. } | Charset::Runs { .. }).then_some(())
    }

    fn glyph_of(&self, table: &[u8], id: u16) -> Option<u16> {
        if id == 0 {
            return Some(0);
        }
        match *self {
            Charset::IsoAdobe | Charset::Expert => None,
            Charset::Each { ids, count } => {
                let ids = table.get(ids..ids + usize::from(count) * 2)?;
                let position = ids.chunks_exact(2).position(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]) == id)?;
                Some(position as u16 + 1)
            }
            Charset::Runs { runs, count, wide } => {
                let mut reader = Reader::new_at(table, runs)?;
                let mut glyph = 1u16;
                for _ in 0..count {
                    let first = reader.u16()?;
                    let more = if wide { reader.u16()? } else { u16::from(reader.u8()?) };
                    if first <= id && u32::from(id) <= u32::from(first) + u32::from(more) {
                        return Some(glyph.wrapping_add(id - first));
                    }
                    glyph = glyph.wrapping_add(more).wrapping_add(1);
                }
                None
            }
        }
    }
}

/// Which font dictionary each glyph belongs to.
enum FdSelect {
    /// Format 0: a byte per glyph.
    Each { at: usize, count: u16 },
    /// Format 3: ranges of glyphs, each given by its first glyph, closed by
    /// a final glyph number.
    Ranges { at: usize },
}

impl FdSelect {
    fn parse(glyphs: u16, reader: &mut Reader) -> Option<FdSelect> {
        match reader.u8()? {
            0 => {
                let at = reader.at;
                reader.bytes(usize::from(glyphs))?;
                Some(FdSelect::Each { at, count: glyphs })
            }
            3 => Some(FdSelect::Ranges { at: reader.at }),
            _ => None,
        }
    }

    fn dict_of(&self, table: &[u8], glyph: u16) -> Option<u8> {
        match *self {
            FdSelect::Each { at, count } => table.get(at..at + usize::from(count))?.get(usize::from(glyph)).copied(),
            FdSelect::Ranges { at } => {
                let mut reader = Reader::new_at(table, at)?;
                let ranges = reader.u16()?;
                if ranges == 0 || ranges == u16::MAX {
                    return None;
                }
                let (mut first, mut dict) = (reader.u16()?, reader.u8()?);
                for _ in 0..ranges {
                    let next = reader.u16()?;
                    if (first..next).contains(&glyph) {
                        return Some(dict);
                    }
                    dict = reader.u8()?;
                    first = next;
                }
                None
            }
        }
    }
}

/// What stopped a charstring. Nothing acts on the reason; the names say
/// what the checks are for.
#[derive(Debug)]
enum Stop {
    CutShort,
    BadOperator,
    BadOperands,
    StackOverflow,
    TooDeep,
    TooLong,
    NoSuchSubroutine,
    NoMoveTo,
    DataAfterEnd,
    BadAccent,
}

/// The state of one glyph's charstring as it runs.
struct Run<'a> {
    cff: &'a Cff,
    table: &'a [u8],
    glyph: u16,
    /// Not looked up until first called; `Some(None)` when the font dictionary has none.
    local_subrs: Option<Option<Index>>,
    builder: &'a mut dyn OutlineBuilder,
    stack: [f32; MAX_OPERANDS],
    len: usize,
    x: f32,
    y: f32,
    /// The first operator may take one operand more than it needs: the
    /// glyph's width, which is not wanted here but has to be recognised.
    width_seen: bool,
    stems: u32,
    moved: bool,
    /// A contour has been started and not yet closed.
    open: bool,
    ended: bool,
    /// The glyph is an accented letter built from two other glyphs.
    accented: bool,
    steps_left: u32,
}

/// The codes by which an accented glyph names its two parts, as string ids.
#[rustfmt::skip]
const STANDARD_ENCODING: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      1,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,
     17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,
     33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,  48,
     49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,  62,  63,  64,
     65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  77,  78,  79,  80,
     81,  82,  83,  84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,  96,  97,  98,  99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110,
      0, 111, 112, 113, 114,   0, 115, 116, 117, 118, 119, 120, 121, 122,   0, 123,
      0, 124, 125, 126, 127, 128, 129, 130, 131,   0, 132, 133,   0, 134, 135, 136,
    137,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0, 138,   0, 139,   0,   0,   0,   0, 140, 141, 142, 143,   0,   0,   0,   0,
      0, 144,   0,   0,   0, 145,   0,   0, 146, 147, 148, 149,   0,   0,   0,   0,
];

/// The number a subroutine index is offset by, which depends on how many there are.
fn bias(subroutines: u32) -> i32 {
    match subroutines {
        0..=1239 => 107,
        1240..=33899 => 1131,
        _ => 32768,
    }
}

/// A float as an integer, if it is within range; fractions are cut off.
fn whole(value: f32) -> Option<i32> {
    (value >= i32::MIN as f32 && value < i32::MAX as f32).then_some(value as i32)
}

impl<'a> Run<'a> {
    fn push(&mut self, value: f32) -> Result<(), Stop> {
        let slot = self.stack.get_mut(self.len).ok_or(Stop::StackOverflow)?;
        *slot = value;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> f32 {
        self.len = self.len.saturating_sub(1);
        self.stack[self.len]
    }

    fn move_to(&mut self) {
        if self.open {
            self.builder.close();
        }
        self.open = true;
        self.moved = true;
        self.builder.move_to(self.x, self.y);
        self.len = 0;
    }

    fn line_by(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        self.builder.line_to(self.x, self.y);
    }

    fn curve(&mut self, [x1, y1, x2, y2, x, y]: [f32; 6]) {
        (self.x, self.y) = (x, y);
        self.builder.curve_to(x1, y1, x2, y2, x, y);
    }

    /// A curve given as six deltas, each point relative to the one before.
    fn curve_by(&mut self, at: usize) {
        let s = &self.stack;
        let (x1, y1) = (self.x + s[at], self.y + s[at + 1]);
        let (x2, y2) = (x1 + s[at + 2], y1 + s[at + 3]);
        (self.x, self.y) = (x2 + s[at + 4], y2 + s[at + 5]);
        self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
    }

    /// The glyph named by a code of the standard encoding.
    fn part(&self, code: f32) -> Result<u16, Stop> {
        let code = whole(code).and_then(|code| u8::try_from(code).ok()).ok_or(Stop::BadAccent)?;
        let id = u16::from(STANDARD_ENCODING[usize::from(code)]);
        let glyph = match self.cff.charset {
            Charset::IsoAdobe => (code <= 228).then_some(id),
            Charset::Expert => None,
            _ => self.cff.charset.glyph_of(self.table, id),
        };
        glyph.ok_or(Stop::BadAccent)
    }

    fn program(&self, glyph: u16) -> Result<&'a [u8], Stop> {
        self.cff.char_strings.get(self.table, u32::from(glyph)).ok_or(Stop::BadAccent)
    }

    fn call(&mut self, subroutines: Index, depth: u8) -> Result<(), Stop> {
        let index = whole(self.pop()).and_then(|index| index.checked_add(bias(subroutines.len())));
        let index = index.and_then(|index| u32::try_from(index).ok()).ok_or(Stop::NoSuchSubroutine)?;
        let program = subroutines.get(self.table, index).ok_or(Stop::NoSuchSubroutine)?;
        self.execute(program, depth + 1)
    }

    fn execute(&mut self, program: &[u8], depth: u8) -> Result<(), Stop> {
        let mut reader = Reader::new(program);
        while !reader.at_end() {
            let operator = reader.u8().ok_or(Stop::CutShort)?;
            self.steps_left = self.steps_left.checked_sub(1).ok_or(Stop::TooLong)?;
            match operator {
                0 | 2 | 9 | 13 | 15 | 16 | 17 => return Err(Stop::BadOperator),
                // hstem, vstem, hstemhm, vstemhm: hints, two operands each.
                1 | 3 | 18 | 23 => {
                    if self.len % 2 == 1 {
                        self.width_seen = true;
                    }
                    self.stems += self.len as u32 >> 1;
                    self.len = 0;
                }
                // vmoveto
                4 => {
                    let at = self.leading_width(1)?;
                    self.y += self.stack[at];
                    self.move_to();
                }
                // rlineto
                5 => {
                    self.drawing(self.len.is_multiple_of(2))?;
                    for at in (0..self.len).step_by(2) {
                        self.line_by(self.stack[at], self.stack[at + 1]);
                    }
                    self.len = 0;
                }
                // hlineto, vlineto: alternately horizontal and vertical.
                6 | 7 => {
                    self.drawing(self.len > 0)?;
                    for at in 0..self.len {
                        match (at % 2 == 0) == (operator == 6) {
                            true => self.x += self.stack[at],
                            false => self.y += self.stack[at],
                        }
                        self.builder.line_to(self.x, self.y);
                    }
                    self.len = 0;
                }
                // rrcurveto
                8 => {
                    self.drawing(self.len.is_multiple_of(6))?;
                    for at in (0..self.len).step_by(6) {
                        self.curve_by(at);
                    }
                    self.len = 0;
                }
                // callsubr
                10 => {
                    self.callable(depth)?;
                    if self.local_subrs.is_none() {
                        if let Kind::CidKeyed { select, local_subrs } = &self.cff.kind {
                            let dict = select.dict_of(self.table, self.glyph);
                            self.local_subrs = Some(dict.and_then(|dict| *local_subrs.get(usize::from(dict))?));
                        }
                    }
                    let subroutines = self.local_subrs.flatten().ok_or(Stop::NoSuchSubroutine)?;
                    self.call(subroutines, depth)?;
                    if self.ended && !self.accented {
                        return if reader.at_end() { Ok(()) } else { Err(Stop::DataAfterEnd) };
                    }
                }
                // return
                11 => break,
                12 => {
                    let operator = reader.u8().ok_or(Stop::CutShort)?;
                    self.flex(operator)?;
                }
                // endchar
                14 => {
                    if self.len == 4 || (!self.width_seen && self.len == 5) {
                        // An accented letter: the base glyph, then the
                        // accent drawn from the given point.
                        let (accent, base) = (self.pop(), self.pop());
                        let (accent, base) = (self.part(accent)?, self.part(base)?);
                        let (dy, dx) = (self.pop(), self.pop());
                        if self.len > 0 {
                            self.width_seen = true;
                            self.len = 0;
                        }
                        self.accented = true;
                        if depth == MAX_DEPTH {
                            return Err(Stop::TooDeep);
                        }
                        self.execute(self.program(base)?, depth + 1)?;
                        (self.x, self.y) = (dx, dy);
                        self.execute(self.program(accent)?, depth + 1)?;
                    } else if self.len == 1 && !self.width_seen {
                        self.width_seen = true;
                        self.len = 0;
                    }
                    if self.open {
                        self.open = false;
                        self.builder.close();
                    }
                    if !reader.at_end() {
                        return Err(Stop::DataAfterEnd);
                    }
                    self.ended = true;
                    break;
                }
                // hintmask, cntrmask: a bit per stem follows.
                19 | 20 => {
                    if self.len % 2 == 1 {
                        self.width_seen = true;
                    }
                    self.stems += self.len as u32 >> 1;
                    self.len = 0;
                    reader.skip((self.stems as usize + 7) >> 3);
                }
                // rmoveto
                21 => {
                    let at = self.leading_width(2)?;
                    self.x += self.stack[at];
                    self.y += self.stack[at + 1];
                    self.move_to();
                }
                // hmoveto
                22 => {
                    let at = self.leading_width(1)?;
                    self.x += self.stack[at];
                    self.move_to();
                }
                // rcurveline: curves, then one line.
                24 => {
                    self.drawing(self.len >= 8 && (self.len - 2).is_multiple_of(6))?;
                    for at in (0..self.len - 2).step_by(6) {
                        self.curve_by(at);
                    }
                    self.line_by(self.stack[self.len - 2], self.stack[self.len - 1]);
                    self.len = 0;
                }
                // rlinecurve: lines, then one curve.
                25 => {
                    self.drawing(self.len >= 8 && (self.len - 6).is_multiple_of(2))?;
                    for at in (0..self.len - 6).step_by(2) {
                        self.line_by(self.stack[at], self.stack[at + 1]);
                    }
                    self.curve_by(self.len - 6);
                    self.len = 0;
                }
                // vvcurveto, hhcurveto: curves that start and end along one axis.
                26 | 27 => {
                    self.drawing(true)?;
                    let vertical = operator == 26;
                    let mut at = 0;
                    if self.len % 2 == 1 {
                        match vertical {
                            true => self.x += self.stack[0],
                            false => self.y += self.stack[0],
                        }
                        at = 1;
                    }
                    if !(self.len - at).is_multiple_of(4) {
                        return Err(Stop::BadOperands);
                    }
                    while at < self.len {
                        let s = &self.stack;
                        let (x1, y1, x2, y2);
                        if vertical {
                            (x1, y1) = (self.x, self.y + s[at]);
                            (x2, y2) = (x1 + s[at + 1], y1 + s[at + 2]);
                            (self.x, self.y) = (x2, y2 + s[at + 3]);
                        } else {
                            (x1, y1) = (self.x + s[at], self.y);
                            (x2, y2) = (x1 + s[at + 1], y1 + s[at + 2]);
                            (self.x, self.y) = (x2 + s[at + 3], y2);
                        }
                        self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
                        at += 4;
                    }
                    self.len = 0;
                }
                28 => {
                    let value = reader.i16().ok_or(Stop::CutShort)?;
                    self.push(f32::from(value))?;
                }
                // callgsubr
                29 => {
                    self.callable(depth)?;
                    self.call(self.cff.global_subrs, depth)?;
                    if self.ended && !self.accented {
                        return if reader.at_end() { Ok(()) } else { Err(Stop::DataAfterEnd) };
                    }
                }
                // vhcurveto, hvcurveto: curves alternately leaving along one
                // axis and arriving along the other.
                30 | 31 => {
                    self.drawing(self.len >= 4)?;
                    let mut horizontal = operator == 31;
                    let mut at = 0;
                    while at < self.len {
                        let left = self.len - at;
                        if left < 4 {
                            return Err(Stop::BadOperands);
                        }
                        let s = &self.stack;
                        // The last curve may give its end point's other coordinate too.
                        let extra = if left == 5 { s[at + 4] } else { 0.0 };
                        let (x1, y1, x2, y2);
                        if horizontal {
                            (x1, y1) = (self.x + s[at], self.y);
                            (x2, y2) = (x1 + s[at + 1], y1 + s[at + 2]);
                            self.y = y2 + s[at + 3];
                            self.x = if left == 5 { x2 + extra } else { x2 };
                        } else {
                            (x1, y1) = (self.x, self.y + s[at]);
                            (x2, y2) = (x1 + s[at + 1], y1 + s[at + 2]);
                            self.x = x2 + s[at + 3];
                            self.y = if left == 5 { y2 + extra } else { y2 };
                        }
                        self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
                        at += if left == 5 { 5 } else { 4 };
                        horizontal = !horizontal;
                    }
                    self.len = 0;
                }
                32..=246 => self.push(f32::from(i16::from(operator) - 139))?,
                247..=250 => {
                    let low = reader.u8().ok_or(Stop::CutShort)?;
                    self.push(f32::from((i16::from(operator) - 247) * 256 + i16::from(low) + 108))?;
                }
                251..=254 => {
                    let low = reader.u8().ok_or(Stop::CutShort)?;
                    self.push(f32::from(-(i16::from(operator) - 251) * 256 - i16::from(low) - 108))?;
                }
                255 => {
                    let value = reader.i32().ok_or(Stop::CutShort)?;
                    self.push(value as f32 / 65536.0)?;
                }
            }
        }
        Ok(())
    }

    /// For the moveto operators, which take `wanted` operands: where they
    /// start, after a width if one leads them.
    fn leading_width(&mut self, wanted: usize) -> Result<usize, Stop> {
        let at = usize::from(self.len == wanted + 1);
        if at == 1 {
            self.width_seen = true;
        }
        if self.len != at + wanted {
            return Err(Stop::BadOperands);
        }
        Ok(at)
    }

    /// Checks made by every drawing operator.
    fn drawing(&self, operands_fit: bool) -> Result<(), Stop> {
        if !self.moved {
            return Err(Stop::NoMoveTo);
        }
        if !operands_fit {
            return Err(Stop::BadOperands);
        }
        Ok(())
    }

    fn callable(&self, depth: u8) -> Result<(), Stop> {
        if self.len == 0 {
            return Err(Stop::BadOperands);
        }
        if depth == MAX_DEPTH {
            return Err(Stop::TooDeep);
        }
        Ok(())
    }

    /// The flex operators: two curves meant to be drawn as a straight line
    /// when shallow enough. They are drawn as the two curves. The four differ
    /// in which coordinates are given and which are implied.
    fn flex(&mut self, operator: u8) -> Result<(), Stop> {
        let s = self.stack;
        let (x, y) = (self.x, self.y);
        match operator {
            // hflex: starts and ends at the same height.
            34 => {
                self.drawing(self.len == 7)?;
                let (x1, y1) = (x + s[0], y);
                let (x2, y2) = (x1 + s[1], y1 + s[2]);
                let x3 = x2 + s[3];
                let x4 = x3 + s[4];
                let x5 = x4 + s[5];
                self.curve([x1, y1, x2, y2, x3, y2]);
                self.curve([x4, y2, x5, y, x5 + s[6], y]);
            }
            // flex: every point in full.
            35 => {
                self.drawing(self.len == 13)?;
                let (x1, y1) = (x + s[0], y + s[1]);
                let (x2, y2) = (x1 + s[2], y1 + s[3]);
                let (x3, y3) = (x2 + s[4], y2 + s[5]);
                let (x4, y4) = (x3 + s[6], y3 + s[7]);
                let (x5, y5) = (x4 + s[8], y4 + s[9]);
                self.curve([x1, y1, x2, y2, x3, y3]);
                self.curve([x4, y4, x5, y5, x5 + s[10], y5 + s[11]]);
            }
            // hflex1: flat in the middle, ends at the starting height.
            36 => {
                self.drawing(self.len == 9)?;
                let (x1, y1) = (x + s[0], y + s[1]);
                let (x2, y2) = (x1 + s[2], y1 + s[3]);
                let x3 = x2 + s[4];
                let x4 = x3 + s[5];
                let (x5, y5) = (x4 + s[6], y2 + s[7]);
                self.curve([x1, y1, x2, y2, x3, y2]);
                self.curve([x4, y2, x5, y5, x5 + s[8], y]);
            }
            // flex1: the last delta runs along whichever axis the curves went further.
            37 => {
                self.drawing(self.len == 11)?;
                let (x1, y1) = (x + s[0], y + s[1]);
                let (x2, y2) = (x1 + s[2], y1 + s[3]);
                let (x3, y3) = (x2 + s[4], y2 + s[5]);
                let (x4, y4) = (x3 + s[6], y3 + s[7]);
                let (x5, y5) = (x4 + s[8], y4 + s[9]);
                let end = if (x5 - x).abs() > (y5 - y).abs() { (x5 + s[10], y) } else { (x, y5 + s[10]) };
                self.curve([x1, y1, x2, y2, x3, y3]);
                self.curve([x4, y4, x5, y5, end.0, end.1]);
            }
            _ => return Err(Stop::BadOperator),
        }
        self.len = 0;
        Ok(())
    }
}
