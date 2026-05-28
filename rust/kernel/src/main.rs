#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::panic::PanicInfo;
use limine::request::FramebufferRequest;
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

mod serial;
mod gdt;
mod interrupts;
mod framebuffer;
mod keyboard;
mod shell;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(3);

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[link_section = ".requests_start_marker"]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial::init();
    serial_println!("[boot] vanta kernel: limine entry");

    if !BASE_REVISION.is_supported() {
        serial_println!("[boot] WARNING: limine base revision unsupported");
    }

    if let Some(fb_resp) = FRAMEBUFFER_REQUEST.response() {
        let fbs = fb_resp.framebuffers();
        if let Some(fb) = fbs.first() {
            serial_println!(
                "[boot] framebuffer {}x{} pitch={} bpp={}",
                fb.width, fb.height, fb.pitch, fb.bpp
            );
            framebuffer::init(fb);
        } else {
            serial_println!("[boot] WARNING: limine returned 0 framebuffers");
        }
    } else {
        serial_println!("[boot] WARNING: no framebuffer response");
    }

    kprintln!("vanta os | kernel terminal");
    kprintln!("-----------------------------------");

    gdt::init();
    kprintln!("[ok] gdt");
    serial_println!("[boot] gdt loaded");

    interrupts::init_idt();
    kprintln!("[ok] idt");
    serial_println!("[boot] idt loaded");

    unsafe {
        let mut pics = interrupts::PICS.lock();
        pics.initialize();
        // unmask IRQ0 (timer) and IRQ1 (keyboard); mask the rest
        pics.write_masks(0b1111_1100, 0b1111_1111);
    }
    x86_64::instructions::interrupts::enable();
    kprintln!("[ok] pic + sti");
    serial_println!("[boot] interrupts enabled");

    shell::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[PANIC] {}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
