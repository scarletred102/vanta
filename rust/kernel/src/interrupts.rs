use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{gdt, serial_println};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::{PrivilegeLevel, VirtAddr};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum HwIrq {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl HwIrq {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });
static IOAPIC_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn use_ioapic() {
    IOAPIC_ACTIVE.store(true, Ordering::Release);
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint
            .set_handler_fn(breakpoint_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);
        idt.general_protection_fault.set_handler_fn(gp_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        unsafe {
            idt[HwIrq::Timer.as_u8()].set_handler_addr(VirtAddr::new(
                vanta_timer_entry as *const () as usize as u64,
            ));
        }
        idt[HwIrq::Keyboard.as_u8()].set_handler_fn(keyboard_handler);
        idt
    };
}

extern "C" {
    fn vanta_timer_entry();
}

global_asm!(
    r#"
    .global vanta_timer_entry
    .extern vanta_timer_tick
vanta_timer_entry:
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    call vanta_timer_tick
    mov rsp, rax
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq
"#
);

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(frame: InterruptStackFrame) {
    serial_println!("[user] ring3 breakpoint: {:#?}", frame);
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("DOUBLE FAULT code={} frame={:#?}", code, frame);
}

extern "x86-interrupt" fn gp_handler(frame: InterruptStackFrame, code: u64) {
    panic!("GP FAULT code={:#x} frame={:#?}", code, frame);
}

extern "x86-interrupt" fn page_fault_handler(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let addr = x86_64::registers::control::Cr2::read();
    panic!(
        "PAGE FAULT addr={:?} code={:?} frame={:#?}",
        addr, code, frame
    );
}

pub fn initialize_timer(frequency_hz: u32) -> bool {
    const PIT_INPUT_HZ: u32 = 1_193_182;
    if frequency_hz == 0 || frequency_hz > PIT_INPUT_HZ {
        return false;
    }
    let divisor = (PIT_INPUT_HZ / frequency_hz).clamp(1, u16::MAX as u32) as u16;
    use x86_64::instructions::port::Port;
    let mut command: Port<u8> = Port::new(0x43);
    let mut channel_zero: Port<u8> = Port::new(0x40);
    unsafe {
        command.write(0x36);
        channel_zero.write(divisor as u8);
        channel_zero.write((divisor >> 8) as u8);
    }
    true
}

#[no_mangle]
extern "C" fn vanta_timer_tick(
    context: *mut crate::scheduler::InterruptContext,
) -> *const crate::scheduler::InterruptContext {
    if IOAPIC_ACTIVE.load(Ordering::Acquire) {
        crate::apic::end_of_interrupt();
    } else {
        unsafe { PICS.lock().notify_end_of_interrupt(HwIrq::Timer.as_u8()) };
    }
    if crate::smp::is_application_processor() {
        crate::smp::note_ap_timer_tick();
        crate::apic::rearm_timer();
    }
    crate::scheduler::timer_tick(context)
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let mut data: Port<u8> = Port::new(0x60);
    let scancode: u8 = unsafe { data.read() };
    crate::keyboard::push_scancode(scancode);
    if IOAPIC_ACTIVE.load(Ordering::Acquire) {
        crate::apic::end_of_interrupt();
    } else {
        unsafe { PICS.lock().notify_end_of_interrupt(HwIrq::Keyboard.as_u8()) };
    }
}
