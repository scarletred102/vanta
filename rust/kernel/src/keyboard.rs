use core::sync::atomic::{AtomicU32, Ordering};
use heapless::mpmc::Q64;

static SCANCODE_QUEUE: Q64<u8> = Q64::new();
static IRQ_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn push_scancode(sc: u8) {
    let n = IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
    if n < 8 {
        crate::serial_println!("[kbd] irq #{} scancode={:#x}", n, sc);
    }
    let _ = SCANCODE_QUEUE.enqueue(sc);
}

pub fn pop_scancode() -> Option<u8> {
    SCANCODE_QUEUE.dequeue()
}
