//! Sector-addressable storage primitives.

use alloc::boxed::Box;
use alloc::vec;
use vanta_gpt::RootPartition;

pub const SECTOR_SIZE: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    OutOfBounds,
    AllocationFailed,
    DeviceUnavailable,
    IoFailed,
    GptMissingRoot,
    GptInvalid,
}

pub trait BlockDevice {
    fn sector_count(&self) -> u64;
    fn read_sector(&self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError>;
    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE])
        -> Result<(), StorageError>;
}

pub fn discover_vanta_root<D: BlockDevice>(device: &D) -> Result<RootPartition, StorageError> {
    vanta_gpt::discover_vanta_root(|sector, buffer| {
        device.read_sector(sector, buffer).map_err(|_| ())
    })
    .map_err(|error| match error {
        vanta_gpt::GptError::MissingRoot => StorageError::GptMissingRoot,
        _ => StorageError::GptInvalid,
    })
}

pub fn has_gpt_signature<D: BlockDevice>(device: &D) -> bool {
    let mut sector = [0_u8; SECTOR_SIZE];
    device.read_sector(1, &mut sector).is_ok() && sector[..8] == *b"EFI PART"
}

pub struct RamDisk {
    sectors: u64,
    data: Box<[u8]>,
}

impl RamDisk {
    pub fn new(sectors: u64) -> Result<Self, StorageError> {
        let bytes = sectors
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(StorageError::AllocationFailed)?;
        let bytes: usize = bytes
            .try_into()
            .map_err(|_| StorageError::AllocationFailed)?;
        let data = vec![0u8; bytes].into_boxed_slice();
        Ok(Self { sectors, data })
    }
}

impl BlockDevice for RamDisk {
    fn sector_count(&self) -> u64 {
        self.sectors
    }

    fn read_sector(&self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if sector >= self.sectors {
            return Err(StorageError::OutOfBounds);
        }
        let start = sector as usize * SECTOR_SIZE;
        buffer.copy_from_slice(&self.data[start..start + SECTOR_SIZE]);
        Ok(())
    }

    fn write_sector(
        &mut self,
        sector: u64,
        buffer: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        if sector >= self.sectors {
            return Err(StorageError::OutOfBounds);
        }
        let start = sector as usize * SECTOR_SIZE;
        self.data[start..start + SECTOR_SIZE].copy_from_slice(buffer);
        Ok(())
    }
}
