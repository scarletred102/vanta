//! Bootloader-mediated application-processor handoff.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use limine::request::MpResponse;
use spin::Mutex;

use crate::process::Process;

static AP_ONLINE: AtomicUsize = AtomicUsize::new(0);
static WORK_READY: AtomicBool = AtomicBool::new(false);
static WORK_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static WORK_QUEUE: Mutex<usize> = Mutex::new(0);
static USER_TASK_READY: AtomicBool = AtomicBool::new(false);
static USER_TASK_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static USER_TASK: Mutex<Option<Box<Process>>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SmpInfo {
    pub reported_cpus: usize,
    pub prepared_cpus: usize,
    pub requested_aps: usize,
    pub online_aps: usize,
    pub bsp_lapic_id: u32,
    pub x2apic_enabled: bool,
    pub work_completed: usize,
}

pub fn reported_cpu_count(response: Option<&MpResponse>) -> usize {
    response.map_or(1, |response| response.cpus().len().max(1))
}

pub fn bootstrap(response: Option<&MpResponse>, prepared_cpus: usize) -> SmpInfo {
    let Some(response) = response else {
        return SmpInfo {
            reported_cpus: 1,
            prepared_cpus,
            requested_aps: 0,
            online_aps: 0,
            bsp_lapic_id: 0,
            x2apic_enabled: false,
            work_completed: 0,
        };
    };

    AP_ONLINE.store(0, Ordering::Release);
    WORK_READY.store(false, Ordering::Release);
    WORK_COMPLETED.store(0, Ordering::Release);
    USER_TASK_READY.store(false, Ordering::Release);
    USER_TASK_COMPLETED.store(0, Ordering::Release);
    *WORK_QUEUE.lock() = 0;
    *USER_TASK.lock() = None;
    let mut requested_aps = 0;
    for cpu in response.cpus() {
        if cpu.lapic_id != response.bsp_lapic_id {
            let slot = requested_aps + 1;
            if slot >= prepared_cpus {
                continue;
            }
            requested_aps += 1;
            cpu.bootstrap(application_processor_entry, slot as u64);
        }
    }
    for _ in 0..10_000_000 {
        if AP_ONLINE.load(Ordering::Acquire) == requested_aps {
            break;
        }
        core::hint::spin_loop();
    }
    *WORK_QUEUE.lock() = requested_aps;
    WORK_READY.store(true, Ordering::Release);
    for _ in 0..10_000_000 {
        if WORK_COMPLETED.load(Ordering::Acquire) == requested_aps {
            break;
        }
        core::hint::spin_loop();
    }

    SmpInfo {
        reported_cpus: response.cpus().len(),
        prepared_cpus,
        requested_aps,
        online_aps: AP_ONLINE.load(Ordering::Acquire),
        bsp_lapic_id: response.bsp_lapic_id,
        x2apic_enabled: response.flags & 1 != 0,
        work_completed: WORK_COMPLETED.load(Ordering::Acquire),
    }
}

unsafe extern "C" fn application_processor_entry(cpu: &limine::mp::MpInfo) -> ! {
    let slot = cpu.extra_argument() as usize;
    if !crate::gdt::initialize_application_processor(slot) {
        halt();
    }
    crate::interrupts::init_idt();
    if !crate::syscall::init() {
        halt();
    }
    AP_ONLINE.fetch_add(1, Ordering::Release);
    while !WORK_READY.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    if take_work() {
        WORK_COMPLETED.fetch_add(1, Ordering::Release);
    }
    loop {
        while !USER_TASK_READY.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
        let task = USER_TASK.lock().take();
        if let Some(task) = task {
            crate::scheduler::start_ap_test(task)
        }
        USER_TASK_READY.store(false, Ordering::Release);
    }
}

pub fn run_user_task(task: Box<Process>) -> bool {
    if AP_ONLINE.load(Ordering::Acquire) == 0 || USER_TASK_READY.load(Ordering::Acquire) {
        return false;
    }
    *USER_TASK.lock() = Some(task);
    USER_TASK_COMPLETED.store(0, Ordering::Release);
    USER_TASK_READY.store(true, Ordering::Release);
    loop {
        if USER_TASK_COMPLETED.load(Ordering::Acquire) != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
}

pub fn ap_user_task_active() -> bool {
    USER_TASK_READY.load(Ordering::Acquire)
}

pub fn finish_user_task() -> ! {
    USER_TASK_READY.store(false, Ordering::Release);
    USER_TASK_COMPLETED.fetch_add(1, Ordering::Release);
    halt()
}

fn take_work() -> bool {
    let mut pending = WORK_QUEUE.lock();
    if *pending == 0 {
        return false;
    }
    *pending -= 1;
    true
}

fn halt() -> ! {
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
