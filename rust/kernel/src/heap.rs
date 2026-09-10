//! Kernel heap and slab allocator backed by Rust page frame mappings.
//!
//! Provides a tiered kmalloc allocator with power-of-2 size classes:
//! 32, 64, 128, 256, 512, 1024, 2048 bytes backed by `KMemCache` slabs,
//! and large allocations (>= 4096 bytes) routed directly to the buddy frame allocator.
//! Dedicated `KMemCache` slab caches are provided for high-frequency kernel objects:
//! `task_struct_cache`, `vma_cache`, `file_descriptor_cache`, and `dentry_cache`.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use spin::Mutex;

use crate::{memory, paging};

const HEAP_BASE: u64 = 0xffff_ff00_0000_0000;
const HEAP_SIZE: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStats {
    pub base: u64,
    pub size: usize,
    pub used: usize,
    pub free: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapInitError {
    AlreadyInitialized,
    OutOfMemory,
    Mapping(paging::MapError),
}

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static TOTAL_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static TOTAL_FREED: AtomicUsize = AtomicUsize::new(0);

struct CacheInner {
    current_page: Option<usize>,
    next_offset: usize,
    freelist: Option<usize>,
}

impl CacheInner {
    const fn empty() -> Self {
        Self {
            current_page: None,
            next_offset: 0,
            freelist: None,
        }
    }
}

pub struct KMemCache {
    name: &'static str,
    object_size: usize,
    alignment: usize,
    inner: Mutex<CacheInner>,
    alloc_count: AtomicUsize,
    reuse_count: AtomicUsize,
    free_count: AtomicUsize,
}

unsafe impl Send for KMemCache {}
unsafe impl Sync for KMemCache {}

impl KMemCache {
    pub const fn new(name: &'static str, object_size: usize, alignment: usize) -> Self {
        Self {
            name,
            object_size,
            alignment,
            inner: Mutex::new(CacheInner::empty()),
            alloc_count: AtomicUsize::new(0),
            reuse_count: AtomicUsize::new(0),
            free_count: AtomicUsize::new(0),
        }
    }

    pub fn alloc(&self) -> Option<*mut u8> {
        let mut inner = self.inner.lock();

        // 1. Recycle from freelist if available
        if let Some(curr) = inner.freelist {
            unsafe {
                let next = (curr as *const usize).read();
                inner.freelist = if next != 0 { Some(next) } else { None };
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
                self.reuse_count.fetch_add(1, Ordering::Relaxed);
                return Some(curr as *mut u8);
            }
        }

        // 2. Carve from currently active slab page
        let align = self.alignment.max(core::mem::align_of::<usize>());
        let raw_size = self.object_size.max(core::mem::size_of::<usize>());
        let obj_size = (raw_size + align - 1) & !(align - 1);

        if let Some(page_vaddr) = inner.current_page {
            if inner.next_offset + obj_size <= memory::PAGE_SIZE as usize {
                let ptr = (page_vaddr + inner.next_offset) as *mut u8;
                inner.next_offset += obj_size;
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
                return Some(ptr);
            }
        }

        // 3. Current page full or uninitialized: allocate new 4 KiB frame from buddy allocator
        let frame = memory::alloc_frame()?;
        let phys = frame.start_address();
        let vaddr = paging::phys_to_virt(phys)? as usize;

        inner.current_page = Some(vaddr);
        inner.next_offset = obj_size;
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        Some(vaddr as *mut u8)
    }

    pub fn free(&self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let curr = ptr as usize;
        let mut inner = self.inner.lock();
        let next = inner.freelist.unwrap_or(0);
        unsafe {
            (curr as *mut usize).write(next);
        }
        inner.freelist = Some(curr);
        self.free_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.alloc_count.load(Ordering::Relaxed),
            self.reuse_count.load(Ordering::Relaxed),
            self.free_count.load(Ordering::Relaxed),
        )
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

// Power-of-2 size-class slab caches: 32, 64, 128, 256, 512, 1024, 2048 bytes
pub static KMALLOC_32: KMemCache = KMemCache::new("kmalloc-32", 32, 32);
pub static KMALLOC_64: KMemCache = KMemCache::new("kmalloc-64", 64, 64);
pub static KMALLOC_128: KMemCache = KMemCache::new("kmalloc-128", 128, 128);
pub static KMALLOC_256: KMemCache = KMemCache::new("kmalloc-256", 256, 256);
pub static KMALLOC_512: KMemCache = KMemCache::new("kmalloc-512", 512, 512);
pub static KMALLOC_1024: KMemCache = KMemCache::new("kmalloc-1024", 1024, 1024);
pub static KMALLOC_2048: KMemCache = KMemCache::new("kmalloc-2048", 2048, 2048);

// Dedicated high-frequency object slab caches
pub static TASK_STRUCT_CACHE: KMemCache = KMemCache::new("task_struct_cache", 768, 16);
pub static VMA_CACHE: KMemCache = KMemCache::new("vma_cache", 64, 8);
pub static FILE_DESCRIPTOR_CACHE: KMemCache = KMemCache::new("file_descriptor_cache", 128, 8);
pub static DENTRY_CACHE: KMemCache = KMemCache::new("dentry_cache", 128, 8);

pub fn alloc_task() -> Option<*mut u8> {
    TASK_STRUCT_CACHE.alloc()
}
pub fn free_task(ptr: *mut u8) {
    TASK_STRUCT_CACHE.free(ptr)
}

pub fn alloc_vma() -> Option<*mut u8> {
    VMA_CACHE.alloc()
}
pub fn free_vma(ptr: *mut u8) {
    VMA_CACHE.free(ptr)
}

pub fn alloc_file_descriptor() -> Option<*mut u8> {
    FILE_DESCRIPTOR_CACHE.alloc()
}
pub fn free_file_descriptor(ptr: *mut u8) {
    FILE_DESCRIPTOR_CACHE.free(ptr)
}

pub fn alloc_dentry() -> Option<*mut u8> {
    DENTRY_CACHE.alloc()
}
pub fn free_dentry(ptr: *mut u8) {
    DENTRY_CACHE.free(ptr)
}

pub struct KernelHeap;

impl KernelHeap {
    pub const fn new() -> Self {
        Self
    }
}

#[global_allocator]
static GLOBAL_HEAP: KernelHeap = KernelHeap::new();

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }

        if !INITIALIZED.load(Ordering::Relaxed) {
            return null_mut();
        }

        let size = layout.size().max(layout.align());
        let (ptr, class_size) = if size <= 32 {
            (KMALLOC_32.alloc().unwrap_or(null_mut()), 32)
        } else if size <= 64 {
            (KMALLOC_64.alloc().unwrap_or(null_mut()), 64)
        } else if size <= 128 {
            (KMALLOC_128.alloc().unwrap_or(null_mut()), 128)
        } else if size <= 256 {
            (KMALLOC_256.alloc().unwrap_or(null_mut()), 256)
        } else if size <= 512 {
            (KMALLOC_512.alloc().unwrap_or(null_mut()), 512)
        } else if size <= 1024 {
            (KMALLOC_1024.alloc().unwrap_or(null_mut()), 1024)
        } else if size <= 2048 {
            (KMALLOC_2048.alloc().unwrap_or(null_mut()), 2048)
        } else {
            // >= 4096 (or > 2048): Route to page-frame allocator directly
            let num_pages = (size + memory::PAGE_SIZE as usize - 1) / memory::PAGE_SIZE as usize;
            let order = num_pages.next_power_of_two().trailing_zeros() as usize;
            let Some(frame) = memory::alloc_frames(order) else {
                return null_mut();
            };
            let Some(vaddr) = paging::phys_to_virt(frame.start_address()) else {
                return null_mut();
            };
            (vaddr as *mut u8, (1usize << order) * memory::PAGE_SIZE as usize)
        };

        if !ptr.is_null() {
            TOTAL_ALLOCATED.fetch_add(class_size, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() || layout.size() == 0 {
            return;
        }

        let size = layout.size().max(layout.align());
        let class_size = if size <= 32 {
            KMALLOC_32.free(ptr);
            32
        } else if size <= 64 {
            KMALLOC_64.free(ptr);
            64
        } else if size <= 128 {
            KMALLOC_128.free(ptr);
            128
        } else if size <= 256 {
            KMALLOC_256.free(ptr);
            256
        } else if size <= 512 {
            KMALLOC_512.free(ptr);
            512
        } else if size <= 1024 {
            KMALLOC_1024.free(ptr);
            1024
        } else if size <= 2048 {
            KMALLOC_2048.free(ptr);
            2048
        } else {
            // >= 4096: Route to page-frame allocator directly
            let num_pages = (size + memory::PAGE_SIZE as usize - 1) / memory::PAGE_SIZE as usize;
            let order = num_pages.next_power_of_two().trailing_zeros() as usize;
            let vaddr = ptr as u64;
            if let Some(phys) = paging::virt_to_phys(vaddr) {
                memory::free_frame(memory::PhysFrame(phys));
            }
            (1usize << order) * memory::PAGE_SIZE as usize
        };

        TOTAL_FREED.fetch_add(class_size, Ordering::Relaxed);
    }
}

pub fn init() -> Result<HeapStats, HeapInitError> {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return Err(HeapInitError::AlreadyInitialized);
    }
    Ok(stats())
}

pub fn stats() -> HeapStats {
    let allocated = TOTAL_ALLOCATED.load(Ordering::Relaxed);
    let freed = TOTAL_FREED.load(Ordering::Relaxed);
    let used = allocated.saturating_sub(freed);
    let total_size = HEAP_SIZE;
    HeapStats {
        base: HEAP_BASE,
        size: total_size,
        used,
        free: total_size.saturating_sub(used),
    }
}

pub fn self_check() {
    let caches: [&KMemCache; 4] = [
        &TASK_STRUCT_CACHE,
        &VMA_CACHE,
        &FILE_DESCRIPTOR_CACHE,
        &DENTRY_CACHE,
    ];

    for cache in caches {
        let p1 = cache.alloc().expect("cache alloc failed");
        let p2 = cache.alloc().expect("cache alloc failed");
        if p1 == p2 {
            panic!("slab cache {} returned duplicate pointer", cache.name());
        }
        cache.free(p1);
        let p3 = cache.alloc().expect("cache realloc failed");
        if p3 != p1 {
            panic!("slab cache {} failed to recycle freed object", cache.name());
        }
        cache.free(p2);
        cache.free(p3);

        let (allocs, reused, _frees) = cache.stats();
        crate::serial_println!(
            "[slab] {}: {} allocs, {} reused",
            cache.name(),
            allocs,
            reused
        );
    }

    crate::serial_println!("[slab] kmalloc size-classes (32..2048) active, >=4096 direct buddy");
}
