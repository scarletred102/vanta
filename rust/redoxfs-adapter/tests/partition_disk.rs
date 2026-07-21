use redoxfs::Disk;
use vanta_gpt::RootPartition;
use vanta_redoxfs_adapter::{RedoxDisk, SectorError, SectorIo, SECTOR_SIZE};

struct RecordingDisk {
    sector_count: u64,
    reads: Vec<u64>,
}

impl SectorIo for RecordingDisk {
    fn sector_count(&self) -> u64 {
        self.sector_count
    }

    fn read_sector(
        &mut self,
        sector: u64,
        buffer: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), SectorError> {
        self.reads.push(sector);
        buffer.fill(sector as u8);
        Ok(())
    }

    fn write_sector(
        &mut self,
        _sector: u64,
        _buffer: &[u8; SECTOR_SIZE],
    ) -> Result<(), SectorError> {
        Ok(())
    }
}

#[test]
fn redox_disk_translates_four_kib_blocks_inside_the_root_partition() {
    let backing = RecordingDisk {
        sector_count: 8192,
        reads: Vec::new(),
    };
    let partition = RootPartition {
        start_lba: 2048,
        end_lba: 4095,
    };
    let mut disk = RedoxDisk::new(backing, partition).expect("bounded device");
    let mut buffer = [0_u8; 4096];

    let count = unsafe { disk.read_at(1, &mut buffer) }.expect("second RedoxFS block");
    let backing = disk.into_inner();

    assert_eq!(count, 4096);
    assert_eq!(backing.reads, (2056..2064).collect::<Vec<_>>());
    assert_eq!(buffer[0], 2056_u64 as u8);
    assert_eq!(buffer[SECTOR_SIZE], 2057_u64 as u8);
}

#[test]
fn redox_disk_rejects_reads_past_the_partition_boundary() {
    let backing = RecordingDisk {
        sector_count: 8192,
        reads: Vec::new(),
    };
    let partition = RootPartition {
        start_lba: 2048,
        end_lba: 2055,
    };
    let mut disk = RedoxDisk::new(backing, partition).expect("bounded device");
    let mut buffer = [0_u8; 4096];

    assert!(unsafe { disk.read_at(1, &mut buffer) }.is_err());
    assert!(disk.into_inner().reads.is_empty());
}
