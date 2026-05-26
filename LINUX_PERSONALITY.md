# Linux Personality — Architecture & Design

## Overview

The Linux personality layer enables VantaOS to run unmodified Linux x86_64 ELF binaries (static or dynamically linked with a musl interpreter). It follows a microkernel philosophy: the kernel does the minimum necessary (intercept syscalls, forward via SHM), and a userspace server implements Linux semantics.

```
Linux binary               Kernel                  Personality Server
─────────────────          ──────────────────       ──────────────────
syscall(write, 1, ...)  →  fast-path intercept  →  SHM ring: nr=1, args
                                                    ← retval written back
                        ←  return retval from SHM
```

---

## Linux ELF Detection

A binary is classified as a Linux ELF by `detect_linux_elf()` in `kernel/elf.zig`:

1. **EI_OSABI byte** (offset 7 in ELF header) equals `0x03` (ELFOSABI_LINUX)
2. **PT_INTERP segment** present — indicates a dynamic ELF with a Linux interpreter path

Static musl binaries typically have OSABI=0x00 (System V) but are explicitly spawned as Linux processes via the `PersonalitySpawn` syscall (syscall 20), bypassing auto-detection.

---

## Syscall Interception Fast Path

```
kernel/ipc/personality.zig   — SyscallShmBlock layout, address constants
kernel/sched/thread.zig      — Thread.personality_shm_phys, .personality_ping, .personality_pong, .fs_base
kernel/syscall/syscall_table.zig — intercept at top of dispatch()
```

### SHM Layout (`SyscallShmBlock`, 64 bytes)

```
offset  field     size  description
──────  ─────     ────  ──────────────────────────────────────
0       nr        8     Linux syscall number (x86_64 ABI)
8       arg0      8     rdi
16      arg1      8     rsi
24      arg2      8     rdx
32      arg3      8     r10
40      arg4      8     r8
48      arg5      8     r9
56      retval    8     return value written by personality server
```

### Virtual Address Map

| Address       | What                                    |
|---------------|-----------------------------------------|
| `0x60000000`  | `LINUX_PERSONALITY_SHM_VIRT` — SHM mapped in Linux process |
| `0x70000000`  | `PSERVER_SHM_VIRT` — same page mapped in personality server |
| `0x20000000+` | `Process.next_mmap_virt` — anonymous mmap base for Linux process |
| `0x40000000`  | Initial `brk` base for Linux process heap |

### Intercept Protocol

1. Linux thread issues `syscall` instruction
2. Kernel's `dispatch()` checks `current_thread.personality_shm_phys != 0`
3. Kernel translates `personality_shm_phys` → kernel VA via HHDM (`phys2virt`)
4. Kernel writes syscall number + 6 args to `SyscallShmBlock`
5. Kernel signals `personality_ping` notification (bit 1)
6. Kernel blocks on `personality_pong.wait(1)`
7. Personality server wakes, reads SHM, dispatches, writes `retval`
8. Personality server signals `personality_pong` (bit 1)
9. Kernel reads `retval` from SHM, returns to Linux thread

---

## PersonalitySpawn (syscall 20)

Called by init (or a launcher) to spawn a Linux binary under the personality server.

```
arg1 = elf_mem_cap    — MemoryCap containing the Linux ELF binary
arg2 = personality_ep — Endpoint cap for the personality server's port
→ returns: Thread cap for the Linux thread
```

Kernel steps:
1. Parse + load Linux ELF into a new Process
2. Allocate 1-page SHM object, map at `LINUX_PERSONALITY_SHM_VIRT` in Linux process
3. Create `ping` + `pong` Notification objects
4. Create Linux `Thread`, set `.personality_shm_phys`, `.personality_ping`, `.personality_pong`
5. Insert temp caps (ShmCap, ping-send, pong-wait, ThreadCap) in caller's table
6. Send `MSG_PERSONALITY_SETUP (0x30)` to personality server (blocking cap_call semantics)
7. Wait for ACK from personality server
8. Enqueue Linux thread
9. Return Thread cap to caller

### MSG_PERSONALITY_SETUP payload

```
payload[0..8]   linux_pid       (u64)
payload[8..16]  linux_tid       (u64)
payload[16..24] pserver_shm_virt — where personality server should map the SHM page
caps[0]         ShmCap          — the SHM page
caps[1]         ping send-only  — signal this when syscall arrives (kernel→server)
caps[2]         pong wait-only  — wait on this for syscall completion (server→kernel)
caps[3]         Thread cap      — for ThreadSetFsBase calls
```

---

## New Kernel Syscalls (Phase 10)

| Number | Name               | Arguments                              | Returns |
|--------|--------------------|----------------------------------------|---------|
| 20     | `PersonalitySpawn` | elf_mem_cap, personality_ep_cap        | thread_cap |
| 21     | `ProcessMmap`      | pid, hint_vaddr, n_pages, prot_flags   | mapped_vaddr |
| 22     | `ProcessMunmap`    | pid, vaddr, n_pages                    | 0 |
| 23     | `ThreadSetFsBase`  | thread_cap, fs_base_addr               | 0 |

`ProcessMmap` and `ProcessMunmap` let the personality server allocate/free anonymous memory in the Linux process's address space. The kernel validates the pid and performs the mapping with proper PTE flags.

`ThreadSetFsBase` sets `Thread.fs_base`, which the scheduler writes to `IA32_FS_BASE` MSR (`0xC0000100`) on every context switch to this thread. This supports musl/glibc TLS via `arch_prctl(ARCH_SET_FS, ...)`.

---

## Personality Server (`servers/linux_personality_server.zig`)

Registered as `sys.personality` in the registry. Listens on slot 9 endpoint.

### Slot Management

Up to 8 concurrent Linux threads (`MAX_SLOTS = 8`). Each slot stores:
- `shm_ptr` — pointer to mapped `SyscallShmBlock`
- `ping_cap` / `pong_cap` — notification caps for the syscall handshake
- `thread_cap` — for `ThreadSetFsBase`
- `pid` — Linux process pid (VantaOS pid)
- `fds_open[64]` — open file descriptor bitmap
- `brk_current` — current brk pointer for the Linux process

### Main Loop

Uses `vanta_cap_poll` to wait on the main port AND all active slot ping notification caps simultaneously. On wake:
- Index 0 → new `MSG_PERSONALITY_SETUP` on main port
- Index N → Linux syscall arrived in slot N-1's SHM block

### Implemented Linux Syscalls (~30)

| Syscall          | Nr  | Implementation |
|------------------|-----|----------------|
| `read`           | 0   | stdin returns EOF; other fds → EBADF |
| `write`          | 1   | fd 1/2 → `vanta_debug_print`; others → EBADF |
| `open`/`openat`  | 2/257 | allocates fd slot (stub, no real VFS) |
| `close`          | 3   | frees fd slot |
| `stat`/`fstat`/`newfstatat` | 4/5/262 | returns zeroed stat struct |
| `lseek`          | 8   | returns 0 |
| `mmap`           | 9   | anonymous only → `ProcessMmap`; file-backed → ENOSYS |
| `mprotect`       | 10  | stub, returns 0 |
| `munmap`         | 11  | calls `ProcessMunmap` |
| `brk`            | 12  | bump allocator via `ProcessMmap` |
| `sigaction`      | 13  | stub, returns 0 |
| `ioctl`          | 16  | stub, returns 0 |
| `writev`         | 20  | fd 1/2 → `vanta_debug_print` per iov |
| `dup`/`dup2`     | 32/33 | fd table bookkeeping |
| `getpid`         | 39  | returns slot.pid |
| `getppid`        | 110 | returns 1 |
| `getuid`/`geteuid`/`getgid`/`getegid` | 102–108 | returns 0 (root) |
| `uname`          | 63  | returns `Linux / vanta / 6.1.0-vanta / x86_64` |
| `arch_prctl`     | 158 | `ARCH_SET_FS` → `ThreadSetFsBase`; `ARCH_GET_FS` → 0 |
| `clone`          | 56  | ENOSYS (Phase 10 limitation) |
| `futex`          | 202 | FUTEX_WAIT → EINVAL (spurious wake); FUTEX_WAKE → 0 |
| `exit`/`exit_group` | 60/231 | marks slot inactive, signals pong |
| `set_tid_address` | 218 | stub, returns 0 |
| `set_robust_list` | 273 | stub, returns 0 |
| `prlimit64`      | 302 | stub, returns 0 |

---

## PTY Server (`servers/pty_server.zig`)

Registered as `sys.pty`. Provides a single pseudo-terminal pair via two in-memory ring buffers:

- `master_to_slave` — data from terminal to program (stdin)
- `slave_to_master` — data from program to terminal (stdout)

| Message          | Code | Description |
|------------------|------|-------------|
| `MSG_PTY_OPEN`   | 0x40 | Returns master/slave cap handles |
| `MSG_PTY_WRITE`  | 0x41 | Write to master or slave side |
| `MSG_PTY_READ`   | 0x42 | Read from master or slave side |
| `MSG_PTY_CLOSE`  | 0x43 | No-op stub |

---

## Test Targets

### Task 6: Static musl hello world

A minimal C program compiled with `musl-gcc -static`:

```c
#include <unistd.h>
int main() {
    write(1, "Hello from Linux!\n", 18);
    return 0;
}
```

Expected VantaOS output:
```
[LOG] Hello from Linux!
[BENCH] VANTA_TEST_PASS
```

The binary is loaded via `PersonalitySpawn`, routed through the personality server.
`write(1, ...)` → `vanta_debug_print` → serial output.

### Task 7: BusyBox statically linked shell (stretch goal)

BusyBox compiled with `--enable-static` and musl. Requires:
- Working `mmap`/`brk` (heap)
- Working `write`/`read` (I/O)
- `arch_prctl(ARCH_SET_FS)` for TLS (musl init)
- `futex` for basic musl locking
- `clone` for any multi-threaded operation (ENOSYS in Phase 10)

BusyBox `sh` interactive use also requires PTY terminal I/O, which is available via the PTY server.

---

## Permanently Unsupported

The following Linux functionality will never be supported in the personality layer — they require fundamental architectural changes incompatible with VantaOS's capability model:

- `fork()` — VantaOS has no copy-on-write process forking
- `execve()` — handled via `PersonalitySpawn` from the init process instead
- `ptrace()` — conflicts with capability-based isolation
- `setuid()`/`setgid()` — no ambient authority in VantaOS
- `/proc` filesystem — would require a dedicated procfs personality server
- Signal delivery (actual) — `sigaction` is stubbed; real async signal delivery is not implemented
- `poll()`/`epoll()` on file descriptors — the personality server is single-threaded; fd-based multiplexing maps to VantaOS `cap_poll` internally
- `mmap()` of files — requires full VFS integration (Phase 8+)
- `socket()`/`bind()`/`connect()` — use VantaOS networking stack directly instead
