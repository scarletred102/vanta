use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use heapless::mpmc::Q64;

static SCANCODE_QUEUE: Q64<u8> = Q64::new();
static IRQ_COUNT: AtomicU32 = AtomicU32::new(0);
static CTRL_HELD: AtomicBool = AtomicBool::new(false);

pub fn push_scancode(sc: u8) {
    let n = IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
    if n < 8 {
        crate::serial_println!("[kbd] irq #{} scancode={:#x}", n, sc);
    }
    if sc == 0x1d {
        CTRL_HELD.store(true, Ordering::Relaxed);
    } else if sc == 0x9d {
        CTRL_HELD.store(false, Ordering::Relaxed);
    } else if sc == 0x2e && CTRL_HELD.load(Ordering::Relaxed) {
        crate::scheduler::interrupt_current(2);
    }
    crate::input::inject_key(sc as u32, sc & 0x80 == 0);
    let _ = SCANCODE_QUEUE.enqueue(sc);
}

pub fn pop_scancode() -> Option<u8> {
    SCANCODE_QUEUE.dequeue()
}
