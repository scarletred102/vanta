#![no_std]

use core::arch::asm;

const SYS_WRITE: u64 = 0x0002;
const SYS_READ: u64 = 0x0001;
const SYS_EXEC: u64 = 0x0012;
const SYS_SPAWN: u64 = 0x0011;
const SYS_WAITPID: u64 = 0x0013;
const SYS_EXIT: u64 = 0x0014;
const SYS_YIELD: u64 = 0x001c;

pub fn write(fd: u64, bytes: &[u8]) -> u64 {
    syscall(SYS_WRITE, fd, bytes.as_ptr() as u64, bytes.len() as u64)
}

pub fn read(fd: u64, bytes: &mut [u8]) -> u64 {
    syscall(SYS_READ, fd, bytes.as_mut_ptr() as u64, bytes.len() as u64)
}

pub fn exec(path: &[u8]) -> u64 {
    syscall(SYS_EXEC, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn spawn(path: &[u8]) -> u64 {
    syscall(SYS_SPAWN, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn wait(pid: u64) -> u64 {
    syscall(SYS_WAITPID, pid, 0, 0)
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
        _ => None,
    }
}

fn syscall(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let result: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number => result,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            lateout("rcx") _,
            lateout("r11") _,
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
