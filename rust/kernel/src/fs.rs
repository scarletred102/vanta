//! Read-only CPIO `newc` initramfs support.
//!
//! The first filesystem boundary is intentionally small and deterministic:
//! Limine boots the kernel, the kernel owns an in-memory archive, and `/bin/init`
//! is resolved through the filesystem before the ELF loader sees it. The parser
//! has no allocation requirement and rejects truncated or malformed entries.

use crate::elf;

const HEADER_SIZE: usize = 110;
const MODE_DIRECTORY: u32 = 0o040000;
const MODE_REGULAR: u32 = 0o100000;
const MODE_TYPE_MASK: u32 = 0o170000;

const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

const fn entry_size(name_length: usize, data_length: usize) -> usize {
    align4(HEADER_SIZE + name_length + 1) + align4(data_length)
}

const INITRAMFS_SIZE: usize = entry_size(3, 0)
    + entry_size(8, 0x300)
    + entry_size(3, 0)
    + entry_size(8, 16)
    + entry_size(10, 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsError {
    Truncated,
    InvalidMagic,
    InvalidHeader,
    InvalidEntry,
    MissingTrailer,
    InvalidPath,
    NotFound,
    NotFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry<'a> {
    pub name: &'a [u8],
    pub data: &'a [u8],
    pub mode: u32,
}

impl Entry<'_> {
    pub fn is_directory(self) -> bool {
        self.mode & MODE_TYPE_MASK == MODE_DIRECTORY
    }

    pub fn is_file(self) -> bool {
        self.mode & MODE_TYPE_MASK == MODE_REGULAR
    }

    pub fn is_executable(self) -> bool {
        self.mode & 0o111 != 0
    }
}

#[derive(Clone, Copy)]
pub struct FileSystem<'a> {
    archive: &'a [u8],
    entry_count: usize,
}

impl<'a> FileSystem<'a> {
    pub fn new(archive: &'a [u8]) -> Result<Self, FsError> {
        let filesystem = Self {
            archive,
            entry_count: 0,
        };
        let entry_count = filesystem.scan()?;
        Ok(Self {
            archive,
            entry_count,
        })
    }

    pub fn entry_count(self) -> usize {
        self.entry_count
    }

    pub fn read(self, path: &str) -> Result<&'a [u8], FsError> {
        let entry = self.find(path)?;
        if !entry.is_file() {
            return Err(FsError::NotFile);
        }
        Ok(entry.data)
    }

    pub fn is_directory(self, path: &str) -> Result<bool, FsError> {
        let path = normalize_path(path)?;
        if path.is_empty() {
            return Ok(true);
        }
        Ok(self.find_bytes(path)?.is_directory())
    }

    pub fn is_executable(self, path: &str) -> Result<bool, FsError> {
        let path = normalize_path(path)?;
        Ok(self.find_bytes(path)?.is_executable())
    }

    fn find(self, path: &str) -> Result<Entry<'a>, FsError> {
        let path = normalize_path(path)?;
        if path.is_empty() {
            return Err(FsError::NotFile);
        }
        self.find_bytes(path)
    }

    fn find_bytes(self, path: &[u8]) -> Result<Entry<'a>, FsError> {
        let mut entries = Entries {
            archive: self.archive,
            offset: 0,
            trailer_seen: false,
        };
        while let Some(entry) = entries.next()? {
            if entry.name == path {
                return Ok(entry);
            }
        }
        Err(FsError::NotFound)
    }

    fn scan(self) -> Result<usize, FsError> {
        let mut entries = Entries {
            archive: self.archive,
            offset: 0,
            trailer_seen: false,
        };
        let mut count = 0;
        while entries.next()?.is_some() {
            count += 1;
        }
        if entries.trailer_seen {
            Ok(count)
        } else {
            Err(FsError::MissingTrailer)
        }
    }
}

pub struct Entries<'a> {
    archive: &'a [u8],
    offset: usize,
    trailer_seen: bool,
}

impl<'a> Entries<'a> {
    pub fn next(&mut self) -> Result<Option<Entry<'a>>, FsError> {
        if self.trailer_seen {
            return Ok(None);
        }
        let header_end = self
            .offset
            .checked_add(HEADER_SIZE)
            .ok_or(FsError::Truncated)?;
        let header = self
            .archive
            .get(self.offset..header_end)
            .ok_or(FsError::Truncated)?;
        if &header[0..6] != b"070701" {
            return Err(if self.offset == 0 {
                FsError::InvalidMagic
            } else {
                FsError::InvalidHeader
            });
        }

        let mode = parse_hex(header, 14)?;
        let file_size = parse_hex(header, 54)? as usize;
        let name_size = parse_hex(header, 94)? as usize;
        if name_size == 0 {
            return Err(FsError::InvalidEntry);
        }

        let name_start = header_end;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or(FsError::Truncated)?;
        let name_with_nul = self
            .archive
            .get(name_start..name_end)
            .ok_or(FsError::Truncated)?;
        if name_with_nul.last().copied() != Some(0) {
            return Err(FsError::InvalidEntry);
        }
        let name = &name_with_nul[..name_with_nul.len() - 1];
        let data_start = align4(name_end);
        let data_end = data_start
            .checked_add(file_size)
            .ok_or(FsError::Truncated)?;
        let data = self
            .archive
            .get(data_start..data_end)
            .ok_or(FsError::Truncated)?;
        self.offset = align4(data_end);

        if name == b"TRAILER!!!" {
            self.trailer_seen = true;
            return Ok(None);
        }
        if name.is_empty() || name.contains(&b'\0') {
            return Err(FsError::InvalidEntry);
        }
        Ok(Some(Entry { name, data, mode }))
    }
}

fn normalize_path(path: &str) -> Result<&[u8], FsError> {
    let mut path = path.as_bytes();
    while path.first() == Some(&b'/') {
        path = &path[1..];
    }
    if path.is_empty() {
        return Ok(path);
    }
    if path
        .split(|byte| *byte == b'/')
        .any(|component| component.is_empty() || component == b"." || component == b"..")
    {
        return Err(FsError::InvalidPath);
    }
    Ok(path)
}

fn parse_hex(bytes: &[u8], offset: usize) -> Result<u32, FsError> {
    let field = bytes.get(offset..offset + 8).ok_or(FsError::Truncated)?;
    let mut value = 0u32;
    for byte in field {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(FsError::InvalidHeader),
        } as u32;
        value = value.checked_mul(16).ok_or(FsError::InvalidHeader)?;
        value = value.checked_add(digit).ok_or(FsError::InvalidHeader)?;
    }
    Ok(value)
}

const fn put_hex(bytes: &mut [u8], offset: usize, value: u32) {
    let digits = *b"0123456789abcdef";
    let mut index = 0;
    while index < 8 {
        let shift = 28 - (index * 4);
        bytes[offset + index] = digits[((value >> shift) & 0xf) as usize];
        index += 1;
    }
}

const fn copy_bytes(destination: &mut [u8], offset: usize, source: &[u8]) {
    let mut index = 0;
    while index < source.len() {
        destination[offset + index] = source[index];
        index += 1;
    }
}

const fn append_entry(archive: &mut [u8], offset: &mut usize, name: &[u8], mode: u32, data: &[u8]) {
    let header = *offset;
    archive[header] = b'0';
    archive[header + 1] = b'7';
    archive[header + 2] = b'0';
    archive[header + 3] = b'7';
    archive[header + 4] = b'0';
    archive[header + 5] = b'1';
    put_hex(archive, header + 6, 1);
    put_hex(archive, header + 14, mode);
    put_hex(archive, header + 22, 0);
    put_hex(archive, header + 30, 0);
    put_hex(archive, header + 38, 1);
    put_hex(archive, header + 46, 0);
    put_hex(archive, header + 54, data.len() as u32);
    put_hex(archive, header + 62, 0);
    put_hex(archive, header + 70, 0);
    put_hex(archive, header + 78, 0);
    put_hex(archive, header + 86, 0);
    put_hex(archive, header + 94, (name.len() + 1) as u32);
    put_hex(archive, header + 102, 0);

    let name_start = header + HEADER_SIZE;
    copy_bytes(archive, name_start, name);
    archive[name_start + name.len()] = 0;
    let data_start = align4(name_start + name.len() + 1);
    copy_bytes(archive, data_start, data);
    *offset = align4(data_start + data.len());
}

const fn make_initramfs() -> [u8; INITRAMFS_SIZE] {
    let mut archive = [0u8; INITRAMFS_SIZE];
    let mut offset = 0;
    append_entry(
        &mut archive,
        &mut offset,
        b"bin",
        MODE_DIRECTORY | 0o755,
        &[],
    );
    append_entry(
        &mut archive,
        &mut offset,
        b"bin/init",
        MODE_REGULAR | 0o755,
        &elf::TEST_ELF,
    );
    append_entry(
        &mut archive,
        &mut offset,
        b"etc",
        MODE_DIRECTORY | 0o755,
        &[],
    );
    append_entry(
        &mut archive,
        &mut offset,
        b"etc/motd",
        MODE_REGULAR | 0o644,
        b"Vanta initramfs\n",
    );
    append_entry(&mut archive, &mut offset, b"TRAILER!!!", 0, &[]);
    archive
}

pub static INITRAMFS: [u8; INITRAMFS_SIZE] = make_initramfs();
