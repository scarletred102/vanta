# Vanta OS Gate D: End-to-End Testing Infrastructure Specification

**Document Version**: 1.0.0  
**Target Milestone**: Gate D (Dynamic ELF, POSIX Signals, Multi-Threading, VirtIO-Net & TCP/IP Stack)  
**Author**: Test Writer Agent  
**Status**: APPROVED & READY FOR IMPLEMENTATION VERIFICATION  

---

## 1. Executive Overview & Testing Philosophy

Gate D introduces four foundational kernel subsystems into Vanta OS:
1. **Dynamic ELF Interpreter Subsystem** (`PT_INTERP`, dual-image address space loading, complete auxiliary vector `auxv`, and dynamic memory protection via `mprotect`/`mmap`).
2. **POSIX Signal Subsystem** (`SYS_rt_sigaction`, `SYS_rt_sigprocmask`, `SYS_rt_sigreturn`, `SYS_kill`, `SYS_tkill`/`SYS_tgkill`, user signal frame construction, and trampoline execution).
3. **Multi-Threading & Futex Subsystem** (`SYS_clone`/`SYS_clone3` with `CLONE_VM`, `CLONE_FS`, `CLONE_FILES`, `CLONE_THREAD`, `CLONE_SETTLS`, `CLONE_CHILD_CLEARTID`; per-thread `FS_BASE` TLS; `SYS_futex` for `FUTEX_WAIT`/`FUTEX_WAKE`; and `SYS_wait4`).
4. **VirtIO-Net & TCP/IP Protocol Stack** (Legacy PCI VirtIO-net driver, RX/TX split virtqueues, dynamic ARP cache, IPv4 routing, ICMP echo replies, UDP sockets, full TCP state machine, and POSIX socket syscalls: `SYS_socket`, `SYS_bind`, `SYS_listen`, `SYS_accept`, `SYS_connect`, `SYS_sendto`, `SYS_recvfrom`, `SYS_getsockopt`, `SYS_setsockopt`).

### 1.1 Invariant Protection & Progressive Testability
The testing framework operates under strict verification invariants:
- **Zero Regression**: All Gate A (GPT reproducibility, RedoxFS persistence, C SDK suite), Gate B (microkernel IPC services, procd/auditd/vfsd authority revocation), Gate C (static Linux personality), reboot persistence, and corrupt-root recovery invariants must pass unmodified.
- **Progressive Testability**: Each milestone (M1 through M5) exposes deterministic observable outputs and syscall contract boundaries that can be verified independently before complete end-to-end integration.
- **Deterministic Oracles**: Every test case derives its pass/fail condition from explicit observable outputs: serial console markers, process exit codes, memory layout assertions, network frame round-trips, and cryptographic hashes.

---

## 2. Four-Tier Testing Methodology Matrix

```
+-------------------------------------------------------------------------------------------------------+
|                                    GATE D TESTING ARCHITECTURE                                         |
+-------------------------------------------------------------------------------------------------------+
|  Tier 1: Feature Coverage            >= 5 targeted test cases per major architectural feature         |
|  Tier 2: Boundary & Corner Cases     Stress limits, malformed inputs, edge values & fault isolation   |
|  Tier 3: Cross-Feature Combinations  Multi-threaded signal handling during concurrent socket I/O      |
|  Tier 4: Real-World Scenarios        Full musl C runtime, multi-client server, host TCP network probes|
+-------------------------------------------------------------------------------------------------------+
```

---

## 3. Tier 1: Detailed Feature Coverage

### 3.1 Feature 1: Dynamic ELF Loader & Interpreter Subsystem

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.1.1** | `test_elf_pt_interp_extraction` | ELF binary with `PT_INTERP` header specifying `/lib/ld-musl-x86_64.so.1`. | Kernel parses interpreter path string without trailing null; locates `/lib/ld-musl-x86_64.so.1` in VFS. | System V AMD64 ABI / Linux ELF Spec |
| **T1.1.2** | `test_dual_elf_address_mapping` | Position-Independent Executable (`ET_DYN`) and dynamic interpreter loaded in same address space. | Main image mapped at base `0x0040_0000`; interpreter mapped at base `0x7f00_0000_0000`; `PT_LOAD` permissions match file headers. | System V ABI §3.3 |
| **T1.1.3** | `test_auxv_population_completeness` | Process spawned with dynamic ELF image. | Initial stack contains complete `auxv` array: `AT_BASE=0x7f00_0000_0000`, `AT_PHDR`, `AT_PHENT=56`, `AT_PHNUM`, `AT_PAGESZ=4096`, `AT_ENTRY=main_entry`, `AT_RANDOM` (16 bytes), `AT_UID=1000`, `AT_GID=1000`, `AT_SECURE=0`, `AT_NULL=0`. | Linux `sys/auxv.h` Specification |
| **T1.1.4** | `test_interpreter_entry_redirection` | Execution handover at ring-3 transition. | Initial `%rip` points to `interp_base + interp_entry`; `%rsp` points to `argc`; `%rdx` is 0; dynamic linker starts execution. | System V AMD64 ABI §3.4 |
| **T1.1.5** | `test_mprotect_segment_relro` | Dynamic linker calls `SYS_mprotect(addr, len, PROT_READ)` on `.data.rel.ro` segment after relocation. | Syscall returns 0; PTE flags updated to remove `MAP_WRITABLE`; subsequent write triggers page fault; read succeeds. | POSIX.1-2017 `mprotect` Spec |
| **T1.1.6** | `test_mmap_anonymous_and_fixed` | Dynamic linker calls `SYS_mmap(addr, len, PROT_READ\|PROT_WRITE, MAP_PRIVATE\|MAP_ANONYMOUS\|MAP_FIXED, -1, 0)`. | Syscall maps zeroed pages at requested virtual address without corrupting adjacent address space. | POSIX.1-2017 `mmap` Spec |

---

### 3.2 Feature 2: POSIX Signal Subsystem

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.2.1** | `test_sigaction_handler_registration` | Calling `SYS_rt_sigaction(SIGUSR1, &new_act, &old_act, 8)` with custom handler address and `SA_RESTORER`. | Syscall returns 0; `old_act` receives previous disposition; kernel records user handler address, flags, and `sa_mask`. | Linux `rt_sigaction(2)` Man Page |
| **T1.2.2** | `test_sigprocmask_blocking_unblocking` | Calling `SYS_rt_sigprocmask(SIG_BLOCK, &mask, &old, 8)` where mask has `SIGUSR1`; subsequent `SIG_UNBLOCK`. | Signals in blocked mask are not delivered while blocked; pending signals delivered immediately upon unblocking. | POSIX.1-2017 `sigprocmask` |
| **T1.2.3** | `test_directed_signal_delivery` | Process sends `SYS_tkill(tid, SIGUSR1)` or `SYS_tgkill(tgid, tid, SIGUSR1)`. | Target thread receives pending signal bit; signal delivery triggered prior to next user-mode return. | Linux `tkill(2)` Man Page |
| **T1.2.4** | `test_user_signal_frame_injection` | Kernel delivers signal to thread with custom handler. | User stack contains aligned `RtSigFrame` (`pretcode`, `ucontext_t`, `siginfo_t`); `%rdi=sig`, `%rsi=&siginfo`, `%rdx=&ucontext`; `%rip=handler`. | System V AMD64 ABI §3.5 / Linux x86_64 Kernel Frame |
| **T1.2.5** | `test_sigreturn_context_restoration` | Signal handler returns to `sa_restorer` trampoline (`mov $15, %rax; syscall`). | `SYS_rt_sigreturn` restores all CPU registers (`rax..r15`, `rflags`, `rsp`, `rip`), original blocked signal mask, and user execution resumes seamlessly. | Linux `rt_sigreturn(2)` Man Page |
| **T1.2.6** | `test_default_signal_actions` | Sending `SIGTERM` or `SIGKILL` to thread group with `SIG_DFL`. | Thread group immediately terminates with `exit_code = 128 + sig`; parent notified via `SIGCHLD`. | POSIX.1-2017 `signal` Semantics |

---

### 3.3 Feature 3: Multi-Threading, Thread Groups & TLS Subsystem

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.3.1** | `test_clone_thread_creation` | Calling `SYS_clone` with `CLONE_VM \| CLONE_FS \| CLONE_FILES \| CLONE_SIGHAND \| CLONE_THREAD \| CLONE_SETTLS`. | Syscall returns new `TID` to parent, `0` to child; child shares `AddressSpace`, file descriptors, and `TGID` with parent. | Linux `clone(2)` Man Page |
| **T1.3.2** | `test_per_thread_tls_fs_base` | Child thread sets TLS base via `CLONE_SETTLS` or `SYS_arch_prctl(ARCH_SET_FS, tls_addr)`. | Reading `%fs:0` returns thread-specific pointer; context switch preserves distinct `FS_BASE` across sibling threads. | x86_64 Architecture Reference Manual |
| **T1.3.3** | `test_thread_stack_isolation` | Parent allocates distinct stack buffer and passes `child_stack` to `SYS_clone`. | Child thread executes on allocated stack pointer without colliding with parent stack frames. | POSIX Threads Specification |
| **T1.3.4** | `test_thread_group_tgid_hierarchy` | Main thread (PID 100) spawns 4 threads (TIDs 101, 102, 103, 104). | `getpid()` returns 100 on all threads; `gettid()` returns unique TID per thread; thread group exit terminates all sibling threads. | Linux Process & Thread Hierarchy |
| **T1.3.5** | `test_shared_descriptor_table` | Thread A opens a file descriptor; Thread B performs read/write on the same descriptor integer. | Thread B successfully accesses descriptor opened by Thread A due to shared `Arc<Mutex<DescriptorTable>>`. | POSIX `CLONE_FILES` Specification |
| **T1.3.6** | `test_exit_group_vs_thread_exit` | Thread B calls `SYS_exit(0)` (only thread dies); Thread A calls `SYS_exit_group(0)` (whole process terminates). | Single thread exit reaps thread without terminating process group; `exit_group` terminates all remaining tasks in `TGID`. | Linux `exit_group(2)` Specification |

---

### 3.4 Feature 4: Futex Subsystem & Synchronization

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.4.1** | `test_futex_wait_value_mismatch` | Calling `SYS_futex(uaddr, FUTEX_WAIT, val=5, 0, 0, 0)` when `*uaddr == 10`. | Syscall returns `-EAGAIN` (`-11`) immediately without blocking caller. | Linux `futex(2)` Specification |
| **T1.4.2** | `test_futex_wait_and_wake_single` | Thread A calls `FUTEX_WAIT` on `uaddr`; Thread B updates `*uaddr` and calls `FUTEX_WAKE(uaddr, 1)`. | Thread A unblocks and returns `0`; `FUTEX_WAKE` returns `1` (number of waiters woken). | Linux `futex(2)` Specification |
| **T1.4.3** | `test_futex_wake_multiple_waiters` | Threads A, B, C call `FUTEX_WAIT` on same `uaddr`; Thread D calls `FUTEX_WAKE(uaddr, 2)`. | Exactly 2 threads unblock; `FUTEX_WAKE` returns `2`; 3rd thread remains in `TaskState::FutexWaiting`. | Linux `futex(2)` Specification |
| **T1.4.4** | `test_futex_child_cleartid_join` | Thread spawned with `CLONE_CHILD_CLEARTID` pointing to `child_tidptr`; parent calls `FUTEX_WAIT(child_tidptr, tid)`. | On child thread exit, kernel writes `0` to `*child_tidptr`, calls `futex_wake`, and unblocks parent `pthread_join`. | Linux `set_tid_address(2)` Spec |
| **T1.4.5** | `test_futex_timeout_expiration` | Calling `FUTEX_WAIT` with `timespec { sec: 0, nsec: 50_000_000 }` (50ms). | Syscall unblocks after ~50ms timer expiration and returns `-ETIMEDOUT` (`-110`). | Linux `futex(2)` Specification |
| **T1.4.6** | `test_wait4_zombie_cleanup` | Parent calls `SYS_wait4(-1, &status, WNOHANG, NULL)`. | Returns PID of exited child, encodes termination status in `status`, and purges zombie task entry. | POSIX.1-2017 `wait4` |

---

### 3.5 Feature 5: VirtIO-Net Driver & Ring Buffer Subsystem

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.5.1** | `test_virtio_net_pci_enumeration` | Kernel scans PCI bus with `virtio-net-pci` attached (`0x1AF4:0x1000`). | Kernel identifies device, enables PCI bus master & I/O space, reads 6-byte MAC address from `DEVICE_CONFIG`. | VirtIO Spec v0.9.5 §2.1 & §5.1 |
| **T1.5.2** | `test_virtio_split_ring_initialization` | Driver initializes RX Queue 0 and TX Queue 1. | Descriptors, Available Ring, and Used Ring formatted in physically contiguous DMA memory (`>= 1MB`). | VirtIO Spec v0.9.5 §2.4.1 |
| **T1.5.3** | `test_rx_buffer_pool_priming` | Driver primes RX queue with `RX_BUFFER_COUNT = 8` physical frames. | Available Ring index updated, device notified via `QUEUE_NOTIFY(0)`, ready for incoming host packets. | VirtIO Spec v0.9.5 §5.1.6 |
| **T1.5.4** | `test_tx_packet_transmission` | Driver transmits Ethernet frame via TX queue. | Prepends 10-byte `virtio_net_hdr`, populates TX descriptor, increments Available Ring, notifies Queue 1, frame received on host network. | VirtIO Spec v0.9.5 §5.1.6 |
| **T1.5.5** | `test_rx_packet_consumption_and_recycle` | Host transmits packet to guest MAC address. | Driver detects Used Ring index advancement, strips 10-byte header, extracts payload, and recycles descriptor back to Available Ring. | VirtIO Spec v0.9.5 §5.1.6 |
| **T1.5.6** | `test_dynamic_arp_cache_resolution` | Guest sends ARP request for gateway IP `10.0.2.2`. | Host replies with ARP response; guest parses response and updates dynamic ARP table with gateway MAC (`52:54:00:12:34:56`). | RFC 826 ARP Protocol |

---

### 3.6 Feature 6: Socket Syscalls & TCP/IP Protocol Stack

| Test ID | Test Name | Input / Precondition | Expected Observable Output / Assertion | Authoritative Source |
|---|---|---|---|---|
| **T1.6.1** | `test_socket_creation_and_close` | Calling `SYS_socket(AF_INET=2, SOCK_STREAM=1, 0)` followed by `SYS_close(fd)`. | Returns valid descriptor index `>= 3`; descriptor is assigned `DescriptorResource::Socket`; `close()` cleanly releases resource. | POSIX.1-2017 `socket`, `close` |
| **T1.6.2** | `test_icmp_echo_ping_response` | External host sends ICMP Echo Request (ping) to guest IP `10.0.2.15`. | Kernel network stack receives ICMP request, computes checksum, and transmits ICMP Echo Reply with matching ID/sequence. | RFC 792 ICMP Protocol |
| **T1.6.3** | `test_udp_sendto_recvfrom` | Guest binds UDP socket to port 5000 and sends datagram to host `10.0.2.2:5000`. | UDP header formatted with valid checksum; datagram transmitted and received intact without connection state. | RFC 768 UDP Protocol |
| **T1.6.4** | `test_tcp_client_three_way_handshake` | Calling `SYS_connect(fd, &sockaddr_in{10.0.2.2:18080}, 16)`. | TCP client sends `SYN`, receives `SYN+ACK`, transmits `ACK`; connection state advances to `ESTABLISHED`. | RFC 793 / RFC 9293 TCP Spec |
| **T1.6.5** | `test_tcp_streaming_send_receive` | Established TCP connection streams 1024 bytes of data bidirectionally. | Data delivered reliably with sequence numbering and ACK advancement; data buffered across multiple segments. | RFC 793 / RFC 9293 TCP Spec |
| **T1.6.6** | `test_tcp_server_bind_listen_accept` | Guest socket binds to port 8080, calls `listen(backlog=5)`, and calls `accept()`. | Incoming connection from client completes handshake, enters backlog queue, and `accept()` returns newly allocated connected socket descriptor. | POSIX.1-2017 `bind`, `listen`, `accept` |

---

## 4. Tier 2: Boundary, Corner Cases & Stress Scenarios

```
+---------------------------------------------------------------------------------------------------------+
|                                    TIER 2: BOUNDARY & CORNER CASES                                       |
+---------------------------------------------------------------------------------------------------------+
```

### 4.1 ELF & Virtual Memory Boundaries
1. **Malformed ELF Magic & Header**:
   - *Input*: Binary with header `\x7fELF\x01...` (32-bit ELF) or corrupted magic bytes `\x7fBAD`.
   - *Expected Result*: Loader rejects execution immediately with `ElfError::InvalidMagic` / `ElfError::UnsupportedBitness` without kernel panic.
2. **Interpreter Path Truncation / Missing File**:
   - *Input*: Binary specifying `PT_INTERP` path `/lib/nonexistent-linker.so.1` or path exceeding 256 bytes.
   - *Expected Result*: Loader fails gracefully with `ENOENT` / `ElfError::InterpreterNotFound`; returns `-ENOENT` to spawn syscall.
3. **Zero-Length & Misaligned Memory Operations**:
   - *Input*: `SYS_mmap(0, 0, PROT_READ, MAP_ANONYMOUS, -1, 0)` or `SYS_mprotect(0x400500, 0, PROT_READ)`.
   - *Expected Result*: Syscall returns `-EINVAL` (`-22`) without corrupting page tables.
4. **W^X Page Protection Enforcement**:
   - *Input*: Memory region marked `PROT_READ | PROT_WRITE` (with `MAP_NO_EXECUTE`); CPU attempts to branch into region.
   - *Expected Result*: CPU triggers Page Fault (#PF) with instruction fetch violation; kernel delivers `SIGSEGV` to the faulting process.

### 4.2 Signal Mask & Frame Boundaries
1. **Unblockable Signal Masking Attempt**:
   - *Input*: Calling `SYS_rt_sigprocmask(SIG_BLOCK, &(1 << (SIGKILL-1) | 1 << (SIGSTOP-1)), ...)` or `SYS_rt_sigaction(SIGKILL, ...)`.
   - *Expected Result*: Kernel preserves `SIGKILL` and `SIGSTOP` as unblockable and uncatchable; returns `-EINVAL` for sigaction on `SIGKILL`.
2. **Signal Frame Alignment & Red Zone**:
   - *Input*: Signal delivered when `%rsp = 0x7fff_ffff_e008` (unaligned).
   - *Expected Result*: Kernel adjusts stack pointer to `(rsp - sizeof(RtSigFrame)) & !15 - 8`, ensuring 16-byte stack alignment compliance upon entering handler.
3. **Recursive Signal Delivery & `SA_NODEFER`**:
   - *Input*: Signal handler for `SIGUSR1` receives second `SIGUSR1` while running.
   - *Expected Result*: Without `SA_NODEFER`, second signal is masked and remains pending until first handler executes `sigreturn`. With `SA_NODEFER`, nested signal frame is pushed onto stack.

### 4.3 Threading & Futex Limits
1. **Thread ID Exhaustion & Max Threads**:
   - *Input*: Process repeatedly spawns threads until reaching system limit (`MAX_THREADS = 64`).
   - *Expected Result*: `SYS_clone` gracefully returns `-EAGAIN` (`-11`) without resource leaks or kernel memory exhaustion.
2. **Futex Spurious Wakeup & Zero Timeout**:
   - *Input*: Calling `SYS_futex(uaddr, FUTEX_WAIT, val, &timespec{0, 0})`.
   - *Expected Result*: Syscall immediately verifies `*uaddr == val` and returns `-ETIMEDOUT` without putting thread to sleep indefinitely.
3. **Cross-Address Space Futex Isolation**:
   - *Input*: Process A and Process B (separate address spaces) wait on identical virtual address `0x4000_0000`.
   - *Expected Result*: `FUTEX_WAKE` in Process A only wakes threads in Process A's address space; Process B's thread remains sleeping.

### 4.4 Networking & TCP Boundaries
1. **TCP Multi-Packet MTU Boundary Streaming**:
   - *Input*: TCP stream payload of 4096 bytes (exceeding standard MTU of 1500 bytes and VirtIO buffer size).
   - *Expected Result*: TCP stack fragments payload into multiple segments (e.g. 1460-byte MSS), manages sequential `SEQ` and `ACK` counters, and delivers reassembled contiguous buffer.
2. **Socket Port Collision on `bind`**:
   - *Input*: Socket 1 binds to `0.0.0.0:8080`; Socket 2 attempts `bind` to `0.0.0.0:8080` without `SO_REUSEPORT`.
   - *Expected Result*: Second `SYS_bind` returns `-EADDRINUSE` (`-98`).
3. **Simultaneous Socket Teardown & Reset**:
   - *Input*: Client transmits data on socket after server has sent `FIN` or `RST`.
   - *Expected Result*: Syscall returns `-EPIPE` (`-32`) and delivers `SIGPIPE` to sender if not ignored.

---

## 5. Tier 3: Cross-Feature Combinations

```
+---------------------------------------------------------------------------------------------------------+
|                                  TIER 3: CROSS-FEATURE COMBINATIONS                                     |
+---------------------------------------------------------------------------------------------------------+
```

### 5.1 Scenario C1: Dynamic Binary + Threading + Signal Delivery
- **Description**: A dynamic executable loaded via `ld-musl-x86_64.so.1` sets up TLS, spawns 3 POSIX threads via `SYS_clone`, and registers a `SIGUSR1` handler with `SA_SIGINFO`. The main thread sends directed `SYS_tkill` signals to specific worker threads while they synchronize using `SYS_futex`.
- **Target Invariants**:
  1. Dynamic linker relocations succeed.
  2. Each thread maintains independent `FS_BASE` and signal mask.
  3. `RtSigFrame` injection occurs on the correct thread stack without corrupting sibling thread contexts.
  4. `SYS_rt_sigreturn` restores execution to worker thread futex wait.
  5. All threads join successfully via `CLONE_CHILD_CLEARTID` and `pthread_join`.

### 5.2 Scenario C2: Multi-Threaded TCP Client + Signal Interruptibility
- **Description**: Multi-threaded client where Thread 1 executes a blocking TCP `connect()` / `recvfrom()`, Thread 2 waits on a `futex` barrier, and a timer delivers `SIGALRM`.
- **Target Invariants**:
  1. If `SA_RESTART` is set, blocking socket syscall resumes transparently.
  2. If `SA_RESTART` is clear, socket syscall returns `-EINTR` (`-4`).
  3. Shared file descriptor table (`CLONE_FILES`) permits Thread 2 to inspect or close socket descriptor opened by Thread 1.

### 5.3 Scenario C3: Dynamic TCP Echo Server + Worker Pool
- **Description**: Dynamic musl C program creates a TCP server socket on port `18080`, spawns a thread pool, accepts incoming connections, passes client socket descriptor to worker thread, worker reads streaming request and writes back formatted response, then cleanly closes socket.
- **Target Invariants**:
  1. `SYS_bind`, `SYS_listen`, `SYS_accept` state transitions operate reliably under multi-threading.
  2. Descriptor reference counting closes socket only when worker terminates or drops descriptor.
  3. VirtIO-net RX/TX virtqueues handle concurrent packet ingestion and transmission.

---

## 6. Tier 4: Real-World Application Scenarios

```
+---------------------------------------------------------------------------------------------------------+
|                                 TIER 4: REAL-WORLD APPLICATION SCENARIOS                                |
+---------------------------------------------------------------------------------------------------------+
```

### 6.1 Application Scenario 1: Dynamic Musl C Suite (`dynamic-hello`)
- **Binary**: `/compat/linux/dynamic-hello` (dynamically linked against `ld-musl-x86_64.so.1` and `libc.so`).
- **Execution Flow**:
  1. Kernel detects `PT_INTERP = "/lib/ld-musl-x86_64.so.1"`.
  2. Kernel loads interpreter into high memory bias (`0x7f00_0000_0000`), sets `auxv` (`AT_BASE`, `AT_PHDR`, `AT_ENTRY`, `AT_RANDOM`).
  3. `ld-musl` performs dynamic relocations (`R_X86_64_RELATIVE`, `R_X86_64_GLOB_DAT`, `R_X86_64_JUMP_SLOT`), initializes TLS via `arch_prctl(ARCH_SET_FS)`.
  4. Calls `main()`, executes `printf("hello from dynamic musl/glibc\n")`.
  5. Exits cleanly with status 0.

### 6.2 Application Scenario 2: Dynamic POSIX Signal Harness (`dynamic-signal`)
- **Binary**: `/compat/linux/dynamic-signal`.
- **Execution Flow**:
  1. Configures `struct sigaction` with custom user handler, `SA_SIGINFO`, and `SA_RESTORER`.
  2. Sets blocked mask via `sigprocmask(SIG_BLOCK, &mask)`.
  3. Raises signal via `kill(getpid(), SIGUSR1)` (verified pending).
  4. Unblocks signal via `sigprocmask(SIG_UNBLOCK, &mask)`.
  5. Kernel injects `rt_sigframe` on stack; handler executes and verifies `siginfo->si_signo == SIGUSR1`.
  6. Handler calls `sigreturn`; execution resumes after `sigprocmask`.
  7. Prints confirmation markers and exits with status 0.

### 6.3 Application Scenario 3: Dynamic Multi-Threading & Futex Harness (`dynamic-threads`)
- **Binary**: `/compat/linux/dynamic-threads`.
- **Execution Flow**:
  1. Spawns 4 worker threads using `pthread_create` (backed by `SYS_clone` and `CLONE_SETTLS`).
  2. Each thread writes unique ID to TLS variable and executes 10,000 iterations of a shared counter guarded by a userland futex mutex (`pthread_mutex_lock`/`unlock`).
  3. Main thread joins all workers using `pthread_join` (backed by `CLONE_CHILD_CLEARTID` and `FUTEX_WAIT`).
  4. Asserts shared counter equals `40,000`.
  5. Prints confirmation markers and exits with status 0.

### 6.4 Application Scenario 4: Dynamic Network TCP/IP Probe (`dynamic-net`)
- **Binary**: `/compat/linux/dynamic-net`.
- **Execution Flow**:
  1. Probes VirtIO network interface initialization.
  2. Performs UDP DNS probe or gateway ARP resolution.
  3. Establishes TCP connection to host test listener at `10.0.2.2:18080`.
  4. Transmits ASCII probe payload `"ping"`.
  5. Receives ASCII response payload `"pong"`.
  6. Sends TCP FIN teardown.
  7. Prints network confirmation markers and exits with status 0.

---

## 7. Master Acceptance Vectors & Required Markers Matrix

The master acceptance runner `rust/test-gpt-qemu.ps1` executes automated QEMU verification and matches serial log output against the authoritative acceptance marker table below.

### 7.1 Complete Marker Verification Table

| Subsystem / Gate | Test Vector Binary | Required Serial Output Marker | Success Condition |
|---|---|---|---|
| **Storage (Gate A)** | Kernel Boot | `[storage] RedoxFS root mounted` | Superblock parsed, root directory mounted |
| **Storage (Gate A)** | First Boot | `[storage] RedoxFS reboot persistence marker: false` | Marker absent on pristine image |
| **Storage (Gate A)** | Reboot Boot | `[storage] RedoxFS reboot persistence marker: true` | Marker preserved across reboot |
| **Native Init** | `/sbin/init` | `[proc] launching native /sbin/init` | Ring-3 init process spawned |
| **Gate A Native** | `/bin/native-gate` | `[native] acceptance: developer-gate ok` | Developer UID 1000 demotion verified |
| **Gate A Native** | `/bin/vsh` suite | `[native] terminal/filesystem acceptance passed` | Shell builtins, pipes, redirections ok |
| **Gate A Native** | `/bin/vsh` | `vanta native shell` | Shell banner rendered |
| **Gate A C SDK** | `hello-vanta.elf` | `hello from C on Vanta` | Static C runtime initialized |
| **Gate A C SDK** | `sdk-smoke-vanta.elf` | `libvanta SDK smoke passed` | Core syscall wrappers ok |
| **Gate A C SDK** | `stdio-smoke-vanta.elf`| `libvanta stdio smoke passed` | Standard I/O buffering ok |
| **Gate A C SDK** | `dir-smoke-vanta.elf`  | `libvanta directory smoke passed` | Directory traversal ok |
| **Gate A C SDK** | `env-smoke-vanta.elf`  | `libvanta environment smoke passed`| Environment variable parsing ok |
| **Gate A C SDK** | `process-smoke-vanta.elf`| `libvanta process smoke passed` | Child spawning ok |
| **Gate A C SDK** | `exec-smoke-vanta.elf` | `[native] acceptance: c-exec-smoke ok` | Execve replacement ok |
| **Gate B IPC** | `/bin/procd` | `[procd] service registered` | Endpoint registered in procd |
| **Gate B IPC** | `/bin/procd` | `[procd] service upgraded` | Hot upgrade endpoint transferred |
| **Gate B IPC** | `/bin/procd` | `[procd] service discovered` | Dynamic lookup returns valid capability |
| **Gate B IPC** | `/bin/procd` | `[procd] stale service authority revoked` | Capability revocation enforced |
| **Gate B IPC** | `/bin/procd` | `[procd] vfs backend passed` | Framed IPC pair backend verified |
| **Gate B IPC** | `/bin/procd` | `[procd] service authority revoked` | Final authority revocation ok |
| **Gate B IPC** | `/bin/procd` | `[native] acceptance: procd-gate ok` | Procd test suite passed |
| **Gate B IPC** | `/bin/auditd` | `[native] acceptance: audit-persistence ok`| Audit log written to RedoxFS disk |
| **Gate B IPC** | Master Summary | `[native] Gate B IPC acceptance passed` | Aggregate Gate B ok |
| **Gate C Static**| `/compat/linux/hello` | `[linux] hello` | Linux static assembly hello ok |
| **Gate C Static**| `/compat/linux/musl-hello` | `[linux-musl] hello` | Musl static hello ok |
| **Gate C Static**| `/compat/linux/musl-alloc` | `[linux-musl] memory allocation passed` | Brk / mmap allocator ok |
| **Gate C Static**| `/compat/linux/musl-io` | `[linux-musl] file io passed` | Read/write/seek ok |
| **Gate C Static**| `/compat/linux/musl-dir` | `[linux-musl] directory iteration passed`| Getdents64 ok |
| **Gate C Static**| `/compat/linux/musl-pipe` | `[linux-musl] pipes and descriptors passed` | Pipe2 / dup3 ok |
| **Gate C Static**| `/compat/linux/musl-proc` | `[linux-musl] posix system info passed` | Uname / sysinfo ok |
| **Gate C Static**| `/compat/linux/musl-script` | `[linux-musl] script sequencing passed` | Executable scripts ok |
| **Gate C Static**| `/compat/linux/musl-server` | `[linux-musl] socket execution passed` | Static socket baseline ok |
| **Gate C Static**| `/compat/linux/unsupported` | `[linuxd] unsupported syscall number=9999`| Unimplemented syscall returns error |
| **Gate C Static**| Master Summary | `[linux] Gate C personality acceptance passed`| Aggregate Gate C ok |
| **Gate D Dynamic**| `/compat/linux/dynamic-hello` | `[linux-dynamic] dynamic interpreter loaded` | Interpreter base & auxv mapped |
| **Gate D Dynamic**| `/compat/linux/dynamic-hello` | `[linux-dynamic] hello from dynamic musl/glibc` | Dynamic C program execution ok |
| **Gate D Signal** | `/compat/linux/dynamic-signal`| `[linux-dynamic] signal handler registered` | `SYS_rt_sigaction` ok |
| **Gate D Signal** | `/compat/linux/dynamic-signal`| `[linux-dynamic] signal delivered and handled` | `RtSigFrame` injected & executed |
| **Gate D Signal** | `/compat/linux/dynamic-signal`| `[linux-dynamic] rt_sigreturn restored context` | `SYS_rt_sigreturn` resumes task |
| **Gate D Threads**| `/compat/linux/dynamic-threads`| `[linux-dynamic] thread spawned` | `SYS_clone` creates thread |
| **Gate D Threads**| `/compat/linux/dynamic-threads`| `[linux-dynamic] thread TLS verified` | `FS_BASE` TLS isolated |
| **Gate D Threads**| `/compat/linux/dynamic-threads`| `[linux-dynamic] futex synchronization passed` | `SYS_futex` wait/wake mutex ok |
| **Gate D Threads**| `/compat/linux/dynamic-threads`| `[linux-dynamic] thread joined successfully` | `pthread_join` / clear_child_tid ok |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] virtio-net adapter initialized` | VirtIO-net driver ready |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] arp resolution passed` | Dynamic ARP table entry populated |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] udp datagram send/receive passed` | UDP packet exchange ok |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] tcp client connection established` | 3-way handshake established |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] tcp payload stream passed` | TCP ping/pong exchange ok |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[net] tcp server listener accepted connection` | TCP server accept ok |
| **Gate D Net** | `/compat/linux/dynamic-net` | `[linux-dynamic] network acceptance passed` | Aggregate dynamic network ok |
| **Gate D Master** | Master Summary | `[native] Gate D dynamic, signals, threads & networking acceptance passed` | Full Gate D acceptance passed |
| **Recovery** | Corrupt Disk Boot | `[recovery] entering kernel recovery shell` | Superblock corruption trapped |
| **Recovery** | Corrupt Disk Boot | `[shell] entering main loop` | Serial emergency shell active |

---

## 8. Harness Execution Invocations

### 8.1 Automated Test Execution Command
```powershell
# Execute complete Gate D Acceptance Suite with VirtIO-Net, Reproducibility, Persistence, and Recovery
powershell -NoProfile -ExecutionPolicy Bypass -File rust/test-gpt-qemu.ps1 -TimeoutSeconds 60
```

### 8.2 Unit & Contract Test Suite
```powershell
# ABI & Translation Contract Verification
cargo test -p vanta-abi
cargo test -p vanta-linuxd

# Image Assembly & Partition Table Determinism
cargo xtask image
```

---

## 9. Conclusion
This testing specification provides full coverage across all four tiers, defining clear interfaces, boundaries, and deterministic acceptance markers to verify Gate D implementation and guard against any regressions in Vanta OS.
