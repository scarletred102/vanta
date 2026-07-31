#![no_std]

use core::arch::asm;

const SYS_WRITE: u64 = 0x0002;
const SYS_READ: u64 = 0x0001;
const SYS_OPEN: u64 = 0x0003;
const SYS_CLOSE: u64 = 0x0004;
const SYS_EXEC: u64 = 0x0012;
const SYS_DUP3: u64 = 0x0005;
const SYS_PIPE2: u64 = 0x0006;
const SYS_SPAWN: u64 = 0x0011;
const SYS_WAITPID: u64 = 0x0013;
const SYS_KILL: u64 = 0x0015;
const SYS_SIGACTION: u64 = 0x0016;
const SYS_EXIT: u64 = 0x0014;
const SYS_YIELD: u64 = 0x001c;
const SYS_FSTAT: u64 = 0x0008;
const SYS_GETDENTS: u64 = 0x0009;
const SYS_MKDIR: u64 = 0x000a;
const SYS_UNLINK: u64 = 0x000b;
const SYS_RENAME: u64 = 0x000c;
pub const READ_WOULD_BLOCK: u64 = u64::MAX - 5;

pub fn write(fd: u64, bytes: &[u8]) -> u64 {
    syscall(SYS_WRITE, fd, bytes.as_ptr() as u64, bytes.len() as u64)
}

pub fn read(fd: u64, bytes: &mut [u8]) -> u64 {
    loop {
        let result = syscall(SYS_READ, fd, bytes.as_mut_ptr() as u64, bytes.len() as u64);
        if result != READ_WOULD_BLOCK {
            return result;
        }
        // Native pipe descriptors are blocking. The kernel's cooperative
        // scheduler yields while the writer or child state makes progress.
        yield_now();
    }
}

pub fn exec(path: &[u8]) -> u64 {
    syscall(SYS_EXEC, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn open(path: &[u8], flags: u64) -> u64 {
    syscall(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags)
}

pub const OPEN_READ: u64 = 0x10;
pub const OPEN_WRITE: u64 = 0x11;
pub const OPEN_CREATE: u64 = 0x13;
pub const OPEN_TRUNCATE: u64 = 0x15;
pub const OPEN_APPEND: u64 = 0x19;

pub fn mkdir(path: &[u8]) -> u64 {
    syscall(SYS_MKDIR, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn unlink(path: &[u8]) -> u64 {
    syscall(SYS_UNLINK, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn rename(old_path: &[u8], new_path: &[u8]) -> u64 {
    syscall4(
        SYS_RENAME,
        old_path.as_ptr() as u64,
        old_path.len() as u64,
        new_path.as_ptr() as u64,
        new_path.len() as u64,
    )
}

pub fn fstat(fd: u64, stat: &mut [u8; 16]) -> u64 {
    syscall(SYS_FSTAT, fd, stat.as_mut_ptr() as u64, 0)
}

pub fn getdents(fd: u64, bytes: &mut [u8]) -> u64 {
    syscall(
        SYS_GETDENTS,
        fd,
        bytes.as_mut_ptr() as u64,
        bytes.len() as u64,
    )
}

pub fn spawn(path: &[u8]) -> u64 {
    spawn_with_stdio(path, u64::MAX, u64::MAX, u64::MAX)
}

pub fn spawn_with_stdio(path: &[u8], stdin: u64, stdout: u64, stderr: u64) -> u64 {
    let stdio = [
        stdin.to_ne_bytes(),
        stdout.to_ne_bytes(),
        stderr.to_ne_bytes(),
    ];
    syscall4(
        SYS_SPAWN,
        path.as_ptr() as u64,
        path.len() as u64,
        stdio.as_ptr() as u64,
        0,
    )
}

pub fn spawn_with_args(path: &[u8], args: &[&[u8]], stdin: u64, stdout: u64, stderr: u64) -> u64 {
    let mut pointers = [0_u64; 8];
    let count = args.len().min(pointers.len());
    for (index, argument) in args.iter().take(count).enumerate() {
        pointers[index] = argument.as_ptr() as u64;
    }
    let native = [
        stdin.to_ne_bytes(),
        stdout.to_ne_bytes(),
        stderr.to_ne_bytes(),
        (pointers.as_ptr() as u64).to_ne_bytes(),
        (count as u64).to_ne_bytes(),
    ];
    syscall4(
        SYS_SPAWN,
        path.as_ptr() as u64,
        path.len() as u64,
        native.as_ptr() as u64,
        1,
    )
}

pub fn arg(index: usize) -> Option<&'static [u8]> {
    let stack: u64;
    unsafe { core::arch::asm!("mov {}, r12", out(reg) stack, options(nostack, preserves_flags)) };
    let argc = unsafe { *(stack as *const u64) as usize };
    if index >= argc || index >= 8 {
        return None;
    }
    let pointer = unsafe { *((stack + 8 + index as u64 * 8) as *const u64) };
    if pointer == 0 {
        return None;
    }
    let mut length = 0usize;
    while length < 128 && unsafe { *((pointer + length as u64) as *const u8) } != 0 {
        length += 1;
    }
    Some(unsafe { core::slice::from_raw_parts(pointer as *const u8, length) })
}

pub fn dup3(old_fd: u64, new_fd: u64) -> u64 {
    syscall(SYS_DUP3, old_fd, new_fd, 0)
}

pub fn pipe2() -> Option<(u64, u64)> {
    let mut fds = [0_u32; 2];
    if syscall(SYS_PIPE2, fds.as_mut_ptr() as u64, 0, 0) == u64::MAX - 1 {
        return None;
    }
    Some((fds[0] as u64, fds[1] as u64))
}

pub fn close(fd: u64) -> u64 {
    syscall(SYS_CLOSE, fd, 0, 0)
}

pub fn wait(pid: u64) -> u64 {
    syscall(SYS_WAITPID, pid, 0, 0)
}

pub fn kill(pid: u64, signal: u64) -> u64 {
    syscall(SYS_KILL, pid, signal, 0)
}

pub fn sigaction(signal: u64, handler: u64, flags: u64) -> u64 {
    let action = [handler.to_ne_bytes(), flags.to_ne_bytes()];
    syscall(SYS_SIGACTION, signal, action.as_ptr() as u64, 0)
}

pub fn exit(code: u64) -> ! {
    let _ = syscall(SYS_EXIT, code, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

pub fn yield_now() {
    let _ = syscall(SYS_YIELD, 0, 0, 0);
}

pub fn command_path(command: &[u8]) -> Option<&'static [u8]> {
    match command {
        b"echo" => Some(b"/bin/echo"),
        b"cat" => Some(b"/bin/cat"),
        b"true" => Some(b"/bin/true"),
        b"false" => Some(b"/bin/false"),
        b"ls" => Some(b"/bin/ls"),
        b"mkdir" => Some(b"/bin/mkdir"),
        b"rm" => Some(b"/bin/rm"),
        b"mv" => Some(b"/bin/mv"),
        b"pwd" => Some(b"/bin/pwd"),
        b"stat" => Some(b"/bin/stat"),
        b"c-hello" => Some(b"/bin/c-hello"),
        _ => None,
    }
}

fn syscall(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    syscall4(number, arg1, arg2, arg3, 0)
}

fn syscall4(number: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    let result: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number => result,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            lateout("rcx") _,
            lateout("r11") _,
            lateout("r8") _,
            lateout("r9") _,
        );
    }
    result
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    exit(127)
}

#[cfg(test)]
mod tests {
    use super::command_path;

    #[test]
    fn resolves_bundled_static_commands() {
        assert_eq!(command_path(b"echo"), Some(&b"/bin/echo"[..]));
        assert_eq!(command_path(b"false"), Some(&b"/bin/false"[..]));
        assert_eq!(command_path(b"missing"), None);
    }
}
