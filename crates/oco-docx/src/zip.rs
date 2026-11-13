//! The zip container: reading archives whose entries are stored or deflated
//! (including zip64), and writing archives of stored entries.
//!
//! The reader goes by the central directory at the end of the file, as Word
//! does, and reads an entry only when asked for it.

use std::io::{self, Read, Seek, SeekFrom};

use crate::inflate::{self, inflate};

/// Why an archive or one of its entries could not be read.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The file is not a zip archive, or the archive is damaged.
    Invalid(&'static str),
    /// A valid archive that uses something this reader does not handle.
    Unsupported(&'static str),
    /// The entry is larger than the caller allows.
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(error) => error.fmt(f),
            Error::Invalid(what) => write!(f, "invalid zip archive: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported zip archive: {what}"),
            Error::TooLarge => f.write_str("entry larger than the limit"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        // Reading past the end means the archive points outside the file.
        match error.kind() {
            io::ErrorKind::UnexpectedEof => Error::Invalid("the file is cut short"),
            _ => Error::Io(error),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

const LOCAL_HEADER: u32 = 0x0403_4B50;
const DIRECTORY_ENTRY: u32 = 0x0201_4B50;
const END_OF_DIRECTORY: u32 = 0x0605_4B50;
const END_OF_DIRECTORY_64: u32 = 0x0606_4B50;
const LOCATOR_64: u32 = 0x0706_4B50;

/// One file in an archive, as the central directory describes it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// Size once decompressed, as the archive claims it.
    pub size: u64,
    compressed: u64,
    crc: u32,
    method: u16,
    flags: u16,
    /// Where the entry's local header starts in the file.
    header: u64,
}

pub struct Archive<R> {
    reader: R,
    entries: Vec<Entry>,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32_at(bytes, at)) | u64::from(u32_at(bytes, at + 4)) << 32
}

impl<R: Read + Seek> Archive<R> {
    pub fn new(mut reader: R) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0))?;
        // The directory's end record is the last thing in the file, bar a comment of up to 64 KB.
        let tail_start = length.saturating_sub(22 + 0xFFFF);
        let tail = read_at(&mut reader, tail_start, (length - tail_start) as usize)?;
        let end = (0..tail.len().saturating_sub(21))
            .rev()
            .find(|&at| u32_at(&tail, at) == END_OF_DIRECTORY)
            .ok_or(Error::Invalid("no end-of-directory record"))?;
        let end_position = tail_start + end as u64;
        let mut count = u64::from(u16_at(&tail, end + 10));
        let mut size = u64::from(u32_at(&tail, end + 12));
        let mut offset = u64::from(u32_at(&tail, end + 16));
        let mut directory_end = end_position;

        // Archives beyond the 16- and 32-bit fields keep the real numbers in a second record,
        // found through a locator right before the end record. It speaks only for
        // the fields that are all ones here; what both records state, this one decides.
        if end >= 20 && u32_at(&tail, end - 20) == LOCATOR_64 {
            let record_position = u64_at(&tail, end - 12);
            let record = read_at(&mut reader, record_position, 56)?;
            if u32_at(&record, 0) != END_OF_DIRECTORY_64 {
                return Err(Error::Invalid("zip64 end-of-directory record"));
            }
            if count == 0xFFFF {
                count = u64_at(&record, 32);
            }
            if size == 0xFFFF_FFFF {
                size = u64_at(&record, 40);
            }
            if offset == 0xFFFF_FFFF {
                offset = u64_at(&record, 48);
                directory_end = record_position;
            }
        }
        if offset.checked_add(size).is_none_or(|end| end > directory_end) {
            return Err(Error::Invalid("the directory lies outside the file"));
        }
        // The directory ends where the end record begins. When it does, that is
        // where it is, whatever offset is written: bytes put in front of an
        // archive shift everything in it. Otherwise the offset is all there is to go by.
        let before_end = directory_end - size;
        let stands_there = read_at(&mut reader, before_end, 4).is_ok_and(|b| u32_at(&b, 0) == DIRECTORY_ENTRY);
        let shift = if stands_there { before_end - offset } else { 0 };

        let directory = read_at(&mut reader, offset + shift, size as usize)?;
        let mut entries = Vec::new();
        let mut at = 0;
        while at + 46 <= directory.len() && u32_at(&directory, at) == DIRECTORY_ENTRY {
            let fixed = &directory[at..at + 46];
            let (name, extra, comment) =
                (u16_at(fixed, 28) as usize, u16_at(fixed, 30) as usize, u16_at(fixed, 32) as usize);
            let Some(rest) = directory.get(at + 46..at + 46 + name + extra + comment) else {
                return Err(Error::Invalid("directory entry cut short"));
            };
            let mut entry = Entry {
                name: decode_name(&rest[..name]),
                size: u64::from(u32_at(fixed, 24)),
                compressed: u64::from(u32_at(fixed, 20)),
                crc: u32_at(fixed, 16),
                method: u16_at(fixed, 10),
                flags: u16_at(fixed, 8),
                header: u64::from(u32_at(fixed, 42)),
            };
            zip64_sizes(&rest[name..name + extra], &mut entry);
            entry.header += shift;
            entries.push(entry);
            at += 46 + name + extra + comment;
        }
        if entries.is_empty() && count > 0 {
            return Err(Error::Invalid("the directory holds no entries"));
        }
        Ok(Archive { reader, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The decompressed bytes of an entry, refused when there are more than `limit` of them.
    pub fn read(&mut self, index: usize, limit: u64) -> Result<Vec<u8>> {
        if self.entries[index].size > limit {
            return Err(Error::TooLarge);
        }
        self.read_packed(index)?.unpack(limit)
    }

    /// An entry as it lies in the archive, to be unpacked when it is wanted.
    pub fn read_packed(&mut self, index: usize) -> Result<Packed> {
        let entry = &self.entries[index];
        if entry.flags & 1 != 0 {
            return Err(Error::Unsupported("encrypted entry"));
        }
        if !matches!(entry.method, 0 | 8) {
            return Err(Error::Unsupported("compression method"));
        }
        let header = read_at(&mut self.reader, entry.header, 30)?;
        if u32_at(&header, 0) != LOCAL_HEADER {
            return Err(Error::Invalid("no entry where the directory points"));
        }
        // The local header repeats the name and has extra fields of its own.
        let start = entry.header + 30 + u64::from(u16_at(&header, 26)) + u64::from(u16_at(&header, 28));
        let length = self.reader.seek(SeekFrom::End(0))?;
        if start.checked_add(entry.compressed).is_none_or(|end| end > length) {
            return Err(Error::Invalid("the file is cut short"));
        }
        let data = read_at(&mut self.reader, start, entry.compressed as usize)?;
        Ok(Packed { data, deflated: entry.method == 8, size: entry.size, crc: entry.crc })
    }
}

/// The content of an entry, still compressed if the archive holds it so.
/// Keeping a picture like this costs neither the time nor the room of unpacking it.
#[derive(Debug, Clone)]
pub struct Packed {
    data: Vec<u8>,
    deflated: bool,
    size: u64,
    crc: u32,
}

impl Packed {
    /// The decompressed bytes, refused when there are more than `limit` of them.
    pub fn unpack(&self, limit: u64) -> Result<Vec<u8>> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let data = if self.deflated {
            inflate(&self.data, self.size as usize, limit).map_err(|error| match error {
                inflate::Error::TooLarge => Error::TooLarge,
                inflate::Error::Truncated => Error::Invalid("compressed data cut short"),
                inflate::Error::Invalid(what) => Error::Invalid(what),
            })?
        } else if self.data.len() > limit {
            return Err(Error::TooLarge);
        } else {
            self.data.clone()
        };
        if crc32(&data) != self.crc {
            return Err(Error::Invalid("checksum mismatch"));
        }
        Ok(data)
    }
}

fn read_at(reader: &mut (impl Read + Seek), position: u64, length: usize) -> Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(position))?;
    let mut bytes = Vec::new();
    // Not `read_exact` into a sized buffer: the length may come from a damaged file.
    reader.take(length as u64).read_to_end(&mut bytes)?;
    if bytes.len() < length {
        return Err(Error::Invalid("the file is cut short"));
    }
    Ok(bytes)
}

/// Sizes and offsets of 4 GB and more live in an extra field, in place of
/// each 32-bit field that is all ones.
fn zip64_sizes(mut extra: &[u8], entry: &mut Entry) {
    while extra.len() >= 4 {
        let (id, size) = (u16_at(extra, 0), u16_at(extra, 2) as usize);
        let Some(mut field) = extra.get(4..4 + size) else {
            return;
        };
        extra = &extra[4 + size..];
        if id != 1 {
            continue;
        }
        for value in [&mut entry.size, &mut entry.compressed, &mut entry.header] {
            if *value == u64::from(u32::MAX) && field.len() >= 8 {
                *value = u64_at(field, 0);
                field = &field[8..];
            }
        }
    }
}

/// Names are UTF-8 when the entry says so, and in practice also when it does not.
fn decode_name(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(name) => name.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// The CRC-32 of zip, gzip and PNG.
pub fn crc32(bytes: &[u8]) -> u32 {
    // Eight tables let the loop take eight bytes per step.
    const TABLES: [[u32; 256]; 8] = {
        let mut tables = [[0u32; 256]; 8];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 1 != 0 { crc >> 1 ^ 0xEDB8_8320 } else { crc >> 1 };
                bit += 1;
            }
            tables[0][i] = crc;
            i += 1;
        }
        let mut table = 1;
        while table < 8 {
            let mut i = 0;
            while i < 256 {
                let previous = tables[table - 1][i];
                tables[table][i] = previous >> 8 ^ tables[0][(previous & 0xFF) as usize];
                i += 1;
            }
            table += 1;
        }
        tables
    };
    let mut crc = !0u32;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let low = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) ^ crc;
        crc = TABLES[7][(low & 0xFF) as usize]
            ^ TABLES[6][(low >> 8 & 0xFF) as usize]
            ^ TABLES[5][(low >> 16 & 0xFF) as usize]
            ^ TABLES[4][(low >> 24) as usize]
            ^ TABLES[3][chunk[4] as usize]
            ^ TABLES[2][chunk[5] as usize]
            ^ TABLES[1][chunk[6] as usize]
            ^ TABLES[0][chunk[7] as usize];
    }
    for &byte in chunks.remainder() {
        crc = crc >> 8 ^ TABLES[0][((crc ^ u32::from(byte)) & 0xFF) as usize];
    }
    !crc
}

/// Builds an archive in memory. Entries are stored, not compressed.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
    directory: Vec<u8>,
    count: u16,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file. Names are UTF-8 with forward slashes.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let offset = self.out.len() as u32;
        let (crc, size) = (crc32(data), data.len() as u32);
        // Version 2.0, UTF-8 names, method 0, no timestamp (the DOS epoch).
        let mut fields = Vec::with_capacity(26);
        for value in [20u16, 0x0800, 0, 0, 0x21] {
            fields.extend_from_slice(&value.to_le_bytes());
        }
        for value in [crc, size, size] {
            fields.extend_from_slice(&value.to_le_bytes());
        }
        fields.extend_from_slice(&(name.len() as u16).to_le_bytes());

        self.out.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        self.out.extend_from_slice(&fields);
        self.out.extend_from_slice(&[0; 2]);
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(data);

        self.directory.extend_from_slice(&DIRECTORY_ENTRY.to_le_bytes());
        self.directory.extend_from_slice(&20u16.to_le_bytes());
        self.directory.extend_from_slice(&fields);
        // No extra field, no comment, disk 0, no attributes.
        self.directory.extend_from_slice(&[0; 12]);
        self.directory.extend_from_slice(&offset.to_le_bytes());
        self.directory.extend_from_slice(name.as_bytes());
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let offset = self.out.len() as u32;
        self.out.extend_from_slice(&self.directory);
        self.out.extend_from_slice(&END_OF_DIRECTORY.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&(self.directory.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&offset.to_le_bytes());
        self.out.extend_from_slice(&[0; 2]);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn archive() -> Vec<u8> {
        let mut writer = Writer::new();
        writer.add("first.txt", b"one");
        writer.add("dir/second.txt", "二".as_bytes());
        writer.add("empty", b"");
        writer.finish()
    }

    #[test]
    fn checksum_is_the_standard_one() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // Longer than one step of the loop, and not a multiple of it.
        assert_eq!(crc32(b"the quick brown fox jumps over the lazy dog"), 0xCE0C_5114);
    }

    #[test]
    fn reads_back_what_it_writes() {
        let mut archive = Archive::new(Cursor::new(archive())).unwrap();
        let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["first.txt", "dir/second.txt", "empty"]);
        assert_eq!(archive.read(0, 100).unwrap(), b"one");
        assert_eq!(archive.read(1, 100).unwrap(), "二".as_bytes());
        assert_eq!(archive.read(2, 100).unwrap(), b"");
        assert!(matches!(archive.read(0, 2), Err(Error::TooLarge)));
    }

    #[test]
    fn reads_an_archive_with_bytes_in_front_and_a_comment_behind() {
        let mut bytes = b"junk in front".to_vec();
        bytes.extend(archive());
        let comment = b"a comment";
        let at = bytes.len() - 2;
        bytes[at] = comment.len() as u8;
        bytes.extend_from_slice(comment);
        let mut archive = Archive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.read(1, 100).unwrap(), "二".as_bytes());
    }

    /// The archive with a zip64 end record and its locator put in before the end record.
    fn with_zip64_record(count: u64, size: u64, offset: u64) -> Vec<u8> {
        let mut bytes = archive();
        let end = bytes.split_off(bytes.len() - 22);
        let record_position = bytes.len() as u64;
        bytes.extend_from_slice(&END_OF_DIRECTORY_64.to_le_bytes());
        bytes.extend_from_slice(&44u64.to_le_bytes());
        bytes.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for value in [count, count, size, offset] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&LOCATOR_64.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&record_position.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend(end);
        bytes
    }

    #[test]
    fn the_zip64_record_speaks_only_for_fields_the_end_record_leaves_open() {
        let plain = archive();
        let end = plain.len() - 22;
        let (size, offset) = (u64::from(u32_at(&plain, end + 12)), u64::from(u32_at(&plain, end + 16)));

        // All ones in the end record: the numbers come from the zip64 record.
        let mut bytes = with_zip64_record(3, size, offset);
        let end = bytes.len() - 22;
        bytes[end + 8..end + 20].fill(0xFF);
        let mut archive = Archive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.entries().len(), 3);
        assert_eq!(archive.read(1, 100).unwrap(), "二".as_bytes());

        // Numbers in both: a zip64 record that points elsewhere is not followed.
        let mut archive = Archive::new(Cursor::new(with_zip64_record(1, 46, 0))).unwrap();
        assert_eq!(archive.entries().len(), 3);
        assert_eq!(archive.read(0, 100).unwrap(), b"one");
    }

    #[test]
    fn reads_a_deflated_entry() {
        // "abc" deflated, put in by hand: method 8 and the sizes of the packed data.
        let mut writer = Writer::new();
        let packed = [0x4B, 0x4C, 0x4A, 0x06, 0x00];
        writer.add("a", &packed);
        let mut bytes = writer.finish();
        let directory = bytes.len() - 22 - 46 - 1;
        for (header, fields) in [(0, 4), (directory, 6)] {
            bytes[header + fields + 4] = 8;
            bytes[header + fields + 10..header + fields + 14].copy_from_slice(&crc32(b"abc").to_le_bytes());
            bytes[header + fields + 18..header + fields + 22].copy_from_slice(&3u32.to_le_bytes());
        }
        let mut archive = Archive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.entries()[0].size, 3);
        assert_eq!(archive.read(0, 100).unwrap(), b"abc");
    }

    #[test]
    fn refuses_a_damaged_archive_without_panicking() {
        let whole = archive();
        assert!(matches!(Archive::new(Cursor::new(b"not a zip".to_vec())), Err(Error::Invalid(_))));
        assert!(matches!(Archive::new(Cursor::new(Vec::new())), Err(Error::Invalid(_))));
        // A flipped bit in the data shows in the checksum.
        let mut flipped = whole.clone();
        flipped[30 + "first.txt".len()] ^= 1;
        assert!(matches!(Archive::new(Cursor::new(flipped)).unwrap().read(0, 100), Err(Error::Invalid(_))));
        // Cut anywhere, or with any one byte changed, it either reads or says why not.
        for end in 0..whole.len() {
            if let Ok(mut archive) = Archive::new(Cursor::new(whole[..end].to_vec())) {
                for index in 0..archive.entries().len() {
                    let _ = archive.read(index, 100);
                }
            }
        }
        for at in 0..whole.len() {
            let mut changed = whole.clone();
            changed[at] = changed[at].wrapping_add(0x81);
            if let Ok(mut archive) = Archive::new(Cursor::new(changed)) {
                for index in 0..archive.entries().len() {
                    let _ = archive.read(index, 100);
                }
            }
        }
    }
}
