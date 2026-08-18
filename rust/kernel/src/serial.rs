use core::fmt::{self, Write};
use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::SerialPort;

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        let mut port = unsafe { SerialPort::new(0x3F8) };
        port.init();
        Mutex::new(port)
    };
}

pub fn init() {
    let _ = SERIAL1.lock();
}

pub fn try_receive() -> Option<u8> {
    let line_status = unsafe { x86_64::instructions::port::PortReadOnly::<u8>::new(0x3FD).read() };
    if line_status & 1 != 0 {
        Some(SERIAL1.lock().receive())
    } else {
        None
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        SERIAL1.lock().write_fmt(args).expect("serial write failed");
    });
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(concat!($fmt, "\n"), $($arg)*));
}
