//! `glyf` and `loca`: TrueType outlines, made of lines and quadratic curves.

use std::ops::Range;

use super::{u16_at, u32_at, OutlineBuilder, Reader};

/// How deep composite glyphs may nest.
const MAX_DEPTH: u8 = 32;
/// How many glyphs one outline may be put together from. Real glyphs use a
/// handful; without a bound, parts nesting parts could multiply without end.
const MAX_PARTS: u32 = 1024;

pub(super) struct Glyf {
    loca: Range<usize>,
    /// `loca` holds 32-bit offsets rather than halved 16-bit ones.
    long: bool,
    /// Entries in `loca`: one more than the glyphs it locates.
    entries: u16,
    glyf: Range<usize>,
}

/// A 2×3 affine map: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy)]
struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Transform {
    const IDENTITY: Transform = Transform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn is_identity(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0 && self.e == 0.0 && self.f == 0.0
    }

    /// The map that applies `inner` first and `self` after.
    fn then(self, inner: Transform) -> Transform {
        Transform {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

type Point = (f32, f32);

fn midpoint(from: Point, to: Point) -> Point {
    (from.0 + 0.5 * (to.0 - from.0), from.1 + 0.5 * (to.1 - from.1))
}

/// Turns a contour's points into drawing commands. Points off the curve are
/// control points; between two of them in a row lies an implied point on
/// the curve, half-way. A contour may begin with control points, which are
/// then used when it closes.
struct Pen<'a> {
    builder: &'a mut dyn OutlineBuilder,
    transform: Transform,
    identity: bool,
    first_on: Option<Point>,
    first_off: Option<Point>,
    last_off: Option<Point>,
}

impl<'a> Pen<'a> {
    fn new(transform: Transform, builder: &'a mut dyn OutlineBuilder) -> Self {
        Pen { builder, transform, identity: transform.is_identity(), first_on: None, first_off: None, last_off: None }
    }

    fn placed(&self, point: Point) -> Point {
        if self.identity {
            point
        } else {
            self.transform.apply(point.0, point.1)
        }
    }

    fn move_to(&mut self, to: Point) {
        let (x, y) = self.placed(to);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, to: Point) {
        let (x, y) = self.placed(to);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, control: Point, to: Point) {
        let ((x1, y1), (x, y)) = (self.placed(control), self.placed(to));
        self.builder.quad_to(x1, y1, x, y);
    }

    fn point(&mut self, point: Point, on_curve: bool, ends_contour: bool) {
        if self.first_on.is_none() {
            if on_curve {
                self.first_on = Some(point);
                self.move_to(point);
            } else if let Some(control) = self.first_off {
                let middle = midpoint(control, point);
                self.first_on = Some(middle);
                self.last_off = Some(point);
                self.move_to(middle);
            } else {
                self.first_off = Some(point);
            }
        } else {
            match (self.last_off, on_curve) {
                (Some(control), true) => {
                    self.last_off = None;
                    self.quad_to(control, point);
                }
                (Some(control), false) => {
                    self.last_off = Some(point);
                    self.quad_to(control, midpoint(control, point));
                }
                (None, true) => self.line_to(point),
                (None, false) => self.last_off = Some(point),
            }
        }
        if ends_contour {
            self.close();
        }
    }

    fn close(&mut self) {
        if let (Some(first), Some(last)) = (self.first_off, self.last_off) {
            self.last_off = None;
            self.quad_to(last, midpoint(last, first));
        }
        match (self.first_on, self.first_off, self.last_off) {
            (Some(start), Some(control), _) | (Some(start), None, Some(control)) => self.quad_to(control, start),
            (Some(start), None, None) => self.line_to(start),
            (None, _, _) => {}
        }
        (self.first_on, self.first_off, self.last_off) = (None, None, None);
        self.builder.close();
    }
}

impl Glyf {
    pub(super) fn new(glyphs: u16, long: bool, loca: Range<usize>, glyf: Range<usize>) -> Glyf {
        let present = loca.len() / if long { 4 } else { 2 };
        let wanted = glyphs.saturating_add(1);
        Glyf { entries: wanted.min(u16::try_from(present).unwrap_or(u16::MAX)), loca, long, glyf }
    }

    /// The bytes of a glyph; `None` for one without an outline, such as a space.
    fn glyph<'a>(&self, file: &'a [u8], glyph: u16) -> Option<&'a [u8]> {
        if glyph.checked_add(1)? >= self.entries {
            return None;
        }
        let loca = file.get(self.loca.clone())?;
        let offset = |index: usize| -> Option<usize> {
            match self.long {
                true => usize::try_from(u32_at(loca, index * 4)?).ok(),
                false => Some(usize::from(u16_at(loca, index * 2)?) * 2),
            }
        };
        let (start, end) = (offset(usize::from(glyph))?, offset(usize::from(glyph) + 1)?);
        if start >= end {
            return None;
        }
        file.get(self.glyf.clone())?.get(start..end)
    }

    pub(super) fn outline(&self, file: &[u8], glyph: u16, builder: &mut dyn OutlineBuilder) {
        if let Some(data) = self.glyph(file, glyph) {
            let mut parts_left = MAX_PARTS;
            self.draw(file, data, 0, &mut parts_left, &mut Pen::new(Transform::IDENTITY, builder));
        }
    }

    /// `None` when the glyph is malformed; what was drawn before stays drawn.
    fn draw(&self, file: &[u8], data: &[u8], depth: u8, parts_left: &mut u32, pen: &mut Pen) -> Option<()> {
        if depth >= MAX_DEPTH {
            return None;
        }
        *parts_left = parts_left.checked_sub(1)?;
        let mut reader = Reader::new(data);
        let contours = reader.i16()?;
        reader.skip(8); // bounding box
        if contours > 0 {
            simple(reader.tail()?, contours as u16, pen)?;
        } else if contours < 0 {
            let mut reader = Reader::new(reader.tail()?);
            while let Some((glyph, transform)) = component(&mut reader) {
                if let Some(data) = self.glyph(file, glyph) {
                    let mut inner = Pen::new(pen.transform.then(transform), &mut *pen.builder);
                    self.draw(file, data, depth + 1, parts_left, &mut inner)?;
                }
            }
        }
        Some(())
    }
}

/// Reads the next component of a composite glyph: which glyph, and how it
/// is placed. `None` at the end, or where the data is cut short.
fn component(reader: &mut Reader) -> Option<(u16, Transform)> {
    const WORDS: u16 = 0x0001;
    const OFFSETS: u16 = 0x0002;
    const SCALE: u16 = 0x0008;
    const MORE: u16 = 0x0020;
    const XY_SCALE: u16 = 0x0040;
    const MATRIX: u16 = 0x0080;

    let flags = reader.u16()?;
    let glyph = reader.u16()?;
    let mut transform = Transform::IDENTITY;
    let fraction = |reader: &mut Reader| reader.i16().map(|n| f32::from(n) / 16384.0);
    if flags & OFFSETS != 0 {
        if flags & WORDS != 0 {
            transform.e = f32::from(reader.i16()?);
            transform.f = f32::from(reader.i16()?);
        } else {
            transform.e = f32::from(reader.i8()?);
            transform.f = f32::from(reader.i8()?);
        }
    } else {
        // The arguments name points to align instead, which is not done:
        // the component stays where it was drawn.
        reader.skip(if flags & WORDS != 0 { 4 } else { 2 });
    }
    if flags & MATRIX != 0 {
        transform.a = fraction(reader)?;
        transform.b = fraction(reader)?;
        transform.c = fraction(reader)?;
        transform.d = fraction(reader)?;
    } else if flags & XY_SCALE != 0 {
        transform.a = fraction(reader)?;
        transform.d = fraction(reader)?;
    } else if flags & SCALE != 0 {
        transform.a = fraction(reader)?;
        transform.d = transform.a;
    }
    if flags & MORE == 0 {
        reader.at = reader.data.len();
    }
    Some((glyph, transform))
}

/// Draws a glyph made of contours. After the header come the index of each
/// contour's last point, the hinting program, then the points: a flag byte
/// each (which may repeat), all x coordinates, all y coordinates, each
/// coordinate a delta from the one before, one or two bytes long.
fn simple(data: &[u8], contours: u16, pen: &mut Pen) -> Option<()> {
    const ON_CURVE: u8 = 0x01;
    const X_SHORT: u8 = 0x02;
    const Y_SHORT: u8 = 0x04;
    const REPEAT: u8 = 0x08;
    const X_SAME_OR_POSITIVE: u8 = 0x10;
    const Y_SAME_OR_POSITIVE: u8 = 0x20;

    let mut reader = Reader::new(data);
    let ends = reader.bytes(usize::from(contours) * 2)?;
    let end = |index: u16| u16_at(ends, usize::from(index) * 2);
    let points = end(contours - 1)?.checked_add(1)?;
    if points == 1 {
        return Some(());
    }
    let program = reader.u16()?;
    reader.skip(usize::from(program));

    // The flags have to be walked once to learn where the coordinates start.
    let flags_start = reader.at;
    let (mut left, mut x_len, mut y_len) = (u32::from(points), 0u32, 0u32);
    while left > 0 {
        let flags = reader.u8()?;
        let times = if flags & REPEAT != 0 { u32::from(reader.u8()?) + 1 } else { 1 };
        if times > left {
            return None;
        }
        let width = |short: u8, same: u8| match (flags & short != 0, flags & same != 0) {
            (true, _) => 1,
            (false, false) => 2,
            (false, true) => 0,
        };
        x_len += width(X_SHORT, X_SAME_OR_POSITIVE) * times;
        y_len += width(Y_SHORT, Y_SAME_OR_POSITIVE) * times;
        left -= times;
    }
    let x_start = reader.at;
    let y_start = x_start + x_len as usize;
    let mut flags = Reader::new(data.get(flags_start..x_start)?);
    let mut xs = Reader::new(data.get(x_start..y_start)?);
    let mut ys = Reader::new(data.get(y_start..y_start + y_len as usize)?);

    let coordinate = |reader: &mut Reader, previous: &mut i16, short: bool, same_or_positive: bool| {
        let delta = match (short, same_or_positive) {
            (true, true) => i16::from(reader.u8().unwrap_or(0)),
            (true, false) => -i16::from(reader.u8().unwrap_or(0)),
            (false, true) => 0,
            (false, false) => reader.i16().unwrap_or(0),
        };
        *previous = previous.wrapping_add(delta);
        *previous
    };
    // Points left in the current contour after this one, and the next contour.
    let (mut remaining, mut contour) = (end(0)?, 1u16);
    let (mut flag, mut repeats) = (0u8, 0u8);
    let (mut x, mut y) = (0i16, 0i16);
    for _ in 0..points {
        let ends_contour = remaining == 0;
        if ends_contour {
            if let Some(next) = end(contour) {
                remaining = next.saturating_sub(end(contour - 1).unwrap_or(0)).saturating_sub(1);
            }
            contour = contour.saturating_add(1);
        } else {
            remaining -= 1;
        }
        if repeats == 0 {
            flag = flags.u8().unwrap_or(0);
            if flag & REPEAT != 0 {
                repeats = flags.u8().unwrap_or(0);
            }
        } else {
            repeats -= 1;
        }
        let px = coordinate(&mut xs, &mut x, flag & X_SHORT != 0, flag & X_SAME_OR_POSITIVE != 0);
        let py = coordinate(&mut ys, &mut y, flag & Y_SHORT != 0, flag & Y_SAME_OR_POSITIVE != 0);
        pen.point((f32::from(px), f32::from(py)), flag & ON_CURVE != 0, ends_contour);
    }
    Some(())
}
