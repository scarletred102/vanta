//! Local APIC discovery and software-enable setup for the bootstrap CPU.

use core::arch::x86_64::__cpuid;
use core::sync::atomic::{AtomicU8, Ordering};

use x86_64::registers::model_specific::Msr;

const IA32_APIC_BASE: u32 = 0x1b;
const X2APIC_ID: u32 = 0x802;
const X2APIC_SPURIOUS_INTERRUPT_VECTOR: u32 = 0x80f;
const APIC_ID: u64 = 0x20;
const APIC_SPURIOUS_INTERRUPT_VECTOR: u64 = 0xf0;
const APIC_EOI: u64 = 0xb0;
const LAPIC_VIRTUAL_BASE: u64 = 0xffff_ffff_fee0_0000;
const APIC_ENABLED: u64 = 1 << 11;
const X2APIC_ENABLED: u64 = 1 << 10;
const SOFTWARE_ENABLED: u32 = 1 << 8;
static ACTIVE_MODE: AtomicU8 = AtomicU8::new(0);

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
