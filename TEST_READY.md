# Vanta OS Gate D: Test Readiness & Verification Matrix

**Document Version**: 1.0.0  
**Target Gate**: Gate D (Dynamic ELF, POSIX Signals, Multi-Threading, VirtIO-Net TCP/IP Stack)  
**Status**: TEST READY & PUBLISHED  
**Reference Specification**: `TEST_INFRA.md`, `PROJECT.md`, `ORIGINAL_REQUEST.md`  

---

## 1. Executive Summary

This document establishes the official **Test Readiness Matrix** for Vanta OS Gate D. The test harness and acceptance infrastructure have been fully specified across all 4 tiers (Feature Coverage, Boundary & Stress, Cross-Feature Combinations, and Real-World Scenarios) and are ready for implementation verification.

The acceptance suite enforces end-to-end regression validation covering **Gate A** (GPT partitioning, RedoxFS root mount, persistence, C SDK), **Gate B** (microkernel IPC services, procd/auditd/vfsd authority revocation), **Gate C** (static Linux personality), **Gate D** (dynamic ELF loading, POSIX signals, multi-threading, TCP/IP stack), **reboot persistence**, and **corrupt-root recovery**.

---

## 2. Milestone Test Matrix & Feature Mapping

| Milestone | Subsystem / Feature Under Test | Acceptance Binary / Vector | Primary Assertions & Expected Markers |
|---|---|---|---|
| **M1** | Dynamic ELF & Auxiliary Vector (`auxv`) | `/compat/linux/dynamic-hello` | `[linux-dynamic] dynamic interpreter loaded`<br>`[linux-dynamic] hello from dynamic musl/glibc` |
| **M1** | Memory Protection & Paging | Kernel Unit / Syscall test | `SYS_mprotect` enforces `PROT_READ` / `PROT_WRITE` / `PROT_EXEC` with TLB invalidation |
| **M2** | POSIX Signal Subsystem | `/compat/linux/dynamic-signal` | `[linux-dynamic] signal handler registered`<br>`[linux-dynamic] signal delivered and handled`<br>`[linux-dynamic] rt_sigreturn restored context` |
| **M2** | Directed Signal Delivery | `SYS_tkill` / `SYS_tgkill` | Pending signal delivered to target thread before user return |
| **M3** | Multi-Threading & Process Hierarchy | `/compat/linux/dynamic-threads` | `[linux-dynamic] thread spawned`<br>`[linux-dynamic] thread TLS verified`<br>`[linux-dynamic] thread joined successfully` |
| **M3** | Futex Subsystem & Synchronization | `/compat/linux/dynamic-threads` | `[linux-dynamic] futex synchronization passed`<br>`FUTEX_WAIT` / `FUTEX_WAKE` mutex ordering |
| **M4** | VirtIO-Net Driver & TCP/IP Stack | `/compat/linux/dynamic-net` | `[net] virtio-net adapter initialized`<br>`[net] arp resolution passed`<br>`[net] udp datagram send/receive passed`<br>`[net] tcp client connection established`<br>`[net] tcp payload stream passed`<br>`[net] tcp server listener accepted connection`<br>`[linux-dynamic] network acceptance passed` |
| **M5** | Full Integration & Gate D Acceptance | Master Init Runner | `[native] Gate D dynamic, signals, threads & networking acceptance passed` |
| **Regression** | Gate A (Developer Gate & Storage) | `/sbin/init`, `/bin/native-gate` | `[storage] RedoxFS root mounted`<br>`[native] acceptance: developer-gate ok`<br>`[native] terminal/filesystem acceptance passed` |
| **Regression** | Gate A (C SDK Suite) | `target/sdk/*.elf` | `hello from C on Vanta`<br>`libvanta SDK smoke passed`<br>`[native] acceptance: c-exec-smoke ok` |
| **Regression** | Gate B (Microkernel IPC & Audit) | `/bin/procd`, `/bin/auditd` | `[procd] service registered`<br>`[procd] service upgraded`<br>`[procd] stale service authority revoked`<br>`[native] Gate B IPC acceptance passed` |
| **Regression** | Gate C (Static Linux Personality) | `/compat/linux/musl-*` | `[linux] hello`<br>`[linux-musl] memory allocation passed`<br>`[linux-musl] socket execution passed`<br>`[linux] Gate C personality acceptance passed` |
| **Regression** | Reboot Persistence Verification | Phase 2 QEMU Boot | `[storage] RedoxFS reboot persistence marker: true` |
| **Regression** | Corrupt-Root Recovery Shell | Phase 3 Truncated Disk Boot | `[recovery] entering kernel recovery shell`<br>`[shell] entering main loop` |

---

## 3. Test Verification Command Suite

### 3.1 Master QEMU Acceptance & Regression Suite
```powershell
# Run the master automated test harness across all phases (first boot, reboot persistence, corrupt-root recovery)
powershell -NoProfile -ExecutionPolicy Bypass -File rust/test-gpt-qemu.ps1 -TimeoutSeconds 60
```

### 3.2 ABI & Compatibility Unit Test Suite
```powershell
# Run ABI contract tests
cargo test -p vanta-abi

# Run Linux compatibility translation tests
cargo test -p vanta-linuxd
```

### 3.3 Disk Image & Build Determinism Test
```powershell
# Validate GPT disk build and double-build bit-for-bit SHA256 reproducibility
cargo xtask image
```

---

## 4. Pass/Fail Acceptance Criteria

A test run is classified as **PASS** if and only if **ALL** of the following conditions are met:
1. **Determinism**: Rebuilding `cargo xtask image` produces identical SHA256 hashes for `target/vanta-gpt.img` and `target/vanta-gpt.manifest`.
2. **Phase 1 (First Boot)**:
   - Kernel boots with UEFI + Limine on dual virtual CPUs (`-smp 2`) and 256MB RAM (`-m 256M`).
   - Legacy VirtIO block and network adapters are initialized (`0x1AF4:0x1000` and `0x1AF4:0x1001`).
   - RedoxFS root partition is mounted; first boot marker indicates `false`.
   - `/sbin/init` spawns and validates Gate A developer demotion (UID 1000) and shell features.
   - Microkernel IPC services (`procd`, `auditd`, `vfsd`) pass all registration, upgrade, revocation, and audit logging checks.
   - Static Linux personality passes all 12 test binaries (`musl-hello`, `musl-alloc`, `musl-io`, `musl-dir`, `musl-pipe`, `musl-proc`, `musl-script`, `musl-server`, `unsupported`).
   - Dynamic Linux subsystem executes `dynamic-hello`, `dynamic-signal`, `dynamic-threads`, and `dynamic-net` with 0 exit codes.
   - Serial log emits `[native] Gate D dynamic, signals, threads & networking acceptance passed`.
3. **Phase 2 (Reboot Persistence)**:
   - Second boot on identical disk verifies persistent state; marker indicates `true`.
   - All Gate A, B, C, and D acceptance vectors execute and pass again.
4. **Phase 3 (Corrupt-Root Recovery)**:
   - Truncated root disk image triggers recovery logic: serial console confirms `[recovery] entering kernel recovery shell` and `[shell] entering main loop`.

---

## 5. Defect Escalation & QA Protocols

In accordance with the test writer role:
- **Test Integrity**: Test writer modifies test infrastructure and specifications only.
- **Bug Discovery**: When an implementation defect is identified during milestone verification, the test writer documents the exact observation, register dump/serial log, failure classification, and notifies the responsible worker agent via message with references to `TEST_INFRA.md`.
- **Sign-Off**: Final sign-off requires passing the full test matrix in `rust/test-gpt-qemu.ps1`.
