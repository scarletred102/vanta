//! Legacy PCI configuration-space discovery for x86 platforms.

use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::port::Port;

const CONFIG_ADDRESS_PORT: u16 = 0xcf8;
const CONFIG_DATA_PORT: u16 = 0xcfc;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static ECAM_REGION: Mutex<Option<crate::acpi::PciEcamRegion>> = Mutex::new(None);
const ECAM_WINDOW: u64 = 0xffff_fea0_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciDevice {
    pub address: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub programming_interface: u8,
}

pub fn devices() -> Vec<PciDevice> {
    let mut devices = Vec::new();
    for bus in 0..=u8::MAX {
        for device in 0..32 {
            let address = PciAddress {
                bus,
                device,
                function: 0,
            };
            let Some(first_function) = device_at(address) else {
                continue;
            };
            devices.push(first_function);
            if read_u32(address, 0x0c) & (1 << 23) == 0 {
                continue;
            }
            for function in 1..8 {
                let address = PciAddress {
                    bus,
                    device,
                    function,
                };
                if let Some(found) = device_at(address) {
                    devices.push(found);
                }
            }
        }
    }
    devices
}

pub fn configure_ecam(region: crate::acpi::PciEcamRegion) -> bool {
    if region.segment_group != 0
        || region.start_bus > region.end_bus
        || region.base_address & 0xfff != 0
    {
        return false;
    }
    *ECAM_REGION.lock() = Some(region);
    true
}

pub fn self_check() -> bool {
    config_address(
        PciAddress {
            bus: 0,
            device: 0,
            function: 0,
        },
        0,
    ) == 0x8000_0000
        && config_address(
            PciAddress {
                bus: u8::MAX,
                device: 31,
                function: 7,
            },
            0xfc,
        ) == 0x80ff_fffc
}

pub fn read_u32(address: PciAddress, offset: u8) -> u32 {
    assert!(offset & 3 == 0, "unaligned PCI configuration read");
    let _lock = CONFIG_LOCK.lock();
    if let Some(region) = *ECAM_REGION.lock() {
        return ecam_read(region, address, offset).unwrap_or(u32::MAX);
    }
    let mut address_port: Port<u32> = Port::new(CONFIG_ADDRESS_PORT);
    let mut data_port: Port<u32> = Port::new(CONFIG_DATA_PORT);
    unsafe {
        address_port.write(config_address(address, offset));
        data_port.read()
    }
}

pub fn write_u32(address: PciAddress, offset: u8, value: u32) {
    assert!(offset & 3 == 0, "unaligned PCI configuration write");
    let _lock = CONFIG_LOCK.lock();
    if let Some(region) = *ECAM_REGION.lock() {
        let _ = ecam_write(region, address, offset, value);
        return;
    }
    let mut address_port: Port<u32> = Port::new(CONFIG_ADDRESS_PORT);
    let mut data_port: Port<u32> = Port::new(CONFIG_DATA_PORT);
    unsafe {
        address_port.write(config_address(address, offset));
        data_port.write(value);
    }
}

fn ecam_read(region: crate::acpi::PciEcamRegion, address: PciAddress, offset: u8) -> Option<u32> {
    let physical = ecam_address(region, address, offset)?;
    map_ecam_page(physical)?;
    let value = unsafe { ((ECAM_WINDOW + (physical & 0xfff)) as *const u32).read_volatile() };
    let _ = crate::paging::unmap(crate::paging::current_address_space(), ECAM_WINDOW);
    Some(value)
}

fn ecam_write(
    region: crate::acpi::PciEcamRegion,
    address: PciAddress,
    offset: u8,
    value: u32,
) -> Option<()> {
    let physical = ecam_address(region, address, offset)?;
    map_ecam_page(physical)?;
    unsafe { ((ECAM_WINDOW + (physical & 0xfff)) as *mut u32).write_volatile(value) };
    let _ = crate::paging::unmap(crate::paging::current_address_space(), ECAM_WINDOW);
    Some(())
}

fn ecam_address(
    region: crate::acpi::PciEcamRegion,
    address: PciAddress,
    offset: u8,
) -> Option<u64> {
    if address.bus < region.start_bus || address.bus > region.end_bus {
        return None;
    }
    region
        .base_address
        .checked_add(((address.bus - region.start_bus) as u64) << 20)?
        .checked_add((address.device as u64) << 15)?
        .checked_add((address.function as u64) << 12)?
        .checked_add((offset & 0xfc) as u64)
}

fn map_ecam_page(physical: u64) -> Option<()> {
    if crate::paging::translate(ECAM_WINDOW).is_some() {
        return None;
    }
    crate::paging::map(
        crate::paging::current_address_space(),
        ECAM_WINDOW,
        physical & !0xfff,
        crate::paging::MAP_WRITABLE | crate::paging::MAP_CACHE_DISABLE,
    )
    .ok()?;
    Some(())
}

fn device_at(address: PciAddress) -> Option<PciDevice> {
    let id = read_u32(address, 0);
    let vendor_id = id as u16;
    if vendor_id == u16::MAX {
        return None;
    }
    let class = read_u32(address, 8);
    Some(PciDevice {
        address,
        vendor_id,
        device_id: (id >> 16) as u16,
        class_code: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        programming_interface: (class >> 8) as u8,
    })
}

const fn config_address(address: PciAddress, offset: u8) -> u32 {
    0x8000_0000
        | ((address.bus as u32) << 16)
        | ((address.device as u32) << 11)
        | ((address.function as u32) << 8)
        | (offset as u32 & 0xfc)
}
