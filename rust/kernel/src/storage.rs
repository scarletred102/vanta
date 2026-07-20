//! Sector-addressable storage primitives.

use alloc::boxed::Box;
use alloc::vec;

pub const SECTOR_SIZE: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    OutOfBounds,
    AllocationFailed,
    DeviceUnavailable,
    IoFailed,
}

pub trait BlockDevice {
    fn sector_count(&self) -> u64;
    fn read_sector(&self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError>;
    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE])
        -> Result<(), StorageError>;
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
