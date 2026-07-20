//! MADT-described IOAPIC routing for legacy timer and keyboard IRQs.

const IOAPIC_WINDOW: u64 = 0xffff_fe90_0000_0000;
const IOREGSEL: u64 = 0;
const IOWIN: u64 = 0x10;
const VERSION: u8 = 1;
const REDIRECTION_BASE: u8 = 0x10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoApicError {
    Missing,
    UnsupportedDestination,
    Map,
    GsiOutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoApicInfo {
    pub entries: u32,
    pub timer_gsi: u32,
    pub keyboard_gsi: u32,
}

pub fn initialize(madt: crate::acpi::MadtInfo, lapic_id: u32) -> Result<IoApicInfo, IoApicError> {
    let descriptor = madt.io_apic.ok_or(IoApicError::Missing)?;
    if lapic_id > u8::MAX as u32 {
        return Err(IoApicError::UnsupportedDestination);
    }
    crate::paging::map(
        crate::paging::current_address_space(),
        IOAPIC_WINDOW,
        descriptor.physical_address as u64,
        crate::paging::MAP_WRITABLE | crate::paging::MAP_CACHE_DISABLE,
    )
    .map_err(|_| IoApicError::Map)?;
    let entries = ((read(VERSION) >> 16) & 0xff) + 1;
    route(
        descriptor.global_irq_base,
        entries,
        madt.timer_gsi,
        32,
        lapic_id as u8,
    )?;
    route(
        descriptor.global_irq_base,
        entries,
        madt.keyboard_gsi,
        33,
        lapic_id as u8,
    )?;
    Ok(IoApicInfo {
        entries,
        timer_gsi: madt.timer_gsi,
        keyboard_gsi: madt.keyboard_gsi,
    })
}

fn route(
    base: u32,
    entries: u32,
    gsi: u32,
    vector: u8,
    destination: u8,
) -> Result<(), IoApicError> {
    let index = gsi
        .checked_sub(base)
        .filter(|index| *index < entries)
        .ok_or(IoApicError::GsiOutOfRange)?;
    let register = REDIRECTION_BASE.wrapping_add((index * 2) as u8);
    write(register + 1, (destination as u32) << 24);
    write(register, vector as u32);
    Ok(())
}

fn read(register: u8) -> u32 {
    unsafe {
        ((IOAPIC_WINDOW + IOREGSEL) as *mut u32).write_volatile(register as u32);
        ((IOAPIC_WINDOW + IOWIN) as *const u32).read_volatile()
    }
}

fn write(register: u8, value: u32) {
    unsafe {
        ((IOAPIC_WINDOW + IOREGSEL) as *mut u32).write_volatile(register as u32);
        ((IOAPIC_WINDOW + IOWIN) as *mut u32).write_volatile(value);
    }
}
