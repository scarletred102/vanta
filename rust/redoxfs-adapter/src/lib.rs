#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use redoxfs::{Disk, FileSystem, Node, Transaction, TreePtr, BLOCK_SIZE};
use syscall::error::{Error, Result, EACCES, EINVAL, EIO, EISDIR, ENOENT, ENOTDIR};
use vanta_abi::Credentials;
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

/// Scheduler-independent RedoxFS root operations over a bounded sector device.
pub struct RedoxFsBackend<D: SectorIo> {
    filesystem: FileSystem<RedoxDisk<D>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendFileInfo {
    pub length: u64,
    pub is_directory: bool,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
}

impl<D: SectorIo> RedoxFsBackend<D> {
    pub fn format(device: D, partition: RootPartition) -> Result<Self> {
        let disk = RedoxDisk::new(device, partition).map_err(|_| Error::new(EIO))?;
        let filesystem = FileSystem::create(disk, None, 0, 0)?;
        Ok(Self { filesystem })
    }

    pub fn open(device: D, partition: RootPartition) -> Result<Self> {
        let disk = RedoxDisk::new(device, partition).map_err(|_| Error::new(EIO))?;
        let filesystem = FileSystem::open(disk, None, None, true)?;
        Ok(Self { filesystem })
    }

    pub fn into_inner(self) -> D {
        self.filesystem.disk.into_inner()
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>> {
        self.read_file_as(path, &Credentials::root())
    }

    pub fn read_file_as(&mut self, path: &str, credentials: &Credentials) -> Result<Vec<u8>> {
        let parts = path_parts(path)?;
        self.filesystem.tx(|tx| {
            let ptr = resolve_with_access(tx, &parts, credentials, Node::MODE_READ)?;
            let node = tx.read_tree(ptr)?;
            if node.data().is_dir() {
                return Err(Error::new(EISDIR));
            }
            let length: usize = node.data().size().try_into().map_err(|_| Error::new(EIO))?;
            let mut contents = alloc::vec![0; length];
            tx.read_node(ptr, 0, &mut contents, 0, 0)?;
            Ok(contents)
        })
    }

    pub fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<()> {
        self.write_file_as(path, contents, &Credentials::root())
    }

    pub fn write_file_as(
        &mut self,
        path: &str,
        contents: &[u8],
        credentials: &Credentials,
    ) -> Result<()> {
        let parts = path_parts(path)?;
        let (name, parent) = split_parent(&parts)?;
        self.filesystem.tx(|tx| {
            let parent =
                resolve_with_access(tx, parent, credentials, Node::MODE_WRITE | Node::MODE_EXEC)?;
            let ptr = match tx.find_node(parent, name) {
                Ok(node) => {
                    if node.data().is_dir() {
                        return Err(Error::new(EISDIR));
                    }
                    if !node
                        .data()
                        .permission(credentials.uid, credentials.gid, Node::MODE_WRITE)
                    {
                        return Err(Error::new(EACCES));
                    }
                    node.ptr()
                }
                Err(error) if error.errno == ENOENT => tx
                    .create_node_with_owner(
                        parent,
                        name,
                        Node::MODE_FILE | (0o666 & !credentials.umask),
                        credentials.uid.into(),
                        credentials.gid.into(),
                        0,
                        0,
                    )?
                    .ptr(),
                Err(error) => return Err(error),
            };
            tx.truncate_node(ptr, 0, 0, 0)?;
            tx.write_node(ptr, 0, contents, 0, 0)?;
            Ok(())
        })
    }

    pub fn file_info(&mut self, path: &str) -> Result<BackendFileInfo> {
        self.file_info_as(path, &Credentials::root())
    }

    pub fn file_info_as(
        &mut self,
        path: &str,
        credentials: &Credentials,
    ) -> Result<BackendFileInfo> {
        let parts = path_parts(path)?;
        self.filesystem.tx(|tx| {
            let ptr = resolve_with_access(tx, &parts, credentials, Node::MODE_READ)?;
            let node = tx.read_tree(ptr)?;
            Ok(BackendFileInfo {
                length: node.data().size(),
                is_directory: node.data().is_dir(),
                uid: node.data().uid(),
                gid: node.data().gid(),
                mode: node.data().mode(),
            })
        })
    }

    pub fn create_dir_all(&mut self, path: &str) -> Result<()> {
        self.create_dir_all_as(path, &Credentials::root())
    }

    pub fn create_dir_all_as(&mut self, path: &str, credentials: &Credentials) -> Result<()> {
        let parts = path_parts(path)?;
        self.filesystem.tx(|tx| {
            let mut current: TreePtr<Node> = TreePtr::root();
            for name in parts {
                let parent = tx.read_tree(current)?;
                current = match tx.find_node(current, name) {
                    Ok(node) if node.data().is_dir() => {
                        if !parent.data().permission(
                            credentials.uid,
                            credentials.gid,
                            Node::MODE_EXEC,
                        ) {
                            return Err(Error::new(EACCES));
                        }
                        node.ptr()
                    }
                    Ok(_) => return Err(Error::new(ENOTDIR)),
                    Err(error) if error.errno == ENOENT => {
                        if !parent.data().permission(
                            credentials.uid,
                            credentials.gid,
                            Node::MODE_WRITE | Node::MODE_EXEC,
                        ) {
                            return Err(Error::new(EACCES));
                        }
                        tx.create_node(
                            current,
                            name,
                            Node::MODE_DIR | (0o777 & !credentials.umask),
                            0,
                            0,
                        )?
                        .ptr()
                    }
                    Err(error) => return Err(error),
                };
            }
            Ok(())
        })
    }

    pub fn list_dir(&mut self, path: &str) -> Result<Vec<String>> {
        self.list_dir_as(path, &Credentials::root())
    }

    pub fn list_dir_as(&mut self, path: &str, credentials: &Credentials) -> Result<Vec<String>> {
        let parts = path_parts(path)?;
        self.filesystem.tx(|tx| {
            let ptr =
                resolve_with_access(tx, &parts, credentials, Node::MODE_READ | Node::MODE_EXEC)?;
            if !tx.read_tree(ptr)?.data().is_dir() {
                return Err(Error::new(ENOTDIR));
            }
            let mut children = Vec::new();
            tx.child_nodes(ptr, &mut children)?;
            let mut names = children
                .iter()
                .filter_map(|entry| entry.name())
                .map(String::from)
                .collect::<Vec<_>>();
            names.sort();
            Ok(names)
        })
    }

    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        self.rename_as(old_path, new_path, &Credentials::root())
    }

    pub fn rename_as(
        &mut self,
        old_path: &str,
        new_path: &str,
        credentials: &Credentials,
    ) -> Result<()> {
        let old_parts = path_parts(old_path)?;
        let new_parts = path_parts(new_path)?;
        let (old_name, old_parent) = split_parent(&old_parts)?;
        let (new_name, new_parent) = split_parent(&new_parts)?;
        self.filesystem.tx(|tx| {
            let old_parent = resolve_with_access(
                tx,
                old_parent,
                credentials,
                Node::MODE_WRITE | Node::MODE_EXEC,
            )?;
            let new_parent = resolve_with_access(
                tx,
                new_parent,
                credentials,
                Node::MODE_WRITE | Node::MODE_EXEC,
            )?;
            tx.rename_node_no_replace(old_parent, old_name, new_parent, new_name)
        })
    }

    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        self.remove_file_as(path, &Credentials::root())
    }

    pub fn remove_file_as(&mut self, path: &str, credentials: &Credentials) -> Result<()> {
        let parts = path_parts(path)?;
        let (name, parent) = split_parent(&parts)?;
        self.filesystem.tx(|tx| {
            let parent =
                resolve_with_access(tx, parent, credentials, Node::MODE_WRITE | Node::MODE_EXEC)?;
            let node = tx.find_node(parent, name)?;
            let mode = if node.data().is_dir() {
                Node::MODE_DIR
            } else {
                Node::MODE_FILE
            };
            tx.remove_node(parent, name, mode)?;
            Ok(())
        })
    }
}

fn path_parts(path: &str) -> Result<Vec<&str>> {
    if !path.starts_with('/') {
        return Err(Error::new(EINVAL));
    }
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.iter().any(|part| *part == "." || *part == "..") {
        return Err(Error::new(EINVAL));
    }
    Ok(parts)
}

fn split_parent<'a>(parts: &'a [&'a str]) -> Result<(&'a str, &'a [&'a str])> {
    let (name, parent) = parts.split_last().ok_or_else(|| Error::new(EINVAL))?;
    Ok((*name, parent))
}

fn resolve_with_access<D: SectorIo>(
    tx: &mut Transaction<RedoxDisk<D>>,
    parts: &[&str],
    credentials: &Credentials,
    operation: u16,
) -> Result<TreePtr<Node>> {
    let mut current: TreePtr<Node> = TreePtr::root();
    for (index, name) in parts.iter().enumerate() {
        let current_node = tx.read_tree(current)?;
        if !current_node
            .data()
            .permission(credentials.uid, credentials.gid, Node::MODE_EXEC)
        {
            return Err(Error::new(EACCES));
        }
        let node = tx.find_node(current, name)?;
        if index + 1 < parts.len() {
            if !node.data().is_dir() {
                return Err(Error::new(ENOTDIR));
            }
        } else if !node
            .data()
            .permission(credentials.uid, credentials.gid, operation)
        {
            return Err(Error::new(EACCES));
        }
        current = node.ptr();
    }
    if parts.is_empty() {
        let root = tx.read_tree(current)?;
        if !root
            .data()
            .permission(credentials.uid, credentials.gid, operation)
        {
            return Err(Error::new(EACCES));
        }
    }
    Ok(current)
}
