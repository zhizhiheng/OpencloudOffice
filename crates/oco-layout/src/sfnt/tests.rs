//! Tests of the font reader.
//!
//! Three kinds. Small fonts built here by hand, whose every value can be
//! checked against the bytes that were written. Digests of everything the
//! reader says about the fonts the engine actually uses, recorded when its
//! output was compared, glyph by glyph, with an independent reader and
//! found identical; these are skipped where the font files are absent.
//! And damaged fonts, which must be refused or read in part, never crash.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{faces_in, Face, OutlineBuilder};

fn be16(value: i32) -> [u8; 2] {
    (value as u16).to_be_bytes()
}

fn be32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

/// A font file holding the given tables.
fn font(magic: u32, tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut file = be32(magic).to_vec();
    file.extend(be16(tables.len() as i32));
    file.extend([0; 6]);
    let mut offset = 12 + tables.len() * 16;
    for (tag, data) in tables {
        file.extend(*tag);
        file.extend(be32(0));
        file.extend(be32(offset as u32));
        file.extend(be32(data.len() as u32));
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        file.extend(data);
        file.resize(file.len().next_multiple_of(4), 0);
    }
    file
}

fn head(units_per_em: i32) -> Vec<u8> {
    let mut head = vec![0; 54];
    head[18..20].copy_from_slice(&be16(units_per_em));
    head
}

fn hhea(ascender: i32, descender: i32, line_gap: i32, full_metrics: i32) -> Vec<u8> {
    let mut hhea = vec![0; 36];
    hhea[4..6].copy_from_slice(&be16(ascender));
    hhea[6..8].copy_from_slice(&be16(descender));
    hhea[8..10].copy_from_slice(&be16(line_gap));
    hhea[34..36].copy_from_slice(&be16(full_metrics));
    hhea
}

fn maxp(version: u32, glyphs: i32) -> Vec<u8> {
    [be32(version).as_slice(), &be16(glyphs)].concat()
}

/// Collects an outline the way the SVG writer does, as path data.
#[derive(Default)]
struct Drawn(String);

impl OutlineBuilder for Drawn {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{x} {y} ");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{x} {y} ");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{x1} {y1} {x} {y} ");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(self.0, "C{x1} {y1} {x2} {y2} {x} {y} ");
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

fn drawn(face: &Face, glyph: u16) -> String {
    let mut path = Drawn::default();
    face.outline(glyph, &mut path);
    path.0
}

/// A glyph of contours whose points are all on the curve and given as
/// 16-bit deltas, which is the plainest encoding there is.
fn polygon(points: &[(i32, i32)]) -> Vec<u8> {
    let mut glyph = be16(1).to_vec();
    glyph.extend([0; 8]);
    glyph.extend(be16(points.len() as i32 - 1));
    glyph.extend(be16(0));
    glyph.extend(vec![0x01; points.len()]);
    let deltas = |pick: fn(&(i32, i32)) -> i32| {
        let mut previous = 0;
        points.iter().map(pick).flat_map(move |value| be16(value - std::mem::replace(&mut previous, value)))
    };
    glyph.extend(deltas(|point| point.0).collect::<Vec<_>>());
    glyph.extend(deltas(|point| point.1).collect::<Vec<_>>());
    glyph
}

/// A composite glyph: the given glyphs, each moved by the given amount.
fn composite_of(parts: &[(i32, (i32, i32))]) -> Vec<u8> {
    let mut glyph = be16(-1).to_vec();
    glyph.extend([0; 8]);
    for (index, (part, (dx, dy))) in parts.iter().enumerate() {
        let more = if index + 1 < parts.len() { 0x0020 } else { 0 };
        glyph.extend([be16(0x0001 | 0x0002 | more), be16(*part), be16(*dx), be16(*dy)].concat());
    }
    glyph
}

/// A TrueType font with these glyphs and nothing else a font may lack.
fn truetype_of(glyphs: &[Vec<u8>]) -> Vec<u8> {
    let (mut loca, mut glyf) = (Vec::new(), Vec::new());
    for glyph in glyphs {
        loca.extend(be32(glyf.len() as u32));
        glyf.extend(glyph);
    }
    loca.extend(be32(glyf.len() as u32));
    let mut head = head(1000);
    head[50..52].copy_from_slice(&be16(1));
    font(
        0x0001_0000,
        &[
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea(800, -200, 0, 1)),
            (b"hmtx", vec![0; 4]),
            (b"loca", loca),
            (b"maxp", maxp(0x0001_0000, glyphs.len() as i32)),
        ],
    )
}

/// A TrueType font of six glyphs: nothing, a triangle, a round shape given
/// only by control points, a composite of two triangles, a shape whose
/// points use the compact encodings, and a composite of the composite.
fn truetype() -> Vec<u8> {
    let triangle = polygon(&[(0, 0), (100, 0), (50, 80)]);
    // Four control points and no point on the curve.
    let mut round = be16(1).to_vec();
    round.extend([0; 8]);
    round.extend([be16(3), be16(0)].concat());
    round.extend([0x00; 4]);
    round.extend([be16(0), be16(50), be16(50), be16(-50)].concat());
    round.extend([be16(50), be16(50), be16(-50), be16(-50)].concat());
    // The triangle attached by matching its point 3 to point 0, which is
    // not done and leaves it where it is; then the triangle at half size
    // moved by (10, 20).
    let mut composite = be16(-1).to_vec();
    composite.extend([0; 8]);
    composite.extend([be16(0x0020), be16(1)].concat());
    composite.extend([3, 0]);
    composite.extend([be16(0x0001 | 0x0002 | 0x0008), be16(1), be16(10), be16(20), be16(0x2000)].concat());
    // (0,0) (10,0) (20,0) (20,10): a repeated flag, one-byte deltas, deltas left out.
    let mut compact = be16(1).to_vec();
    compact.extend([0; 8]);
    compact.extend([be16(3), be16(0)].concat());
    compact.extend([0x31, 0x3B, 1, 0x35]);
    compact.extend([10, 10]);
    compact.extend([10]);

    // The composite mirrored at twice the size and moved by (100, 0). (The
    // scale is a 2.14 fixed-point number, which reaches -2 but not 2.)
    let mut nested = be16(-1).to_vec();
    nested.extend([0; 8]);
    nested.extend([be16(0x0001 | 0x0002 | 0x0008), be16(3), be16(100), be16(0), be16(-0x8000)].concat());

    let glyphs = [Vec::new(), triangle, round, composite, compact, nested];
    let (mut loca, mut glyf) = (Vec::new(), Vec::new());
    for glyph in &glyphs {
        loca.extend(be16(glyf.len() as i32 / 2));
        glyf.extend(glyph);
        glyf.resize(glyf.len().next_multiple_of(2), 0);
    }
    loca.extend(be16(glyf.len() as i32 / 2));

    // 'A' and 'B' by a delta; 'a' to 'c' through the glyph array, where 0
    // means "no glyph"; and the closing segment every such table has.
    let mut bmp = [be16(4), be16(0), be16(0), be16(6), be16(0), be16(0), be16(0)].concat();
    bmp.extend([be16(0x42), be16(0x63), be16(0xFFFF), be16(0)].concat());
    bmp.extend([be16(0x41), be16(0x61), be16(0xFFFF)].concat());
    bmp.extend([be16(1 - 0x41), be16(0), be16(1)].concat());
    bmp.extend([be16(0), be16(4), be16(0)].concat());
    bmp.extend([be16(4), be16(0), be16(2)].concat());
    let mut full = [be16(12), be16(0)].concat();
    full.extend([be32(28), be32(0), be32(1), be32(0x1F600), be32(0x1F600), be32(3)].concat());
    let mut cmap = [be16(0), be16(2)].concat();
    cmap.extend([be16(3).as_slice(), &be16(1), &be32(20), &be16(3), &be16(10), &be32(20 + bmp.len() as u32)].concat());
    cmap.extend(bmp);
    cmap.extend(full);

    let utf16 = |text: &str| text.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let strings = [utf16("Test Sans"), b"Skipped".to_vec(), utf16("测试")];
    let mut name = [be16(0), be16(3), be16(6 + 3 * 12)].concat();
    let mut offset = 0;
    for ((platform, encoding, language, id), text) in
        [(3, 1, 0x409, 1), (1, 0, 0, 1), (3, 1, 0x804, 16)].iter().zip(&strings)
    {
        name.extend([be16(*platform), be16(*encoding), be16(*language), be16(*id)].concat());
        name.extend([be16(text.len() as i32), be16(offset)].concat());
        offset += text.len() as i32;
    }
    name.extend(strings.concat());

    let mut os2 = vec![0; 96];
    os2[0..2].copy_from_slice(&be16(4));
    os2[26..28].copy_from_slice(&be16(50));
    os2[28..30].copy_from_slice(&be16(250));
    os2[62..64].copy_from_slice(&be16(0x20 | 0x80));
    for (at, value) in [(68, 750), (70, -250), (72, 50), (74, 900), (76, 300)] {
        os2[at..at + 2].copy_from_slice(&be16(value));
    }
    let mut post = vec![0; 32];
    post[0..4].copy_from_slice(&be32(0x0003_0000));
    post[8..10].copy_from_slice(&be16(-100));
    post[10..12].copy_from_slice(&be16(50));

    let hmtx = [be16(500), be16(0), be16(600), be16(0), be16(0), be16(0), be16(0), be16(0)].concat();
    font(
        0x0001_0000,
        &[
            (b"OS/2", os2),
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head(1000)),
            (b"hhea", hhea(800, -200, 90, 2)),
            (b"hmtx", hmtx),
            (b"loca", loca),
            (b"maxp", maxp(0x0001_0000, 6)),
            (b"name", name),
            (b"post", post),
        ],
    )
}

fn open(data: Vec<u8>) -> Face {
    Face::parse(data.into(), 0).expect("the font opens")
}

#[test]
fn reads_the_metrics_and_names_of_a_font() {
    let face = open(truetype());
    assert_eq!(face.units_per_em(), 1000);
    let hhea = face.hhea();
    assert_eq!((hhea.ascender, hhea.descender, hhea.line_gap), (800, -200, 90));
    let os2 = face.os2().unwrap();
    assert!(os2.use_typographic_metrics && os2.bold && !os2.italic);
    assert_eq!((os2.typographic_ascender, os2.typographic_descender, os2.typographic_line_gap), (750, -250, 50));
    // The Windows descent is stored as a distance; it is handed out as a coordinate.
    assert_eq!((os2.windows_ascender, os2.windows_descender), (900, -300));
    assert_eq!((os2.strikeout_position, os2.strikeout_thickness), (250, 50));
    let post = face.post().unwrap();
    assert_eq!((post.underline_position, post.underline_thickness, post.slanted), (-100, 50, false));
    assert!(face.is_bold() && !face.is_italic());
    // The name stored in a legacy one-byte encoding is passed over.
    assert_eq!(face.family_names(), ["Test Sans", "测试"]);
}

#[test]
fn maps_characters_to_glyphs() {
    let face = open(truetype());
    assert_eq!(face.glyph_index('A'), Some(1));
    assert_eq!(face.glyph_index('B'), Some(2));
    assert_eq!(face.glyph_index('C'), None);
    assert_eq!(face.glyph_index('a'), Some(4));
    assert_eq!(face.glyph_index('b'), None, "0 in the glyph array means the character is not mapped");
    assert_eq!(face.glyph_index('c'), Some(2));
    assert_eq!(face.glyph_index('\u{FFFF}'), Some(0), "the closing segment maps to the missing glyph");
    assert_eq!(face.glyph_index('😀'), Some(3), "found in the second table, which covers all of Unicode");
    assert_eq!(face.glyph_index('😁'), None);
}

#[test]
fn advances_repeat_the_last_full_metric() {
    let face = open(truetype());
    let advances: Vec<_> = (0..7).map(|glyph| face.advance(glyph)).collect();
    assert_eq!(advances, [Some(500), Some(600), Some(600), Some(600), Some(600), Some(600), None]);
}

#[test]
fn draws_truetype_outlines() {
    let face = open(truetype());
    assert_eq!(drawn(&face, 0), "");
    assert_eq!(drawn(&face, 1), "M0 0 L100 0 L50 80 L0 0 Z");
    // Between two control points lies a point on the curve, half-way.
    assert_eq!(drawn(&face, 2), "M25 75 Q50 100 75 75 Q100 50 75 25 Q50 0 25 25 Q0 50 25 75 Z");
    assert_eq!(drawn(&face, 3), "M0 0 L100 0 L50 80 L0 0 ZM10 20 L60 20 L35 60 L10 20 Z");
    assert_eq!(drawn(&face, 4), "M0 0 L10 0 L20 0 L20 10 L0 0 Z");
    // A part of a part: halved and moved first, then mirrored, doubled and moved.
    assert_eq!(drawn(&face, 5), "M100 0 L-100 0 L0 -160 L100 0 ZM80 -40 L-20 -40 L30 -120 L80 -40 Z");
    assert_eq!(drawn(&face, 6), "");
}

/// A number as a charstring writes it.
fn operand(value: i32) -> Vec<u8> {
    match value {
        -107..=107 => vec![(value + 139) as u8],
        108..=1131 => vec![247 + ((value - 108) >> 8) as u8, (value - 108) as u8],
        -1131..=-108 => vec![251 + ((-value - 108) >> 8) as u8, (-value - 108) as u8],
        _ => [vec![28], be16(value).to_vec()].concat(),
    }
}

fn program(steps: &[(&[i32], &[u8])]) -> Vec<u8> {
    steps
        .iter()
        .flat_map(|(operands, operator)| operands.iter().flat_map(|n| operand(*n)).chain(operator.iter().copied()))
        .collect()
}

/// An INDEX with one-byte offsets.
fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut index = be16(items.len() as i32).to_vec();
    if items.is_empty() {
        return index;
    }
    index.push(1);
    let mut offset = 1;
    index.push(offset);
    for item in items {
        offset += item.len() as u8;
        index.push(offset);
    }
    index.extend(items.concat());
    index
}

/// An OpenType font with PostScript outlines: an empty glyph and one that
/// calls a global and a local subroutine.
fn postscript() -> Vec<u8> {
    let half = [255, 0x00, 0x00, 0x80, 0x00]; // 0.5 as a 16.16 fixed-point number
    let minus_half = [255, 0xFF, 0xFF, 0x80, 0x00];
    let glyph = [
        program(&[
            (&[50, 100, 200], &[21]),                                        // a width, then rmoveto
            (&[-107], &[29]),                                                // callgsubr 0
            (&[-107], &[10]),                                                // callsubr 0
            (&[10, 0, 10, 10, 10, 0, 10, 0, 10, -10, 10, 5, 50], &[12, 35]), // flex
            (&[10, 10, 5, 10, 10, 10, 10], &[12, 34]),                       // hflex
            (&[10, 5, 10, 5, 10, 10, 10, -5, 10], &[12, 36]),                // hflex1
            (&[10, 5, 10, 5, 10, 0, 10, 0, 10, -5, 7], &[12, 37]),           // flex1
        ]),
        half.to_vec(),
        minus_half.to_vec(),
        program(&[
            (&[], &[5]),                   // rlineto, by the two fractions
            (&[10, 20, 30, 40, 5], &[30]), // vhcurveto, with the extra operand
            (&[-2000], &[6]),              // hlineto
            (&[], &[14]),                  // endchar
        ]),
    ]
    .concat();
    let global = program(&[(&[300, 0], &[5]), (&[], &[11])]); // rlineto, return
    let local = program(&[(&[10, 20, 30, 40], &[27]), (&[], &[11])]); // hhcurveto, return
    postscript_of(glyph, &[global], &[local])
}

/// An OpenType font whose glyph 1 is the given program.
fn postscript_of(glyph: Vec<u8>, global: &[Vec<u8>], local: &[Vec<u8>]) -> Vec<u8> {
    let fixed = |value: usize| [vec![29], be32(value as u32).to_vec()].concat();
    let front = [vec![1, 0, 4, 1], index(&[b"Test".to_vec()])].concat();
    let top_len = index(&[vec![0; 17]]).len();
    let rest = [index(&[]), index(global)].concat();
    let char_strings = index(&[vec![14], glyph]);
    let char_strings_at = front.len() + top_len + rest.len();
    let private_at = char_strings_at + char_strings.len();
    let private = [fixed(6), vec![19]].concat(); // local subroutines right after
    let top = [fixed(char_strings_at), vec![17], fixed(private.len()), fixed(private_at), vec![18]].concat();
    let cff = [front, index(&[top]), rest, char_strings, private, index(local)].concat();

    let hmtx = [be16(0), be16(0), be16(700), be16(0)].concat();
    font(
        0x4F54_544F,
        &[
            (b"CFF ", cff),
            (b"head", head(1000)),
            (b"hhea", hhea(880, -120, 0, 2)),
            (b"hmtx", hmtx),
            (b"maxp", maxp(0x0000_5000, 2)),
        ],
    )
}

#[test]
fn draws_postscript_outlines() {
    let face = open(postscript());
    assert_eq!(drawn(&face, 0), "");
    assert_eq!(
        drawn(&face, 1),
        "M100 200 L400 200 C410 200 430 230 470 230 \
         C480 230 490 240 500 240 C510 240 520 230 530 235 \
         C540 235 550 240 560 240 C570 240 580 235 590 235 \
         C600 240 610 245 620 245 C630 245 640 240 650 235 \
         C660 240 670 245 680 245 C690 245 700 240 707 235 \
         L707.5 234.5 C707.5 244.5 727.5 274.5 767.5 279.5 L-1232.5 279.5 Z"
    );
    assert_eq!(face.advance(1), Some(700));
    assert!(face.os2().is_none() && face.post().is_none() && face.family_names().is_empty());
}

#[test]
fn opens_the_faces_of_a_collection() {
    let (first, second) = (truetype(), postscript());
    let mut collection =
        [be32(0x7474_6366), be32(0x0001_0000), be32(2), be32(20), be32(20 + first.len() as u32)].concat();
    // Table offsets count from the start of the file, so each face's directory is moved along.
    for face in [&first, &second] {
        let moved_by = collection.len() as u32;
        let tables = usize::from(u16::from_be_bytes([face[4], face[5]]));
        let mut face = face.clone();
        for record in 0..tables {
            let at = 12 + record * 16 + 8;
            let offset = u32::from_be_bytes([face[at], face[at + 1], face[at + 2], face[at + 3]]);
            face[at..at + 4].copy_from_slice(&be32(offset + moved_by));
        }
        collection.extend(face);
    }
    assert_eq!(faces_in(&collection), 2);
    assert_eq!(faces_in(&first), 1);
    let data: Arc<[u8]> = collection.into();
    assert_eq!(drawn(&Face::parse(Arc::clone(&data), 0).unwrap(), 1), "M0 0 L100 0 L50 80 L0 0 Z");
    assert_eq!(Face::parse(Arc::clone(&data), 1).unwrap().advance(1), Some(700));
    assert!(Face::parse(Arc::clone(&data), 2).is_none());
    assert!(Face::parse(first.into(), 1).is_none(), "a single font has only face 0");
}

#[test]
fn refuses_what_is_not_a_font() {
    assert!(Face::parse(Vec::new().into(), 0).is_none());
    assert!(Face::parse(b"PK\x03\x04 not a font at all".to_vec().into(), 0).is_none());
    // Without its `head` table a font cannot be read.
    let mut headless = truetype();
    let at = headless.windows(4).position(|tag| tag == b"head").unwrap();
    headless[at..at + 4].copy_from_slice(b"hxxx");
    assert!(Face::parse(headless.into(), 0).is_none());
}

/// A glyph may be built from other glyphs, and those from others. A font
/// can use that to ask for an astronomical amount of drawing in a few
/// hundred bytes; the reader stops long before.
#[test]
fn outlines_that_multiply_without_end_are_cut_off() {
    // Each glyph is two of the one before: glyph 31 would be 2³¹ triangles.
    let mut glyphs = vec![polygon(&[(0, 0), (100, 0), (50, 80)])];
    for previous in 0..31 {
        glyphs.push(composite_of(&[(previous, (0, 0)), (previous, (5, 5))]));
    }
    let face = open(truetype_of(&glyphs));
    assert_eq!(drawn(&face, 2).matches('Z').count(), 4);
    let triangles = drawn(&face, 31).matches('Z').count();
    assert!((1..1024).contains(&triangles), "{triangles} triangles drawn");

    // Each subroutine calls the next ten times, as deep as calls may go:
    // 10⁹ calls of the last, which does nothing.
    let call = |subroutine: i32| program(&[(&[subroutine - 107], &[10])]);
    let mut subroutines: Vec<Vec<u8>> = (1..=9).map(|next| call(next).repeat(10)).collect();
    subroutines.push(vec![11]);
    let glyph = |subroutines: &[Vec<u8>]| {
        let glyph = program(&[(&[0, 0], &[21]), (&[-107], &[10]), (&[], &[14])]);
        drawn(&open(postscript_of(glyph, &[], subroutines)), 1)
    };
    assert_eq!(glyph(&subroutines), "M0 0 ", "given up before the end of the glyph");
    // The same glyph with a harmless chain of calls is drawn to its end.
    let harmless: Vec<Vec<u8>> = (1..=9).map(call).chain([vec![11]]).collect();
    assert_eq!(glyph(&harmless), "M0 0 Z");
}

/// FNV-1a, enough to tell whether anything changed.
struct Digest(u64);

impl Digest {
    fn new() -> Self {
        Digest(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn number(&mut self, value: i64) {
        self.bytes(&value.to_be_bytes());
    }
}

impl OutlineBuilder for Digest {
    fn move_to(&mut self, x: f32, y: f32) {
        self.bytes(b"M");
        [x, y].iter().for_each(|v| self.bytes(&v.to_bits().to_be_bytes()));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.bytes(b"L");
        [x, y].iter().for_each(|v| self.bytes(&v.to_bits().to_be_bytes()));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.bytes(b"Q");
        [x1, y1, x, y].iter().for_each(|v| self.bytes(&v.to_bits().to_be_bytes()));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.bytes(b"C");
        [x1, y1, x2, y2, x, y].iter().for_each(|v| self.bytes(&v.to_bits().to_be_bytes()));
    }
    fn close(&mut self) {
        self.bytes(b"Z");
    }
}

/// Everything the reader says about a face, folded into one number: its
/// metrics and names, the glyph of every character up to U+2FFFF, and the
/// advance and outline of every glyph number there can be.
fn digest(face: &Face) -> u64 {
    let mut digest = Digest::new();
    let hhea = face.hhea();
    for value in [i64::from(face.units_per_em()), hhea.ascender.into(), hhea.descender.into(), hhea.line_gap.into()] {
        digest.number(value);
    }
    digest.bytes(format!("{:?} {:?} {} {}", face.os2(), face.post(), face.is_bold(), face.is_italic()).as_bytes());
    digest.bytes(face.family_names().join("\n").as_bytes());
    for ch in (0..0x30000).filter_map(char::from_u32) {
        if let Some(glyph) = face.glyph_index(ch) {
            digest.number(i64::from(u32::from(ch)) << 16 | i64::from(glyph));
        }
    }
    for glyph in 0..=u16::MAX {
        digest.number(face.advance(glyph).map_or(-1, i64::from));
        face.outline(glyph, &mut digest);
    }
    digest.0
}

/// Tells one release of a font file from another: its length and its
/// beginning, where the table directory holds a checksum of every table.
fn file_digest(data: &[u8]) -> u64 {
    let mut digest = Digest::new();
    digest.number(data.len() as i64);
    digest.bytes(&data[..data.len().min(4096)]);
    digest.0
}

/// The font files the engine draws with: those the project fetches and the
/// metric-compatible Latin fonts a system carries.
fn engine_fonts() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs = [
        root.join("fonts"),
        "/usr/share/fonts/truetype/liberation".into(),
        "/usr/share/fonts/truetype/crosextra".into(),
    ];
    let mut files: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .filter(|path| matches!(path.extension().and_then(|e| e.to_str()), Some("ttf" | "otf" | "ttc")))
        .collect();
    files.push("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into());
    files.push("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf".into());
    files.sort();
    files
}

/// (file name, which release of the file, digest of what is read from its
/// first face). To record a new release of a font, run this test with
/// `OCO_PRINT_FONT_DIGESTS=1 -- --nocapture` and paste what it prints; but
/// do that only for a file this list does not have. A digest that changes
/// for a file that is listed means the reader now reads the font differently.
#[rustfmt::skip]
const RECORDED: &[(&str, u64, u64)] = &[
    ("Caladea-Bold.ttf", 0x4f6391bbaba802fa, 0xd8771962bf1ef98e),
    ("Caladea-BoldItalic.ttf", 0x087c7f216062fa90, 0x8ebc67d228273a0d),
    ("Caladea-Italic.ttf", 0xbce644d4f39541c8, 0x8a57473467a6fc9e),
    ("Caladea-Regular.ttf", 0x388464ec120c3ffb, 0x88f00e77564fa437),
    ("Carlito-Bold.ttf", 0x1c4e96d32da1f1b3, 0x7fec0b7e7ec0fcb3),
    ("Carlito-BoldItalic.ttf", 0x5146d85da6fb28bb, 0x3c0ecf1ead07b194),
    ("Carlito-Italic.ttf", 0x7e042b314d56eb29, 0xdc3e69326e52fa36),
    ("Carlito-Regular.ttf", 0x70787406a0ca8aa0, 0x5187940212645ced),
    ("DejaVuSans.ttf", 0x76e3cb2dec7f4166, 0x611f3d7127fe2b21),
    ("DejaVuSerif.ttf", 0x9133932f3c0ee852, 0x8f7c69ceb3d1bd5c),
    ("LXGWWenKai-Regular.ttf", 0x60c9c2c86757817d, 0x8ec92db50892291d),
    ("LiberationMono-Bold.ttf", 0xe9794bfaa569e5f1, 0x71e75afc4400a14f),
    ("LiberationMono-BoldItalic.ttf", 0x8c47036e1ed3659a, 0x6a5e5a3ffb7e9c77),
    ("LiberationMono-Italic.ttf", 0x66f79d7b8154665c, 0x0cf177d29f28b64e),
    ("LiberationMono-Regular.ttf", 0x2bf1a7babe4e9a8a, 0x464280bad666fb9e),
    ("LiberationSans-Bold.ttf", 0xe2c2952870bf663d, 0x4ab2311f41ec2c73),
    ("LiberationSans-BoldItalic.ttf", 0x8a3e7a23154eb9e2, 0x9a15a1981b9a2974),
    ("LiberationSans-Italic.ttf", 0xefb442a66931ab6e, 0x414234852c417ef9),
    ("LiberationSans-Regular.ttf", 0xe10827a54cc4e029, 0xff83206636dba8aa),
    ("LiberationSerif-Bold.ttf", 0xa4436cb99fcc0b15, 0xd1ca7beb18d6b3cf),
    ("LiberationSerif-BoldItalic.ttf", 0x6330c1f7cb838594, 0x5fab67628b6c780f),
    ("LiberationSerif-Italic.ttf", 0xa571456f56dcb3f0, 0xb61ff0fa376bd15f),
    ("LiberationSerif-Regular.ttf", 0xcc699858bb37d72d, 0x56aa05f73ae68f68),
    ("SourceHanSansCN-Bold.otf", 0x202e08437c42ab8d, 0x467512202e16d134),
    ("SourceHanSansCN-Regular.otf", 0xe5a94bb594974f71, 0xfebab35c7dea6f5d),
    ("SourceHanSerifCN-Bold.otf", 0x8ed12add08afb266, 0xa8ae58ae63c3ed29),
    ("SourceHanSerifCN-Regular.otf", 0x6eacbde377ec8e87, 0x92b0cdf67d60b9c5),
];

#[test]
fn reads_the_engine_fonts_as_recorded() {
    let print = std::env::var_os("OCO_PRINT_FONT_DIGESTS").is_some();
    // Each font on its own thread: every glyph of every font is drawn.
    let check = |path: PathBuf| -> bool {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(data) = std::fs::read(&path) else { return false };
        let of_file = file_digest(&data);
        let recorded = RECORDED.iter().find(|(recorded, file, _)| *recorded == name && *file == of_file);
        if recorded.is_none() && !print {
            eprintln!("{name}: not the file that was recorded, skipped");
            return false;
        }
        let face = Face::parse(data.into(), 0).unwrap_or_else(|| panic!("{name} opens"));
        let of_face = digest(&face);
        if print {
            println!("    (\"{name}\", 0x{of_file:016x}, 0x{of_face:016x}),");
        }
        recorded.is_some_and(|(_, _, recorded)| {
            assert_eq!(of_face, *recorded, "{name} is read differently from when it was verified");
            true
        })
    };
    let checked = std::thread::scope(|scope| {
        let threads: Vec<_> = engine_fonts().into_iter().map(|path| scope.spawn(|| check(path))).collect();
        threads.into_iter().map(|thread| thread.join().expect("the check ran")).filter(|ok| *ok).count()
    });
    if checked == 0 {
        eprintln!("no recorded font found; run tools/fonts/fetch.sh to check the font reader against real fonts");
    }
}

/// A small generator of repeatable pseudo-random numbers.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

/// Reads whatever can be read from possibly damaged font data.
fn exercise(data: &[u8]) {
    let data: Arc<[u8]> = data.into();
    for index in 0..faces_in(&data).min(3) {
        let Some(face) = Face::parse(Arc::clone(&data), index) else { continue };
        let _ = (face.units_per_em(), face.hhea(), face.os2(), face.post(), face.is_bold(), face.is_italic());
        let _ = face.family_names();
        let mut sink = Digest::new();
        let chars = ('\0'..='\u{17F}').chain("中文测试あ한😀\u{FFFF}\u{2A6D6}".chars());
        let glyphs: Vec<u16> = chars.filter_map(|ch| face.glyph_index(ch)).chain((0..48).map(|n| n * 1361)).collect();
        for glyph in glyphs.into_iter().chain([u16::MAX, u16::MAX - 1]) {
            let _ = face.advance(glyph);
            face.outline(glyph, &mut sink);
        }
    }
}

/// Where the tables of a font lie, so that damage can be aimed at them.
fn table_ranges(data: &[u8]) -> Vec<(usize, usize)> {
    let start = if data.starts_with(b"ttcf") {
        u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as usize
    } else {
        0
    };
    let count = usize::from(u16::from_be_bytes([data[start + 4], data[start + 5]]));
    let mut ranges = vec![(0, start + 12 + count * 16)];
    for record in data[start + 12..start + 12 + count * 16].chunks_exact(16) {
        let field =
            |at: usize| u32::from_be_bytes([record[at], record[at + 1], record[at + 2], record[at + 3]]) as usize;
        if [b"head", b"hhea", b"maxp", b"hmtx", b"cmap", b"loca", b"glyf", b"CFF ", b"name", b"OS/2", b"post"]
            .iter()
            .any(|tag| &record[..4] == *tag)
        {
            ranges.push((field(8), field(12)));
        }
    }
    ranges
}

fn damage(original: &[u8], rounds: usize, random: &mut Random) {
    let ranges = table_ranges(original);
    for round in 0..rounds {
        let mut data = original.to_vec();
        let (start, len) = ranges[random.below(ranges.len())];
        // Mostly near the start of a table, where the structure is.
        let within = if round % 3 == 0 { random.below(len) } else { random.below(len.min(96)) };
        let at = (start + within).min(data.len() - 1);
        match round % 5 {
            0 => data.truncate(at),
            1 => data[at] = 0xFF,
            2 => data[at] = 0,
            3 => data[at] = random.next() as u8,
            _ => {
                for _ in 0..4 {
                    let at = (start + random.below(len)).min(data.len() - 1);
                    data[at] = random.next() as u8;
                }
            }
        }
        exercise(&data);
    }
}

#[test]
fn damaged_fonts_do_not_crash_the_reader() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for original in [truetype(), postscript()] {
        // Every truncation and every single-byte change of the hand-made fonts.
        for at in 0..original.len() {
            exercise(&original[..at]);
            for value in [0x00, 0x01, 0x7F, 0x80, 0xFF] {
                let mut data = original.clone();
                data[at] = value;
                exercise(&data);
            }
        }
        damage(&original, 2000, &mut random);
    }
    // The real fonts, where present, each on its own thread.
    std::thread::scope(|scope| {
        for (index, path) in engine_fonts().into_iter().enumerate() {
            scope.spawn(move || {
                let Ok(original) = std::fs::read(&path) else { return };
                let mut random = Random(0x9E37_79B9_7F4A_7C15 ^ (index as u64 + 1) << 32);
                // The large East Asian fonts take long to copy; a few rounds each.
                damage(&original, if original.len() > 2_000_000 { 6 } else { 60 }, &mut random);
            });
        }
    });
}
