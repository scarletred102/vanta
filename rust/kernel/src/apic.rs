//! Local APIC discovery and software-enable setup for the bootstrap CPU.

use core::arch::x86_64::{__cpuid, __cpuid_count, _rdtsc};
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use x86_64::registers::model_specific::Msr;

const IA32_APIC_BASE: u32 = 0x1b;
const X2APIC_ID: u32 = 0x802;
const X2APIC_SPURIOUS_INTERRUPT_VECTOR: u32 = 0x80f;
const X2APIC_LVT_TIMER: u32 = 0x832;
const X2APIC_INITIAL_COUNT: u32 = 0x838;
const X2APIC_DIVIDE_CONFIGURATION: u32 = 0x83e;
const IA32_TSC_DEADLINE: u32 = 0x6e0;
const APIC_ID: u64 = 0x20;
const APIC_SPURIOUS_INTERRUPT_VECTOR: u64 = 0xf0;
const APIC_EOI: u64 = 0xb0;
const APIC_LVT_TIMER: u64 = 0x320;
const APIC_INITIAL_COUNT: u64 = 0x380;
const APIC_DIVIDE_CONFIGURATION: u64 = 0x3e0;
const LAPIC_VIRTUAL_BASE: u64 = 0xffff_ffff_fee0_0000;
const APIC_ENABLED: u64 = 1 << 11;
const X2APIC_ENABLED: u64 = 1 << 10;
const SOFTWARE_ENABLED: u32 = 1 << 8;
const TIMER_VECTOR: u32 = 32;
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_TSC_DEADLINE: u32 = 2 << 17;
const TIMER_DIVIDE_16: u32 = 0x3;
static ACTIVE_MODE: AtomicU8 = AtomicU8::new(0);
static TIMER_MODE: AtomicU8 = AtomicU8::new(0);
static TIMER_TSC_DELTA: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicMode {
    Unavailable,
    Disabled,
    XApic,
    X2Apic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApicInfo {
    pub mode: ApicMode,
    pub physical_base: u64,
    pub lapic_id: u32,
    pub x2apic_supported: bool,
}

pub fn initialize() -> ApicInfo {
    let features = __cpuid(1);
    let x2apic_supported = features.ecx & (1 << 21) != 0;
    if features.edx & (1 << 9) == 0 {
        let info = ApicInfo {
            mode: ApicMode::Unavailable,
            physical_base: 0,
            lapic_id: 0,
            x2apic_supported,
        };
        ACTIVE_MODE.store(0, Ordering::Release);
        return info;
    }

    let base_msr = Msr::new(IA32_APIC_BASE);
    let base = unsafe { base_msr.read() };
    let physical_base = base & 0x000f_ffff_ffff_f000;
    if base & APIC_ENABLED == 0 {
        let info = ApicInfo {
            mode: ApicMode::Disabled,
            physical_base,
            lapic_id: 0,
            x2apic_supported,
        };
        ACTIVE_MODE.store(0, Ordering::Release);
        return info;
    }
    if base & X2APIC_ENABLED != 0 {
        let mut spurious = Msr::new(X2APIC_SPURIOUS_INTERRUPT_VECTOR);
        let value = unsafe { spurious.read() };
        unsafe {
            spurious.write((value & !0xff) | SOFTWARE_ENABLED as u64 | 0xff);
        }
        let id = unsafe { Msr::new(X2APIC_ID).read() } as u32;
        let info = ApicInfo {
            mode: ApicMode::X2Apic,
            physical_base,
            lapic_id: id,
            x2apic_supported,
        };
        ACTIVE_MODE.store(2, Ordering::Release);
        return info;
    }

    let mapped = crate::paging::map(
        crate::paging::current_address_space(),
        LAPIC_VIRTUAL_BASE,
        physical_base,
        crate::paging::MAP_WRITABLE | crate::paging::MAP_CACHE_DISABLE,
    );
    if !matches!(mapped, Ok(()) | Err(crate::paging::MapError::AlreadyMapped)) {
        let info = ApicInfo {
            mode: ApicMode::Disabled,
            physical_base,
            lapic_id: 0,
            x2apic_supported,
        };
        ACTIVE_MODE.store(0, Ordering::Release);
        return info;
    }
    let spurious = (LAPIC_VIRTUAL_BASE + APIC_SPURIOUS_INTERRUPT_VECTOR) as *mut u32;
    let value = unsafe { spurious.read_volatile() };
    unsafe {
        spurious.write_volatile((value & !0xff) | SOFTWARE_ENABLED | 0xff);
    }
    let id = unsafe { ((LAPIC_VIRTUAL_BASE + APIC_ID) as *const u32).read_volatile() >> 24 };
    ACTIVE_MODE.store(1, Ordering::Release);
    ApicInfo {
        mode: ApicMode::XApic,
        physical_base,
        lapic_id: id,
        x2apic_supported,
    }
}

pub fn end_of_interrupt() {
    match ACTIVE_MODE.load(Ordering::Acquire) {
        1 => unsafe { ((LAPIC_VIRTUAL_BASE + APIC_EOI) as *mut u32).write_volatile(0) },
        2 => unsafe { Msr::new(0x80b).write(0) },
        _ => {}
    }
}

/// Configure a per-CPU timer. TSC-deadline mode is preferred because its rate
/// can be derived from CPUID; old APICs fall back to a conservative periodic
/// count that is sufficient for QEMU and remains an explicit calibration gap
/// for physical hardware.
pub fn initialize_timer(frequency_hz: u32) -> bool {
    if frequency_hz == 0 || ACTIVE_MODE.load(Ordering::Acquire) == 0 {
        return false;
    }

    if let Some(tsc_hz) = tsc_frequency_hz() {
        let delta = tsc_hz / frequency_hz as u64;
        if delta != 0 && __cpuid(1).ecx & (1 << 24) != 0 {
            write_lvt_timer(TIMER_VECTOR | TIMER_TSC_DEADLINE);
            TIMER_TSC_DELTA.store(delta, Ordering::Release);
            TIMER_MODE.store(1, Ordering::Release);
            rearm_timer();
            return true;
        }
    }

    write_divide_configuration(TIMER_DIVIDE_16);
    write_lvt_timer(TIMER_VECTOR | TIMER_PERIODIC);
    write_initial_count(62_500);
    TIMER_MODE.store(2, Ordering::Release);
    true
}

pub fn rearm_timer() {
    if TIMER_MODE.load(Ordering::Acquire) != 1 {
        return;
    }
    let deadline = unsafe { _rdtsc() }.wrapping_add(TIMER_TSC_DELTA.load(Ordering::Acquire));
    unsafe { Msr::new(IA32_TSC_DEADLINE).write(deadline) };
}

fn write_lvt_timer(value: u32) {
    match ACTIVE_MODE.load(Ordering::Acquire) {
        1 => unsafe { ((LAPIC_VIRTUAL_BASE + APIC_LVT_TIMER) as *mut u32).write_volatile(value) },
        2 => unsafe { Msr::new(X2APIC_LVT_TIMER).write(value as u64) },
        _ => {}
    }
}

fn write_initial_count(value: u32) {
    match ACTIVE_MODE.load(Ordering::Acquire) {
        1 => unsafe {
            ((LAPIC_VIRTUAL_BASE + APIC_INITIAL_COUNT) as *mut u32).write_volatile(value)
        },
        2 => unsafe { Msr::new(X2APIC_INITIAL_COUNT).write(value as u64) },
        _ => {}
    }
}

fn write_divide_configuration(value: u32) {
    match ACTIVE_MODE.load(Ordering::Acquire) {
        1 => unsafe {
            ((LAPIC_VIRTUAL_BASE + APIC_DIVIDE_CONFIGURATION) as *mut u32).write_volatile(value)
        },
        2 => unsafe { Msr::new(X2APIC_DIVIDE_CONFIGURATION).write(value as u64) },
        _ => {}
    }
}

fn tsc_frequency_hz() -> Option<u64> {
    if __cpuid(0).eax >= 0x15 {
        let leaf = __cpuid_count(0x15, 0);
        if leaf.eax != 0 && leaf.ebx != 0 && leaf.ecx != 0 {
            return (leaf.ecx as u64)
                .checked_mul(leaf.ebx as u64)
                .map(|value| value / leaf.eax as u64)
                .filter(|value| *value != 0);
        }
    }
    if __cpuid(0).eax >= 0x16 {
        let base_mhz = __cpuid_count(0x16, 0).eax as u64;
        return base_mhz.checked_mul(1_000_000).filter(|value| *value != 0);
    }
    None
}
