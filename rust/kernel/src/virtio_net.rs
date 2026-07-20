//! Legacy VirtIO-net driver with polling RX/TX queues for QEMU user networking.

use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};

use x86_64::instructions::port::Port;

use crate::memory::{self, PhysFrame, PAGE_SIZE};
use crate::paging;

const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_NET_LEGACY_ID: u16 = 0x1000;
const VIRTIO_NET_MODERN_ID: u16 = 0x1041;
const QUEUE_SELECT: u16 = 0x0e;
const QUEUE_SIZE: u16 = 0x0c;
const QUEUE_ADDRESS: u16 = 0x08;
const QUEUE_NOTIFY: u16 = 0x10;
const DEVICE_STATUS: u16 = 0x12;
const DEVICE_CONFIG: u16 = 0x14;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const DESC_WRITE: u16 = 2;
const DMA_MIN_PHYSICAL: u64 = 0x10_0000;
const VIRTIO_NET_HEADER_SIZE: usize = 10;
const FRAME_BUFFER_SIZE: usize = PAGE_SIZE as usize;
const RX_BUFFER_COUNT: usize = 8;
const POLL_ATTEMPTS: usize = 1_000_000;
const FEATURE_MAC: u32 = 1 << 5;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

struct Queue {
    physical: u64,
    _frames: Vec<PhysFrame>,
    size: u16,
    avail_offset: usize,
    used_offset: usize,
    avail_index: u16,
    used_index: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioNetError {
    NotFound,
    UnsupportedDevice,
    QueueUnavailable,
    AllocationFailed,
    DeviceFailed,
    FrameTooLarge,
    TransmitTimeout,
}

pub struct VirtioNet {
    io_base: u16,
    mac: [u8; 6],
    tx: Queue,
    rx: Queue,
    tx_buffer: PhysFrame,
    rx_buffers: Vec<PhysFrame>,
    _dma_reservations: Vec<PhysFrame>,
}

impl VirtioNet {
    pub fn probe() -> Result<Self, VirtioNetError> {
        let (address, device_id) = find_device().ok_or(VirtioNetError::NotFound)?;
        if device_id == VIRTIO_NET_MODERN_ID {
            return Err(VirtioNetError::UnsupportedDevice);
        }
        let bar = crate::pci::read_u32(address, 0x10);
        if bar & 1 == 0 {
            return Err(VirtioNetError::UnsupportedDevice);
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
        port_write16(io_base, QUEUE_SELECT, 0);
        port_write32(io_base, QUEUE_ADDRESS, 0);
        write_status(io_base, STATUS_ACKNOWLEDGE);
        write_status(io_base, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        let host_features = port_read32(io_base, 0);
        port_write32(io_base, 0x04, host_features & FEATURE_MAC);

        let mut reservations = Vec::new();
        let rx = configure_queue(io_base, 0, &mut reservations)?;
        let tx = configure_queue(io_base, 1, &mut reservations)?;
        let tx_buffer = allocate_dma_frame(&mut reservations)?;
        zero_physical(tx_buffer.start_address(), FRAME_BUFFER_SIZE);
        let rx_count = RX_BUFFER_COUNT.min(rx.size as usize);
        let mut rx_buffers = Vec::with_capacity(rx_count);
        for _ in 0..rx_count {
            let frame = allocate_dma_frame(&mut reservations)?;
            zero_physical(frame.start_address(), FRAME_BUFFER_SIZE);
            rx_buffers.push(frame);
        }

        let mut device = Self {
            io_base,
            mac: read_mac(io_base),
            tx,
            rx,
            tx_buffer,
            rx_buffers,
            _dma_reservations: reservations,
        };
        device.prime_receive_queue();
        port_write8(
            io_base,
            DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
        if port_read8(io_base, DEVICE_STATUS) & 0x80 != 0 {
            return Err(VirtioNetError::DeviceFailed);
        }
        Ok(device)
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), VirtioNetError> {
        if frame.len() + VIRTIO_NET_HEADER_SIZE > FRAME_BUFFER_SIZE {
            return Err(VirtioNetError::FrameTooLarge);
        }
        let buffer = paging::phys_to_virt(self.tx_buffer.start_address())
            .ok_or(VirtioNetError::AllocationFailed)?;
        unsafe {
            core::ptr::write_bytes(buffer as *mut u8, 0, VIRTIO_NET_HEADER_SIZE);
            core::ptr::copy_nonoverlapping(
                frame.as_ptr(),
                (buffer + VIRTIO_NET_HEADER_SIZE as u64) as *mut u8,
                frame.len(),
            );
        }
        let queue = queue_virtual(&self.tx)?;
        unsafe {
            (queue as *mut Descriptor).write_volatile(Descriptor {
                address: self.tx_buffer.start_address(),
                length: (VIRTIO_NET_HEADER_SIZE + frame.len()) as u32,
                flags: 0,
                next: 0,
            });
        }
        publish_descriptor(&mut self.tx, queue, 0);
        port_write16(self.io_base, QUEUE_NOTIFY, 1);
        for _ in 0..POLL_ATTEMPTS {
            if used_available(&self.tx, queue) {
                self.tx.used_index = self.tx.used_index.wrapping_add(1);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(VirtioNetError::TransmitTimeout)
    }

    pub fn receive(&mut self) -> Result<Option<Vec<u8>>, VirtioNetError> {
        let queue = queue_virtual(&self.rx)?;
        if !used_available(&self.rx, queue) {
            return Ok(None);
        }
        let used = (queue + self.rx.used_offset as u64) as *const u8;
        let slot = self.rx.used_index as usize % self.rx.size as usize;
        let descriptor = unsafe { (used.add(4 + 8 * slot) as *const u32).read_volatile() } as usize;
        let length = unsafe { (used.add(8 + 8 * slot) as *const u32).read_volatile() } as usize;
        if descriptor >= self.rx_buffers.len() {
            return Err(VirtioNetError::DeviceFailed);
        }
        self.rx.used_index = self.rx.used_index.wrapping_add(1);
        let payload_length = length
            .saturating_sub(VIRTIO_NET_HEADER_SIZE)
            .min(FRAME_BUFFER_SIZE.saturating_sub(VIRTIO_NET_HEADER_SIZE));
        let buffer = paging::phys_to_virt(self.rx_buffers[descriptor].start_address())
            .ok_or(VirtioNetError::AllocationFailed)?;
        let mut frame = Vec::with_capacity(payload_length);
        unsafe {
            frame.set_len(payload_length);
            core::ptr::copy_nonoverlapping(
                (buffer + VIRTIO_NET_HEADER_SIZE as u64) as *const u8,
                frame.as_mut_ptr(),
                payload_length,
            );
        }
        publish_descriptor(&mut self.rx, queue, descriptor as u16);
        port_write16(self.io_base, QUEUE_NOTIFY, 0);
        Ok(Some(frame))
    }

    fn prime_receive_queue(&mut self) {
        let queue = queue_virtual(&self.rx).expect("RX queue not mapped");
        for (index, frame) in self.rx_buffers.iter().enumerate() {
            unsafe {
                (queue as *mut Descriptor)
                    .add(index)
                    .write_volatile(Descriptor {
                        address: frame.start_address(),
                        length: FRAME_BUFFER_SIZE as u32,
                        flags: DESC_WRITE,
                        next: 0,
                    });
            }
            publish_descriptor(&mut self.rx, queue, index as u16);
        }
        port_write16(self.io_base, QUEUE_NOTIFY, 0);
    }
}

fn configure_queue(
    io_base: u16,
    index: u16,
    reservations: &mut Vec<PhysFrame>,
) -> Result<Queue, VirtioNetError> {
    port_write16(io_base, QUEUE_SELECT, index);
    let size = port_read16(io_base, QUEUE_SIZE) as usize;
    if size == 0 {
        return Err(VirtioNetError::QueueUnavailable);
    }
    let avail_offset = 16 * size;
    let used_offset = align_up(avail_offset + 6 + 2 * size, PAGE_SIZE as usize);
    let bytes = used_offset + 6 + 8 * size;
    let frames = allocate_contiguous_frames(bytes, reservations)?;
    let physical = frames[0].start_address();
    zero_physical(physical, frames.len() * PAGE_SIZE as usize);
    port_write32(io_base, QUEUE_ADDRESS, (physical / PAGE_SIZE) as u32);
    Ok(Queue {
        physical,
        _frames: frames,
        size: size as u16,
        avail_offset,
        used_offset,
        avail_index: 0,
        used_index: 0,
    })
}

fn publish_descriptor(queue: &mut Queue, queue_virtual: u64, descriptor: u16) {
    let avail = (queue_virtual + queue.avail_offset as u64) as *mut u16;
    let slot = queue.avail_index as usize % queue.size as usize;
    unsafe {
        avail.add(2 + slot).write_volatile(descriptor);
    }
    fence(Ordering::SeqCst);
    queue.avail_index = queue.avail_index.wrapping_add(1);
    unsafe { avail.add(1).write_volatile(queue.avail_index) };
    fence(Ordering::SeqCst);
}

fn used_available(queue: &Queue, queue_virtual: u64) -> bool {
    fence(Ordering::SeqCst);
    (unsafe {
        ((queue_virtual + queue.used_offset as u64) as *const u16)
            .add(1)
            .read_volatile()
    }) != queue.used_index
}

fn queue_virtual(queue: &Queue) -> Result<u64, VirtioNetError> {
    paging::phys_to_virt(queue.physical).ok_or(VirtioNetError::AllocationFailed)
}

fn find_device() -> Option<(crate::pci::PciAddress, u16)> {
    crate::pci::devices()
        .into_iter()
        .find(|device| {
            device.vendor_id == VIRTIO_VENDOR_ID
                && (device.device_id == VIRTIO_NET_LEGACY_ID
                    || device.device_id == VIRTIO_NET_MODERN_ID)
        })
        .map(|device| (device.address, device.device_id))
}

fn read_mac(base: u16) -> [u8; 6] {
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = port_read8(base, DEVICE_CONFIG + index as u16);
    }
    mac
}

fn allocate_contiguous_frames(
    bytes: usize,
    reservations: &mut Vec<PhysFrame>,
) -> Result<Vec<PhysFrame>, VirtioNetError> {
    let count = bytes.div_ceil(PAGE_SIZE as usize);
    let mut frames: Vec<PhysFrame> = Vec::with_capacity(count);
    for _ in 0..count {
        let frame = allocate_dma_frame(reservations)?;
        if let Some(previous) = frames.last() {
            if frame.start_address() != previous.start_address() + PAGE_SIZE {
                frames.push(frame);
                for frame in frames.drain(..) {
                    let _ = memory::free_frame(frame);
                }
                return Err(VirtioNetError::AllocationFailed);
            }
        }
        frames.push(frame);
    }
    Ok(frames)
}

fn allocate_dma_frame(reservations: &mut Vec<PhysFrame>) -> Result<PhysFrame, VirtioNetError> {
    loop {
        let frame = memory::alloc_frame().ok_or(VirtioNetError::AllocationFailed)?;
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

fn port_read16(base: u16, offset: u16) -> u16 {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_read32(base: u16, offset: u16) -> u32 {
    let mut port: Port<u32> = Port::new(base + offset);
    unsafe { port.read() }
}

fn port_write8(base: u16, offset: u16, value: u8) {
    let mut port: Port<u8> = Port::new(base + offset);
    unsafe { port.write(value) };
}

fn port_write16(base: u16, offset: u16, value: u16) {
    let mut port: Port<u16> = Port::new(base + offset);
    unsafe { port.write(value) };
}

fn port_write32(base: u16, offset: u16, value: u32) {
    let mut port: Port<u32> = Port::new(base + offset);
    unsafe { port.write(value) };
}
