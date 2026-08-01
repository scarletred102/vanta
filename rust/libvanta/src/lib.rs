#![cfg_attr(not(test), no_std)]

use core::arch::asm;
use vanta_abi::{AbiInfo, Syscall};

pub const VANTA_OK: isize = 0;
pub const VANTA_EIO: i32 = 5;
pub const VANTA_EBADF: i32 = 9;
pub const VANTA_EINVAL: i32 = 22;
pub const VANTA_ENOSYS: i32 = 38;
const BOOTSTRAP_HEAP_SIZE: usize = 64 * 1024;

static mut ERRNO: i32 = 0;
static mut BOOTSTRAP_HEAP: [u8; BOOTSTRAP_HEAP_SIZE] = [0; BOOTSTRAP_HEAP_SIZE];
static mut BOOTSTRAP_HEAP_OFFSET: usize = 0;

#[cfg(target_os = "none")]
extern "C" {
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

/// Minimal freestanding CRT entry point. The kernel starts a process with
/// argc/argv at the initial user stack pointer, matching the native ELF ABI.
#[no_mangle]
#[cfg(target_os = "none")]
pub unsafe extern "C" fn _start() -> ! {
    let stack: *const u64;
    asm!("mov {}, rsp", out(reg) stack, options(nostack, preserves_flags));
    let argc = *stack as i32;
    let argv = stack.add(1) as *const *const u8;
    vanta_exit(main(argc, argv));
}

#[no_mangle]
pub extern "C" fn vanta_errno_location() -> *mut i32 {
    core::ptr::addr_of_mut!(ERRNO)
}

#[no_mangle]
pub extern "C" fn vanta_write(fd: u64, buffer: *const u8, length: usize) -> isize {
    call(Syscall::Write, [fd, buffer as u64, length as u64, 0])
}

#[no_mangle]
pub extern "C" fn vanta_read(fd: u64, buffer: *mut u8, length: usize) -> isize {
    call(Syscall::Read, [fd, buffer as u64, length as u64, 0])
}

#[no_mangle]
pub extern "C" fn vanta_open(path: *const u8, length: usize, flags: u64) -> isize {
    call(Syscall::OpenAt, [path as u64, length as u64, flags, 0])
}

#[no_mangle]
pub extern "C" fn vanta_close(fd: u64) -> isize {
    call(Syscall::Close, [fd, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_spawn(path: *const u8, length: usize) -> isize {
    call(Syscall::SpawnVe, [path as u64, length as u64, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_waitpid(pid: u64) -> isize {
    call(Syscall::WaitPid, [pid, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_get_abi_info(info: *mut AbiInfo) -> isize {
    call(
        Syscall::GetAbiInfo,
        [info as u64, core::mem::size_of::<AbiInfo>() as u64, 0, 0],
    )
}

#[no_mangle]
pub extern "C" fn vanta_malloc(size: usize) -> *mut u8 {
    let aligned_size = match size.checked_add(7).map(|value| value & !7) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    unsafe {
        let end = match BOOTSTRAP_HEAP_OFFSET.checked_add(aligned_size) {
            Some(value) if value <= BOOTSTRAP_HEAP_SIZE => value,
            _ => return core::ptr::null_mut(),
        };
        let pointer =
            (core::ptr::addr_of_mut!(BOOTSTRAP_HEAP) as *mut u8).add(BOOTSTRAP_HEAP_OFFSET);
        BOOTSTRAP_HEAP_OFFSET = end;
        pointer
    }
}

#[no_mangle]
pub extern "C" fn vanta_free(_: *mut u8) {}

#[no_mangle]
pub extern "C" fn vanta_exit(status: i32) -> ! {
    let _ = call(Syscall::Exit, [status as u64, 0, 0, 0]);
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    vanta_exit(127)
}

fn call(syscall: Syscall, args: [u64; 4]) -> isize {
    let result: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") syscall.number() as u64 => result,
            in("rdi") args[0],
            in("rsi") args[1],
            in("rdx") args[2],
            in("r10") args[3],
            lateout("rcx") _,
            lateout("r11") _,
            lateout("r8") _,
            lateout("r9") _,
        );
    }
    let value = result as isize;
    if value < 0 {
        unsafe { ERRNO = (-value) as i32 };
    } else {
        unsafe { ERRNO = 0 };
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_file_functions_use_native_numbers() {
        assert_eq!(Syscall::Read.number(), 0x0001);
        assert_eq!(Syscall::Write.number(), 0x0002);
        assert_eq!(Syscall::OpenAt.number(), 0x0003);
        assert_eq!(Syscall::SpawnVe.number(), 0x0011);
    }

    #[test]
    fn errno_pointer_is_stable() {
        let first = vanta_errno_location();
        let second = vanta_errno_location();
        assert_eq!(first, second);
    }
}
