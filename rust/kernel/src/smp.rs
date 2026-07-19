//! Bootloader-mediated application-processor handoff.

use core::sync::atomic::{AtomicUsize, Ordering};

use limine::request::MpResponse;

static AP_ONLINE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SmpInfo {
    pub reported_cpus: usize,
    pub requested_aps: usize,
    pub online_aps: usize,
    pub bsp_lapic_id: u32,
    pub x2apic_enabled: bool,
}

pub fn bootstrap(response: Option<&MpResponse>) -> SmpInfo {
    let Some(response) = response else {
        return SmpInfo {
            reported_cpus: 1,
            requested_aps: 0,
            online_aps: 0,
            bsp_lapic_id: 0,
            x2apic_enabled: false,
        };
    };

    AP_ONLINE.store(0, Ordering::Release);
    let mut requested_aps = 0;
    for cpu in response.cpus() {
        if cpu.lapic_id != response.bsp_lapic_id {
            requested_aps += 1;
            cpu.bootstrap(application_processor_entry, cpu.lapic_id as u64);
        }
    }
    for _ in 0..10_000_000 {
        if AP_ONLINE.load(Ordering::Acquire) == requested_aps {
            break;
        }
        core::hint::spin_loop();
    }

    SmpInfo {
        reported_cpus: response.cpus().len(),
        requested_aps,
        online_aps: AP_ONLINE.load(Ordering::Acquire),
        bsp_lapic_id: response.bsp_lapic_id,
        x2apic_enabled: response.flags & 1 != 0,
    }
}

unsafe extern "C" fn application_processor_entry(cpu: &limine::mp::MpInfo) -> ! {
    let _lapic_id = cpu.extra_argument();
    AP_ONLINE.fetch_add(1, Ordering::Release);
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
