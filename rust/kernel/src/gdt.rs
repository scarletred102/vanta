use lazy_static::lazy_static;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::{CS, Segment, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5;

static mut DOUBLE_FAULT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK));
            stack_start + STACK_SIZE as u64
        };
        tss
    };
}

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&TSS));
        (gdt, Selectors { code, data, tss })
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
