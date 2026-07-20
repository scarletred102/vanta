//! Minimal legacy VirtIO block driver used for persistent sector I/O.

use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};

use spin::Mutex;
use x86_64::instructions::port::Port;

use crate::memory::{self, PhysFrame, PAGE_SIZE};
use crate::paging;
use crate::serial_println;
use crate::storage::{BlockDevice, StorageError, SECTOR_SIZE};

const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_BLOCK_LEGACY_ID: u16 = 0x1001;
const VIRTIO_BLOCK_MODERN_ID: u16 = 0x1042;
const QUEUE_SELECT: u16 = 0x0e;
const QUEUE_SIZE: u16 = 0x0c;
const QUEUE_ADDRESS: u16 = 0x08;
const QUEUE_NOTIFY: u16 = 0x10;
const DEVICE_STATUS: u16 = 0x12;
const DEVICE_CONFIG: u16 = 0x14;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
const REQUEST_READ: u32 = 0;
const REQUEST_WRITE: u32 = 1;
const REQUEST_HEADER_SIZE: u64 = 16;
const REQUEST_TIMEOUT: usize = 10_000_000;
const SUPPORTED_FEATURES: u32 = (1 << 28) | (1 << 29);
const DMA_MIN_PHYSICAL: u64 = 0x10_0000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

struct QueueState {
    avail_index: u16,
    used_index: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioError {
    NotFound,
    UnsupportedBar,
    QueueUnavailable,
    AllocationFailed,
    DeviceFailed,
}

pub struct VirtioBlock {
    io_base: u16,
    sectors: u64,
    queue_phys: u64,
    _queue_frames: Vec<PhysFrame>,
    buffer_phys: u64,
    _buffer_frame: PhysFrame,
    _dma_reservations: Vec<PhysFrame>,
    queue_size: u16,
    avail_offset: usize,
    used_offset: usize,
    queue_state: Mutex<QueueState>,
}

impl VirtioBlock {
    pub fn probe() -> Result<Self, VirtioError> {
        let (address, device_id) = find_device().ok_or(VirtioError::NotFound)?;
        if device_id == VIRTIO_BLOCK_MODERN_ID {
            return Err(VirtioError::UnsupportedBar);
        }
        let bar = crate::pci::read_u32(address, 0x10);
        if bar & 1 == 0 {
            return Err(VirtioError::UnsupportedBar);
        }
        let io_base = (bar & 0xfffc) as u16;
        let command = crate::pci::read_u32(address, 0x04) as u16 | 0x0004 | 0x0001;
        let previous_command = crate::pci::read_u32(address, 0x04);
        crate::pci::write_u32(
            address,
            0x04,
            (previous_command & 0xffff_0000) | command as u32,
        );

        write_status(io_base, 0);
        let _ = port_read8(io_base, DEVICE_STATUS);
        port_write16(io_base, QUEUE_SELECT, 0);
        port_write32(io_base, QUEUE_ADDRESS, 0);
        write_status(io_base, STATUS_ACKNOWLEDGE);
        write_status(io_base, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        let host_features = port_read32(io_base, 0);
        port_write32(io_base, 0x04, host_features & SUPPORTED_FEATURES);
        port_write16(io_base, QUEUE_SELECT, 0);
        let queue_size = port_read16(io_base, QUEUE_SIZE) as usize;
        if queue_size == 0 {
            return Err(VirtioError::QueueUnavailable);
        }
        let avail_offset = 16 * queue_size;
        let used_offset = align_up(avail_offset + 6 + 2 * queue_size, PAGE_SIZE as usize);
        let queue_bytes = used_offset + 6 + 8 * queue_size;
        let mut dma_reservations = Vec::new();
        let queue_frames = allocate_contiguous_frames(queue_bytes, &mut dma_reservations)?;
        let queue_phys = queue_frames[0].start_address();
        zero_physical(queue_phys, queue_frames.len() * PAGE_SIZE as usize);

        let buffer_frame = allocate_dma_frame(&mut dma_reservations)?;
        let buffer_phys = buffer_frame.start_address();
        zero_physical(buffer_phys, PAGE_SIZE as usize);

        port_write32(io_base, QUEUE_ADDRESS, (queue_phys / PAGE_SIZE) as u32);
        port_write8(
            io_base,
            DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
        let sectors = (port_read32(io_base, DEVICE_CONFIG) as u64)
            | ((port_read32(io_base, DEVICE_CONFIG + 4) as u64) << 32);
        serial_println!(
            "[virtio] legacy pci={:02x}:{:02x} queue={} sectors={} pfn={:#x}",
            address.bus,
            address.device,
            queue_size,
            sectors,
            port_read32(io_base, QUEUE_ADDRESS),
        );
        if sectors == 0 {
            return Err(VirtioError::DeviceFailed);
        }
        Ok(Self {
            io_base,
            sectors,
            queue_phys,
            _queue_frames: queue_frames,
            buffer_phys,
            _buffer_frame: buffer_frame,
            _dma_reservations: dma_reservations,
            queue_size: queue_size as u16,
            avail_offset,
            used_offset,
            queue_state: Mutex::new(QueueState {
                avail_index: 0,
                used_index: 0,
            }),
        })
    }

    fn request(&self, write: bool, sector: u64, data: *mut u8) -> Result<(), StorageError> {
        if sector >= self.sectors {
            return Err(StorageError::OutOfBounds);
        }
        let queue = paging::phys_to_virt(self.queue_phys).ok_or(StorageError::DeviceUnavailable)?;
        let buffer =
            paging::phys_to_virt(self.buffer_phys).ok_or(StorageError::DeviceUnavailable)?;
        let header = buffer as *mut u8;
        unsafe {
            write_u32(header, if write { REQUEST_WRITE } else { REQUEST_READ });
            write_u32(header.add(4), 0);
            write_u64(header.add(8), sector);
        }
        let payload = (buffer + REQUEST_HEADER_SIZE) as *mut u8;
        if write {
            unsafe { core::ptr::copy_nonoverlapping(data, payload, SECTOR_SIZE) };
        }
        unsafe {
            (buffer as *mut u8)
                .add(REQUEST_HEADER_SIZE as usize + SECTOR_SIZE)
                .write_volatile(0xff)
        };

        let descriptors = queue as *mut Descriptor;
        unsafe {
            descriptors.add(0).write_volatile(Descriptor {
                address: self.buffer_phys,
                length: 16,
                flags: DESC_NEXT,
                next: 1,
            });
            descriptors.add(1).write_volatile(Descriptor {
                address: self.buffer_phys + REQUEST_HEADER_SIZE,
                length: SECTOR_SIZE as u32,
                flags: DESC_NEXT | if write { 0 } else { DESC_WRITE },
                next: 2,
            });
            descriptors.add(2).write_volatile(Descriptor {
                address: self.buffer_phys + REQUEST_HEADER_SIZE + SECTOR_SIZE as u64,
                length: 1,
                flags: DESC_WRITE,
                next: 0,
            });
        }
        let mut queue_state = self.queue_state.lock();
        let avail = (queue + self.avail_offset as u64) as *mut u16;
        let next_avail = queue_state.avail_index.wrapping_add(1);
        let ring_slot = queue_state.avail_index as usize % self.queue_size as usize;
        unsafe {
            avail.add(2 + ring_slot).write_volatile(0);
        }
        fence(Ordering::SeqCst);
        unsafe { avail.add(1).write_volatile(next_avail) };
        fence(Ordering::SeqCst);
        port_write16(self.io_base, QUEUE_NOTIFY, 0);
        let used = (queue + self.used_offset as u64) as *const u16;
        for attempt in 0..REQUEST_TIMEOUT {
            let index = unsafe { used.add(1).read_volatile() };
            if index != queue_state.used_index {
                let status = unsafe {
                    (buffer as *const u8)
                        .add(REQUEST_HEADER_SIZE as usize + SECTOR_SIZE)
                        .read_volatile()
                };
                if status != 0 {
                    return Err(StorageError::IoFailed);
                }
                if !write {
                    unsafe { core::ptr::copy_nonoverlapping(payload, data, SECTOR_SIZE) };
                }
                queue_state.avail_index = next_avail;
                queue_state.used_index = index;
                return Ok(());
            }
            if attempt & 0xffff == 0xffff {
                x86_64::instructions::hlt();
            } else {
                core::hint::spin_loop();
            }
        }
        serial_println!(
            "[virtio] request timeout write={} sector={} avail-idx={} head={} used-idx={} status={:#x} isr={:#x}",
            write,
            sector,
            unsafe { avail.add(1).read_volatile() },
            unsafe { avail.add(2 + ring_slot).read_volatile() },
            unsafe { used.add(1).read_volatile() },
            port_read8(self.io_base, DEVICE_STATUS),
            port_read8(self.io_base, 0x13),
        );
        Err(StorageError::IoFailed)
    }
}

impl BlockDevice for VirtioBlock {
    fn sector_count(&self) -> u64 {
        self.sectors
    }

    fn read_sector(&self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        self.request(false, sector, buffer.as_mut_ptr())
    }

    fn write_sector(
        &mut self,
        sector: u64,
        buffer: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        self.request(true, sector, buffer.as_ptr() as *mut u8)
    }
}

fn find_device() -> Option<(crate::pci::PciAddress, u16)> {
    crate::pci::devices()
        .into_iter()
        .find(|device| {
            device.vendor_id == VIRTIO_VENDOR_ID
                && (device.device_id == VIRTIO_BLOCK_LEGACY_ID
                    || device.device_id == VIRTIO_BLOCK_MODERN_ID)
        })
        .map(|device| (device.address, device.device_id))
}

fn allocate_contiguous_frames(
    bytes: usize,
    reservations: &mut Vec<PhysFrame>,
) -> Result<Vec<PhysFrame>, VirtioError> {
    let count = (bytes + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let frame = allocate_dma_frame(reservations).map_err(|_| {
            for frame in frames.drain(..) {
                let _ = memory::free_frame(frame);
            }
            VirtioError::AllocationFailed
        })?;
        if let Some(previous) = frames.last() {
            if frame.start_address() != previous.start_address() + PAGE_SIZE {
                frames.push(frame);
                for frame in frames.drain(..) {
                    let _ = memory::free_frame(frame);
                }
                return Err(VirtioError::AllocationFailed);
            }
        }
        frames.push(frame);
    }
    Ok(frames)
}

fn allocate_dma_frame(reservations: &mut Vec<PhysFrame>) -> Result<PhysFrame, VirtioError> {
    loop {
        let frame = memory::alloc_frame().ok_or(VirtioError::AllocationFailed)?;
        if frame.start_address() >= DMA_MIN_PHYSICAL {
            return Ok(frame);
        }
        reservations.push(frame);
    }
}

fn zero_physical(physical: u64, length: usize) {
    if let Some(virtual_address) = paging::phys_to_virt(physical) {
        unsafe { core::ptr::write_bytes(virtual_address as *mut u8, 0, length) };
    }
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn write_status(base: u16, status: u8) {
    port_write8(base, DEVICE_STATUS, status);
}

fn port_write8(base: u16, offset: u16, value: u8) {
    let mut port: Port<u8> = Port::new(base + offset);
    unsafe { port.write(value) };
}

fn port_read16(base: u16, offset: u16) -> u16 {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_read8(base: u16, offset: u16) -> u8 {
    let mut port: Port<u8> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_write16(base: u16, offset: u16, value: u16) {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.write(value) };
}

fn port_read32(base: u16, offset: u16) -> u32 {
    let mut port: Port<u32> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_write32(base: u16, offset: u16, value: u32) {
    let mut port: Port<u32> = Port::new(base + offset);
    unsafe { port.write(value) };
}

unsafe fn write_u32(pointer: *mut u8, value: u32) {
    (pointer as *mut u32).write_volatile(value.to_le());
}

unsafe fn write_u64(pointer: *mut u8, value: u64) {
    (pointer as *mut u64).write_volatile(value.to_le());
}
