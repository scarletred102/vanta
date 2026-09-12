//! In-memory tmpfs implementation backed by physical buddy allocator frames.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use crate::memory::{alloc_frame, free_frame, PhysFrame};
use crate::paging::phys_to_virt;
use crate::vfs::{Filesystem, InodeMetadata, StatFs, VfsError};

enum TmpNodeKind {
    File {
        pages: Vec<PhysFrame>,
        size: usize,
    },
    Directory {
        entries: BTreeMap<String, u64>,
    },
    Symlink {
        target: String,
    },
}

struct TmpNode {
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    atime: u64,
    mtime: u64,
    ctime: u64,
    nlink: u32,
    open_count: u32,
    kind: TmpNodeKind,
}

impl Drop for TmpNode {
    fn drop(&mut self) {
        if let TmpNodeKind::File { ref mut pages, .. } = self.kind {
            for frame in pages.drain(..) {
                free_frame(frame);
            }
        }
    }
}

pub struct TmpFs {
    next_ino: AtomicU64,
    nodes: Mutex<BTreeMap<u64, TmpNode>>,
}

impl TmpFs {
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        let root = TmpNode {
            ino: 1,
            mode: 0o040777, // Directory with rwxrwxrwx
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            nlink: 2,
            open_count: 0,
            kind: TmpNodeKind::Directory {
                entries: BTreeMap::new(),
            },
        };
        nodes.insert(1, root);
        Self {
            next_ino: AtomicU64::new(2),
            nodes: Mutex::new(nodes),
        }
    }
}

impl Filesystem for TmpFs {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError> {
        if name == "." || name.is_empty() {
            return Ok(parent);
        }
        let nodes = self.nodes.lock();
        let node = nodes.get(&parent).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref entries } = node.kind else {
            return Err(VfsError::NotDirectory);
        };
        if name == ".." {
            return Ok(parent);
        }
        entries.get(name).copied().ok_or(VfsError::NotFound)
    }

    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError> {
        let nodes = self.nodes.lock();
        let node = nodes.get(&ino).ok_or(VfsError::NotFound)?;
        let size = match node.kind {
            TmpNodeKind::File { size, .. } => size as u64,
            TmpNodeKind::Directory { ref entries } => entries.len() as u64 * 64,
            TmpNodeKind::Symlink { ref target } => target.len() as u64,
        };
        Ok(InodeMetadata {
            ino: node.ino,
            size,
            mode: node.mode,
            uid: node.uid,
            gid: node.gid,
            atime: node.atime,
            mtime: node.mtime,
            ctime: node.ctime,
        })
    }

    fn create(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError> {
        let mut nodes = self.nodes.lock();
        let node = nodes.get_mut(&parent).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref mut entries } = node.kind else {
            return Err(VfsError::NotDirectory);
        };
        if entries.contains_key(name) {
            return Err(VfsError::AlreadyExists);
        }
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        let new_node = TmpNode {
            ino,
            mode: 0o100000 | (mode & 0o7777),
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            nlink: 1,
            open_count: 0,
            kind: TmpNodeKind::File {
                pages: Vec::new(),
                size: 0,
            },
        };
        entries.insert(String::from(name), ino);
        nodes.insert(ino, new_node);
        Ok(ino)
    }

    fn mkdir(&self, parent: u64, name: &str, mode: u32) -> Result<u64, VfsError> {
        let mut nodes = self.nodes.lock();
        let node = nodes.get_mut(&parent).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref mut entries } = node.kind else {
            return Err(VfsError::NotDirectory);
        };
        if entries.contains_key(name) {
            return Err(VfsError::AlreadyExists);
        }
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        let new_node = TmpNode {
            ino,
            mode: 0o040000 | (mode & 0o7777),
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            nlink: 2,
            open_count: 0,
            kind: TmpNodeKind::Directory {
                entries: BTreeMap::new(),
            },
        };
        entries.insert(String::from(name), ino);
        nodes.insert(ino, new_node);
        Ok(ino)
    }

    fn unlink(&self, parent: u64, name: &str) -> Result<(), VfsError> {
        let mut nodes = self.nodes.lock();
        let target_ino = {
            let node = nodes.get(&parent).ok_or(VfsError::NotFound)?;
            let TmpNodeKind::Directory { ref entries } = node.kind else {
                return Err(VfsError::NotDirectory);
            };
            entries.get(name).copied().ok_or(VfsError::NotFound)?
        };
        let is_dir = {
            let target_node = nodes.get(&target_ino).ok_or(VfsError::NotFound)?;
            matches!(target_node.kind, TmpNodeKind::Directory { .. })
        };
        if is_dir {
            return Err(VfsError::IsDirectory);
        }
        let node = nodes.get_mut(&parent).ok_or(VfsError::NotFound)?;
        if let TmpNodeKind::Directory { ref mut entries } = node.kind {
            entries.remove(name);
        }
        let should_remove = if let Some(target_node) = nodes.get_mut(&target_ino) {
            target_node.nlink = target_node.nlink.saturating_sub(1);
            target_node.nlink == 0 && target_node.open_count == 0
        } else {
            false
        };
        if should_remove {
            nodes.remove(&target_ino);
        }
        Ok(())
    }

    fn rmdir(&self, parent: u64, name: &str) -> Result<(), VfsError> {
        let mut nodes = self.nodes.lock();
        let target_ino = {
            let node = nodes.get(&parent).ok_or(VfsError::NotFound)?;
            let TmpNodeKind::Directory { ref entries } = node.kind else {
                return Err(VfsError::NotDirectory);
            };
            entries.get(name).copied().ok_or(VfsError::NotFound)?
        };
        let is_empty_dir = {
            let target_node = nodes.get(&target_ino).ok_or(VfsError::NotFound)?;
            let TmpNodeKind::Directory { ref entries } = target_node.kind else {
                return Err(VfsError::NotDirectory);
            };
            if !entries.is_empty() {
                return Err(VfsError::NotEmpty);
            }
            true
        };
        if !is_empty_dir {
            return Err(VfsError::NotEmpty);
        }
        let node = nodes.get_mut(&parent).ok_or(VfsError::NotFound)?;
        if let TmpNodeKind::Directory { ref mut entries } = node.kind {
            entries.remove(name);
        }
        let should_remove = if let Some(target_node) = nodes.get_mut(&target_ino) {
            target_node.nlink = target_node.nlink.saturating_sub(1);
            target_node.nlink == 0 && target_node.open_count == 0
        } else {
            false
        };
        if should_remove {
            nodes.remove(&target_ino);
        }
        Ok(())
    }

    fn symlink(&self, parent: u64, name: &str, target: &str) -> Result<u64, VfsError> {
        let mut nodes = self.nodes.lock();
        let node = nodes.get_mut(&parent).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref mut entries } = node.kind else {
            return Err(VfsError::NotDirectory);
        };
        if entries.contains_key(name) {
            return Err(VfsError::AlreadyExists);
        }
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        let new_node = TmpNode {
            ino,
            mode: 0o120777,
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            nlink: 1,
            open_count: 0,
            kind: TmpNodeKind::Symlink {
                target: String::from(target),
            },
        };
        entries.insert(String::from(name), ino);
        nodes.insert(ino, new_node);
        Ok(ino)
    }

    fn open_inode(&self, ino: u64) {
        let mut nodes = self.nodes.lock();
        if let Some(node) = nodes.get_mut(&ino) {
            node.open_count = node.open_count.saturating_add(1);
        }
    }

    fn close_inode(&self, ino: u64) {
        let mut nodes = self.nodes.lock();
        let should_remove = if let Some(node) = nodes.get_mut(&ino) {
            node.open_count = node.open_count.saturating_sub(1);
            node.nlink == 0 && node.open_count == 0
        } else {
            false
        };
        if should_remove {
            nodes.remove(&ino);
        }
    }

    fn readlink(&self, ino: u64) -> Result<String, VfsError> {
        let nodes = self.nodes.lock();
        let node = nodes.get(&ino).ok_or(VfsError::NotFound)?;
        match node.kind {
            TmpNodeKind::Symlink { ref target } => Ok(target.clone()),
            _ => Err(VfsError::InvalidPath),
        }
    }

    fn rename(&self, old_parent: u64, old: &str, new_parent: u64, new: &str) -> Result<(), VfsError> {
        let mut nodes = self.nodes.lock();
        let target_ino = {
            let old_node = nodes.get_mut(&old_parent).ok_or(VfsError::NotFound)?;
            let TmpNodeKind::Directory { ref mut entries } = old_node.kind else {
                return Err(VfsError::NotDirectory);
            };
            entries.remove(old).ok_or(VfsError::NotFound)?
        };
        let new_node = nodes.get_mut(&new_parent).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref mut entries } = new_node.kind else {
            return Err(VfsError::NotDirectory);
        };
        entries.insert(String::from(new), target_ino);
        Ok(())
    }

    fn read(&self, ino: u64, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let nodes = self.nodes.lock();
        let node = nodes.get(&ino).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::File { ref pages, size } = node.kind else {
            return Err(VfsError::IsDirectory);
        };
        let offset = offset as usize;
        if offset >= size {
            return Ok(0);
        }
        let to_read = core::cmp::min(buf.len(), size - offset);
        let mut read_bytes = 0;
        while read_bytes < to_read {
            let curr_offset = offset + read_bytes;
            let page_idx = curr_offset / 4096;
            let page_off = curr_offset % 4096;
            let chunk_len = core::cmp::min(to_read - read_bytes, 4096 - page_off);
            let frame = pages[page_idx];
            let virt = phys_to_virt(frame.start_address())
                .ok_or(VfsError::Storage(crate::storage::StorageError::IoFailed))?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (virt as *const u8).add(page_off),
                    buf.as_mut_ptr().add(read_bytes),
                    chunk_len,
                );
            }
            read_bytes += chunk_len;
        }
        Ok(to_read)
    }

    fn write(&self, ino: u64, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        let mut nodes = self.nodes.lock();
        let node = nodes.get_mut(&ino).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::File {
            ref mut pages,
            ref mut size,
        } = node.kind
        else {
            return Err(VfsError::IsDirectory);
        };
        let offset = offset as usize;
        let end = offset.checked_add(buf.len()).ok_or(VfsError::FileTooLarge)?;
        let needed_pages = (end + 4095) / 4096;
        while pages.len() < needed_pages {
            let frame = alloc_frame().ok_or(VfsError::NoSpace)?;
            let virt = phys_to_virt(frame.start_address()).ok_or(VfsError::NoSpace)?;
            unsafe {
                core::ptr::write_bytes(virt as *mut u8, 0, 4096);
            }
            pages.push(frame);
        }
        let mut written = 0;
        while written < buf.len() {
            let curr_offset = offset + written;
            let page_idx = curr_offset / 4096;
            let page_off = curr_offset % 4096;
            let chunk_len = core::cmp::min(buf.len() - written, 4096 - page_off);
            let frame = pages[page_idx];
            let virt = phys_to_virt(frame.start_address())
                .ok_or(VfsError::Storage(crate::storage::StorageError::IoFailed))?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buf.as_ptr().add(written),
                    (virt as *mut u8).add(page_off),
                    chunk_len,
                );
            }
            written += chunk_len;
        }
        if end > *size {
            *size = end;
        }
        Ok(buf.len())
    }

    fn truncate(&self, ino: u64, size: u64) -> Result<(), VfsError> {
        let mut nodes = self.nodes.lock();
        let node = nodes.get_mut(&ino).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::File {
            ref mut pages,
            size: ref mut cur_size,
        } = node.kind
        else {
            return Err(VfsError::IsDirectory);
        };
        let size = size as usize;
        let needed_pages = (size + 4095) / 4096;
        if needed_pages < pages.len() {
            for frame in pages.drain(needed_pages..) {
                free_frame(frame);
            }
        }
        *cur_size = size;
        Ok(())
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x01021994, // TMPFS_MAGIC
            f_bsize: 4096,
            f_blocks: crate::memory::stats().tracked_frames as u64,
            f_bfree: crate::memory::free_frames_count() as u64,
            f_bavail: crate::memory::free_frames_count() as u64,
            f_files: 100000,
            f_ffree: 100000,
        })
    }

    fn sync(&self) -> Result<(), VfsError> {
        Ok(())
    }

    fn read_dir(&self, ino: u64) -> Result<Vec<String>, VfsError> {
        let nodes = self.nodes.lock();
        let node = nodes.get(&ino).ok_or(VfsError::NotFound)?;
        let TmpNodeKind::Directory { ref entries } = node.kind else {
            return Err(VfsError::NotDirectory);
        };
        Ok(entries.keys().cloned().collect())
    }
}
