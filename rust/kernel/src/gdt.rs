use core::arch::asm;
use lazy_static::lazy_static;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5;

static mut DOUBLE_FAULT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
static mut KERNEL_INTERRUPT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK));
            stack_start + STACK_SIZE as u64
        };
        tss.privilege_stack_table[0] = {
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(KERNEL_INTERRUPT_STACK));
            stack_start + STACK_SIZE as u64
        };
        tss
    };
}

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&TSS));
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        (
            gdt,
            Selectors {
                code,
                data,
                tss,
                user_code,
                user_data,
            },
        )
    };
}

pub fn init() {
    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code);
        DS::set_reg(GDT.1.data);
        ES::set_reg(GDT.1.data);
        SS::set_reg(GDT.1.data);
        load_tss(GDT.1.tss);
    }
}

pub fn syscall_selectors() -> (
    SegmentSelector,
    SegmentSelector,
    SegmentSelector,
    SegmentSelector,
) {
    (GDT.1.user_code, GDT.1.user_data, GDT.1.code, GDT.1.data)
}

pub fn user_interrupt_selectors() -> (u64, u64) {
    (GDT.1.user_code.0 as u64, GDT.1.user_data.0 as u64)
}

pub unsafe fn enter_user(entry: u64, stack: u64) -> ! {
    let user_code = GDT.1.user_code.0 as u64;
    let user_data = GDT.1.user_data.0 as u64;
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
