//! Inflate (RFC 1951): undoes the deflate compression that zip entries use.

/// Why a deflate stream could not be decompressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The stream ends before its last block does.
    Truncated,
    /// The stream breaks the format; the text says where.
    Invalid(&'static str),
    /// The output would be larger than the caller allows.
    TooLarge,
}

type Result<T> = std::result::Result<T, Error>;

/// Decompresses a whole deflate stream, refusing to produce more than `limit` bytes.
/// `expected` is the size the container claims, used only to reserve memory.
pub fn inflate(input: &[u8], expected: usize, limit: usize) -> Result<Vec<u8>> {
    // The claimed size is not to be trusted with an allocation of its own size.
    let mut out = Vec::with_capacity(expected.min(limit).min(input.len().saturating_mul(64)));
    let mut bits = Bits { input, pos: 0, hold: 0, count: 0 };
    let mut fixed = None;
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => stored(&mut bits, &mut out, limit)?,
            1 => {
                let (literals, distances) = fixed.get_or_insert_with(fixed_tables);
                block(&mut bits, &mut out, limit, literals, distances)?;
            }
            2 => {
                let (literals, distances) = dynamic_tables(&mut bits)?;
                block(&mut bits, &mut out, limit, &literals, &distances)?;
            }
            _ => return Err(Error::Invalid("reserved block type")),
        }
        if last {
            return Ok(out);
        }
    }
}

/// Reads the input from its low bits up, as deflate packs them.
struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    hold: u64,
    count: u32,
}

impl Bits<'_> {
    /// Tops the reservoir up to at least 56 bits, or to whatever the input still has.
    #[inline]
    fn refill(&mut self) {
        if let Some(word) = self.input.get(self.pos..self.pos + 8) {
            self.hold |= u64::from_le_bytes(word.try_into().unwrap()) << self.count;
            self.pos += ((63 - self.count) >> 3) as usize;
            self.count |= 56;
        } else {
            while self.count <= 56 && self.pos < self.input.len() {
                self.hold |= u64::from(self.input[self.pos]) << self.count;
                self.pos += 1;
                self.count += 8;
            }
        }
    }

    #[inline]
    fn drop(&mut self, n: u32) -> Result<()> {
        if n > self.count {
            return Err(Error::Truncated);
        }
        self.hold >>= n;
        self.count -= n;
        Ok(())
    }

    /// The next `n` bits (at most 32) as a number.
    #[inline]
    fn take(&mut self, n: u32) -> Result<u32> {
        if self.count < n {
            self.refill();
        }
        let value = (self.hold & ((1 << n) - 1)) as u32;
        self.drop(n)?;
        Ok(value)
    }

    /// Decodes one symbol. The reservoir must have been refilled.
    #[inline]
    fn symbol(&mut self, table: &Table) -> Result<u32> {
        let mut entry = table.entries[(self.hold & table.mask) as usize];
        if entry & LINK != 0 {
            self.drop(table.bits)?;
            entry = table.entries[(entry >> 8) as usize + (self.hold & ((1 << (entry & 0xF)) - 1)) as usize];
        }
        if entry == 0 {
            return Err(Error::Invalid("code not in the table"));
        }
        self.drop(entry & 0xF)?;
        Ok(entry >> 8)
    }
}

/// Marks a first-level entry that points at a second-level table.
const LINK: u32 = 0x10;

/// A Huffman code as a lookup table indexed by the next bits of input.
///
/// An entry holds the symbol above bit 8 and the code's length in its low
/// four bits; zero means no code starts with these bits. Codes longer than
/// the first level are reached through a `LINK` entry, which holds the
/// offset of a second-level table above bit 8 and the number of further
/// bits that table is indexed by in its low four bits.
struct Table {
    entries: Vec<u32>,
    bits: u32,
    mask: u64,
}

impl Table {
    fn new(lengths: &[u8], bits: u32) -> Result<Self> {
        let mut count = [0u32; 16];
        for &length in lengths {
            count[length as usize] += 1;
        }
        count[0] = 0;
        // More codes than the lengths leave room for cannot be told apart.
        let mut left = 1i32;
        let mut next = [0u32; 16];
        let mut code = 0;
        for length in 1..16 {
            left = (left << 1) - count[length] as i32;
            if left < 0 {
                return Err(Error::Invalid("over-subscribed code"));
            }
            code = (code + count[length - 1]) << 1;
            next[length] = code;
        }

        let size = 1usize << bits;
        let mut entries = vec![0u32; size];
        // How many bits the longest code under each first-level prefix goes on for.
        let mut longest = vec![0u8; size];
        let mut codes = Vec::with_capacity(lengths.len());
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let length = u32::from(length);
            let code = next[length as usize];
            next[length as usize] += 1;
            // Codes are read from their most significant bit, the stream from its least.
            let reversed = code.reverse_bits() >> (32 - length);
            if length > bits {
                let prefix = reversed as usize & (size - 1);
                longest[prefix] = longest[prefix].max((length - bits) as u8);
            }
            codes.push((symbol as u32, length, reversed));
        }
        for (prefix, &extra) in longest.iter().enumerate() {
            if extra > 0 {
                let offset = entries.len() as u32;
                entries.resize(entries.len() + (1 << extra), 0);
                entries[prefix] = offset << 8 | LINK | u32::from(extra);
            }
        }
        for (symbol, length, reversed) in codes {
            if length <= bits {
                for slot in (reversed as usize..size).step_by(1 << length) {
                    entries[slot] = symbol << 8 | length;
                }
            } else {
                let link = entries[reversed as usize & (size - 1)];
                let (offset, extra) = ((link >> 8) as usize, link & 0xF);
                // What is left of the code after the first level picks the slot.
                let entry = symbol << 8 | (length - bits);
                for slot in ((reversed >> bits) as usize..1 << extra).step_by(1 << (length - bits)) {
                    entries[offset + slot] = entry;
                }
            }
        }
        Ok(Table { entries, bits, mask: (size - 1) as u64 })
    }
}

const LITERAL_BITS: u32 = 10;
const DISTANCE_BITS: u32 = 8;

fn fixed_tables() -> (Table, Table) {
    let mut lengths = [8u8; 288];
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    let literals = Table::new(&lengths, LITERAL_BITS).expect("the fixed code is complete");
    let distances = Table::new(&[5; 30], DISTANCE_BITS).expect("the fixed code is complete");
    (literals, distances)
}

fn dynamic_tables(bits: &mut Bits) -> Result<(Table, Table)> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let literals = bits.take(5)? as usize + 257;
    let distances = bits.take(5)? as usize + 1;
    let count = bits.take(4)? as usize + 4;
    if literals > 286 || distances > 30 {
        return Err(Error::Invalid("too many codes"));
    }
    let mut meta = [0u8; 19];
    for &symbol in &ORDER[..count] {
        meta[symbol] = bits.take(3)? as u8;
    }
    let meta = Table::new(&meta, 7)?;

    let mut lengths = [0u8; 286 + 30];
    let total = literals + distances;
    let mut filled = 0;
    while filled < total {
        bits.refill();
        let (value, repeat) = match bits.symbol(&meta)? {
            symbol @ 0..=15 => (symbol as u8, 1),
            16 => {
                let Some(&previous) = filled.checked_sub(1).and_then(|i| lengths.get(i)) else {
                    return Err(Error::Invalid("nothing to repeat"));
                };
                (previous, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if filled + repeat > total {
            return Err(Error::Invalid("too many code lengths"));
        }
        lengths[filled..filled + repeat].fill(value);
        filled += repeat;
    }
    if lengths[256] == 0 {
        return Err(Error::Invalid("no end-of-block code"));
    }
    Ok((Table::new(&lengths[..literals], LITERAL_BITS)?, Table::new(&lengths[literals..total], DISTANCE_BITS)?))
}

fn stored(bits: &mut Bits, out: &mut Vec<u8>, limit: usize) -> Result<()> {
    // The block starts at the next whole byte; hand back what the reservoir holds beyond it.
    bits.pos -= (bits.count / 8) as usize;
    bits.hold = 0;
    bits.count = 0;
    let header = bits.input.get(bits.pos..bits.pos + 4).ok_or(Error::Truncated)?;
    let length = u16::from_le_bytes([header[0], header[1]]);
    if length != !u16::from_le_bytes([header[2], header[3]]) {
        return Err(Error::Invalid("stored block length"));
    }
    let start = bits.pos + 4;
    let data = bits.input.get(start..start + length as usize).ok_or(Error::Truncated)?;
    if out.len() + data.len() > limit {
        return Err(Error::TooLarge);
    }
    out.extend_from_slice(data);
    bits.pos = start + data.len();
    Ok(())
}

fn block(bits: &mut Bits, out: &mut Vec<u8>, limit: usize, literals: &Table, distances: &Table) -> Result<()> {
    const LENGTH_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227,
        258,
    ];
    const LENGTH_EXTRA: [u8; 29] =
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DISTANCE_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
        6145, 8193, 12289, 16385, 24577,
    ];
    const DISTANCE_EXTRA: [u8; 30] =
        [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    loop {
        // One refill covers a whole length and distance pair: 15 + 5 + 15 + 13 bits.
        bits.refill();
        let symbol = bits.symbol(literals)?;
        if symbol < 256 {
            if out.len() >= limit {
                return Err(Error::TooLarge);
            }
            out.push(symbol as u8);
            continue;
        }
        if symbol == 256 {
            return Ok(());
        }
        let index = symbol as usize - 257;
        let Some(&base) = LENGTH_BASE.get(index) else {
            return Err(Error::Invalid("length code"));
        };
        let extra = u32::from(LENGTH_EXTRA[index]);
        let length = base as usize + (bits.hold & ((1 << extra) - 1)) as usize;
        bits.drop(extra)?;

        let index = bits.symbol(distances)? as usize;
        let Some(&base) = DISTANCE_BASE.get(index) else {
            return Err(Error::Invalid("distance code"));
        };
        let extra = u32::from(DISTANCE_EXTRA[index]);
        let distance = base as usize + (bits.hold & ((1 << extra) - 1)) as usize;
        bits.drop(extra)?;

        if distance > out.len() {
            return Err(Error::Invalid("distance reaches before the start"));
        }
        if out.len() + length > limit {
            return Err(Error::TooLarge);
        }
        let start = out.len() - distance;
        if distance >= length {
            out.extend_from_within(start..start + length);
        } else {
            // The copy overlaps what it writes: a short pattern repeated.
            for i in 0..length {
                let byte = out[start + i];
                out.push(byte);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflates_each_kind_of_block() {
        // A stored block.
        assert_eq!(inflate(&[1, 3, 0, 0xFC, 0xFF, b'a', b'b', b'c'], 0, 100).unwrap(), b"abc");
        // The fixed code: "abc" as three literals.
        assert_eq!(inflate(&[0x4B, 0x4C, 0x4A, 0x06, 0x00], 0, 100).unwrap(), b"abc");
        // The fixed code with a match that overlaps what it writes: 'a', then 99 more of it.
        assert_eq!(inflate(&[0x4B, 0x4C, 0xA4, 0x3D, 0x00, 0x00], 0, 100).unwrap(), [b'a'; 100]);
        // The fixed code with matches further back.
        let text = "aaaaaaaabbbbccd".repeat(6) + &"abcabcabd".repeat(5);
        let packed = [
            0x4B, 0x4C, 0x84, 0x80, 0xA4, 0xA4, 0xA4, 0xA4, 0xE4, 0xE4, 0x94, 0x44, 0x2A, 0x72, 0x93, 0x92, 0x13, 0x93,
            0x92, 0x13, 0x93, 0x88, 0x62, 0x00, 0x00,
        ];
        assert_eq!(inflate(&packed, 0, 1000).unwrap(), text.as_bytes());
        // A dynamic code for the same text, as zlib writes it without matches.
        let packed = [
            0x05, 0xC1, 0x01, 0x01, 0x00, 0x00, 0x08, 0xC3, 0xA0, 0xAC, 0xEC, 0xF6, 0xCF, 0x20, 0x00, 0x54, 0x6D, 0x07,
            0x50, 0xB5, 0x1D, 0x40, 0xD5, 0x76, 0x00, 0x55, 0xDB, 0x01, 0x54, 0x6D, 0x07, 0x50, 0xB5, 0x9D, 0xA6, 0xE9,
            0x34, 0x4D, 0xA7, 0x69, 0x3A, 0x4D, 0xD3, 0x69, 0x9A, 0xEE, 0x01,
        ];
        assert_eq!(inflate(&packed, 0, 1000).unwrap(), text.as_bytes());
    }

    #[test]
    fn decodes_codes_longer_than_the_first_level_of_the_table() {
        // Lengths 1, 2, 3 ... 14, 15, 15: every length there is, complete.
        let lengths: Vec<u8> = (1..=15).chain([15]).collect();
        let table = Table::new(&lengths, 6).unwrap();
        // Write each symbol's code, most significant bit first, into a stream read from the low bit.
        let (mut stream, mut used, mut code) = (vec![0u8; 32], 0usize, 0u32);
        for (symbol, &length) in lengths.iter().enumerate() {
            // Canonical codes of a ladder like this one: 0, 10, 110, ... and at the end 1...10 and 1...11.
            let bits = if symbol == 15 { code >> 1 } else { code };
            for i in (0..length).rev() {
                stream[used / 8] |= ((bits >> i & 1) as u8) << (used % 8);
                used += 1;
            }
            code = (bits + 1) << 1;
        }
        let mut bits = Bits { input: &stream, pos: 0, hold: 0, count: 0 };
        for symbol in 0..lengths.len() as u32 {
            bits.refill();
            assert_eq!(bits.symbol(&table), Ok(symbol));
        }
        assert_eq!(Table::new(&[1, 1, 1], 6).err(), Some(Error::Invalid("over-subscribed code")));
    }

    #[test]
    fn refuses_what_is_cut_short_too_large_or_malformed() {
        let whole = [0x4B, 0x4C, 0x4A, 0x06, 0x00];
        for end in 0..whole.len() - 1 {
            assert_eq!(inflate(&whole[..end], 0, 100), Err(Error::Truncated), "cut at {end}");
        }
        assert_eq!(inflate(&whole, 0, 2), Err(Error::TooLarge));
        assert_eq!(inflate(&[1, 3, 0, 0xFC, 0xFF, b'a'], 0, 100), Err(Error::Truncated));
        assert_eq!(inflate(&[1, 3, 0, 0, 0, b'a', b'b', b'c'], 0, 100), Err(Error::Invalid("stored block length")));
        assert_eq!(inflate(&[7, 0, 0], 0, 100), Err(Error::Invalid("reserved block type")));
        // A match that points before the first byte.
        assert_eq!(inflate(&[0x03, 0x02, 0x00], 0, 100), Err(Error::Invalid("distance reaches before the start")));
    }

    #[test]
    fn survives_any_bytes() {
        // A cheap generator is enough: the point is that nothing panics or runs away.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for round in 0..20_000 {
            let length = 1 + round % 64;
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect();
            if let Ok(out) = inflate(&bytes, 0, 4096) {
                assert!(out.len() <= 4096);
            }
        }
    }
}
