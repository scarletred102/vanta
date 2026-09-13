//! Unified Page Cache for file-backed 4 KiB physical page frames.
//!
//! Stores physical memory frames indexed by `(mount_id, ino, page_index)`.
//! Supports write-back caching, dirty page tracking, contiguous batched flush,
//! file truncation, cache invalidation, and zero-copy `MAP_SHARED` memory mappings.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;

use crate::memory::{alloc_frame, free_frame, PhysFrame, PAGE_SIZE};
use crate::paging::phys_to_virt;
use crate::vfs::{Filesystem, VfsError};

pub const PAGE_CACHE_MAX_PAGES: usize = 16_384; // 64 MiB max in-memory page cache

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PageKey {
    pub mount_id: usize,
    pub ino: u64,
    pub page_index: u64,
}

pub struct PageEntry {
    pub frame: PhysFrame,
    pub dirty: bool,
    pub valid_bytes: usize,
    pub lru_seq: u64,
}

pub struct PageCache {
    pub entries: BTreeMap<PageKey, PageEntry>,
    pub file_sizes: BTreeMap<(usize, u64), u64>,
    pub max_entries: usize,
    pub next_seq: u64,
    pub hits: u64,
    pub misses: u64,
}

impl PageCache {
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            file_sizes: BTreeMap::new(),
            max_entries: PAGE_CACHE_MAX_PAGES,
            next_seq: 1,
            hits: 0,
            misses: 0,
        }
    }

    pub fn stats(&self) -> (u64, u64, usize) {
        (self.hits, self.misses, self.entries.len())
    }

    pub fn get_file_size(&self, mount_id: usize, ino: u64) -> Option<u64> {
        self.file_sizes.get(&(mount_id, ino)).copied()
    }

    pub fn set_file_size(&mut self, mount_id: usize, ino: u64, size: u64) {
        self.file_sizes.insert((mount_id, ino), size);
    }
}

pub static PAGE_CACHE: Mutex<PageCache> = Mutex::new(PageCache::new());

pub fn get_file_size(mount_id: usize, ino: u64) -> Option<u64> {
    PAGE_CACHE.lock().get_file_size(mount_id, ino)
}

pub fn set_file_size(mount_id: usize, ino: u64, size: u64) {
    PAGE_CACHE.lock().set_file_size(mount_id, ino, size);
}

/// Read from a page-cached file into `buf` starting at `offset`.
pub fn read(
    mount_id: usize,
    fs: &dyn Filesystem,
    ino: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, VfsError> {
    if buf.is_empty() {
        return Ok(0);
    }

    // Determine current effective file size (cached in-memory size or filesystem disk size)
    let current_size = {
        let cache = PAGE_CACHE.lock();
        cache
            .get_file_size(mount_id, ino)
            .unwrap_or_else(|| fs.read_inode(ino).map(|m| m.size).unwrap_or(0))
    };

    if offset >= current_size {
        return Ok(0);
    }

    let to_read = (buf.len() as u64).min(current_size.saturating_sub(offset)) as usize;
    if to_read == 0 {
        return Ok(0);
    }

    let mut bytes_copied = 0usize;
    while bytes_copied < to_read {
        let current_offset = offset + bytes_copied as u64;
        let page_index = current_offset / (PAGE_SIZE as u64);
        let page_offset = (current_offset % (PAGE_SIZE as u64)) as usize;
        let chunk_len = ((PAGE_SIZE as usize - page_offset).min(to_read - bytes_copied))
            .min(current_size.saturating_sub(current_offset) as usize);

        if chunk_len == 0 {
            break;
        }

        let key = PageKey {
            mount_id,
            ino,
            page_index,
        };

        let mut cache = PAGE_CACHE.lock();
        let existing = cache.entries.get_mut(&key).map(|e| {
            e.lru_seq = e.lru_seq.saturating_add(1);
            e.frame
        });

        let frame = if let Some(f) = existing {
            cache.hits += 1;
            f
        } else {
            cache.misses += 1;
            drop(cache);

            // Read 4 KiB from disk
            let frame = alloc_frame().ok_or(VfsError::NoSpace)?;
            let virt = phys_to_virt(frame.start_address()).ok_or(VfsError::RedoxFs)?;
            let frame_slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PAGE_SIZE as usize) };
            
            let disk_offset = page_index * (PAGE_SIZE as u64);
            let read_bytes = fs.read(ino, disk_offset, frame_slice).unwrap_or(0);
            if read_bytes < PAGE_SIZE as usize {
                frame_slice[read_bytes..].fill(0);
            }

            let mut cache = PAGE_CACHE.lock();
            cache.next_seq += 1;
            let seq = cache.next_seq;
            cache.entries.insert(
                key,
                PageEntry {
                    frame,
                    dirty: false,
                    valid_bytes: read_bytes,
                    lru_seq: seq,
                },
            );
            frame
        };

        // Copy chunk into destination buffer
        let virt = phys_to_virt(frame.start_address()).ok_or(VfsError::RedoxFs)?;
        let frame_slice = unsafe { core::slice::from_raw_parts(virt as *const u8, PAGE_SIZE as usize) };
        buf[bytes_copied..bytes_copied + chunk_len]
            .copy_from_slice(&frame_slice[page_offset..page_offset + chunk_len]);

        bytes_copied += chunk_len;
    }

    Ok(bytes_copied)
}

/// Write `buf` into a page-cached file starting at `offset`.
pub fn write(
    mount_id: usize,
    fs: &dyn Filesystem,
    ino: u64,
    offset: u64,
    buf: &[u8],
) -> Result<usize, VfsError> {
    if buf.is_empty() {
        return Ok(0);
    }

    let mut bytes_written = 0usize;
    let initial_size = {
        let cache = PAGE_CACHE.lock();
        cache
            .get_file_size(mount_id, ino)
            .unwrap_or_else(|| fs.read_inode(ino).map(|m| m.size).unwrap_or(0))
    };
    let new_size = initial_size.max(offset + buf.len() as u64);

    while bytes_written < buf.len() {
        let current_offset = offset + bytes_written as u64;
        let page_index = current_offset / (PAGE_SIZE as u64);
        let page_offset = (current_offset % (PAGE_SIZE as u64)) as usize;
        let chunk_len = (PAGE_SIZE as usize - page_offset).min(buf.len() - bytes_written);

        let key = PageKey {
            mount_id,
            ino,
            page_index,
        };

        let mut cache = PAGE_CACHE.lock();
        let existing = cache.entries.get_mut(&key).map(|entry| {
            entry.dirty = true;
            entry.valid_bytes = entry.valid_bytes.max(page_offset + chunk_len);
            entry.lru_seq = entry.lru_seq.saturating_add(1);
            entry.frame
        });

        let frame = if let Some(f) = existing {
            cache.hits += 1;
            f
        } else {
            cache.misses += 1;
            drop(cache);

            let frame = alloc_frame().ok_or(VfsError::NoSpace)?;
            let virt = phys_to_virt(frame.start_address()).ok_or(VfsError::RedoxFs)?;
            let frame_slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PAGE_SIZE as usize) };

            // If this is a partial page write and data exists on disk, read disk first
            let page_start_disk = page_index * (PAGE_SIZE as u64);
            let mut valid_bytes = 0usize;
            if (page_offset > 0 || chunk_len < PAGE_SIZE as usize) && page_start_disk < initial_size {
                let r = fs.read(ino, page_start_disk, frame_slice).unwrap_or(0);
                valid_bytes = r;
                if r < PAGE_SIZE as usize {
                    frame_slice[r..].fill(0);
                }
            } else if page_offset > 0 || chunk_len < PAGE_SIZE as usize {
                frame_slice.fill(0);
            }

            valid_bytes = valid_bytes.max(page_offset + chunk_len);

            let mut cache = PAGE_CACHE.lock();
            cache.next_seq += 1;
            let seq = cache.next_seq;
            cache.entries.insert(
                key,
                PageEntry {
                    frame,
                    dirty: true,
                    valid_bytes,
                    lru_seq: seq,
                },
            );
            frame
        };

        // Copy source bytes into the cached physical page frame
        let virt = phys_to_virt(frame.start_address()).ok_or(VfsError::RedoxFs)?;
        let frame_slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PAGE_SIZE as usize) };
        frame_slice[page_offset..page_offset + chunk_len]
            .copy_from_slice(&buf[bytes_written..bytes_written + chunk_len]);

        bytes_written += chunk_len;
    }

    PAGE_CACHE.lock().set_file_size(mount_id, ino, new_size);
    Ok(bytes_written)
}

/// Retrieve or allocate/load physical frame for `(mount_id, ino, page_index)`.
/// Used for zero-copy file-backed `mmap` demand paging.
pub fn get_or_fetch_frame(
    mount_id: usize,
    fs: &dyn Filesystem,
    ino: u64,
    file_size: u64,
    page_index: u64,
) -> Result<PhysFrame, ()> {
    let key = PageKey {
        mount_id,
        ino,
        page_index,
    };

    let mut cache = PAGE_CACHE.lock();
    let existing = cache.entries.get_mut(&key).map(|entry| {
        entry.lru_seq = entry.lru_seq.saturating_add(1);
        entry.frame
    });
    if let Some(f) = existing {
        cache.hits += 1;
        return Ok(f);
    }
    cache.misses += 1;
    drop(cache);

    let frame = alloc_frame().ok_or(())?;
    let virt = phys_to_virt(frame.start_address()).ok_or(())?;
    let frame_slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PAGE_SIZE as usize) };

    let disk_offset = page_index * (PAGE_SIZE as u64);
    let read_bytes = if disk_offset < file_size {
        fs.read(ino, disk_offset, frame_slice).unwrap_or(0)
    } else {
        0
    };
    if read_bytes < PAGE_SIZE as usize {
        frame_slice[read_bytes..].fill(0);
    }

    let mut cache = PAGE_CACHE.lock();
    cache.next_seq += 1;
    let seq = cache.next_seq;
    cache.entries.insert(
        key,
        PageEntry {
            frame,
            dirty: false,
            valid_bytes: read_bytes,
            lru_seq: seq,
        },
    );

    Ok(frame)
}

pub fn mark_dirty(mount_id: usize, ino: u64, page_index: u64) {
    let key = PageKey {
        mount_id,
        ino,
        page_index,
    };
    let mut cache = PAGE_CACHE.lock();
    if let Some(entry) = cache.entries.get_mut(&key) {
        entry.dirty = true;
    }
}

/// Flush all dirty pages for a specific inode to disk in contiguous batches.
pub fn flush_inode(mount_id: usize, fs: &dyn Filesystem, ino: u64) -> Result<(), VfsError> {
    let cached_size = PAGE_CACHE.lock().get_file_size(mount_id, ino);
    if let Some(sz) = cached_size {
        let _ = fs.truncate(ino, sz);
    }

    // Collect dirty pages for (mount_id, ino)
    let dirty_pages: Vec<(u64, PhysFrame, usize)> = {
        let cache = PAGE_CACHE.lock();
        cache
            .entries
            .iter()
            .filter(|(k, e)| k.mount_id == mount_id && k.ino == ino && e.dirty)
            .map(|(k, e)| (k.page_index, e.frame, e.valid_bytes))
            .collect()
    };

    if dirty_pages.is_empty() {
        return fs.sync();
    }

    // Batch contiguous dirty pages together into write buffers (up to 16 pages / 64 KiB)
    const BATCH_PAGES: usize = 16;
    let mut index = 0;
    while index < dirty_pages.len() {
        let (start_page, _, _) = dirty_pages[index];
        let mut batch_data = Vec::with_capacity(BATCH_PAGES * (PAGE_SIZE as usize));
        
        let mut count = 0;
        while index + count < dirty_pages.len() && count < BATCH_PAGES {
            let (curr_page, curr_frame, _) = dirty_pages[index + count];
            if curr_page != start_page + count as u64 {
                break;
            }
            let virt = phys_to_virt(curr_frame.start_address()).ok_or(VfsError::RedoxFs)?;
            let slice = unsafe { core::slice::from_raw_parts(virt as *const u8, PAGE_SIZE as usize) };
            batch_data.extend_from_slice(slice);
            count += 1;
        }

        let write_offset = start_page * (PAGE_SIZE as u64);
        fs.write(ino, write_offset, &batch_data)?;

        // Mark flushed pages as clean
        {
            let mut cache = PAGE_CACHE.lock();
            for c in 0..count {
                let page_idx = start_page + c as u64;
                let k = PageKey {
                    mount_id,
                    ino,
                    page_index: page_idx,
                };
                if let Some(entry) = cache.entries.get_mut(&k) {
                    entry.dirty = false;
                }
            }
        }

        index += count;
    }

    fs.sync()
}

/// Flush all dirty pages across all mounted filesystems.
pub fn flush_all() -> Result<(), VfsError> {
    let inodes: Vec<(usize, u64)> = {
        let cache = PAGE_CACHE.lock();
        let mut set = Vec::new();
        for k in cache.entries.keys() {
            let pair = (k.mount_id, k.ino);
            if !set.contains(&pair) {
                set.push(pair);
            }
        }
        set
    };

    for (mount_id, ino) in inodes {
        if let Some(fs) = crate::vfs::get_mount_fs(mount_id) {
            let _ = flush_inode(mount_id, fs.as_ref(), ino);
        }
    }

    Ok(())
}

/// Truncate cached pages for `(mount_id, ino)` beyond `new_size`.
pub fn truncate(mount_id: usize, ino: u64, new_size: u64) {
    let mut cache = PAGE_CACHE.lock();
    cache.set_file_size(mount_id, ino, new_size);

    let first_removed_page = (new_size + (PAGE_SIZE as u64 - 1)) / (PAGE_SIZE as u64);
    let keys_to_remove: Vec<PageKey> = cache
        .entries
        .range(
            PageKey {
                mount_id,
                ino,
                page_index: first_removed_page,
            }..=PageKey {
                mount_id,
                ino,
                page_index: u64::MAX,
            },
        )
        .map(|(k, _)| *k)
        .collect();

    for k in keys_to_remove {
        if let Some(entry) = cache.entries.remove(&k) {
            free_frame(entry.frame);
        }
    }

    // Zero the boundary page remainder if new_size cuts into a page
    let boundary_page = new_size / (PAGE_SIZE as u64);
    let boundary_offset = (new_size % (PAGE_SIZE as u64)) as usize;
    if boundary_offset > 0 {
        let k = PageKey {
            mount_id,
            ino,
            page_index: boundary_page,
        };
        if let Some(entry) = cache.entries.get_mut(&k) {
            if let Some(virt) = phys_to_virt(entry.frame.start_address()) {
                let slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, PAGE_SIZE as usize) };
                slice[boundary_offset..].fill(0);
                entry.valid_bytes = boundary_offset;
                entry.dirty = true;
            }
        }
    }
}

/// Invalidate and free all cached pages for an unlinked inode.
pub fn invalidate_inode(mount_id: usize, ino: u64) {
    let mut cache = PAGE_CACHE.lock();
    cache.file_sizes.remove(&(mount_id, ino));

    let keys_to_remove: Vec<PageKey> = cache
        .entries
        .range(
            PageKey {
                mount_id,
                ino,
                page_index: 0,
            }..=PageKey {
                mount_id,
                ino,
                page_index: u64::MAX,
            },
        )
        .map(|(k, _)| *k)
        .collect();

    for k in keys_to_remove {
        if let Some(entry) = cache.entries.remove(&k) {
            free_frame(entry.frame);
        }
    }
}

static FLUSHER_TICKS: AtomicU64 = AtomicU64::new(0);
static FLUSHER_RUN_COUNT: AtomicU64 = AtomicU64::new(0);
static FLUSHER_PENDING: AtomicBool = AtomicBool::new(false);

/// Background flusher tick, invoked on BSP timer tick.
/// Wakes every 500 ms (500 ticks) by requesting background flush.
pub fn flusher_tick() {
    let ticks = FLUSHER_TICKS.fetch_add(1, Ordering::Relaxed);
    if ticks > 0 && ticks % 500 == 0 {
        FLUSHER_PENDING.store(true, Ordering::Release);
    }
}

pub fn is_flusher_pending() -> bool {
    FLUSHER_PENDING.load(Ordering::Acquire)
}

/// Run background flusher in kernel task/scheduler context safely.
pub fn run_flusher() {
    if FLUSHER_PENDING.swap(false, Ordering::AcqRel) {
        FLUSHER_RUN_COUNT.fetch_add(1, Ordering::Relaxed);
        let _ = flush_all();
    }
}

pub fn flusher_run_count() -> u64 {
    FLUSHER_RUN_COUNT.load(Ordering::Relaxed)
}

pub fn dirty_count() -> usize {
    let cache = PAGE_CACHE.lock();
    cache.entries.values().filter(|e| e.dirty).count()
}

pub fn cached_pages_count() -> usize {
    let cache = PAGE_CACHE.lock();
    cache.entries.len()
}

/// Evict up to `max_count` clean pages from the page cache LRU list,
/// immediately reclaiming physical frames for the Buddy Allocator.
/// Uses `try_lock()` to avoid deadlocking if called under memory pressure during a cache operation.
pub fn evict_clean_pages(max_count: usize) -> usize {
    let mut cache = match PAGE_CACHE.try_lock() {
        Some(c) => c,
        None => return 0,
    };

    let mut clean_keys: Vec<(PageKey, u64, PhysFrame)> = cache
        .entries
        .iter()
        .filter(|(_, e)| !e.dirty)
        .map(|(k, e)| (*k, e.lru_seq, e.frame))
        .collect();

    if clean_keys.is_empty() {
        return 0;
    }

    clean_keys.sort_by_key(|(_, lru, _)| *lru);

    let to_evict = clean_keys.len().min(max_count);
    let mut evicted = 0;
    for (k, _, frame) in &clean_keys[..to_evict] {
        if let Some(entry) = cache.entries.remove(k) {
            if !entry.dirty {
                free_frame(*frame);
                evicted += 1;
            } else {
                cache.entries.insert(*k, entry);
            }
        }
    }
    evicted
}

