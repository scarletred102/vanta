//! Minimal legacy/transitional VirtIO-RNG (entropy) device driver.
//!
//! Conforms to VirtIO Specification 0.9.5 / 1.0 (Device type 4: Entropy device).
//! Reads hardware entropy from host hypervisor (QEMU virtio-rng-pci) into the kernel entropy pool.

use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};
use x86_64::instructions::port::Port;

use crate::memory::{self, PhysFrame, PAGE_SIZE};
use crate::paging;

const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_RNG_LEGACY_ID: u16 = 0x1005;
const VIRTIO_RNG_MODERN_ID: u16 = 0x1044;

const QUEUE_SELECT: u16 = 0x0e;
const QUEUE_SIZE: u16 = 0x0c;
const QUEUE_ADDRESS: u16 = 0x08;
const QUEUE_NOTIFY: u16 = 0x10;
const DEVICE_STATUS: u16 = 0x12;

const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;

const DESC_WRITE: u16 = 2;
const DMA_MIN_PHYSICAL: u64 = 0x10_0000;
const POLL_ATTEMPTS: usize = 2_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioRngError {
    NotFound,
    UnsupportedBar,
    QueueUnavailable,
    AllocationFailed,
    Timeout,
}

pub struct VirtioRng {
    io_base: u16,
    queue_phys: u64,
    _queue_frames: Vec<PhysFrame>,
    buffer_phys: u64,
    _buffer_frame: PhysFrame,
    _dma_reservations: Vec<PhysFrame>,
    queue_size: u16,
    avail_offset: usize,
    used_offset: usize,
    avail_index: u16,
    used_index: u16,
}

impl VirtioRng {
    pub fn probe() -> Result<Self, VirtioRngError> {
        let (address, _device_id) = find_device().ok_or(VirtioRngError::NotFound)?;

        // Scan BARs (0..6) for an I/O space BAR
        let mut io_base = None;
        for bar_idx in 0..6 {
            let bar = crate::pci::read_u32(address, 0x10 + bar_idx * 4);
            if bar & 1 == 1 {
                io_base = Some((bar & 0xfffc) as u16);
                break;
            }
        }
        let io_base = io_base.ok_or(VirtioRngError::UnsupportedBar)?;

        // Enable Bus Mastering (bit 2) and I/O Space (bit 0) in PCI Command register
        let command = crate::pci::read_u32(address, 0x04) as u16 | 0x0004 | 0x0001;
        let previous_command = crate::pci::read_u32(address, 0x04);
        crate::pci::write_u32(
            address,
            0x04,
            (previous_command & 0xffff_0000) | command as u32,
        );

        // Reset device
        write_status(io_base, 0);
        let _ = port_read8(io_base, DEVICE_STATUS);
        port_write16(io_base, QUEUE_SELECT, 0);
        port_write32(io_base, QUEUE_ADDRESS, 0);
        write_status(io_base, STATUS_ACKNOWLEDGE);
        write_status(io_base, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        // Select queue 0 (requestq)
        port_write16(io_base, QUEUE_SELECT, 0);
        let queue_size = port_read16(io_base, QUEUE_SIZE) as usize;
        if queue_size == 0 {
            return Err(VirtioRngError::QueueUnavailable);
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
        write_status(
            io_base,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );

        Ok(Self {
            io_base,
            queue_phys,
            _queue_frames: queue_frames,
            buffer_phys,
            _buffer_frame: buffer_frame,
            _dma_reservations: dma_reservations,
            queue_size: queue_size as u16,
            avail_offset,
            used_offset,
            avail_index: 0,
            used_index: 0,
        })
    }

    /// Requests random bytes from the host device into `dest`.
    /// Returns the number of bytes written.
    pub fn get_entropy(&mut self, dest: &mut [u8]) -> Result<usize, VirtioRngError> {
        if dest.is_empty() {
            return Ok(0);
        }
        let request_len = dest.len().min(PAGE_SIZE as usize);
        let queue = paging::phys_to_virt(self.queue_phys).ok_or(VirtioRngError::AllocationFailed)?;
        let buffer = paging::phys_to_virt(self.buffer_phys).ok_or(VirtioRngError::AllocationFailed)?;

        // Set descriptor 0 as a device-writable buffer
        let descriptors = queue as *mut Descriptor;
        unsafe {
            descriptors.write_volatile(Descriptor {
                address: self.buffer_phys,
                length: request_len as u32,
                flags: DESC_WRITE,
                next: 0,
            });
        }

        // Add descriptor 0 to the available ring
        let avail = (queue + self.avail_offset as u64) as *mut u16;
        let ring_slot = self.avail_index as usize % self.queue_size as usize;
        let next_avail = self.avail_index.wrapping_add(1);
        unsafe {
            avail.add(2 + ring_slot).write_volatile(0);
        }
        fence(Ordering::SeqCst);
        unsafe {
            avail.add(1).write_volatile(next_avail);
        }
        fence(Ordering::SeqCst);

        // Notify queue 0
        port_write16(self.io_base, QUEUE_NOTIFY, 0);

        // Wait for used ring update
        let used = (queue + self.used_offset as u64) as *const u16;
        for _ in 0..POLL_ATTEMPTS {
            let index = unsafe { used.add(1).read_volatile() };
            if index != self.used_index {
                // Device finished request. Read used element length from used ring.
                // used ring layout: flags: u16, idx: u16, ring: [vring_used_elem; queue_size]
                // Each element is 8 bytes: id (u32), len (u32)
                let used_elem_ptr = (queue
                    + self.used_offset as u64
                    + 4
                    + (self.used_index as u64 % self.queue_size as u64) * 8)
                    as *const u32;
                let bytes_written = unsafe { used_elem_ptr.add(1).read_volatile() } as usize;
                let actual_len = bytes_written.min(request_len);

                unsafe {
                    core::ptr::copy_nonoverlapping(buffer as *const u8, dest.as_mut_ptr(), actual_len);
                }

                self.avail_index = next_avail;
                self.used_index = index;
                return Ok(actual_len);
            }
            core::hint::spin_loop();
        }

        Err(VirtioRngError::Timeout)
    }
}

fn find_device() -> Option<(crate::pci::PciAddress, u16)> {
    crate::pci::devices()
        .into_iter()
        .find(|device| {
            device.vendor_id == VIRTIO_VENDOR_ID
                && (device.device_id == VIRTIO_RNG_LEGACY_ID
                    || device.device_id == VIRTIO_RNG_MODERN_ID)
        })
        .map(|device| (device.address, device.device_id))
}

fn allocate_contiguous_frames(
    bytes: usize,
    reservations: &mut Vec<PhysFrame>,
) -> Result<Vec<PhysFrame>, VirtioRngError> {
    let count = bytes.div_ceil(PAGE_SIZE as usize);
    let order = count.next_power_of_two().trailing_zeros() as usize;
    loop {
        let base_frame = memory::alloc_frames(order).ok_or(VirtioRngError::AllocationFailed)?;
        if base_frame.start_address() >= DMA_MIN_PHYSICAL {
            let mut frames = Vec::with_capacity(count);
            for i in 0..count {
                frames.push(PhysFrame(base_frame.start_address() + (i as u64) * PAGE_SIZE));
            }
            return Ok(frames);
        }
        reservations.push(base_frame);
    }
}

fn allocate_dma_frame(reservations: &mut Vec<PhysFrame>) -> Result<PhysFrame, VirtioRngError> {
    loop {
        let frame = memory::alloc_frame().ok_or(VirtioRngError::AllocationFailed)?;
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

fn port_read8(base: u16, offset: u16) -> u8 {
    let mut port: Port<u8> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_write8(base: u16, offset: u16, value: u8) {
    let mut port: Port<u8> = Port::new(base + offset);
    unsafe { port.write(value) }
}

fn port_read16(base: u16, offset: u16) -> u16 {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_write16(base: u16, offset: u16, value: u16) {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.write(value) }
}

fn port_write32(base: u16, offset: u16, value: u32) {
    let mut port: Port<u32> = Port::new(base + offset);
    unsafe { port.write(value) }
}
