//! Bootloader-mediated application-processor handoff.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use limine::request::MpResponse;
use spin::Mutex;

use crate::process::Process;

static AP_ONLINE: AtomicUsize = AtomicUsize::new(0);
static WORK_READY: AtomicBool = AtomicBool::new(false);
static WORK_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static WORK_QUEUE: Mutex<usize> = Mutex::new(0);
static USER_TASKS: Mutex<Vec<Box<Process>>> = Mutex::new(Vec::new());
static USER_TASKS_DISPATCHED: AtomicUsize = AtomicUsize::new(0);
static AP_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);

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
    USER_TASKS_DISPATCHED.store(0, Ordering::Release);
    AP_TIMER_TICKS.store(0, Ordering::Release);
    *WORK_QUEUE.lock() = 0;
    USER_TASKS.lock().clear();
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
    let apic = crate::apic::initialize();
    let timer_ready = crate::apic::initialize_timer(100);
    crate::serial_println!(
        "[smp] AP slot={} lapic={} apic={:?} local-timer={}",
        slot,
        apic.lapic_id,
        apic.mode,
        timer_ready
    );
    AP_ONLINE.fetch_add(1, Ordering::Release);
    while !WORK_READY.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    if take_work() {
        WORK_COMPLETED.fetch_add(1, Ordering::Release);
    }
    ap_idle()
}

pub fn enqueue_user_tasks(mut tasks: Vec<Box<Process>>) -> bool {
    if AP_ONLINE.load(Ordering::Acquire) == 0 {
        return false;
    }
    let count = tasks.len();
    if count == 0 {
        return true;
    }
    USER_TASKS.lock().append(&mut tasks);
    USER_TASKS_DISPATCHED.fetch_add(count, Ordering::Release);
    true
}

pub fn dispatched_user_tasks() -> usize {
    USER_TASKS_DISPATCHED.load(Ordering::Acquire)
}

pub fn is_application_processor() -> bool {
    crate::syscall::current_cpu_index() != 0
}

pub fn on_user_task_complete() {
    crate::serial_println!(
        "[smp] AP cpu={} run queue complete dispatched={} timer-ticks={}",
        crate::syscall::current_cpu_index(),
        USER_TASKS_DISPATCHED.load(Ordering::Acquire),
        AP_TIMER_TICKS.load(Ordering::Acquire)
    );
}

pub fn note_ap_timer_tick() {
    AP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn ap_idle() -> ! {
    loop {
        let tasks = {
            let mut queued = USER_TASKS.lock();
            if queued.is_empty() {
                None
            } else {
                Some(core::mem::take(&mut *queued))
            }
        };
        if let Some(tasks) = tasks {
            x86_64::instructions::interrupts::enable();
            unsafe { crate::scheduler::start_ap(tasks) }
        }
        core::hint::spin_loop();
    }
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
