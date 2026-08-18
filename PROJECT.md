# Project: Vanta OS — Gate D Implementation

## Architecture
Vanta OS is a hybrid microkernel operating system written in Rust, featuring a native IPC / microkernel capability subsystem, RedoxFS persistent storage on GPT disks, a POSIX compatibility layer (`vanta_linuxd`), and native driver subsystems.

Gate D introduces the following core architectural subsystems:
1. **Dynamic ELF Subsystem**:
   - `PT_INTERP` header detection and dynamic linker loading (`/lib/ld-musl-x86_64.so.1` / `ld-linux-x86-64.so.2`) at dedicated memory bias (`0x7f00_0000_0000`).
   - Auxiliary vector (`auxv`) setup populated with `AT_BASE`, `AT_PHDR`, `AT_PHNUM`, `AT_ENTRY`, `AT_PAGESZ`, `AT_RANDOM`, `AT_CLKTCK`, `AT_UID`, `AT_GID`, `AT_SECURE`.
   - Virtual memory management enhancements: `paging::protect()`, `mprotect` syscall, 6-argument syscall ABI dispatch, and file-backed/anonymous `mmap`.
2. **POSIX Signal Subsystem**:
   - Signal delivery and return mechanisms implementing `SYS_rt_sigaction`, `SYS_rt_sigprocmask`, `SYS_rt_sigreturn`, `SYS_kill`, `SYS_tkill`/`SYS_tgkill`.
   - Signal frame (`rt_sigframe`) injection on the user stack with full register context (`sigcontext`/`ucontext_t`), `sa_restorer` trampoline invocation, and atomic signal mask management.
3. **Multi-Threading & Futex Subsystem**:
   - Thread Groups and Process Hierarchy: separation of `PID` (`TGID`) and `TID`.
   - `SYS_clone` / `SYS_clone3` with `CLONE_VM`, `CLONE_FS`, `CLONE_FILES`, `CLONE_THREAD`, `CLONE_SETTLS`, `CLONE_CHILD_CLEARTID`.
   - Per-thread `FS_BASE` TLS tracking and `SYS_arch_prctl` support.
   - `SYS_futex` with `FUTEX_WAIT`, `FUTEX_WAKE`, `FUTEX_PRIVATE_FLAG`, supporting POSIX thread synchronization and deterministic `pthread_join`.
4. **VirtIO-Net & TCP/IP Network Stack**:
   - VirtIO-net driver on legacy PCI (`0x1AF4:0x1000`), split virtqueues (RX Queue 0, TX Queue 1), multi-buffer ring management.
   - TCP/IP protocol stack: Ethernet frame parsing, dynamic ARP cache, IPv4 routing, ICMP echo reply, UDP sockets, and TCP state machine (LISTEN, SYN_SENT, SYN_RECV, ESTABLISHED, FIN_WAIT, CLOSE_WAIT, etc.).
   - Socket syscall interface: `SYS_socket`, `SYS_bind`, `SYS_listen`, `SYS_accept`, `SYS_connect`, `SYS_sendto`/`SYS_write`, `SYS_recvfrom`/`SYS_read`, `SYS_getsockopt`/`SYS_setsockopt`, `SYS_getsockname`/`SYS_getpeername`, `SYS_close`.
5. **Acceptance Test Vectors & QEMU Regression Harness**:
   - Dynamic test vectors: `dynamic-hello`, `dynamic-signal`, `dynamic-threads`, `dynamic-net`.
   - Automated QEMU test harness (`rust/test-gpt-qemu.ps1`) verifying all Gate A, Gate B, Gate C, Gate D requirements, reboot persistence, and corrupt-root recovery.

## Code Layout
- `rust/kernel/src/`:
  - `elf.rs` — ELF parser, `PT_INTERP`, program headers, dynamic interpreter detection
  - `process.rs` — Process/Task definitions, address space layout, stack allocation, `auxv` injection, `mmap_file`, `mprotect`
  - `paging.rs` — Page table management, `map`, `unmap`, `protect`
  - `scheduler.rs` — Preemptive scheduler, thread groups (`TGID`/`TID`), `clone`, futex engine, signal delivery (`inject_signal_frame`)
  - `syscall.rs` — Syscall assembly entry (`vanta_syscall_entry`), 6-argument dispatcher, signal/thread/net syscall handlers
  - `virtio_net.rs` — Legacy VirtIO-net driver, RX/TX virtqueues, packet buffers
  - `net.rs` — Packet encodings (Ethernet, ARP, IPv4, ICMP, UDP, TCP)
  - `network.rs` — TCP/IP state machine, ARP cache, socket table, connection buffering
- `rust/linuxd/src/lib.rs` — Linux syscall ABI mapping, constants, structs (`rt_sigaction`, `clone_args`, `sockaddr_in`)
- `rust/abi/src/lib.rs` — Shared microkernel ABI types
- `rust/xtask/src/main.rs` — Image packaging, dynamic musl toolchain integration, RedoxFS formatting
- `rust/test-gpt-qemu.ps1` — Master QEMU acceptance and regression test harness

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | Dynamic ELF & `PT_INTERP` Parsing | Extract interpreter path, validate `ET_DYN`/`ET_EXEC`, handle dual-image loading | M1 | survey |
| 2 | Complete Auxiliary Vector (`auxv`) | Populate `AT_BASE`, `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_PAGESZ`, `AT_ENTRY`, `AT_RANDOM`, `AT_CLKTCK`, `AT_UID`, `AT_GID`, `AT_SECURE` | M1 | survey |
| 3 | Page Protection & Memory Syscalls | Implement `paging::protect()`, `mprotect` syscall, 6-argument syscall dispatch, `mmap` flags | M1 | survey |
| 4 | POSIX Signal Syscalls | `SYS_rt_sigaction`, `SYS_rt_sigprocmask`, `SYS_rt_sigreturn`, `SYS_kill`, `SYS_tkill`/`SYS_tgkill` | M2 | survey |
| 5 | Signal Frame Injection & Return | `rt_sigframe` user stack construction (`ucontext_t`, `siginfo_t`, `sigcontext`), `sa_restorer` trampoline execution, atomic mask restore | M2 | survey |
| 6 | Thread Groups & `clone` Syscall | `SYS_clone` / `SYS_clone3` with `CLONE_VM`, `CLONE_FS`, `CLONE_FILES`, `CLONE_THREAD`, `CLONE_SETTLS`, `CLONE_CHILD_CLEARTID`, `TID` vs `TGID` | M3 | survey |
| 7 | TLS & `FS_BASE` Management | Per-thread `FS_BASE` register state, `SYS_arch_prctl` (`ARCH_SET_FS`/`ARCH_GET_FS`), thread context switching | M3 | survey |
| 8 | Futex Subsystem & Wait Reaping | `SYS_futex` (`FUTEX_WAIT`, `FUTEX_WAKE`, `FUTEX_PRIVATE_FLAG`), `set_tid_address`, `SYS_wait4` | M3 | survey |
| 9 | VirtIO-Net Driver & Ring Management | Legacy VirtIO PCI `0x1AF4:0x1000`, split virtqueues (RX/TX), packet buffer queuing | M4 | survey |
| 10 | TCP/IP Stack & Multi-Socket Engine | Ethernet, ARP, IPv4, ICMP, UDP, TCP state machine (client & server), streaming buffers | M4 | survey |
| 11 | Socket Syscall Subsystem | `SYS_socket`, `SYS_bind`, `SYS_listen`, `SYS_accept`, `SYS_connect`, `SYS_sendto`, `SYS_recvfrom`, `SYS_getsockopt`, `SYS_setsockopt`, `SYS_getsockname`, `SYS_getpeername` | M4 | survey |
| 12 | Dynamic Test Vectors & Toolchain | Package `ld-musl-x86_64.so.1`, build `dynamic-hello`, `dynamic-signal`, `dynamic-threads`, `dynamic-net` in RedoxFS image | M5 / Test Track | survey |
| 13 | Master QEMU Regression & Acceptance | Expand `test-gpt-qemu.ps1` with `virtio-net-pci`, verify Gates A, B, C, D, reboot persistence, corrupt-root recovery | M5 / Test Track | survey |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Dynamic ELF Loader & Memory Protection | `PT_INTERP` dual ELF loader, complete `auxv`, `paging::protect()`, `mprotect`, 6-arg syscall dispatch | none | DONE |
| M2 | POSIX Signal Subsystem | `rt_sigaction`, `rt_sigprocmask`, `rt_sigreturn`, `tkill`/`tgkill`, signal frame injection & context restore | M1 | PLANNED |
| M3 | Multi-Threading & Futex Subsystem | `clone`/`clone3`, thread groups (`TGID`/`TID`), `FS_BASE` TLS, `futex` wait/wake, `wait4` | M1, M2 | PLANNED |
| M4 | VirtIO-Net Driver & TCP/IP Stack | VirtIO-net RX/TX rings, TCP/IP stack (ARP, IPv4, ICMP, UDP, TCP), complete socket syscalls | M1 | PLANNED |
| M5 | Gate D Integration & Acceptance | Build dynamic test binaries, wire `test-gpt-qemu.ps1` network harness, run full Gate A/B/C/D verification | M1, M2, M3, M4, Test Track | PLANNED |
| Test Track | E2E Testing Suite & Infrastructure | Design 4-tier requirement-driven test cases, test runner, publish `TEST_READY.md` | none | DONE |

## Interface Contracts

### M1 ↔ M2 (Dynamic Loader & Signal Interface)
- `Process` / `Task` address space: Main executable loaded at `0x40_0000`, dynamic interpreter loaded at `0x7f00_0000_0000`. User stack top at `0x7fff_ffff_0000`.
- Syscall dispatch: 6 register arguments passed to handlers (`rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`).
- Memory protection: `paging::protect(pml4, virt_addr, pages, flags)` modifies PTE bits and invalidates TLB page.

### M2 ↔ M3 (Signals & Threading Interface)
- Process Group / Thread hierarchy: Each `Task` has `tid: u64`, `tgid: u64`. Single-threaded process has `tid == tgid`. Threads created with `CLONE_THREAD` share `tgid`, `AddressSpace`, file descriptor table, and signal dispositions.
- Signal frame: Stack allocated at `rsp - sizeof(rt_sigframe)` (16-byte aligned). `rt_sigframe` contains `pretcode` pointing to `sa_restorer` (or default trampoline), `siginfo_t`, and `ucontext_t` (with `sigcontext` registers `r8..r15, rdi, rsi, rbp, rbx, rdx, rax, rcx, rsp, rip, eflags, cs, gs, fs`).
- Signal return: `SYS_rt_sigreturn` takes no arguments, reads `rt_sigframe` from `current_task.rsp`, restores all CPU registers and blocked signal mask, and jumps to user RIP.

### M3 ↔ M4 (Threading, Sockets & File Descriptors)
- File descriptor table: Stored as `Arc<Mutex<DescriptorTable>>` shared across threads in the same process group when `CLONE_FILES` is set.
- Sockets implement `DescriptorResource::Socket(Arc<Mutex<SocketHandle>>)`. Syscalls `read`, `write`, `close`, `poll` operate polymorphically across files, pipes, IPC channels, and sockets.

### M4 ↔ M5 (Network & Acceptance Harness)
- QEMU invocation: `-netdev user,id=net0 -device virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0`.
- Guest IP: `10.0.2.15`, Gateway/Host IP: `10.0.2.2`, Subnet Mask: `255.255.255.0`.
- Network test vectors: `dynamic-net` performs TCP client connection to host listener (`10.0.2.2:18080`) sending probe banner and receiving expected response.
