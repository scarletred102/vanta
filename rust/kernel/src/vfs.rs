//! Writable VantaFS volume and root VFS mount.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;
use vanta_abi::Credentials;
use vanta_gpt::RootPartition;
use vanta_redoxfs_adapter::{redoxfs, RedoxFsBackend, SectorError, SectorIo};

use crate::storage::{BlockDevice, RamDisk, StorageError, SECTOR_SIZE};
use crate::virtio::VirtioBlock;

const MAGIC: &[u8; 8] = b"VANTA1FS";
const SUPERBLOCK_SECTOR: u64 = 0;
const DIRECTORY_SECTOR: u64 = 1;
const FIRST_DATA_SECTOR: u32 = 2;
const MAX_DIRECTORY_ENTRIES: usize = 8;
const DIRECTORY_ENTRY_SIZE: usize = 64;
const MAX_PATH_LENGTH: usize = 48;

static ROOT: Mutex<Option<Arc<VantaFsAdapter>>> = Mutex::new(None);
static REDOX_ROOT: Mutex<Option<Arc<RedoxFsAdapter>>> = Mutex::new(None);
static MOUNT_TABLE: Mutex<MountTable> = Mutex::new(MountTable::new());

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

impl SectorIo for RootDevice {
    fn sector_count(&self) -> u64 {
        BlockDevice::sector_count(self)
    }

    fn read_sector(
        &mut self,
        sector: u64,
        buffer: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), SectorError> {
        BlockDevice::read_sector(self, sector, buffer).map_err(|_| SectorError::Io)
    }

    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> Result<(), SectorError> {
        BlockDevice::write_sector(self, sector, buffer).map_err(|_| SectorError::Io)
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
    NotDirectory,
    NotEmpty,
    NoSpace,
    FileTooLarge,
    RedoxFs,
    PermissionDenied,
    ReadOnlyFilesystem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InodeMetadata {
    pub ino: u64,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatFs {
    pub f_type: u64,
    pub f_bsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
}

pub mod mount_flags {
    pub const MS_RDONLY: u32 = 1;
    pub const MS_NOSUID: u32 = 2;
    pub const MS_NODEV: u32 = 4;
    pub const MS_NOEXEC: u32 = 8;
    pub const MS_SYNCHRONOUS: u32 = 16;
    pub const MS_REMOUNT: u32 = 32;
    pub const MS_MANDLOCK: u32 = 64;
    pub const MS_DIRSYNC: u32 = 128;
    pub const MS_NOATIME: u32 = 1024;
    pub const MS_NODIRATIME: u32 = 2048;
    pub const MS_BIND: u32 = 4096;
}

pub mod umount_flags {
    pub const MNT_FORCE: u32 = 1;
    pub const MNT_DETACH: u32 = 2;
    pub const MNT_EXPIRE: u32 = 4;
    pub const UMOUNT_NOFOLLOW: u32 = 8;
}

pub trait Filesystem: Send + Sync {
    fn root_inode(&self) -> u64;
    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError>;
    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError>;
    fn create(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError>;
    fn mkdir(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError>;
    fn unlink(&self, parent: u64, name: &str) -> Result<(), VfsError>;
    fn rmdir(&self, parent: u64, name: &str) -> Result<(), VfsError>;
    fn symlink(&self, parent: u64, name: &str, target: &str) -> Result<u64, VfsError>;
    fn readlink(&self, ino: u64) -> Result<String, VfsError>;
    fn rename(&self, old_parent: u64, old_name: &str, new_parent: u64, new_name: &str) -> Result<(), VfsError>;
    fn read(&self, ino: u64, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError>;
    fn write(&self, ino: u64, offset: u64, buf: &[u8]) -> Result<usize, VfsError>;
    fn truncate(&self, ino: u64, size: u64) -> Result<(), VfsError>;
    fn statfs(&self) -> Result<StatFs, VfsError>;
    fn sync(&self) -> Result<(), VfsError>;
    fn read_dir(&self, ino: u64) -> Result<Vec<String>, VfsError>;
    fn open_inode(&self, _ino: u64) {}
    fn close_inode(&self, _ino: u64) {}

    fn resolve_path(&self, path: &str) -> Result<u64, VfsError> {
        let mut curr = self.root_inode();
        let path = path.trim_matches('/');
        if path.is_empty() {
            return Ok(curr);
        }
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            curr = self.lookup(curr, component)?;
        }
        Ok(curr)
    }

    fn read_file_path(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        let ino = self.resolve_path(path)?;
        let meta = self.read_inode(ino)?;
        let mut data = alloc::vec![0u8; meta.size as usize];
        self.read(ino, 0, &mut data)?;
        Ok(data)
    }

    fn write_file_path(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let path = path.trim_matches('/');
        let (parent_path, file_name) = match path.rfind('/') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => ("", path),
        };
        let parent_ino = self.resolve_path(parent_path)?;
        let ino = match self.lookup(parent_ino, file_name) {
            Ok(existing) => existing,
            Err(VfsError::NotFound) => self.create(parent_ino, file_name, 0o644)?,
            Err(e) => return Err(e),
        };
        self.truncate(ino, 0)?;
        self.write(ino, 0, data)?;
        Ok(())
    }

    fn remove_file_path(&self, path: &str) -> Result<(), VfsError> {
        let path = path.trim_matches('/');
        let (parent_path, file_name) = match path.rfind('/') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => ("", path),
        };
        let parent_ino = self.resolve_path(parent_path)?;
        let ino = self.lookup(parent_ino, file_name)?;
        let meta = self.read_inode(ino)?;
        if (meta.mode & 0o170000) == 0o040000 {
            self.rmdir(parent_ino, file_name)
        } else {
            self.unlink(parent_ino, file_name)
        }
    }

    fn rename_path(&self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        let old_path = old_path.trim_matches('/');
        let (old_parent_path, old_name) = match old_path.rfind('/') {
            Some(idx) => (&old_path[..idx], &old_path[idx + 1..]),
            None => ("", old_path),
        };
        let new_path = new_path.trim_matches('/');
        let (new_parent_path, new_name) = match new_path.rfind('/') {
            Some(idx) => (&new_path[..idx], &new_path[idx + 1..]),
            None => ("", new_path),
        };
        let old_parent_ino = self.resolve_path(old_parent_path)?;
        let new_parent_ino = self.resolve_path(new_parent_path)?;
        self.rename(old_parent_ino, old_name, new_parent_ino, new_name)
    }

    fn create_dir_path(&self, path: &str) -> Result<(), VfsError> {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return Ok(());
        }
        let mut curr = self.root_inode();
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            curr = match self.lookup(curr, component) {
                Ok(ino) => ino,
                Err(VfsError::NotFound) => self.mkdir(curr, component, 0o755)?,
                Err(e) => return Err(e),
            };
        }
        Ok(())
    }

    fn list_dir_path(&self, path: &str) -> Result<Vec<String>, VfsError> {
        let ino = self.resolve_path(path)?;
        self.read_dir(ino)
    }

    fn file_info_path(&self, path: &str) -> Result<FileInfo, VfsError> {
        let ino = self.resolve_path(path)?;
        let meta = self.read_inode(ino)?;
        let length = meta.size as usize;
        Ok(FileInfo {
            length,
            allocated_sectors: if length == 0 { 0 } else { ((length + 511) / 512) as u32 },
            is_directory: (meta.mode & 0o170000) == 0o040000,
            uid: meta.uid,
            gid: meta.gid,
            mode: meta.mode as u16,
        })
    }

    fn read_at_path(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let ino = self.resolve_path(path)?;
        self.read(ino, offset, buf)
    }

    fn write_at_path(&self, path: &str, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        let ino = self.resolve_path(path)?;
        self.write(ino, offset, buf)
    }

    fn truncate_path(&self, path: &str, size: u64) -> Result<(), VfsError> {
        let ino = self.resolve_path(path)?;
        self.truncate(ino, size)
    }
}

pub fn normalize_mount_target(target: &str) -> String {
    let t = target.trim();
    if t.is_empty() || t == "/" {
        return String::from("/");
    }
    let mut s = String::new();
    if !t.starts_with('/') {
        s.push('/');
    }
    s.push_str(t.trim_end_matches('/'));
    s
}

pub struct MountPoint {
    pub prefix: String,
    pub fs: Arc<dyn Filesystem>,
    pub flags: u32,
}

pub struct MountTable {
    mounts: Vec<MountPoint>,
}

impl MountTable {
    pub const fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    pub fn mount(&mut self, target: &str, fs: Arc<dyn Filesystem>, flags: u32) -> Result<(), VfsError> {
        let norm_target = normalize_mount_target(target);
        if let Some(pos) = self.mounts.iter().position(|m| m.prefix == norm_target) {
            if (flags & mount_flags::MS_REMOUNT) != 0 || norm_target == "/" {
                self.mounts[pos] = MountPoint {
                    prefix: norm_target,
                    fs,
                    flags,
                };
                return Ok(());
            } else {
                return Err(VfsError::AlreadyExists);
            }
        }
        self.mounts.push(MountPoint {
            prefix: norm_target,
            fs,
            flags,
        });
        self.mounts.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
        Ok(())
    }

    pub fn umount(&mut self, target: &str, _flags: u32) -> Result<(), VfsError> {
        let norm_target = normalize_mount_target(target);
        if norm_target == "/" {
            return Err(VfsError::InvalidPath);
        }
        if let Some(pos) = self.mounts.iter().position(|m| m.prefix == norm_target) {
            self.mounts.remove(pos);
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }

    pub fn resolve<'a>(&'a self, path: &'a str) -> Result<(&'a MountPoint, &'a str), VfsError> {
        let norm_path = if path.is_empty() { "/" } else { path };
        for mount in &self.mounts {
            if mount.prefix == "/" {
                continue;
            }
            if norm_path == mount.prefix {
                return Ok((mount, ""));
            }
            if let Some(rest) = norm_path.strip_prefix(&mount.prefix) {
                if rest.starts_with('/') {
                    return Ok((mount, rest.trim_start_matches('/')));
                }
            }
        }
        for mount in &self.mounts {
            if mount.prefix == "/" {
                let rel = norm_path.trim_start_matches('/');
                return Ok((mount, rel));
            }
        }
        Err(VfsError::NotMounted)
    }

    pub fn find_mount(&self, target: &str) -> Option<&MountPoint> {
        let norm = normalize_mount_target(target);
        self.mounts.iter().find(|m| m.prefix == norm)
    }

    pub fn child_mount_names(&self, parent_dir: &str) -> Vec<String> {
        let parent = normalize_mount_target(parent_dir);
        let prefix = if parent == "/" {
            String::from("/")
        } else {
            let mut s = parent.clone();
            s.push('/');
            s
        };
        let mut children = Vec::new();
        for mount in &self.mounts {
            if mount.prefix == "/" {
                continue;
            }
            if let Some(rest) = mount.prefix.strip_prefix(&prefix) {
                let name = rest.split('/').next().unwrap_or("");
                if !name.is_empty() && !children.iter().any(|c| c == name) {
                    children.push(String::from(name));
                }
            }
        }
        children
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub length: usize,
    pub allocated_sectors: u32,
    pub is_directory: bool,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
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
            uid: 0,
            gid: 0,
            mode: if record.kind == EntryKind::Directory {
                0o040755
            } else {
                0o100644
            },
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

pub struct VantaFsAdapter {
    inner: Mutex<Vfs<RootDevice>>,
}

impl VantaFsAdapter {
    pub fn new(inner: Vfs<RootDevice>) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl Filesystem for VantaFsAdapter {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, _parent: u64, _name: &str) -> Result<u64, VfsError> {
        Ok(1)
    }

    fn read_inode(&self, _ino: u64) -> Result<InodeMetadata, VfsError> {
        Ok(InodeMetadata {
            ino: 1,
            size: 0,
            mode: 0o040755,
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        })
    }

    fn create(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Ok(1)
    }

    fn mkdir(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Ok(1)
    }

    fn unlink(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Ok(())
    }

    fn rmdir(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Ok(())
    }

    fn symlink(&self, _parent: u64, _name: &str, _target: &str) -> Result<u64, VfsError> {
        Err(VfsError::InvalidPath)
    }

    fn readlink(&self, _ino: u64) -> Result<String, VfsError> {
        Err(VfsError::NotFound)
    }

    fn rename(&self, _old_parent: u64, _old_name: &str, _new_parent: u64, _new_name: &str) -> Result<(), VfsError> {
        Ok(())
    }

    fn read(&self, _ino: u64, _offset: u64, _buf: &mut [u8]) -> Result<usize, VfsError> {
        Ok(0)
    }

    fn write(&self, _ino: u64, _offset: u64, _buf: &[u8]) -> Result<usize, VfsError> {
        Ok(0)
    }

    fn truncate(&self, _ino: u64, _size: u64) -> Result<(), VfsError> {
        Ok(())
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x56414e54,
            f_bsize: 512,
            f_blocks: 1000,
            f_bfree: 500,
            f_bavail: 500,
            f_files: 100,
            f_ffree: 50,
        })
    }

    fn sync(&self) -> Result<(), VfsError> {
        Ok(())
    }

    fn read_dir(&self, _ino: u64) -> Result<Vec<String>, VfsError> {
        self.inner.lock().list()
    }

    fn read_file_path(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        self.inner.lock().read(path)
    }

    fn write_file_path(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.inner.lock().write(path, data)
    }

    fn remove_file_path(&self, path: &str) -> Result<(), VfsError> {
        self.inner.lock().remove(path)
    }

    fn rename_path(&self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        self.inner.lock().rename(old_path, new_path)
    }

    fn create_dir_path(&self, path: &str) -> Result<(), VfsError> {
        self.inner.lock().create_dir(path)
    }

    fn list_dir_path(&self, path: &str) -> Result<Vec<String>, VfsError> {
        let prefix = if path == "/" || path.is_empty() {
            String::from("/")
        } else {
            let mut p = String::from("/");
            p.push_str(path.trim_matches('/'));
            p.push('/');
            p
        };
        let mut names = Vec::new();
        for entry in self.inner.lock().list()? {
            if let Some(name) = entry.strip_prefix(&prefix) {
                if !name.is_empty() && !name.contains('/') {
                    names.push(name.trim_end_matches('/').into());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn file_info_path(&self, path: &str) -> Result<FileInfo, VfsError> {
        self.inner.lock().info(path)
    }

    fn read_at_path(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let full = self.inner.lock().read(path)?;
        let start = (offset as usize).min(full.len());
        let end = start.saturating_add(buf.len()).min(full.len());
        let count = end - start;
        buf[..count].copy_from_slice(&full[start..end]);
        Ok(count)
    }

    fn write_at_path(&self, path: &str, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        let mut inner = self.inner.lock();
        let mut full = inner.read(path).unwrap_or_default();
        let start = offset as usize;
        let end = start.saturating_add(buf.len());
        if end > full.len() {
            full.resize(end, 0);
        }
        full[start..end].copy_from_slice(buf);
        inner.write(path, &full)?;
        Ok(buf.len())
    }

    fn truncate_path(&self, path: &str, size: u64) -> Result<(), VfsError> {
        let mut inner = self.inner.lock();
        let mut full = inner.read(path).unwrap_or_default();
        full.resize(size as usize, 0);
        inner.write(path, &full)
    }
}

fn ensure_absolute(path: &str) -> String {
    let mut s = String::from("/");
    s.push_str(path.trim_start_matches('/'));
    s
}

pub struct RedoxFsAdapter {
    backend: Mutex<Option<RedoxFsBackend<RootDevice>>>,
}

impl RedoxFsAdapter {
    pub fn new(backend: RedoxFsBackend<RootDevice>) -> Self {
        Self {
            backend: Mutex::new(Some(backend)),
        }
    }

    pub fn read_raw_sector(&self, sector: u64, buffer: &mut [u8; 512]) -> Result<(), StorageError> {
        let mut guard = self.backend.lock();
        if let Some(backend) = guard.as_mut() {
            backend.read_raw_sector(sector, buffer).map_err(|_| StorageError::IoFailed)
        } else {
            Err(StorageError::DeviceUnavailable)
        }
    }

    pub fn write_raw_sector(&self, sector: u64, buffer: &[u8; 512]) -> Result<(), StorageError> {
        let mut guard = self.backend.lock();
        if let Some(backend) = guard.as_mut() {
            backend.write_raw_sector(sector, buffer).map_err(|_| StorageError::IoFailed)
        } else {
            Err(StorageError::DeviceUnavailable)
        }
    }
}

impl Filesystem for RedoxFsAdapter {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let node = tx.find_node(redoxfs::TreePtr::new(parent as u32), name)?;
            Ok(node.ptr().id() as u64)
        }).map_err(|_| VfsError::NotFound)
    }

    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let node: redoxfs::TreeData<redoxfs::Node> = tx.read_tree(redoxfs::TreePtr::new(ino as u32))?;
            Ok(InodeMetadata {
                ino,
                size: node.data().size(),
                mode: node.data().mode() as u32,
                uid: node.data().uid(),
                gid: node.data().gid(),
                atime: 0,
                mtime: node.data().mtime().0,
                ctime: node.data().ctime().0,
            })
        }).map_err(|_| VfsError::NotFound)
    }

    fn create(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let node = tx.create_node(
                redoxfs::TreePtr::new(parent as u32),
                name,
                redoxfs::Node::MODE_FILE | (mode as u16 & 0o777),
                0,
                0,
            )?;
            Ok(node.ptr().id() as u64)
        }).map_err(|_| VfsError::NoSpace)
    }

    fn mkdir(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let node = tx.create_node(
                redoxfs::TreePtr::new(parent as u32),
                name,
                redoxfs::Node::MODE_DIR | (mode as u16 & 0o777),
                0,
                0,
            )?;
            Ok(node.ptr().id() as u64)
        }).map_err(|_| VfsError::NoSpace)
    }

    fn unlink(&self, parent: u64, name: &str) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let parent_ptr = redoxfs::TreePtr::new(parent as u32);
            let node = tx.find_node(parent_ptr, name)?;
            let mode = if node.data().is_dir() {
                redoxfs::Node::MODE_DIR
            } else {
                redoxfs::Node::MODE_FILE
            };
            tx.remove_node(parent_ptr, name, mode)?;
            Ok(())
        }).map_err(|_| VfsError::NotFound)
    }

    fn rmdir(&self, parent: u64, name: &str) -> Result<(), VfsError> {
        self.unlink(parent, name)
    }

    fn symlink(&self, _parent: u64, _name: &str, _target: &str) -> Result<u64, VfsError> {
        Err(VfsError::RedoxFs)
    }

    fn readlink(&self, _ino: u64) -> Result<String, VfsError> {
        Err(VfsError::NotFound)
    }

    fn rename(&self, old_parent: u64, old_name: &str, new_parent: u64, new_name: &str) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            tx.rename_node_no_replace(
                redoxfs::TreePtr::new(old_parent as u32),
                old_name,
                redoxfs::TreePtr::new(new_parent as u32),
                new_name,
            )
        }).map_err(|_| VfsError::RedoxFs)
    }

    fn read(&self, ino: u64, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            tx.read_node(redoxfs::TreePtr::new(ino as u32), offset, buf, 0, 0)
        }).map_err(|_| VfsError::Storage(StorageError::IoFailed))
    }

    fn write(&self, ino: u64, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            tx.write_node(redoxfs::TreePtr::new(ino as u32), offset, buf, 0, 0)
        }).map_err(|_| VfsError::Storage(StorageError::IoFailed))
    }

    fn truncate(&self, ino: u64, size: u64) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            tx.truncate_node(redoxfs::TreePtr::new(ino as u32), size, 0, 0)
        }).map_err(|_| VfsError::RedoxFs)
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x56414e54,
            f_bsize: 4096,
            f_blocks: 262144,
            f_bfree: 200000,
            f_bavail: 200000,
            f_files: 65536,
            f_ffree: 65000,
        })
    }

    fn sync(&self) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            tx.sync(true)?;
            Ok(())
        }).map_err(|_| VfsError::Storage(StorageError::IoFailed))
    }

    fn read_dir(&self, ino: u64) -> Result<Vec<String>, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.filesystem.tx(|tx| {
            let mut children = Vec::new();
            tx.child_nodes(redoxfs::TreePtr::new(ino as u32), &mut children)?;
            let mut names = children
                .iter()
                .filter_map(|entry| entry.name())
                .map(String::from)
                .collect::<Vec<_>>();
            names.sort();
            Ok(names)
        }).map_err(|_| VfsError::NotFound)
    }

    fn read_file_path(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.read_file(&ensure_absolute(path)).map_err(|_| VfsError::RedoxFs)
    }

    fn write_file_path(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.write_file(&ensure_absolute(path), data).map_err(|_| VfsError::RedoxFs)
    }

    fn remove_file_path(&self, path: &str) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.remove_file(&ensure_absolute(path)).map_err(|_| VfsError::RedoxFs)
    }

    fn rename_path(&self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.rename(&ensure_absolute(old_path), &ensure_absolute(new_path)).map_err(|_| VfsError::RedoxFs)
    }

    fn create_dir_path(&self, path: &str) -> Result<(), VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.create_dir_all(&ensure_absolute(path)).map_err(|_| VfsError::RedoxFs)
    }

    fn list_dir_path(&self, path: &str) -> Result<Vec<String>, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        backend.list_dir(&ensure_absolute(path)).map_err(|_| VfsError::RedoxFs)
    }

    fn file_info_path(&self, path: &str) -> Result<FileInfo, VfsError> {
        let mut guard = self.backend.lock();
        let backend = guard.as_mut().ok_or(VfsError::NotMounted)?;
        let info = backend.file_info(&ensure_absolute(path)).map_err(|_| VfsError::RedoxFs)?;
        let length: usize = info.length.try_into().map_err(|_| VfsError::FileTooLarge)?;
        Ok(FileInfo {
            length,
            allocated_sectors: if length == 0 { 0 } else { ((length + 511) / 512) as u32 },
            is_directory: info.is_directory,
            uid: info.uid,
            gid: info.gid,
            mode: info.mode,
        })
    }
}

pub fn initialize_root(sectors: u64) -> Result<(), VfsError> {
    let disk = RamDisk::new(sectors).map_err(VfsError::Storage)?;
    let filesystem = VantaFs::format(RootDevice::Ram(disk))?;
    let mut vfs = Vfs::new();
    vfs.mount_root(filesystem)?;
    let adapter = Arc::new(VantaFsAdapter::new(vfs));
    *ROOT.lock() = Some(adapter.clone());
    let mut table = MOUNT_TABLE.lock();
    table.mount("/", adapter, 0)?;

    let tmpfs = Arc::new(crate::tmpfs::TmpFs::new());
    table.mount("/tmp", tmpfs, 0)?;
    Ok(())
}

pub fn mount_virtio_root(device: VirtioBlock) -> Result<bool, VfsError> {
    let (filesystem, existed) = VantaFs::mount_or_format(RootDevice::Virtio(device))?;
    let mut vfs = Vfs::new();
    vfs.mount_root(filesystem)?;
    let adapter = Arc::new(VantaFsAdapter::new(vfs));
    *ROOT.lock() = Some(adapter.clone());
    MOUNT_TABLE.lock().mount("/", adapter, mount_flags::MS_REMOUNT)?;
    Ok(existed)
}

pub fn mount_virtio_redox_root(
    device: VirtioBlock,
    partition: RootPartition,
) -> Result<(), VfsError> {
    let backend = RedoxFsBackend::open(RootDevice::Virtio(device), partition)
        .map_err(|_| VfsError::RedoxFs)?;
    let adapter = Arc::new(RedoxFsAdapter::new(backend));
    *REDOX_ROOT.lock() = Some(adapter.clone());
    MOUNT_TABLE.lock().mount("/", adapter, mount_flags::MS_REMOUNT)?;
    Ok(())
}

pub fn remount_root() -> Result<(), VfsError> {
    let redox_guard = REDOX_ROOT.lock();
    if let Some(adapter) = redox_guard.as_ref() {
        let mut backend_guard = adapter.backend.lock();
        if let Some(backend) = backend_guard.take() {
            let device = backend.into_inner();
            let partition = match &device {
                RootDevice::Ram(_) => return Err(VfsError::RedoxFs),
                RootDevice::Virtio(_) => {
                    crate::storage::discover_vanta_root(&device).map_err(VfsError::Storage)?
                }
            };
            let new_backend = RedoxFsBackend::open(device, partition).map_err(|_| VfsError::RedoxFs)?;
            *backend_guard = Some(new_backend);
            return Ok(());
        }
    }
    drop(redox_guard);
    let root_guard = ROOT.lock();
    if let Some(adapter) = root_guard.as_ref() {
        let mut vfs = adapter.inner.lock();
        let filesystem = vfs.unmount_root()?;
        let remounted = VantaFs::mount(filesystem.into_device())?;
        vfs.mount_root(remounted)?;
        return Ok(());
    }
    Err(VfsError::NotMounted)
}

pub fn read_block_sector(sector: u64, buffer: &mut [u8; 512]) -> Result<(), StorageError> {
    if let Some(adapter) = REDOX_ROOT.lock().as_ref() {
        adapter.read_raw_sector(sector, buffer)
    } else {
        Err(StorageError::DeviceUnavailable)
    }
}

pub fn write_block_sector(sector: u64, buffer: &[u8; 512]) -> Result<(), StorageError> {
    if let Some(adapter) = REDOX_ROOT.lock().as_ref() {
        adapter.write_raw_sector(sector, buffer)
    } else {
        Err(StorageError::DeviceUnavailable)
    }
}

pub fn mount_filesystem(target: &str, fs: Arc<dyn Filesystem>, flags: u32) -> Result<(), VfsError> {
    MOUNT_TABLE.lock().mount(target, fs, flags)
}

pub fn unmount_filesystem(target: &str, flags: u32) -> Result<(), VfsError> {
    MOUNT_TABLE.lock().umount(target, flags)
}

pub fn is_writable_mount(path: &str) -> bool {
    let table = MOUNT_TABLE.lock();
    if let Ok((mount, _)) = table.resolve(path) {
        (mount.flags & mount_flags::MS_RDONLY) == 0
    } else {
        false
    }
}

pub fn can_user_mutate(path: &str, credentials: &Credentials) -> bool {
    if credentials.is_root() {
        return is_writable_mount(path);
    }
    let table = MOUNT_TABLE.lock();
    let Ok((mount, rel)) = table.resolve(path) else {
        return false;
    };
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return false;
    }

    // World-writable mount root (e.g. /tmp, /mnt/ram tmpfs)
    if let Ok(root_meta) = mount.fs.read_inode(mount.fs.root_inode()) {
        if (root_meta.mode & 0o002) != 0 {
            return true;
        }
    }

    // Home directory of user
    if path == "/home/vanta" || path.starts_with("/home/vanta/") {
        return true;
    }

    // Parent directory permissions
    let parent_path = match rel.rfind('/') {
        Some(idx) => &rel[..idx],
        None => "",
    };
    if let Ok(parent_info) = if parent_path.is_empty() {
        mount.fs.read_inode(mount.fs.root_inode()).map(|m| FileInfo {
            length: m.size as usize,
            allocated_sectors: 0,
            is_directory: true,
            uid: m.uid,
            gid: m.gid,
            mode: m.mode as u16,
        })
    } else {
        mount.fs.file_info_path(parent_path)
    } {
        if parent_info.uid == credentials.uid && (parent_info.mode & 0o200) != 0 {
            return true;
        }
        if parent_info.gid == credentials.gid && (parent_info.mode & 0o020) != 0 {
            return true;
        }
        if (parent_info.mode & 0o002) != 0 {
            return true;
        }
    }

    // File's own permissions if it exists
    if let Ok(info) = mount.fs.file_info_path(rel) {
        if info.uid == credentials.uid && (info.mode & 0o200) != 0 {
            return true;
        }
        if info.gid == credentials.gid && (info.mode & 0o020) != 0 {
            return true;
        }
        if (info.mode & 0o002) != 0 {
            return true;
        }
    }

    false
}

pub fn read_root(path: &str) -> Result<Vec<u8>, VfsError> {
    read_root_as(path, &Credentials::root())
}

pub fn read_root_as(path: &str, _credentials: &Credentials) -> Result<Vec<u8>, VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    mount.fs.read_file_path(rel)
}

pub fn write_root(path: &str, data: &[u8]) -> Result<(), VfsError> {
    write_root_as(path, data, &Credentials::root())
}

pub fn write_root_as(path: &str, data: &[u8], _credentials: &Credentials) -> Result<(), VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    mount.fs.write_file_path(rel, data)
}

pub fn read_root_at(path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
    read_root_at_as(path, offset, buf, &Credentials::root())
}

pub fn read_root_at_as(path: &str, offset: u64, buf: &mut [u8], _credentials: &Credentials) -> Result<usize, VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    mount.fs.read_at_path(rel, offset, buf)
}

pub fn write_root_at(path: &str, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
    write_root_at_as(path, offset, buf, &Credentials::root())
}

pub fn write_root_at_as(path: &str, offset: u64, buf: &[u8], _credentials: &Credentials) -> Result<usize, VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    mount.fs.write_at_path(rel, offset, buf)
}

pub fn truncate_root(path: &str, size: u64) -> Result<(), VfsError> {
    truncate_root_as(path, size, &Credentials::root())
}

pub fn truncate_root_as(path: &str, size: u64, _credentials: &Credentials) -> Result<(), VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    mount.fs.truncate_path(rel, size)
}

pub fn open_path(
    path: &str,
    writable: bool,
    append: bool,
) -> Result<(Arc<dyn Filesystem>, u64, usize), VfsError> {
    let table = MOUNT_TABLE.lock();
    let (mount, rel) = table.resolve(path)?;
    if writable && (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    let ino = mount.fs.resolve_path(rel)?;
    let meta = mount.fs.read_inode(ino)?;
    if (meta.mode & 0o170000) == 0o040000 {
        return Err(VfsError::IsDirectory);
    }
    mount.fs.open_inode(ino);
    let initial_offset = if append { meta.size as usize } else { 0 };
    Ok((mount.fs.clone(), ino, initial_offset))
}

pub fn list_root() -> Result<Vec<String>, VfsError> {
    list_dir_root("/")
}

pub fn list_dir_root(path: &str) -> Result<Vec<String>, VfsError> {
    list_dir_root_as(path, &Credentials::root())
}

pub fn list_dir_root_as(path: &str, _credentials: &Credentials) -> Result<Vec<String>, VfsError> {
    let table = MOUNT_TABLE.lock();
    let norm = normalize_mount_target(path);
    let mut names = if let Some(mount) = table.find_mount(&norm) {
        mount.fs.list_dir_path("")?
    } else {
        let (mount, rel) = table.resolve(path)?;
        mount.fs.list_dir_path(rel)?
    };
    for child in table.child_mount_names(path) {
        if !names.contains(&child) {
            names.push(child);
        }
    }
    names.sort();
    Ok(names)
}

pub fn remove_root(path: &str) -> Result<(), VfsError> {
    remove_root_as(path, &Credentials::root())
}

pub fn remove_root_as(path: &str, _credentials: &Credentials) -> Result<(), VfsError> {
    let table = MOUNT_TABLE.lock();
    let norm = normalize_mount_target(path);
    if table.find_mount(&norm).is_some() {
        return Err(VfsError::IsDirectory);
    }
    let (mount, rel) = table.resolve(path)?;
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    mount.fs.remove_file_path(rel)
}

pub fn rename_root(old_path: &str, new_path: &str) -> Result<(), VfsError> {
    rename_root_as(old_path, new_path, &Credentials::root())
}

pub fn rename_root_as(
    old_path: &str,
    new_path: &str,
    _credentials: &Credentials,
) -> Result<(), VfsError> {
    let table = MOUNT_TABLE.lock();
    let (old_mount, old_rel) = table.resolve(old_path)?;
    let (new_mount, new_rel) = table.resolve(new_path)?;
    if !Arc::ptr_eq(&old_mount.fs, &new_mount.fs) {
        return Err(VfsError::InvalidPath);
    }
    if (new_mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    old_mount.fs.rename_path(old_rel, new_rel)
}

pub fn file_info_root(path: &str) -> Result<FileInfo, VfsError> {
    file_info_root_as(path, &Credentials::root())
}

pub fn file_info_root_as(path: &str, _credentials: &Credentials) -> Result<FileInfo, VfsError> {
    let table = MOUNT_TABLE.lock();
    let norm = normalize_mount_target(path);
    if let Some(mount) = table.find_mount(&norm) {
        if let Ok(meta) = mount.fs.read_inode(mount.fs.root_inode()) {
            return Ok(FileInfo {
                length: meta.size as usize,
                allocated_sectors: 0,
                is_directory: true,
                uid: meta.uid,
                gid: meta.gid,
                mode: meta.mode as u16,
            });
        }
    }
    let (mount, rel) = table.resolve(path)?;
    if rel.is_empty() {
        let meta = mount.fs.read_inode(mount.fs.root_inode())?;
        return Ok(FileInfo {
            length: meta.size as usize,
            allocated_sectors: 0,
            is_directory: true,
            uid: meta.uid,
            gid: meta.gid,
            mode: meta.mode as u16,
        });
    }
    mount.fs.file_info_path(rel)
}

pub fn create_dir_root(path: &str) -> Result<(), VfsError> {
    create_dir_root_as(path, &Credentials::root())
}

pub fn create_dir_root_as(path: &str, _credentials: &Credentials) -> Result<(), VfsError> {
    let table = MOUNT_TABLE.lock();
    let norm = normalize_mount_target(path);
    if table.find_mount(&norm).is_some() {
        return Ok(());
    }
    let (mount, rel) = table.resolve(path)?;
    if (mount.flags & mount_flags::MS_RDONLY) != 0 {
        return Err(VfsError::ReadOnlyFilesystem);
    }
    if rel.is_empty() {
        return Ok(());
    }
    mount.fs.create_dir_path(rel)
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
