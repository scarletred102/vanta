//! Synthetic devfs filesystem exposing hardware and character device nodes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::vfs::{Filesystem, InodeMetadata, StatFs, VfsError};

pub struct DevFs;

impl DevFs {
    pub fn new() -> Self {
        Self
    }
}

impl Filesystem for DevFs {
    fn root_inode(&self) -> u64 {
        1
    }

    fn lookup(&self, parent: u64, name: &str) -> Result<u64, VfsError> {
        if parent == 1 {
            match name {
                "." | "" => Ok(1),
                ".." => Ok(1),
                "null" => Ok(2),
                "zero" => Ok(3),
                "urandom" => Ok(4),
                "random" => Ok(5),
                "console" => Ok(6),
                "tty" => Ok(7),
                "pts" => Ok(8),
                "ptmx" => Ok(9),
                "shm" => Ok(10),
                _ => Err(VfsError::NotFound),
            }
        } else if parent == 8 {
            match name {
                "." | "" => Ok(8),
                ".." => Ok(1),
                _ => {
                    if let Ok(id) = name.parse::<u32>() {
                        if crate::pty::has_pty(id) {
                            Ok(100 + id as u64)
                        } else {
                            Err(VfsError::NotFound)
                        }
                    } else {
                        Err(VfsError::NotFound)
                    }
                }
            }
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn read_inode(&self, ino: u64) -> Result<InodeMetadata, VfsError> {
        let (mode, size) = match ino {
            1 => (0o040755, 0),
            2..=7 | 9 => (0o020666, 0), // Character device, rw-rw-rw-
            8 => (0o040755, 0),
            10 => (0o040777, 0),
            100..=1000000 => {
                let pty_id = (ino - 100) as u32;
                if crate::pty::has_pty(pty_id) {
                    (0o020666, 0)
                } else {
                    return Err(VfsError::NotFound);
                }
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

    fn read(&self, ino: u64, _offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        match ino {
            2 => Ok(0), // /dev/null: EOF
            3 => {      // /dev/zero: zeros
                buf.fill(0);
                Ok(buf.len())
            }
            4 | 5 => {  // /dev/urandom, /dev/random
                let _ = crate::random::get_random_bytes(buf);
                Ok(buf.len())
            }
            6 | 7 => Ok(0), // /dev/console, /dev/tty
            _ => Err(VfsError::InvalidPath),
        }
    }

    fn write(&self, ino: u64, _offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        match ino {
            2 | 3 | 4 | 5 => Ok(buf.len()), // /dev/null, /dev/zero, /dev/urandom: discard/consume
            6 | 7 => {
                if let Ok(s) = core::str::from_utf8(buf) {
                    crate::serial_print!("{}", s);
                }
                Ok(buf.len())
            }
            _ => Err(VfsError::ReadOnlyFilesystem),
        }
    }

    fn truncate(&self, _ino: u64, _size: u64) -> Result<(), VfsError> {
        Ok(())
    }

    fn statfs(&self) -> Result<StatFs, VfsError> {
        Ok(StatFs {
            f_type: 0x01021994, // TMPFS_MAGIC
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
        if ino == 1 {
            Ok(alloc::vec![
                ".".into(),
                "..".into(),
                "null".into(),
                "zero".into(),
                "urandom".into(),
                "random".into(),
                "console".into(),
                "tty".into(),
                "pts".into(),
                "ptmx".into(),
                "shm".into(),
            ])
        } else if ino == 8 {
            let mut entries = alloc::vec![".".into(), "..".into()];
            for id in crate::pty::list_ptys() {
                entries.push(alloc::format!("{}", id));
            }
            Ok(entries)
        } else {
            Err(VfsError::NotDirectory)
        }
    }

    fn is_page_cached(&self) -> bool {
        false
    }
}
