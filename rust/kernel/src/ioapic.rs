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

use spin::Mutex;

struct IoApicState {
    base: u32,
    entries: u32,
    lapic_id: u8,
}

static STATE: Mutex<Option<IoApicState>> = Mutex::new(None);

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
    *STATE.lock() = Some(IoApicState {
        base: descriptor.global_irq_base,
        entries,
        lapic_id: lapic_id as u8,
    });
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

pub fn route_irq(
    gsi: u32,
    vector: u8,
    level_triggered: bool,
    active_low: bool,
) -> Result<(), IoApicError> {
    let state = STATE.lock();
    let state = state.as_ref().ok_or(IoApicError::Missing)?;
    let index = gsi
        .checked_sub(state.base)
        .filter(|index| *index < state.entries)
        .ok_or(IoApicError::GsiOutOfRange)?;
    let register = REDIRECTION_BASE.wrapping_add((index * 2) as u8);
    let mut flags = vector as u32;
    if active_low {
        flags |= 1 << 13;
    }
    if level_triggered {
        flags |= 1 << 15;
    }
    write(register + 1, (state.lapic_id as u32) << 24);
    write(register, flags);
    Ok(())
}

pub fn mask_irq(gsi: u32, mask: bool) -> Result<(), IoApicError> {
    let state = STATE.lock();
    let state = state.as_ref().ok_or(IoApicError::Missing)?;
    let index = gsi
        .checked_sub(state.base)
        .filter(|index| *index < state.entries)
        .ok_or(IoApicError::GsiOutOfRange)?;
    let register = REDIRECTION_BASE.wrapping_add((index * 2) as u8);
    let current = read(register);
    if mask {
        write(register, current | (1 << 16));
    } else {
        write(register, current & !(1 << 16));
    }
    Ok(())
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
