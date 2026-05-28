use crate::{gdt, serial_println};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum HwIrq {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl HwIrq {
    fn as_u8(self) -> u8 { self as u8 }
    fn as_usize(self) -> usize { self as usize }
}

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.general_protection_fault.set_handler_fn(gp_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[HwIrq::Timer.as_u8()].set_handler_fn(timer_handler);
        idt[HwIrq::Keyboard.as_u8()].set_handler_fn(keyboard_handler);
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(frame: InterruptStackFrame) {
    serial_println!("[int] breakpoint: {:#?}", frame);
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("DOUBLE FAULT code={} frame={:#?}", code, frame);
}

extern "x86-interrupt" fn gp_handler(frame: InterruptStackFrame, code: u64) {
    panic!("GP FAULT code={:#x} frame={:#?}", code, frame);
}

extern "x86-interrupt" fn page_fault_handler(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let addr = x86_64::registers::control::Cr2::read();
    panic!("PAGE FAULT addr={:?} code={:?} frame={:#?}", addr, code, frame);
}

extern "x86-interrupt" fn timer_handler(_frame: InterruptStackFrame) {
    unsafe {
        PICS.lock().notify_end_of_interrupt(HwIrq::Timer.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let mut data: Port<u8> = Port::new(0x60);
    let scancode: u8 = unsafe { data.read() };
    crate::keyboard::push_scancode(scancode);
    unsafe {
        PICS.lock().notify_end_of_interrupt(HwIrq::Keyboard.as_u8());
    }
}
