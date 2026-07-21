use vanta_gpt::RootPartition;
use vanta_redoxfs_adapter::{RedoxFsBackend, SectorError, SectorIo, SECTOR_SIZE};

struct MemoryDisk {
    sectors: Vec<[u8; SECTOR_SIZE]>,
}

impl MemoryDisk {
    fn new(sectors: usize) -> Self {
        Self {
            sectors: vec![[0; SECTOR_SIZE]; sectors],
        }
    }
}

impl SectorIo for MemoryDisk {
    fn sector_count(&self) -> u64 {
        self.sectors.len() as u64
    }

    fn read_sector(
        &mut self,
        sector: u64,
        buffer: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), SectorError> {
        *buffer = *self
            .sectors
            .get(sector as usize)
            .ok_or(SectorError::OutOfBounds)?;
        Ok(())
    }

    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> Result<(), SectorError> {
        *self
            .sectors
            .get_mut(sector as usize)
            .ok_or(SectorError::OutOfBounds)? = *buffer;
        Ok(())
    }
}

#[test]
fn backend_persists_nested_file_lifecycle() {
    let partition = RootPartition {
        start_lba: 0,
        end_lba: 65_535,
    };
    let mut backend =
        RedoxFsBackend::format(MemoryDisk::new(65_536), partition).expect("format RedoxFS root");

    backend.create_dir_all("/etc").expect("create directory");
    backend
        .write_file("/etc/vanta.conf", b"terminal=true\n")
        .expect("write file");
    assert_eq!(
        backend.read_file("/etc/vanta.conf").unwrap(),
        b"terminal=true\n"
    );
    let info = backend.file_info("/etc/vanta.conf").expect("file info");
    assert_eq!(info.length, 14);
    assert!(!info.is_directory);
    assert_eq!(backend.list_dir("/etc").unwrap(), vec!["vanta.conf"]);
    backend
        .rename("/etc/vanta.conf", "/etc/vanta.toml")
        .expect("rename file");

    let disk = backend.into_inner();
    let mut backend = RedoxFsBackend::open(disk, partition).expect("reopen RedoxFS root");
    assert_eq!(
        backend.read_file("/etc/vanta.toml").unwrap(),
        b"terminal=true\n"
    );
    backend.remove_file("/etc/vanta.toml").expect("remove file");
    assert!(backend.read_file("/etc/vanta.toml").is_err());
}
