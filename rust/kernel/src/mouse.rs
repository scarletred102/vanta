//! PS/2 Mouse Driver for hardware mouse motion and clicks.

use core::sync::atomic::{AtomicU8, Ordering};
use x86_64::instructions::port::Port;

static PACKET_INDEX: AtomicU8 = AtomicU8::new(0);
static mut PACKET_BYTES: [u8; 3] = [0; 3];

pub fn init() {
    unsafe {
        let mut cmd: Port<u8> = Port::new(0x64);
        let mut data: Port<u8> = Port::new(0x60);

        // 1. Enable auxiliary device (mouse port)
        cmd.write(0xa8);

        // 2. Enable IRQ 12 in controller config byte
        cmd.write(0x20);
        let status = data.read() | 0x02; // Set bit 1 (mouse IRQ 12 enable)
        cmd.write(0x60);
        data.write(status);

        // 3. Set default mouse settings
        cmd.write(0xd4);
        data.write(0xf6);
        let _ = data.read(); // Read ACK 0xfa

        // 4. Enable packet streaming
        cmd.write(0xd4);
        data.write(0xf4);
        let _ = data.read(); // Read ACK 0xfa
    }
}

pub fn process_byte(byte: u8) {
    let index = PACKET_INDEX.load(Ordering::Relaxed);
    unsafe {
        if index == 0 {
            // First byte must have bit 3 set (sync bit)
            if byte & 0x08 == 0 {
                return;
            }
            PACKET_BYTES[0] = byte;
            PACKET_INDEX.store(1, Ordering::Relaxed);
        } else if index == 1 {
            PACKET_BYTES[1] = byte;
            PACKET_INDEX.store(2, Ordering::Relaxed);
        } else if index == 2 {
            PACKET_BYTES[2] = byte;
            PACKET_INDEX.store(0, Ordering::Relaxed);

            let flags = PACKET_BYTES[0];
            let raw_x = PACKET_BYTES[1];
            let raw_y = PACKET_BYTES[2];

            let dx = if flags & 0x10 != 0 {
                (raw_x as i8) as i32
            } else {
                raw_x as i32
            };

            let dy = if flags & 0x20 != 0 {
                -((raw_y as i8) as i32)
            } else {
                -(raw_y as i32)
            };

            let buttons = (flags & 0x07) as u32;
            crate::input::inject_mouse_motion(dx, dy, buttons);
        }
    }
}
