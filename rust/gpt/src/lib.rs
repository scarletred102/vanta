#![no_std]

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const MIN_HEADER_SIZE: usize = 92;
const TYPE_GUID_SIZE: usize = 16;
const PARTITION_START_OFFSET: usize = 32;
const PARTITION_END_OFFSET: usize = 40;

/// The little-endian GPT representation of
/// `5d2f0d4e-9cff-4b2f-a9b6-6bf9eaa4d201`.
pub const VANTA_ROOT_TYPE_GUID: [u8; TYPE_GUID_SIZE] = [
    0x4e, 0x0d, 0x2f, 0x5d, 0xff, 0x9c, 0x2f, 0x4b, 0xa9, 0xb6, 0x6b, 0xf9, 0xea, 0xa4, 0xd2, 0x01,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootPartition {
    pub start_lba: u64,
    pub end_lba: u64,
}

impl RootPartition {
    pub const fn sector_count(self) -> u64 {
        self.end_lba - self.start_lba + 1
    }

    pub const fn absolute_lba(self, relative_lba: u64) -> Option<u64> {
        if relative_lba >= self.sector_count() {
            return None;
        }
        Some(self.start_lba + relative_lba)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GptError {
    InvalidSignature,
    InvalidHeader,
    InvalidHeaderCrc,
    InvalidPartitionArrayCrc,
    InvalidPartitionLayout,
    MissingRoot,
}

pub fn find_vanta_root(header: &[u8], entries: &[u8]) -> Result<RootPartition, GptError> {
    if header.get(..GPT_SIGNATURE.len()) != Some(GPT_SIGNATURE) {
        return Err(GptError::InvalidSignature);
    }

    let header_size = read_u32(header, 12)? as usize;
    if !(MIN_HEADER_SIZE..=header.len()).contains(&header_size) {
        return Err(GptError::InvalidHeader);
    }
    let expected_header_crc = read_u32(header, 16)?;
    if crc32_with_zeroed_range(&header[..header_size], 16, 4) != expected_header_crc {
        return Err(GptError::InvalidHeaderCrc);
    }

    let current_lba = read_u64(header, 24)?;
    let backup_lba = read_u64(header, 32)?;
    let first_usable_lba = read_u64(header, 40)?;
    let last_usable_lba = read_u64(header, 48)?;
    let entry_count = read_u32(header, 80)? as usize;
    let entry_size = read_u32(header, 84)? as usize;
    let expected_entries_crc = read_u32(header, 88)?;
    let entry_bytes = entry_count
        .checked_mul(entry_size)
        .ok_or(GptError::InvalidHeader)?;

    if current_lba != 1
        || backup_lba <= current_lba
        || first_usable_lba > last_usable_lba
        || entry_size < PARTITION_END_OFFSET + core::mem::size_of::<u64>()
        || entries.len() < entry_bytes
    {
        return Err(GptError::InvalidHeader);
    }
    if crc32(&entries[..entry_bytes]) != expected_entries_crc {
        return Err(GptError::InvalidPartitionArrayCrc);
    }

    for index in 0..entry_count {
        let offset = index * entry_size;
        let entry = &entries[offset..offset + entry_size];
        if entry[..TYPE_GUID_SIZE] != VANTA_ROOT_TYPE_GUID {
            continue;
        }
        let start_lba = read_u64(entry, PARTITION_START_OFFSET)?;
        let end_lba = read_u64(entry, PARTITION_END_OFFSET)?;
        if start_lba < first_usable_lba || end_lba > last_usable_lba || start_lba > end_lba {
            return Err(GptError::InvalidPartitionLayout);
        }
        return Ok(RootPartition { start_lba, end_lba });
    }

    Err(GptError::MissingRoot)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, GptError> {
    let values: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(GptError::InvalidHeader)?
        .try_into()
        .map_err(|_| GptError::InvalidHeader)?;
    Ok(u32::from_le_bytes(values))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, GptError> {
    let values: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or(GptError::InvalidHeader)?
        .try_into()
        .map_err(|_| GptError::InvalidHeader)?;
    Ok(u64::from_le_bytes(values))
}

fn crc32_with_zeroed_range(bytes: &[u8], start: usize, len: usize) -> u32 {
    let mut crc = !0_u32;
    for (index, &value) in bytes.iter().enumerate() {
        crc = crc32_step(
            crc,
            if (start..start + len).contains(&index) {
                0
            } else {
                value
            },
        );
    }
    !crc
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &value in bytes {
        crc = crc32_step(crc, value);
    }
    !crc
}

fn crc32_step(mut crc: u32, value: u8) -> u32 {
    crc ^= value as u32;
    for _ in 0..8 {
        crc = if crc & 1 == 0 {
            crc >> 1
        } else {
            (crc >> 1) ^ 0xedb8_8320
        };
    }
    crc
}
