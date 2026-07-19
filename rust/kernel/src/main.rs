#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::panic::PanicInfo;
use limine::request::{FramebufferRequest, MemmapRequest};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

mod serial;
mod gdt;
mod interrupts;
mod framebuffer;
mod keyboard;
mod shell;
mod memory;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(3);

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[link_section = ".requests"]
static MEMMAP_REQUEST: MemmapRequest = MemmapRequest::new();

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

    if let Some(memmap_resp) = MEMMAP_REQUEST.response() {
        let stats = memory::init(memmap_resp);
        serial_println!(
            "[mm] entries={} usable={} MiB frames={} tracked={}",
            stats.map_entries,
            stats.usable_bytes / (1024 * 1024),
            stats.usable_frames,
            stats.tracked_frames
        );

        let first = memory::alloc_frame();
        let second = memory::alloc_frame();
        match (first, second) {
            (Some(first), Some(second)) if first != second => {
                serial_println!(
                    "[mm] frame allocator self-check passed: {:#x}, {:#x}",
                    first.start_address(),
                    second.start_address()
                );
            }
            _ => serial_println!("[mm] WARNING: frame allocator self-check failed"),
        }
    } else {
        serial_println!("[mm] WARNING: no Limine memory-map response");
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
