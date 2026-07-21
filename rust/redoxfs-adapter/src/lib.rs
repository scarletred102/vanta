#![no_std]

use redoxfs::{Disk, BLOCK_SIZE};
use syscall::error::{Error, Result, EIO};
use vanta_gpt::RootPartition;

pub const SECTOR_SIZE: usize = 512;
const SECTORS_PER_BLOCK: u64 = BLOCK_SIZE / SECTOR_SIZE as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectorError {
    OutOfBounds,
    Io,
}

pub trait SectorIo {
    fn sector_count(&self) -> u64;
    fn read_sector(
        &mut self,
        sector: u64,
        buffer: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), SectorError>;
    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> Result<(), SectorError>;
}

pub struct RedoxDisk<D> {
    device: D,
    partition: RootPartition,
}

impl<D: SectorIo> RedoxDisk<D> {
    pub fn new(device: D, partition: RootPartition) -> core::result::Result<Self, SectorError> {
        if partition.end_lba >= device.sector_count() {
            return Err(SectorError::OutOfBounds);
        }
        Ok(Self { device, partition })
    }

    pub fn into_inner(self) -> D {
        self.device
    }

    fn translate(&self, block: u64, byte_len: usize) -> Result<(u64, usize)> {
        if byte_len % SECTOR_SIZE != 0 {
            return Err(Error::new(EIO));
        }
        let relative_sector = block
            .checked_mul(SECTORS_PER_BLOCK)
            .ok_or_else(|| Error::new(EIO))?;
        let sector_count = (byte_len / SECTOR_SIZE) as u64;
        if sector_count == 0 {
            return Ok((relative_sector, 0));
        }
        let last_sector = relative_sector
            .checked_add(sector_count - 1)
            .ok_or_else(|| Error::new(EIO))?;
        let first_lba = self
            .partition
            .absolute_lba(relative_sector)
            .ok_or_else(|| Error::new(EIO))?;
        self.partition
            .absolute_lba(last_sector)
            .ok_or_else(|| Error::new(EIO))?;
        Ok((first_lba, sector_count as usize))
    }
}

impl<D: SectorIo> Disk for RedoxDisk<D> {
    unsafe fn read_at(&mut self, block: u64, buffer: &mut [u8]) -> Result<usize> {
        let (first_lba, sector_count) = self.translate(block, buffer.len())?;
        for (index, destination) in buffer.chunks_exact_mut(SECTOR_SIZE).enumerate() {
            let mut sector = [0_u8; SECTOR_SIZE];
            self.device
                .read_sector(first_lba + index as u64, &mut sector)
                .map_err(|_| Error::new(EIO))?;
            destination.copy_from_slice(&sector);
        }
        debug_assert_eq!(sector_count, buffer.len() / SECTOR_SIZE);
        Ok(buffer.len())
    }

    unsafe fn write_at(&mut self, block: u64, buffer: &[u8]) -> Result<usize> {
        let (first_lba, sector_count) = self.translate(block, buffer.len())?;
        for (index, source) in buffer.chunks_exact(SECTOR_SIZE).enumerate() {
            let mut sector = [0_u8; SECTOR_SIZE];
            sector.copy_from_slice(source);
            self.device
                .write_sector(first_lba + index as u64, &sector)
                .map_err(|_| Error::new(EIO))?;
        }
        debug_assert_eq!(sector_count, buffer.len() / SECTOR_SIZE);
        Ok(buffer.len())
    }

    fn size(&mut self) -> Result<u64> {
        Ok(self.partition.sector_count() * SECTOR_SIZE as u64)
    }
}
