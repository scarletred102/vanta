use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::asm;

use spin::Mutex;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const MAX_CPUS: usize = 8;
const STACK_SIZE: usize = 4096 * 5;

#[repr(align(16))]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct KernelStack([u8; STACK_SIZE]);

static mut DOUBLE_FAULT_STACKS: [KernelStack; MAX_CPUS] = [KernelStack([0; STACK_SIZE]); MAX_CPUS];
static mut KERNEL_INTERRUPT_STACKS: [KernelStack; MAX_CPUS] =
    [KernelStack([0; STACK_SIZE]); MAX_CPUS];

#[derive(Clone, Copy)]
struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
}

struct PerCpuGdt {
    gdt: GlobalDescriptorTable,
    selectors: Selectors,
    _tss: TaskStateSegment,
}

static PER_CPU_GDTS: Mutex<Vec<&'static PerCpuGdt>> = Mutex::new(Vec::new());

pub fn initialize_bootstrap(reported_cpus: usize) -> usize {
    let cpu_count = reported_cpus.clamp(1, MAX_CPUS);
    let mut states = PER_CPU_GDTS.lock();
    if states.is_empty() {
        for index in 0..cpu_count {
            states.push(new_per_cpu_gdt(index));
        }
    }
    let prepared = states.len();
    drop(states);
    assert!(load_cpu(0), "missing bootstrap CPU GDT");
    prepared
}

pub fn initialize_application_processor(index: usize) -> bool {
    load_cpu(index)
}

fn new_per_cpu_gdt(index: usize) -> &'static PerCpuGdt {
    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_top(
        core::ptr::addr_of!(DOUBLE_FAULT_STACKS).cast::<KernelStack>(),
        index,
    );
    tss.privilege_stack_table[0] = stack_top(
        core::ptr::addr_of!(KERNEL_INTERRUPT_STACKS).cast::<KernelStack>(),
        index,
    );

    let state = Box::leak(Box::new(PerCpuGdt {
        gdt: GlobalDescriptorTable::new(),
        selectors: Selectors {
            code: SegmentSelector(0),
            data: SegmentSelector(0),
            tss: SegmentSelector(0),
            user_code: SegmentSelector(0),
            user_data: SegmentSelector(0),
        },
        _tss: tss,
    }));
    let tss: &'static TaskStateSegment = &state._tss;
    state.selectors.code = state.gdt.append(Descriptor::kernel_code_segment());
    state.selectors.data = state.gdt.append(Descriptor::kernel_data_segment());
    state.selectors.tss = state.gdt.append(Descriptor::tss_segment(tss));
    state.selectors.user_data = state.gdt.append(Descriptor::user_data_segment());
    state.selectors.user_code = state.gdt.append(Descriptor::user_code_segment());
    state
}

fn stack_top(stacks: *const KernelStack, index: usize) -> VirtAddr {
    let stack = unsafe { stacks.add(index) };
    VirtAddr::from_ptr(stack) + STACK_SIZE as u64
}

fn load_cpu(index: usize) -> bool {
    let states = PER_CPU_GDTS.lock();
    let Some(state) = states.get(index) else {
        return false;
    };
    let state: &'static PerCpuGdt = state;
    state.gdt.load();
    unsafe {
        CS::set_reg(state.selectors.code);
        DS::set_reg(state.selectors.data);
        ES::set_reg(state.selectors.data);
        SS::set_reg(state.selectors.data);
        load_tss(state.selectors.tss);
    }
    crate::syscall::initialize_cpu_local(index)
}

fn bootstrap_selectors() -> Selectors {
    PER_CPU_GDTS
        .lock()
        .first()
        .expect("bootstrap GDT not initialized")
        .selectors
}

pub fn syscall_selectors() -> (
    SegmentSelector,
    SegmentSelector,
    SegmentSelector,
    SegmentSelector,
) {
    let selectors = bootstrap_selectors();
    (
        selectors.user_code,
        selectors.user_data,
        selectors.code,
        selectors.data,
    )
}

pub fn user_interrupt_selectors() -> (u64, u64) {
    let selectors = bootstrap_selectors();
    (selectors.user_code.0 as u64, selectors.user_data.0 as u64)
}

pub unsafe fn enter_user(entry: u64, stack: u64) -> ! {
    let (user_code, user_data) = user_interrupt_selectors();
    let mut rflags: u64;
    unsafe {
        asm!("pushfq; pop {rflags}", rflags = out(reg) rflags, options(nostack));
    }
    rflags |= 1 << 9;

    unsafe {
        asm!(
            "push {user_data}",
            "push {stack}",
            "push {rflags}",
            "push {user_code}",
            "push {entry}",
            "iretq",
            user_data = in(reg) user_data,
            stack = in(reg) stack,
            rflags = in(reg) rflags,
            user_code = in(reg) user_code,
            entry = in(reg) entry,
            options(noreturn)
        );
    }
}
