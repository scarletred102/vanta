//! Synthetic sysfs filesystem exposing device topology and network interfaces.

use alloc::string::String;
use alloc::vec::Vec;

use crate::vfs::{Filesystem, InodeMetadata, StatFs, VfsError};

pub struct SysFs;

impl SysFs {
    pub fn new() -> Self {
        Self
    }

    fn content_for_ino(&self, ino: u64) -> Option<Vec<u8>> {
        match ino {
            5 => Some(alloc::vec::Vec::from("up\n")),
            6 => Some(alloc::vec::Vec::from("52:54:00:12:34:56\n")),
            9 => Some(alloc::vec::Vec::from("1048576\n")),
            _ => None,
        }
    }
}

impl Filesystem for SysFs {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError> {
        match (parent, name) {
            (1, "." | "") => Ok(1),
            (1, "..") => Ok(1),
            (1, "class") => Ok(2),
            (1, "block") => Ok(7),

            (2, "." | "") => Ok(2),
            (2, "..") => Ok(1),
            (2, "net") => Ok(3),

            (3, "." | "") => Ok(3),
            (3, "..") => Ok(2),
            (3, "eth0") => Ok(4),

            (4, "." | "") => Ok(4),
            (4, "..") => Ok(3),
            (4, "operstate") => Ok(5),
            (4, "address") => Ok(6),

            (7, "." | "") => Ok(7),
            (7, "..") => Ok(1),
            (7, "vda") => Ok(8),

            (8, "." | "") => Ok(8),
            (8, "..") => Ok(7),
            (8, "size") => Ok(9),

            _ => Err(VfsError::NotFound),
        }
    }

    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError> {
        let (mode, size) = match ino {
            1..=4 | 7 | 8 => (0o040555, 0),
            5 | 6 | 9 => {
                let s = self.content_for_ino(ino).map(|c| c.len() as u64).unwrap_or(0);
                (0o100444, s)
            }
            _ => return Err(VfsError::NotFound),
        };
        Ok(InodeMetadata {
            ino,
            size,
            mode,
            uid: 0,
            gid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        })
    }

    fn create(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn mkdir(&self, _parent: u64, _name: &str, _mode: u32) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn unlink(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn rmdir(&self, _parent: u64, _name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn symlink(&self, _parent: u64, _name: &str, _target: &str) -> Result<u64, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn readlink(&self, _ino: u64) -> Result<String, VfsError> {
        Err(VfsError::InvalidPath)
    }

    fn rename(&self, _old_parent: u64, _old_name: &str, _new_parent: u64, _new_name: &str) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn read(&self, ino: u64, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let content = self.content_for_ino(ino).ok_or(VfsError::NotFound)?;
        let start = (offset as usize).min(content.len());
        let end = start.saturating_add(buf.len()).min(content.len());
        let n = end - start;
        buf[..n].copy_from_slice(&content[start..end]);
        Ok(n)
    }

    fn write(&self, _ino: u64, _offset: u64, _buf: &[u8]) -> Result<usize, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn truncate(&self, _ino: u64, _size: u64) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x62656572, // SYSFS_MAGIC
            f_bsize: 4096,
            f_blocks: 1024,
            f_bfree: 1024,
            f_bavail: 1024,
            f_files: 64,
            f_ffree: 50,
        })
    }

    fn sync(&self) -> Result<(), VfsError> {
        Ok(())
    }

    fn read_dir(&self, ino: u64) -> Result<Vec<String>, VfsError> {
        match ino {
            1 => Ok(alloc::vec![".".into(), "..".into(), "class".into(), "block".into()]),
            2 => Ok(alloc::vec![".".into(), "..".into(), "net".into()]),
            3 => Ok(alloc::vec![".".into(), "..".into(), "eth0".into()]),
            4 => Ok(alloc::vec![".".into(), "..".into(), "operstate".into(), "address".into()]),
            7 => Ok(alloc::vec![".".into(), "..".into(), "vda".into()]),
            8 => Ok(alloc::vec![".".into(), "..".into(), "size".into()]),
            _ => Err(VfsError::NotDirectory),
        }
    }

    fn is_page_cached(&self) -> bool {
        false
    }
}
