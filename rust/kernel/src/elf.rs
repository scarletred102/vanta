//! Minimal ELF64 parsing for the first Vanta user process.
//!
//! The parser only accepts the parts needed by the process loader: little-endian
//! x86_64 images with ordinary program headers. It does not interpret sections,
//! relocations, symbols, or dynamic linking.

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 0x3e;

pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfError {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEncoding,
    UnsupportedType,
    UnsupportedMachine,
    InvalidHeaderSize,
    InvalidProgramHeaders,
    NoLoadSegments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElfImage<'a> {
    bytes: &'a [u8],
    pub entry: u64,
    program_header_offset: usize,
    program_header_size: usize,
    program_header_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramHeader {
    pub kind: u32,
    pub flags: u32,
    pub offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
}

impl<'a> ElfImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        if bytes.len() < ELF_HEADER_SIZE {
            return Err(ElfError::Truncated);
        }
        if &bytes[0..4] != b"\x7fELF" {
            return Err(ElfError::BadMagic);
        }
        if bytes[4] != ELFCLASS64 {
            return Err(ElfError::UnsupportedClass);
        }
        if bytes[5] != ELFDATA2LSB || bytes[6] != 1 {
            return Err(ElfError::UnsupportedEncoding);
        }

        let image_type = read_u16(bytes, 16).ok_or(ElfError::Truncated)?;
        if image_type != ET_EXEC && image_type != ET_DYN {
            return Err(ElfError::UnsupportedType);
        }
        if read_u16(bytes, 18).ok_or(ElfError::Truncated)? != EM_X86_64 {
            return Err(ElfError::UnsupportedMachine);
        }

        let header_size = read_u16(bytes, 52).ok_or(ElfError::Truncated)? as usize;
        let program_header_size = read_u16(bytes, 54).ok_or(ElfError::Truncated)? as usize;
        let program_header_count = read_u16(bytes, 56).ok_or(ElfError::Truncated)? as usize;
        if header_size < ELF_HEADER_SIZE || program_header_size < PROGRAM_HEADER_SIZE {
            return Err(ElfError::InvalidHeaderSize);
        }

        let program_header_offset: usize = read_u64(bytes, 32)
            .ok_or(ElfError::Truncated)?
            .try_into()
            .map_err(|_| ElfError::InvalidProgramHeaders)?;
        let table_size = program_header_size
            .checked_mul(program_header_count)
            .ok_or(ElfError::InvalidProgramHeaders)?;
        let table_end = program_header_offset
            .checked_add(table_size)
            .ok_or(ElfError::InvalidProgramHeaders)?;
        if table_end > bytes.len() {
            return Err(ElfError::InvalidProgramHeaders);
        }

        let image = Self {
            bytes,
            entry: read_u64(bytes, 24).ok_or(ElfError::Truncated)?,
            program_header_offset,
            program_header_size,
            program_header_count,
        };
        if !image.program_headers().any(|header| header.kind == PT_LOAD) {
            return Err(ElfError::NoLoadSegments);
        }
        Ok(image)
    }

    pub fn program_headers(self) -> ProgramHeaders<'a> {
        ProgramHeaders {
            image: self,
            index: 0,
        }
    }

    pub fn file_bytes(self, header: ProgramHeader) -> Option<&'a [u8]> {
        let start: usize = header.offset.try_into().ok()?;
        let length: usize = header.file_size.try_into().ok()?;
        let end = start.checked_add(length)?;
        self.bytes.get(start..end)
    }
}

pub struct ProgramHeaders<'a> {
    image: ElfImage<'a>,
    index: usize,
}

impl<'a> Iterator for ProgramHeaders<'a> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.image.program_header_count {
            return None;
        }
        let offset = self
            .image
            .program_header_offset
            .checked_add(self.index.checked_mul(self.image.program_header_size)?)?;
        self.index += 1;
        Some(ProgramHeader {
            kind: read_u32(self.image.bytes, offset)?,
            flags: read_u32(self.image.bytes, offset + 4)?,
            offset: read_u64(self.image.bytes, offset + 8)?,
            virtual_address: read_u64(self.image.bytes, offset + 16)?,
            file_size: read_u64(self.image.bytes, offset + 32)?,
            memory_size: read_u64(self.image.bytes, offset + 40)?,
            alignment: read_u64(self.image.bytes, offset + 48)?,
        })
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

const fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    let raw = value.to_le_bytes();
    bytes[offset] = raw[0];
    bytes[offset + 1] = raw[1];
}

const fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    let raw = value.to_le_bytes();
    let mut index = 0;
    while index < raw.len() {
        bytes[offset + index] = raw[index];
        index += 1;
    }
}

const fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    let raw = value.to_le_bytes();
    let mut index = 0;
    while index < raw.len() {
        bytes[offset + index] = raw[index];
        index += 1;
    }
}

pub const TEST_DATA_ADDRESS: u64 = 0x0040_1000;

const fn make_test_elf() -> [u8; 0x300] {
    let mut image = [0u8; 0x300];
    image[0] = 0x7f;
    image[1] = b'E';
    image[2] = b'L';
    image[3] = b'F';
    image[4] = ELFCLASS64;
    image[5] = ELFDATA2LSB;
    image[6] = 1;
    put_u16(&mut image, 16, ET_EXEC);
    put_u16(&mut image, 18, EM_X86_64);
    put_u32(&mut image, 20, 1);
    put_u64(&mut image, 24, 0x0040_0000);
    put_u64(&mut image, 32, ELF_HEADER_SIZE as u64);
    put_u16(&mut image, 52, ELF_HEADER_SIZE as u16);
    put_u16(&mut image, 54, PROGRAM_HEADER_SIZE as u16);
    put_u16(&mut image, 56, 2);

    put_u32(&mut image, ELF_HEADER_SIZE, PT_LOAD);
    put_u32(&mut image, ELF_HEADER_SIZE + 4, PF_X);
    put_u64(&mut image, ELF_HEADER_SIZE + 8, 0x100);
    put_u64(&mut image, ELF_HEADER_SIZE + 16, 0x0040_0000);
    put_u64(&mut image, ELF_HEADER_SIZE + 32, 128);
    put_u64(&mut image, ELF_HEADER_SIZE + 40, 128);
    put_u64(&mut image, ELF_HEADER_SIZE + 48, 0x1000);

    let second_header = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;
    put_u32(&mut image, second_header, PT_LOAD);
    put_u32(&mut image, second_header + 4, PF_W);
    put_u64(&mut image, second_header + 8, 0x200);
    put_u64(&mut image, second_header + 16, TEST_DATA_ADDRESS);
    put_u64(&mut image, second_header + 32, 11);
    put_u64(&mut image, second_header + 40, 128);
    put_u64(&mut image, second_header + 48, 0x1000);

    // getpid; trap if it did not return a task identity.
    image[0x100] = 0x48;
    image[0x101] = 0xc7;
    image[0x102] = 0xc0;
    image[0x103] = 39;
    image[0x107] = 0x0f;
    image[0x108] = 0x05;
    image[0x109] = 0x48;
    image[0x10a] = 0x85;
    image[0x10b] = 0xc0;
    image[0x10c] = 0x75;
    image[0x10d] = 2;
    image[0x10e] = 0x0f;
    image[0x10f] = 0x0b;

    // open("/etc/config"), then retain the descriptor in rbx.
    image[0x110] = 0x48;
    image[0x111] = 0xc7;
    image[0x112] = 0xc0;
    image[0x113] = 2;
    image[0x117] = 0x48;
    image[0x118] = 0xbf;
    put_u64(&mut image, 0x119, TEST_DATA_ADDRESS);
    image[0x121] = 0x48;
    image[0x122] = 0xc7;
    image[0x123] = 0xc6;
    put_u32(&mut image, 0x124, 11);
    image[0x128] = 0x0f;
    image[0x129] = 0x05;
    image[0x12a] = 0x48;
    image[0x12b] = 0x89;
    image[0x12c] = 0xc3;

    // read(fd, data + 64, 64); write(data + 64, returned_length).
    image[0x12d] = 0x31;
    image[0x12e] = 0xc0;
    image[0x12f] = 0x48;
    image[0x130] = 0x89;
    image[0x131] = 0xdf;
    image[0x132] = 0x48;
    image[0x133] = 0xbe;
    put_u64(&mut image, 0x134, TEST_DATA_ADDRESS + 64);
    image[0x13c] = 0x48;
    image[0x13d] = 0xc7;
    image[0x13e] = 0xc2;
    put_u32(&mut image, 0x13f, 64);
    image[0x143] = 0x0f;
    image[0x144] = 0x05;
    image[0x145] = 0x48;
    image[0x146] = 0x89;
    image[0x147] = 0xc2;
    image[0x148] = 0x48;
    image[0x149] = 0xc7;
    image[0x14a] = 0xc0;
    image[0x14b] = 1;
    image[0x14f] = 0x48;
    image[0x150] = 0xbf;
    put_u64(&mut image, 0x151, TEST_DATA_ADDRESS + 64);
    image[0x159] = 0x48;
    image[0x15a] = 0x89;
    image[0x15b] = 0xd6;
    image[0x15c] = 0x0f;
    image[0x15d] = 0x05;

    // close(fd); yield; exit(0); ud2 if exit unexpectedly returns.
    image[0x15e] = 0x48;
    image[0x15f] = 0xc7;
    image[0x160] = 0xc0;
    image[0x161] = 3;
    image[0x165] = 0x48;
    image[0x166] = 0x89;
    image[0x167] = 0xdf;
    image[0x168] = 0x0f;
    image[0x169] = 0x05;
    image[0x16a] = 0x48;
    image[0x16b] = 0xc7;
    image[0x16c] = 0xc0;
    image[0x16d] = 24;
    image[0x171] = 0x0f;
    image[0x172] = 0x05;
    image[0x173] = 0x48;
    image[0x174] = 0xc7;
    image[0x175] = 0xc0;
    image[0x176] = 60;
    image[0x17a] = 0x31;
    image[0x17b] = 0xff;
    image[0x17c] = 0x0f;
    image[0x17d] = 0x05;
    image[0x17e] = 0x0f;
    image[0x17f] = 0x0b;
    image[0x200] = b'/';
    image[0x201] = b'e';
    image[0x202] = b't';
    image[0x203] = b'c';
    image[0x204] = b'/';
    image[0x205] = b'c';
    image[0x206] = b'o';
    image[0x207] = b'n';
    image[0x208] = b'f';
    image[0x209] = b'i';
    image[0x20a] = b'g';
    image
}

pub const TEST_ELF: [u8; 0x300] = make_test_elf();
