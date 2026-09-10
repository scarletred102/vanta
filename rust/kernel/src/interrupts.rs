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
    VirtioNet = PIC_1_OFFSET + 8,
    Mouse = PIC_1_OFFSET + 12,
}

impl HwIrq {
    pub fn as_u8(self) -> u8 {
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
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.general_protection_fault.set_handler_fn(gp_handler);
        unsafe {
            idt.page_fault.set_handler_addr(VirtAddr::new(
                vanta_page_fault_entry as *const () as usize as u64,
            ));
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
            idt[HwIrq::Timer.as_u8()].set_handler_addr(VirtAddr::new(
                vanta_timer_entry as *const () as usize as u64,
            ));
        }
        idt[HwIrq::Keyboard.as_u8()].set_handler_fn(keyboard_handler);
        idt[HwIrq::VirtioNet.as_u8()].set_handler_fn(virtio_net_handler);
        idt[HwIrq::Mouse.as_u8()].set_handler_fn(mouse_handler);
        idt
    };
}

extern "C" {
    fn vanta_timer_entry();
    fn vanta_page_fault_entry();
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
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    mov rdi, rsp
    call vanta_timer_tick
    mov rsp, rax
    test byte ptr [rsp + 128], 3
    jz 2f
    swapgs
2:
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

    .global vanta_page_fault_entry
    .extern vanta_page_fault_dispatch
    .extern vanta_syscall_restore_context
vanta_page_fault_entry:
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
    test byte ptr [rsp + 136], 3
    jz 1f
    swapgs
1:
    mov rdi, rsp
    call vanta_page_fault_dispatch
    test rax, rax
    jnz 2f
    test byte ptr [rsp + 136], 3
    jz 3f
    swapgs
3:
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
    add rsp, 8
    iretq
2:
    jmp vanta_syscall_restore_context
"#
);

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(frame: InterruptStackFrame) {
    serial_println!("[user] ring3 breakpoint: {:#?}", frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(frame: InterruptStackFrame) {
    panic!("INVALID OPCODE frame={:#?}", frame);
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("DOUBLE FAULT code={} frame={:#?}", code, frame);
}

extern "x86-interrupt" fn gp_handler(frame: InterruptStackFrame, code: u64) {
    panic!("GP FAULT code={:#x} frame={:#?}", code, frame);
}

#[repr(C)]
pub struct PageFaultContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[no_mangle]
extern "C" fn vanta_page_fault_dispatch(
    ctx: *mut PageFaultContext,
) -> *const crate::syscall::UserContext {
    let frame = unsafe { &*ctx };
    let fault_vaddr = x86_64::registers::control::Cr2::read().map_or(0, |value| value.as_u64());
    let space = crate::paging::current_address_space();
    let code = PageFaultErrorCode::from_bits_truncate(frame.error_code);

    if code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
        match crate::paging::resolve_cow_page(space, fault_vaddr) {
            Ok(true) => return core::ptr::null(),
            Ok(false) => {}
            Err(e) => {
                panic!("PAGE FAULT: COW resolution error {:?} at {:#x}", e, fault_vaddr);
            }
        }
    }

    if !code.contains(PageFaultErrorCode::PROTECTION_VIOLATION) {
        if let Ok(true) = crate::paging::resolve_swapped_page(space, fault_vaddr) {
            return core::ptr::null();
        }

        if fault_vaddr < 0x0000_8000_0000_0000 {
            let is_write = code.contains(PageFaultErrorCode::CAUSED_BY_WRITE);
            if let Ok(true) = crate::vma::resolve_demand_page(space, fault_vaddr, is_write) {
                return core::ptr::null();
            }
        }
    }

    let is_user = (frame.cs & 3) == 3 || code.contains(PageFaultErrorCode::USER_MODE);
    if is_user {
        crate::serial_println!(
            "[fault] user task killed by SIGSEGV at {:#x} (rip={:#x}, code={:#x})",
            fault_vaddr,
            frame.rip,
            frame.error_code
        );
        crate::scheduler::exit_group_current(128 + 11)
    } else {
        panic!(
            "KERNEL PAGE FAULT addr={:#x} code={:#x} rip={:#x}",
            fault_vaddr,
            frame.error_code,
            frame.rip
        );
    }
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

extern "x86-interrupt" fn mouse_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let mut data: Port<u8> = Port::new(0x60);
    let byte: u8 = unsafe { data.read() };
    crate::mouse::process_byte(byte);
    if IOAPIC_ACTIVE.load(Ordering::Acquire) {
        crate::apic::end_of_interrupt();
    } else {
        unsafe { PICS.lock().notify_end_of_interrupt(HwIrq::Mouse.as_u8()) };
    }
}

extern "x86-interrupt" fn virtio_net_handler(_frame: InterruptStackFrame) {
    crate::virtio_net::handle_interrupt();
    if IOAPIC_ACTIVE.load(Ordering::Acquire) {
        crate::apic::end_of_interrupt();
    } else {
        unsafe { PICS.lock().notify_end_of_interrupt(HwIrq::VirtioNet.as_u8()) };
    }
}
