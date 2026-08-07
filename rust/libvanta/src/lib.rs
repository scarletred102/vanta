#![cfg_attr(not(test), no_std)]

use core::arch::asm;
use vanta_abi::{AbiInfo, SignalAction, Syscall};

pub const VANTA_OK: isize = 0;
pub const VANTA_EIO: i32 = 5;
pub const VANTA_EBADF: i32 = 9;
pub const VANTA_EINVAL: i32 = 22;
pub const VANTA_ENOSYS: i32 = 38;
const BOOTSTRAP_HEAP_SIZE: usize = 64 * 1024;
const VANTA_FILE_BUFFER_SIZE: usize = 256;
const VANTA_FILE_READ: u32 = 1;
const VANTA_FILE_WRITE: u32 = 2;

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

#[repr(C)]
pub struct VantaStat {
    pub size: u64,
    pub mode: u64,
}

#[repr(C)]
pub struct VantaPipe {
    pub read_fd: u32,
    pub write_fd: u32,
}

#[repr(C)]
pub struct VantaStream {
    pub fd: u64,
}

#[no_mangle]
pub extern "C" fn vanta_stream_open(
    path: *const u8,
    length: usize,
    flags: u64,
    stream: *mut VantaStream,
) -> isize {
    let fd = vanta_open(path, length, flags);
    if fd < 0 {
        return fd;
    }
    unsafe {
        (*stream).fd = fd as u64;
    }
    0
}

#[no_mangle]
pub extern "C" fn vanta_stream_read(
    stream: *mut VantaStream,
    buffer: *mut u8,
    length: usize,
) -> isize {
    unsafe { vanta_read((*stream).fd, buffer, length) }
}

#[no_mangle]
pub extern "C" fn vanta_stream_write(
    stream: *mut VantaStream,
    buffer: *const u8,
    length: usize,
) -> isize {
    unsafe { vanta_write((*stream).fd, buffer, length) }
}

#[no_mangle]
pub extern "C" fn vanta_stream_close(stream: *mut VantaStream) -> isize {
    unsafe { vanta_close((*stream).fd) }
}

#[no_mangle]
pub extern "C" fn vanta_stream_flush(_: *mut VantaStream) -> isize {
    0
}

#[repr(C)]
pub struct VantaFile {
    pub fd: u64,
    pub mode: u32,
    pub buffer_pos: u32,
    pub buffer_len: u32,
    pub buffer: [u8; VANTA_FILE_BUFFER_SIZE],
}

#[no_mangle]
pub extern "C" fn vanta_file_open(
    path: *const u8,
    length: usize,
    flags: u64,
    file: *mut VantaFile,
) -> isize {
    let fd = vanta_open(path, length, flags);
    if fd < 0 {
        return fd;
    }
    let mode = if flags & 1 != 0 {
        VANTA_FILE_WRITE
    } else {
        VANTA_FILE_READ
    };
    unsafe {
        core::ptr::write(
            file,
            VantaFile {
                fd: fd as u64,
                mode,
                buffer_pos: 0,
                buffer_len: 0,
                buffer: [0; VANTA_FILE_BUFFER_SIZE],
            },
        );
    }
    0
}

#[no_mangle]
pub extern "C" fn vanta_file_flush(file: *mut VantaFile) -> isize {
    unsafe {
        if (*file).mode != VANTA_FILE_WRITE || (*file).buffer_len == 0 {
            return 0;
        }
        let length = (*file).buffer_len as usize;
        let written = vanta_write((*file).fd, (*file).buffer.as_ptr(), length);
        if written < 0 {
            return written;
        }
        if written as usize != length {
            let remaining = length - written as usize;
            core::ptr::copy(
                (*file).buffer.as_ptr().add(written as usize),
                (*file).buffer.as_mut_ptr(),
                remaining,
            );
            (*file).buffer_len = remaining as u32;
            return -(VANTA_EIO as isize);
        }
        (*file).buffer_len = 0;
        (*file).buffer_pos = 0;
    }
    0
}

#[no_mangle]
pub extern "C" fn vanta_file_write(
    file: *mut VantaFile,
    buffer: *const u8,
    length: usize,
) -> isize {
    unsafe {
        if (*file).mode != VANTA_FILE_WRITE {
            return -(VANTA_EBADF as isize);
        }
        let mut offset = 0;
        while offset < length {
            if (*file).buffer_len as usize == VANTA_FILE_BUFFER_SIZE && vanta_file_flush(file) < 0 {
                return -(VANTA_EIO as isize);
            }
            let available = VANTA_FILE_BUFFER_SIZE - (*file).buffer_len as usize;
            let count = core::cmp::min(available, length - offset);
            core::ptr::copy_nonoverlapping(
                buffer.add(offset),
                (*file).buffer.as_mut_ptr().add((*file).buffer_len as usize),
                count,
            );
            (*file).buffer_len += count as u32;
            offset += count;
        }
    }
    length as isize
}

#[no_mangle]
pub extern "C" fn vanta_file_getc(file: *mut VantaFile) -> isize {
    unsafe {
        if (*file).mode != VANTA_FILE_READ {
            return -(VANTA_EBADF as isize);
        }
        if (*file).buffer_pos >= (*file).buffer_len {
            let count = vanta_read(
                (*file).fd,
                (*file).buffer.as_mut_ptr(),
                VANTA_FILE_BUFFER_SIZE,
            );
            if count <= 0 {
                return count;
            }
            (*file).buffer_pos = 0;
            (*file).buffer_len = count as u32;
        }
        let byte = (*file).buffer[(*file).buffer_pos as usize];
        (*file).buffer_pos += 1;
        byte as isize
    }
}

#[no_mangle]
pub extern "C" fn vanta_file_putc(file: *mut VantaFile, byte: u8) -> isize {
    let result = vanta_file_write(file, &byte, 1);
    if result < 0 {
        result
    } else {
        byte as isize
    }
}

#[no_mangle]
pub extern "C" fn vanta_file_close(file: *mut VantaFile) -> isize {
    let flush_result = vanta_file_flush(file);
    if flush_result < 0 {
        return flush_result;
    }
    unsafe { vanta_close((*file).fd) }
}

#[no_mangle]
pub extern "C" fn vanta_dup(fd: u64) -> isize {
    call(Syscall::Dup3, [fd, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_pipe(pipe: *mut VantaPipe) -> isize {
    call(Syscall::Pipe2, [pipe as u64, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_fstat(fd: u64, stat: *mut VantaStat) -> isize {
    call(Syscall::FStat, [fd, stat as u64, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_getdents(fd: u64, buffer: *mut u8, length: usize) -> isize {
    call(Syscall::GetDents, [fd, buffer as u64, length as u64, 0])
}

#[no_mangle]
pub extern "C" fn vanta_mkdir(path: *const u8, length: usize) -> isize {
    call(Syscall::MkDirAt, [path as u64, length as u64, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_unlink(path: *const u8, length: usize) -> isize {
    call(Syscall::UnlinkAt, [path as u64, length as u64, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_rename(
    old_path: *const u8,
    old_length: usize,
    new_path: *const u8,
    new_length: usize,
) -> isize {
    call(
        Syscall::RenameAt,
        [
            old_path as u64,
            old_length as u64,
            new_path as u64,
            new_length as u64,
        ],
    )
}

#[no_mangle]
pub extern "C" fn vanta_getpid() -> isize {
    call(Syscall::GetPid, [0, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_getppid() -> isize {
    call(Syscall::GetPpid, [0, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_yield() -> isize {
    call(Syscall::Yield, [0, 0, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_kill(pid: u64, signal: u64) -> isize {
    call(Syscall::Kill, [pid, signal, 0, 0])
}

#[no_mangle]
pub extern "C" fn vanta_sigaction(signal: u64, handler: u64, flags: u64) -> isize {
    let action = SignalAction { handler, flags };
    call(
        Syscall::SigAction,
        [signal, &action as *const SignalAction as u64, 0, 0],
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
