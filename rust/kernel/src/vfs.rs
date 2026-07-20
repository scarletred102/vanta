//! Writable VantaFS volume and root VFS mount.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use crate::storage::{BlockDevice, RamDisk, StorageError, SECTOR_SIZE};
use crate::virtio::VirtioBlock;

const MAGIC: &[u8; 8] = b"VANTA1FS";
const SUPERBLOCK_SECTOR: u64 = 0;
const DIRECTORY_SECTOR: u64 = 1;
const FIRST_DATA_SECTOR: u32 = 2;
const MAX_DIRECTORY_ENTRIES: usize = 8;
const DIRECTORY_ENTRY_SIZE: usize = 64;
const MAX_PATH_LENGTH: usize = 48;

static ROOT: Mutex<Vfs<RootDevice>> = Mutex::new(Vfs::new());
static TMP: Mutex<Option<VantaFs<RamDisk>>> = Mutex::new(None);

pub enum RootDevice {
    Ram(RamDisk),
    Virtio(VirtioBlock),
}

impl BlockDevice for RootDevice {
    fn sector_count(&self) -> u64 {
        match self {
            Self::Ram(device) => device.sector_count(),
            Self::Virtio(device) => device.sector_count(),
        }
    }

    fn read_sector(&self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        match self {
            Self::Ram(device) => device.read_sector(sector, buffer),
            Self::Virtio(device) => device.read_sector(sector, buffer),
        }
    }

    fn write_sector(
        &mut self,
        sector: u64,
        buffer: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        match self {
            Self::Ram(device) => device.write_sector(sector, buffer),
            Self::Virtio(device) => device.write_sector(sector, buffer),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsError {
    Storage(StorageError),
    InvalidFormat,
    NotMounted,
    InvalidPath,
    NameTooLong,
    NotFound,
    AlreadyExists,
    IsDirectory,
    NotEmpty,
    NoSpace,
    FileTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub length: usize,
    pub allocated_sectors: u32,
    pub is_directory: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    File,
    Directory,
}

impl From<StorageError> for VfsError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

#[derive(Clone, Copy)]
struct FileRecord {
    kind: EntryKind,
    name: [u8; MAX_PATH_LENGTH],
    name_length: usize,
    start_sector: u32,
    sector_count: u32,
    length: usize,
}

impl FileRecord {
    const fn empty() -> Self {
        Self {
            kind: EntryKind::File,
            name: [0; MAX_PATH_LENGTH],
            name_length: 0,
            start_sector: 0,
            sector_count: 0,
            length: 0,
        }
    }

    fn matches(&self, path: &[u8]) -> bool {
        self.name_length == path.len() && &self.name[..self.name_length] == path
    }
}

pub struct VantaFs<D: BlockDevice> {
    device: D,
    sector_count: u64,
}

impl<D: BlockDevice> VantaFs<D> {
    pub fn format(mut device: D) -> Result<Self, VfsError> {
        if device.sector_count() <= FIRST_DATA_SECTOR as u64 {
            return Err(VfsError::NoSpace);
        }
        let mut superblock = [0u8; SECTOR_SIZE];
        superblock[..MAGIC.len()].copy_from_slice(MAGIC);
        put_u32(&mut superblock, 8, 1);
        put_u32(&mut superblock, 12, SECTOR_SIZE as u32);
        put_u64(&mut superblock, 16, device.sector_count());
        put_u32(&mut superblock, 24, DIRECTORY_SECTOR as u32);
        put_u32(&mut superblock, 28, MAX_DIRECTORY_ENTRIES as u32);
        device.write_sector(SUPERBLOCK_SECTOR, &superblock)?;
        device.write_sector(DIRECTORY_SECTOR, &[0; SECTOR_SIZE])?;
        Ok(Self {
            sector_count: device.sector_count(),
            device,
        })
    }

    pub fn mount(device: D) -> Result<Self, VfsError> {
        let mut superblock = [0u8; SECTOR_SIZE];
        device.read_sector(SUPERBLOCK_SECTOR, &mut superblock)?;
        if &superblock[..MAGIC.len()] != MAGIC
            || get_u32(&superblock, 8) != 1
            || get_u32(&superblock, 12) != SECTOR_SIZE as u32
            || get_u64(&superblock, 16) != device.sector_count()
            || get_u32(&superblock, 24) != DIRECTORY_SECTOR as u32
            || get_u32(&superblock, 28) != MAX_DIRECTORY_ENTRIES as u32
        {
            return Err(VfsError::InvalidFormat);
        }
        Ok(Self {
            sector_count: device.sector_count(),
            device,
        })
    }

    pub fn mount_or_format(device: D) -> Result<(Self, bool), VfsError> {
        let mut superblock = [0u8; SECTOR_SIZE];
        device.read_sector(SUPERBLOCK_SECTOR, &mut superblock)?;
        let formatted = &superblock[..MAGIC.len()] == MAGIC
            && get_u32(&superblock, 8) == 1
            && get_u32(&superblock, 12) == SECTOR_SIZE as u32
            && get_u64(&superblock, 16) == device.sector_count()
            && get_u32(&superblock, 24) == DIRECTORY_SECTOR as u32
            && get_u32(&superblock, 28) == MAX_DIRECTORY_ENTRIES as u32;
        if formatted {
            Ok((Self::mount(device)?, true))
        } else {
            Ok((Self::format(device)?, false))
        }
    }

    pub fn into_device(self) -> D {
        self.device
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, VfsError> {
        let path = normalize_path(path)?;
        let (_, record) = self.find_record(path)?.ok_or(VfsError::NotFound)?;
        if record.kind == EntryKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        let mut data = vec![0u8; record.length];
        let mut sector = [0u8; SECTOR_SIZE];
        let mut copied = 0;
        for index in 0..record.sector_count {
            self.device
                .read_sector(record.start_sector as u64 + index as u64, &mut sector)?;
            let count = (record.length - copied).min(SECTOR_SIZE);
            data[copied..copied + count].copy_from_slice(&sector[..count]);
            copied += count;
            if copied == record.length {
                break;
            }
        }
        Ok(data)
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let path = normalize_path(path)?;
        let required_sectors: u32 = ((data.len().max(1) + SECTOR_SIZE - 1) / SECTOR_SIZE)
            .try_into()
            .map_err(|_| VfsError::FileTooLarge)?;
        let existing = self.find_record(path)?;
        let (index, mut record) = if let Some((index, mut record)) = existing {
            if record.kind == EntryKind::Directory {
                return Err(VfsError::IsDirectory);
            }
            if record.sector_count != required_sectors {
                record.start_sector = self.allocate(required_sectors, Some(index))?;
                record.sector_count = required_sectors;
            }
            (index, record)
        } else {
            let index = self.first_free_index()?;
            let mut record = FileRecord::empty();
            record.name[..path.len()].copy_from_slice(path);
            record.name_length = path.len();
            record.start_sector = self.allocate(required_sectors, None)?;
            record.sector_count = required_sectors;
            (index, record)
        };
        record.length = data.len();

        let mut sector = [0u8; SECTOR_SIZE];
        for offset in 0..record.sector_count as usize {
            sector.fill(0);
            let start = offset * SECTOR_SIZE;
            if start < data.len() {
                let count = (data.len() - start).min(SECTOR_SIZE);
                sector[..count].copy_from_slice(&data[start..start + count]);
            }
            self.device
                .write_sector(record.start_sector as u64 + offset as u64, &sector)?;
        }
        self.write_record(index, record)
    }

    pub fn remove_file(&mut self, path: &str) -> Result<(), VfsError> {
        let path = normalize_path(path)?;
        let (index, record) = self.find_record(path)?.ok_or(VfsError::NotFound)?;
        if record.kind == EntryKind::Directory && self.has_children(path)? {
            return Err(VfsError::NotEmpty);
        }
        self.clear_record(index)
    }

    pub fn create_dir(&mut self, path: &str) -> Result<(), VfsError> {
        let path = normalize_path(path)?;
        if self.find_record(path)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        let index = self.first_free_index()?;
        let mut record = FileRecord::empty();
        record.kind = EntryKind::Directory;
        record.name[..path.len()].copy_from_slice(path);
        record.name_length = path.len();
        self.write_record(index, record)
    }

    pub fn rename_file(&mut self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        let old_path = normalize_path(old_path)?;
        let new_path = normalize_path(new_path)?;
        if old_path == new_path {
            return Ok(());
        }
        let (index, mut record) = self.find_record(old_path)?.ok_or(VfsError::NotFound)?;
        if record.kind == EntryKind::Directory && self.has_children(old_path)? {
            return Err(VfsError::NotEmpty);
        }
        if self.find_record(new_path)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        record.name.fill(0);
        record.name[..new_path.len()].copy_from_slice(new_path);
        record.name_length = new_path.len();
        self.write_record(index, record)
    }

    pub fn file_info(&mut self, path: &str) -> Result<FileInfo, VfsError> {
        let path = normalize_path(path)?;
        let (_, record) = self.find_record(path)?.ok_or(VfsError::NotFound)?;
        Ok(FileInfo {
            length: record.length,
            allocated_sectors: record.sector_count,
            is_directory: record.kind == EntryKind::Directory,
        })
    }

    pub fn list_files(&mut self) -> Result<Vec<String>, VfsError> {
        let records = self.records()?;
        let mut paths = Vec::new();
        for record in records.iter().filter(|record| record.name_length != 0) {
            let name = core::str::from_utf8(&record.name[..record.name_length])
                .map_err(|_| VfsError::InvalidFormat)?;
            let mut path = String::from("/");
            path.push_str(name);
            if record.kind == EntryKind::Directory {
                path.push('/');
            }
            paths.push(path);
        }
        Ok(paths)
    }

    fn find_record(&mut self, path: &[u8]) -> Result<Option<(usize, FileRecord)>, VfsError> {
        let records = self.records()?;
        Ok(records
            .iter()
            .enumerate()
            .find(|(_, record)| record.name_length != 0 && record.matches(path))
            .map(|(index, record)| (index, *record)))
    }

    fn has_children(&mut self, path: &[u8]) -> Result<bool, VfsError> {
        let records = self.records()?;
        Ok(records.iter().any(|record| {
            record.name_length > path.len()
                && record.name[..record.name_length].starts_with(path)
                && record.name[path.len()] == b'/'
        }))
    }

    fn records(&mut self) -> Result<[FileRecord; MAX_DIRECTORY_ENTRIES], VfsError> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.device.read_sector(DIRECTORY_SECTOR, &mut sector)?;
        let mut records = [FileRecord::empty(); MAX_DIRECTORY_ENTRIES];
        for (index, record) in records.iter_mut().enumerate() {
            let offset = index * DIRECTORY_ENTRY_SIZE;
            let active = sector[offset];
            if active == 0 {
                continue;
            }
            record.kind = match active {
                1 => EntryKind::File,
                2 => EntryKind::Directory,
                _ => return Err(VfsError::InvalidFormat),
            };
            if active != 1 && active != 2 {
                return Err(VfsError::InvalidFormat);
            }
            record.name_length = sector[offset + 1] as usize;
            if record.name_length == 0 || record.name_length > MAX_PATH_LENGTH {
                return Err(VfsError::InvalidFormat);
            }
            record
                .name
                .copy_from_slice(&sector[offset + 2..offset + 50]);
            record.start_sector = get_u32(&sector, offset + 50);
            record.sector_count = get_u32(&sector, offset + 54);
            record.length = get_u32(&sector, offset + 58) as usize;
            if record.name_length == 0 {
                record.start_sector = 0;
                record.sector_count = 0;
                record.length = 0;
            }
            if record.sector_count != 0
                && (record.start_sector < FIRST_DATA_SECTOR
                    || record.start_sector as u64 + record.sector_count as u64 > self.sector_count)
            {
                return Err(VfsError::InvalidFormat);
            }
            if record.length > record.sector_count as usize * SECTOR_SIZE {
                return Err(VfsError::InvalidFormat);
            }
            if record.kind == EntryKind::Directory
                && (record.length != 0 || record.sector_count != 0 || record.start_sector != 0)
            {
                return Err(VfsError::InvalidFormat);
            }
        }
        Ok(records)
    }

    fn write_record(&mut self, index: usize, record: FileRecord) -> Result<(), VfsError> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.device.read_sector(DIRECTORY_SECTOR, &mut sector)?;
        let offset = index * DIRECTORY_ENTRY_SIZE;
        sector[offset] = match record.kind {
            EntryKind::File => 1,
            EntryKind::Directory => 2,
        };
        sector[offset + 1] = record.name_length as u8;
        sector[offset + 2..offset + 50].copy_from_slice(&record.name);
        put_u32(&mut sector, offset + 50, record.start_sector);
        put_u32(&mut sector, offset + 54, record.sector_count);
        let length: u32 = record
            .length
            .try_into()
            .map_err(|_| VfsError::FileTooLarge)?;
        put_u32(&mut sector, offset + 58, length);
        self.device.write_sector(DIRECTORY_SECTOR, &sector)?;
        Ok(())
    }

    fn clear_record(&mut self, index: usize) -> Result<(), VfsError> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.device.read_sector(DIRECTORY_SECTOR, &mut sector)?;
        let offset = index * DIRECTORY_ENTRY_SIZE;
        sector[offset..offset + DIRECTORY_ENTRY_SIZE].fill(0);
        self.device.write_sector(DIRECTORY_SECTOR, &sector)?;
        Ok(())
    }

    fn first_free_index(&mut self) -> Result<usize, VfsError> {
        self.records()?
            .iter()
            .position(|record| record.name_length == 0)
            .ok_or(VfsError::NoSpace)
    }

    fn allocate(&mut self, sectors: u32, ignored_index: Option<usize>) -> Result<u32, VfsError> {
        let records = self.records()?;
        let last_start = self
            .sector_count
            .checked_sub(sectors as u64)
            .ok_or(VfsError::NoSpace)?;
        for candidate in FIRST_DATA_SECTOR as u64..=last_start {
            let end = candidate + sectors as u64;
            let available = records.iter().enumerate().all(|(index, record)| {
                index == ignored_index.unwrap_or(usize::MAX)
                    || record.name_length == 0
                    || end <= record.start_sector as u64
                    || candidate >= record.start_sector as u64 + record.sector_count as u64
            });
            if available {
                return candidate.try_into().map_err(|_| VfsError::NoSpace);
            }
        }
        Err(VfsError::NoSpace)
    }
}

pub struct Vfs<D: BlockDevice> {
    root: Option<VantaFs<D>>,
}

impl<D: BlockDevice> Vfs<D> {
    pub const fn new() -> Self {
        Self { root: None }
    }

    pub fn mount_root(&mut self, filesystem: VantaFs<D>) -> Result<(), VfsError> {
        if self.root.is_some() {
            return Err(VfsError::InvalidFormat);
        }
        self.root = Some(filesystem);
        Ok(())
    }

    pub fn unmount_root(&mut self) -> Result<VantaFs<D>, VfsError> {
        self.root.take().ok_or(VfsError::NotMounted)
    }

    pub fn replace_root(&mut self, filesystem: VantaFs<D>) {
        self.root = Some(filesystem);
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .read_file(path)
    }

    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .write_file(path, data)
    }

    pub fn list(&mut self) -> Result<Vec<String>, VfsError> {
        self.root.as_mut().ok_or(VfsError::NotMounted)?.list_files()
    }

    pub fn remove(&mut self, path: &str) -> Result<(), VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .remove_file(path)
    }

    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .rename_file(old_path, new_path)
    }

    pub fn info(&mut self, path: &str) -> Result<FileInfo, VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .file_info(path)
    }

    pub fn create_dir(&mut self, path: &str) -> Result<(), VfsError> {
        self.root
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .create_dir(path)
    }
}

pub fn initialize_root(sectors: u64) -> Result<(), VfsError> {
    let disk = RamDisk::new(sectors).map_err(VfsError::Storage)?;
    let filesystem = VantaFs::format(RootDevice::Ram(disk))?;
    ROOT.lock().mount_root(filesystem)?;
    let tmp_disk = RamDisk::new(32).map_err(VfsError::Storage)?;
    *TMP.lock() = Some(VantaFs::format(tmp_disk)?);
    Ok(())
}

pub fn mount_virtio_root(device: VirtioBlock) -> Result<bool, VfsError> {
    let (filesystem, existed) = VantaFs::mount_or_format(RootDevice::Virtio(device))?;
    ROOT.lock().replace_root(filesystem);
    Ok(existed)
}

pub fn remount_root() -> Result<(), VfsError> {
    let mut root = ROOT.lock();
    let filesystem = root.unmount_root()?;
    root.mount_root(VantaFs::mount(filesystem.into_device())?)
}

pub fn read_root(path: &str) -> Result<Vec<u8>, VfsError> {
    if let Some(path) = tmp_path(path) {
        return TMP
            .lock()
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .read_file(path);
    }
    ROOT.lock().read(path)
}

pub fn write_root(path: &str, data: &[u8]) -> Result<(), VfsError> {
    if let Some(path) = tmp_path(path) {
        return TMP
            .lock()
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .write_file(path, data);
    }
    ROOT.lock().write(path, data)
}

pub fn list_root() -> Result<Vec<String>, VfsError> {
    let mut paths = ROOT.lock().list()?;
    paths.push(String::from("/tmp/"));
    let mut tmp = TMP.lock();
    for path in tmp.as_mut().ok_or(VfsError::NotMounted)?.list_files()? {
        let mut mounted = String::from("/tmp/");
        mounted.push_str(path.trim_start_matches('/'));
        paths.push(mounted);
    }
    Ok(paths)
}

pub fn remove_root(path: &str) -> Result<(), VfsError> {
    if let Some(path) = tmp_path(path) {
        return TMP
            .lock()
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .remove_file(path);
    }
    ROOT.lock().remove(path)
}

pub fn rename_root(old_path: &str, new_path: &str) -> Result<(), VfsError> {
    match (tmp_path(old_path), tmp_path(new_path)) {
        (Some(old_path), Some(new_path)) => {
            return TMP
                .lock()
                .as_mut()
                .ok_or(VfsError::NotMounted)?
                .rename_file(old_path, new_path)
        }
        (Some(_), None) | (None, Some(_)) => return Err(VfsError::InvalidPath),
        (None, None) => {}
    }
    ROOT.lock().rename(old_path, new_path)
}

pub fn file_info_root(path: &str) -> Result<FileInfo, VfsError> {
    if path == "/tmp" || path == "/tmp/" {
        return Ok(FileInfo {
            length: 0,
            allocated_sectors: 0,
            is_directory: true,
        });
    }
    if let Some(path) = tmp_path(path) {
        return TMP
            .lock()
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .file_info(path);
    }
    ROOT.lock().info(path)
}

pub fn create_dir_root(path: &str) -> Result<(), VfsError> {
    if path == "/tmp" || path == "/tmp/" {
        return Ok(());
    }
    if let Some(path) = tmp_path(path) {
        return TMP
            .lock()
            .as_mut()
            .ok_or(VfsError::NotMounted)?
            .create_dir(path);
    }
    ROOT.lock().create_dir(path)
}

fn tmp_path(path: &str) -> Option<&str> {
    path.strip_prefix("/tmp/")
        .or_else(|| path.strip_prefix("tmp/"))
}

fn normalize_path(path: &str) -> Result<&[u8], VfsError> {
    let path = path
        .as_bytes()
        .strip_prefix(b"/")
        .unwrap_or(path.as_bytes());
    if path.is_empty()
        || path.len() > MAX_PATH_LENGTH
        || path
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || component == b"." || component == b"..")
    {
        return Err(if path.len() > MAX_PATH_LENGTH {
            VfsError::NameTooLong
        } else {
            VfsError::InvalidPath
        });
    }
    Ok(path)
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
