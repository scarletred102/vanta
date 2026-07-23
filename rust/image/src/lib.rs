//! Host-side Vanta GPT image construction.

use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

use redoxfs::{Disk, FileSystem, Node, TreePtr, BLOCK_SIZE};
use syscall::error::{Error as SyscallError, EIO};
use vanta_gpt::{RootPartition, VANTA_ROOT_TYPE_GUID};

pub const SECTOR_SIZE: usize = 512;
const GPT_ENTRY_SIZE: usize = 128;
const GPT_ENTRY_COUNT: usize = 128;
const GPT_ENTRIES_LBA: u64 = 2;
const GPT_FIRST_USABLE_LBA: u64 = 34;
const ESP_START_LBA: u64 = 2_048;
const ESP_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageOptions {
    pub esp_sectors: u64,
    pub root_sectors: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ImageContents<'a> {
    pub boot_efi: &'a [u8],
    pub kernel: &'a [u8],
    pub limine_config: &'a [u8],
    pub root_files: &'a [RootFile<'a>],
}

#[derive(Clone, Copy, Debug)]
pub struct RootFile<'a> {
    pub path: &'a str,
    pub contents: &'a [u8],
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug)]
pub enum ImageError {
    InvalidLayout,
    Io(io::Error),
    RedoxFs(SyscallError),
}

impl From<io::Error> for ImageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct BuiltImage {
    bytes: Vec<u8>,
    esp: RootPartition,
    root: RootPartition,
}

impl BuiltImage {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn esp_bytes(&self) -> &[u8] {
        partition_bytes(&self.bytes, self.esp)
    }

    pub fn root_bytes(&self) -> &[u8] {
        partition_bytes(&self.bytes, self.root)
    }

    pub const fn root_partition(&self) -> RootPartition {
        self.root
    }
}

pub fn build_image(
    options: ImageOptions,
    contents: ImageContents<'_>,
) -> Result<BuiltImage, ImageError> {
    if options.esp_sectors == 0
        || options.root_sectors == 0
        || options.root_sectors % (BLOCK_SIZE / SECTOR_SIZE as u64) != 0
    {
        return Err(ImageError::InvalidLayout);
    }

    let esp = RootPartition {
        start_lba: ESP_START_LBA,
        end_lba: ESP_START_LBA
            .checked_add(options.esp_sectors - 1)
            .ok_or(ImageError::InvalidLayout)?,
    };
    let root = RootPartition {
        start_lba: esp
            .end_lba
            .checked_add(1)
            .ok_or(ImageError::InvalidLayout)?,
        end_lba: esp
            .end_lba
            .checked_add(options.root_sectors)
            .ok_or(ImageError::InvalidLayout)?,
    };
    let total_sectors = root
        .end_lba
        .checked_add(34)
        .ok_or(ImageError::InvalidLayout)?;
    let image_len = usize::try_from(
        total_sectors
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(ImageError::InvalidLayout)?,
    )
    .map_err(|_| ImageError::InvalidLayout)?;
    let mut bytes = vec![0_u8; image_len];

    let esp_bytes = build_esp(options.esp_sectors, contents)?;
    let root_bytes = build_redoxfs(options.root_sectors, contents.root_files)?;
    copy_partition(&mut bytes, esp, &esp_bytes)?;
    copy_partition(&mut bytes, root, &root_bytes)?;
    write_gpt(&mut bytes, esp, root, total_sectors)?;

    Ok(BuiltImage { bytes, esp, root })
}

fn build_esp(sectors: u64, contents: ImageContents<'_>) -> Result<Vec<u8>, ImageError> {
    let size = usize::try_from(
        sectors
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(ImageError::InvalidLayout)?,
    )
    .map_err(|_| ImageError::InvalidLayout)?;
    let disk = SharedCursor::new(size);
    fatfs::format_volume(
        disk.clone(),
        fatfs::FormatVolumeOptions::new().volume_label(*b"VANTA ESP  "),
    )?;
    let fs = fatfs::FileSystem::new(disk.clone(), fatfs::FsOptions::new())?;
    let root = fs.root_dir();
    root.create_dir("EFI")?;
    root.create_dir("EFI/BOOT")?;
    root.create_dir("boot")?;
    write_fat_file(&root, "EFI/BOOT/BOOTX64.EFI", contents.boot_efi)?;
    write_fat_file(&root, "boot/vanta-kernel", contents.kernel)?;
    write_fat_file(&root, "limine.conf", contents.limine_config)?;
    drop(root);
    fs.unmount()?;
    Ok(disk.snapshot())
}

fn write_fat_file(
    root: &fatfs::Dir<SharedCursor>,
    path: &str,
    contents: &[u8],
) -> Result<(), ImageError> {
    let mut file = root.create_file(path)?;
    file.write_all(contents)?;
    Ok(())
}

fn build_redoxfs(sectors: u64, root_files: &[RootFile<'_>]) -> Result<Vec<u8>, ImageError> {
    let bytes = usize::try_from(
        sectors
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(ImageError::InvalidLayout)?,
    )
    .map_err(|_| ImageError::InvalidLayout)?;
    let mut fs =
        FileSystem::create(MemoryDisk::new(bytes), None, 0, 0).map_err(ImageError::RedoxFs)?;
    fs.tx(|tx| {
        let etc = tx
            .create_node(TreePtr::root(), "etc", Node::MODE_DIR | 0o755, 0, 0)?
            .ptr();
        let home = tx
            .create_node(TreePtr::root(), "home", Node::MODE_DIR | 0o755, 0, 0)?
            .ptr();
        tx.create_node(home, "vanta", Node::MODE_DIR | 0o755, 1000, 1000)?;
        let config = tx
            .create_node(etc, "config", Node::MODE_FILE | 0o644, 0, 0)?
            .ptr();
        tx.write_node(config, 0, b"vanta-vfs-syscall\n", 0, 0)?;
        for file in root_files {
            install_root_file(tx, *file)?;
        }
        Ok(())
    })
    .map_err(ImageError::RedoxFs)?;
    Ok(fs.disk.into_bytes())
}

fn install_root_file(
    tx: &mut redoxfs::Transaction<MemoryDisk>,
    file: RootFile<'_>,
) -> Result<(), SyscallError> {
    let mut components = file.path.split('/').filter(|part| !part.is_empty());
    let name = components
        .next_back()
        .ok_or_else(|| SyscallError::new(EIO))?;
    let mut parent = TreePtr::root();
    for component in components {
        parent = match tx.find_node(parent, component) {
            Ok(node) if node.data().is_dir() => node.ptr(),
            Ok(_) => return Err(SyscallError::new(EIO)),
            Err(error) if error.errno == syscall::error::ENOENT => tx
                .create_node(parent, component, Node::MODE_DIR | 0o755, 0, 0)?
                .ptr(),
            Err(error) => return Err(error),
        };
    }
    let node = tx
        .create_node(
            parent,
            name,
            Node::MODE_FILE | file.mode,
            file.uid.into(),
            file.gid.into(),
        )?
        .ptr();
    tx.write_node(node, 0, file.contents, 0, 0)?;
    Ok(())
}

fn copy_partition(
    image: &mut [u8],
    partition: RootPartition,
    contents: &[u8],
) -> Result<(), ImageError> {
    let destination = partition_bytes_mut(image, partition);
    if destination.len() != contents.len() {
        return Err(ImageError::InvalidLayout);
    }
    destination.copy_from_slice(contents);
    Ok(())
}

fn partition_bytes(image: &[u8], partition: RootPartition) -> &[u8] {
    let start = partition.start_lba as usize * SECTOR_SIZE;
    let end = (partition.end_lba as usize + 1) * SECTOR_SIZE;
    &image[start..end]
}

fn partition_bytes_mut(image: &mut [u8], partition: RootPartition) -> &mut [u8] {
    let start = partition.start_lba as usize * SECTOR_SIZE;
    let end = (partition.end_lba as usize + 1) * SECTOR_SIZE;
    &mut image[start..end]
}

fn write_gpt(
    image: &mut [u8],
    esp: RootPartition,
    root: RootPartition,
    total_sectors: u64,
) -> Result<(), ImageError> {
    let entries_len = GPT_ENTRY_SIZE * GPT_ENTRY_COUNT;
    let mut entries = vec![0_u8; entries_len];
    write_entry(&mut entries[..GPT_ENTRY_SIZE], ESP_TYPE_GUID, esp, 1);
    write_entry(
        &mut entries[GPT_ENTRY_SIZE..GPT_ENTRY_SIZE * 2],
        VANTA_ROOT_TYPE_GUID,
        root,
        2,
    );
    let entries_crc = crc32(&entries);
    let entries_sectors = (entries_len / SECTOR_SIZE) as u64;
    write_at_lba(image, GPT_ENTRIES_LBA, &entries)?;
    write_at_lba(image, total_sectors - 1 - entries_sectors, &entries)?;
    write_header(
        image,
        1,
        total_sectors - 1,
        GPT_ENTRIES_LBA,
        total_sectors - 34,
        entries_crc,
    )?;
    write_header(
        image,
        total_sectors - 1,
        1,
        total_sectors - 1 - entries_sectors,
        total_sectors - 34,
        entries_crc,
    )?;
    write_protective_mbr(image, total_sectors)?;
    Ok(())
}

fn write_entry(entry: &mut [u8], type_guid: [u8; 16], partition: RootPartition, id: u8) {
    entry[..16].copy_from_slice(&type_guid);
    entry[16] = id;
    entry[32..40].copy_from_slice(&partition.start_lba.to_le_bytes());
    entry[40..48].copy_from_slice(&partition.end_lba.to_le_bytes());
}

fn write_header(
    image: &mut [u8],
    current_lba: u64,
    backup_lba: u64,
    entries_lba: u64,
    last_usable_lba: u64,
    entries_crc: u32,
) -> Result<(), ImageError> {
    let mut header = [0_u8; SECTOR_SIZE];
    header[..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000_u32.to_le_bytes());
    header[12..16].copy_from_slice(&92_u32.to_le_bytes());
    header[24..32].copy_from_slice(&current_lba.to_le_bytes());
    header[32..40].copy_from_slice(&backup_lba.to_le_bytes());
    header[40..48].copy_from_slice(&GPT_FIRST_USABLE_LBA.to_le_bytes());
    header[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
    header[56] = 0x56;
    header[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    header[80..84].copy_from_slice(&(GPT_ENTRY_COUNT as u32).to_le_bytes());
    header[84..88].copy_from_slice(&(GPT_ENTRY_SIZE as u32).to_le_bytes());
    header[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let checksum = crc32(&header[..92]);
    header[16..20].copy_from_slice(&checksum.to_le_bytes());
    write_at_lba(image, current_lba, &header)
}

fn write_protective_mbr(image: &mut [u8], total_sectors: u64) -> Result<(), ImageError> {
    let mbr = image
        .get_mut(..SECTOR_SIZE)
        .ok_or(ImageError::InvalidLayout)?;
    mbr[446 + 4] = 0xee;
    mbr[446 + 8..446 + 12].copy_from_slice(&1_u32.to_le_bytes());
    mbr[446 + 12..446 + 16].copy_from_slice(
        &(total_sectors.saturating_sub(1).min(u32::MAX as u64) as u32).to_le_bytes(),
    );
    mbr[510..512].copy_from_slice(&[0x55, 0xaa]);
    Ok(())
}

fn write_at_lba(image: &mut [u8], lba: u64, contents: &[u8]) -> Result<(), ImageError> {
    let start = usize::try_from(
        lba.checked_mul(SECTOR_SIZE as u64)
            .ok_or(ImageError::InvalidLayout)?,
    )
    .map_err(|_| ImageError::InvalidLayout)?;
    let end = start
        .checked_add(contents.len())
        .ok_or(ImageError::InvalidLayout)?;
    let destination = image.get_mut(start..end).ok_or(ImageError::InvalidLayout)?;
    destination.copy_from_slice(contents);
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

#[derive(Clone)]
struct SharedCursor {
    bytes: Rc<RefCell<Vec<u8>>>,
    position: u64,
}

impl SharedCursor {
    fn new(size: usize) -> Self {
        Self {
            bytes: Rc::new(RefCell::new(vec![0_u8; size])),
            position: 0,
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.borrow().clone()
    }
}

impl Read for SharedCursor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let bytes = self.bytes.borrow();
        let start = usize::try_from(self.position).unwrap_or(bytes.len());
        let count = buffer.len().min(bytes.len().saturating_sub(start));
        buffer[..count].copy_from_slice(&bytes[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Write for SharedCursor {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut bytes = self.bytes.borrow_mut();
        let start = usize::try_from(self.position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "position too large"))?;
        let end = start
            .checked_add(buffer.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "position too large"))?;
        if end > bytes.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "ESP is full"));
        }
        bytes[start..end].copy_from_slice(buffer);
        self.position = end as u64;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedCursor {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let len = self.bytes.borrow().len() as i128;
        let target = match position {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::Current(delta) => self.position as i128 + delta as i128,
            SeekFrom::End(delta) => len + delta as i128,
        };
        if target < 0 || target > u64::MAX as i128 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

struct MemoryDisk {
    bytes: Vec<u8>,
}

impl MemoryDisk {
    fn new(size: usize) -> Self {
        Self {
            bytes: vec![0_u8; size],
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redoxfs_root_contains_requested_native_programs() {
        let init = RootFile {
            path: "/sbin/init",
            contents: b"init",
            mode: 0o755,
            uid: 0,
            gid: 0,
        };
        let shell = RootFile {
            path: "/bin/vsh",
            contents: b"vsh",
            mode: 0o755,
            uid: 1000,
            gid: 1000,
        };
        let image = build_image(
            ImageOptions {
                esp_sectors: 8_192,
                root_sectors: 8_192,
            },
            ImageContents {
                boot_efi: b"boot",
                kernel: b"kernel",
                limine_config: b"config",
                root_files: &[init, shell],
            },
        )
        .unwrap();

        let mut fs = FileSystem::open(
            MemoryDisk {
                bytes: image.root_bytes().to_vec(),
            },
            None,
            None,
            true,
        )
        .unwrap();
        fs.tx(|tx| {
            let sbin = tx.find_node(TreePtr::root(), "sbin")?.ptr();
            let init = tx.find_node(sbin, "init")?;
            assert_eq!(init.data().mode(), Node::MODE_FILE | 0o755);
            let bin = tx.find_node(TreePtr::root(), "bin")?.ptr();
            let shell = tx.find_node(bin, "vsh")?;
            assert_eq!(shell.data().mode(), Node::MODE_FILE | 0o755);
            Ok(())
        })
        .unwrap();
    }
}

impl Disk for MemoryDisk {
    unsafe fn read_at(&mut self, block: u64, buffer: &mut [u8]) -> Result<usize, SyscallError> {
        let range = self
            .range(block, buffer.len())
            .ok_or_else(|| SyscallError::new(EIO))?;
        buffer.copy_from_slice(&self.bytes[range]);
        Ok(buffer.len())
    }

    unsafe fn write_at(&mut self, block: u64, buffer: &[u8]) -> Result<usize, SyscallError> {
        let range = self
            .range(block, buffer.len())
            .ok_or_else(|| SyscallError::new(EIO))?;
        self.bytes[range].copy_from_slice(buffer);
        Ok(buffer.len())
    }

    fn size(&mut self) -> Result<u64, SyscallError> {
        Ok(self.bytes.len() as u64)
    }
}

impl MemoryDisk {
    fn range(&self, block: u64, len: usize) -> Option<std::ops::Range<usize>> {
        let start = usize::try_from(block.checked_mul(BLOCK_SIZE)?).ok()?;
        let end = start.checked_add(len)?;
        (end <= self.bytes.len()).then_some(start..end)
    }
}
