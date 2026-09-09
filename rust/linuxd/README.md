# linuxd Linux personality translation broker

`vanta-linuxd` is the system call and ABI translation contract for the Linux x86_64 personality. It translates foreign Linux system calls into native Vanta microkernel operations at the kernel boundary while preserving the integrity of the native Vanta capability ABI.

## Capabilities

- **Static & Dynamic ELF**: Supports both static musl binaries and dynamic ELF binaries using `ld-musl-x86_64.so.1` / `ld-linux-x86-64.so.2` with complete auxiliary vector (`auxv`) initialization.
- **Memory Management**: Translates Linux `mmap`, `munmap`, `mprotect`, and `brk` calls with standard memory protection flags.
- **Signals**: Supports POSIX realtime signal delivery, signal masking (`rt_sigprocmask`), signal action installation (`rt_sigaction`), directed thread signals (`tkill`/`tgkill`), and context restore (`rt_sigreturn`).
- **Concurrency & Futexes**: Supports `clone`/`clone3` with thread-group tracking (`TGID`/`TID`), thread-local storage (`FS_BASE`), and `futex` (`FUTEX_WAIT`/`FUTEX_WAKE`).
- **Pipes & I/O Multiplexing**: Implements non-blocking and blocking pipes, `dup`/`dup2`/`dup3`, `epoll` (`epoll_create1`, `epoll_ctl`, `epoll_wait`), and `eventfd2`.
- **Networking**: Maps BSD/Linux socket primitives (`socket`, `bind`, `listen`, `accept`, `connect`, `sendto`, `recvfrom`) to the native VirtIO-net network stack.
- **POSIX Environment**: Full support for termios window size ioctls (`TIOCGWINSZ`), process groups (`setpgid`, `getpgrp`), and session IDs (`setsid`, `getsid`), powering standard shells like BusyBox `ash`.
