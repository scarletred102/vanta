# Vanta — Rust kernel rewrite

This is the active Rust-native rewrite. For the project overview and the Zig
capability kernel track, see the [repository README](../README.md).

Rust x86_64 kernel foundation. It boots via UEFI + Limine, draws an in-kernel
terminal to the framebuffer, and echoes PS/2 keystrokes. The rewrite follows
Linux-style subsystem boundaries while retaining Vanta's own microkernel and
capability model; it is not a copy of Linux.

## Linux reference source

Vanta's pinned Linux reference is version 6.18.39. Fetch it outside git with:

```powershell
..\scripts\fetch-linux.ps1
```

The script verifies the official archive checksum before extraction. The
source is intentionally kept out of this repository; Vanta code is written
independently in Rust.

## Run

```powershell
.\run.ps1            # build + boot with a graphical window
```

```bash
./run.sh             # same, from git-bash
HEADLESS=1 ./run.sh  # serial-only, useful for CI
```

Run the checked QEMU regressions from PowerShell:

```powershell
.\test-qemu.ps1          # RAM-root lifecycle and SMP checks
.\test-qemu.ps1 -Virtio  # also require persistent legacy VirtIO storage
.\test-qemu.ps1 -Network # require VFS-configured ARP, ICMP, UDP DNS, and ring-3 TCP
```

Build the persistent GPT disk artifact:

```powershell
cargo xtask image
```

This writes `target/vanta-gpt.img` and `target/vanta-gpt.manifest`. The image
has a FAT ESP containing Limine and the kernel plus a bounded, formatted
RedoxFS root partition. The kernel mounts that RedoxFS partition as the
writable root and enters the serial recovery path if it is absent or invalid.

Requires:
- rustup with a nightly toolchain (a `rust-toolchain.toml` pins this) on a
  `x86_64-pc-windows-gnu` host
- QEMU bundled with edk2 UEFI firmware (`qemu-system-x86_64` + the
  `edk2-x86_64-code.fd` that ships under `share\`)
- The Limine `BOOTX64.EFI` (checked into `esp/EFI/BOOT/`)

The build script compiles `kernel/`, copies the binary into `esp/boot/`, then
boots QEMU pointed at the ESP via the `fat:rw:` trick — no ISO tools required.

## Layout

```
rust/
  Cargo.toml                 # workspace: ABI, image, kernel, SDK, services
  xtask/                     # reproducible SDK and GPT image commands
  abi/                       # versioned native Vanta ABI v0
  gpt/ image/                # GPT validation and RedoxFS image builder
  redoxfs-adapter/           # credential-aware RedoxFS backend boundary
  libvanta/                  # freestanding C ABI bootstrap library
  linuxd/ services/          # compatibility and service contracts
  kernel/
    Cargo.toml
    rust-toolchain.toml      # nightly
    linker.ld                # higher-half kernel, Limine PHDRs
    .cargo/config.toml       # x86_64-unknown-none target + link flags
    src/
      main.rs                # _start, request statics, init order
      elf.rs                 # ELF64 parser + embedded CPL3 test image
      fs.rs                  # read-only CPIO newc initramfs
      storage.rs             # sector block-device trait + RAM block driver
      virtio.rs              # legacy VirtIO PCI block driver + DMA split ring
      vfs.rs                 # RedoxFS root adapter + recovery VantaFS
      serial.rs              # COM1 logger
      gdt.rs                 # per-CPU GDT/TSS/stacks + ring-3 entry
      interrupts.rs          # IDT, PIC/PIT, exception + IRQ handlers
      apic.rs                # local-APIC discovery + MMIO/x2APIC setup
      acpi.rs                # checked RSDP/XSDT discovery and MADT/MCFG summaries
      smp.rs                 # Limine AP handoff + locked kernel work queue
      framebuffer.rs         # Limine framebuffer + bitmap font
      memory.rs              # memory-map accounting + physical frames
      paging.rs              # HHDM translation + mutable page-table manager
      pci.rs                 # serialized legacy PCI configuration-space discovery
      heap.rs                # mapped free-list + global Rust allocator
      process.rs              # ELF PT_LOAD mapping + user stack lifecycle
      scheduler.rs           # tasks, descriptors, pipe queues, signals, waits
      syscall.rs              # per-CPU syscall/sysret entry + native ABI
      keyboard.rs            # IRQ1 scancode queue
      shell.rs               # decoder + echo loop
  test-qemu.ps1              # legacy/VirtIO/network QEMU regressions
  test-gpt-qemu.ps1          # GPT, RedoxFS, native-init acceptance
  esp/
    EFI/BOOT/BOOTX64.EFI     # Limine UEFI bootloader
    boot/vanta-kernel        # built kernel (gitignored)
    limine.conf
  run.ps1 / run.sh
```

## What works

- UEFI → Limine → kernel `_start` in long mode
- Framebuffer init (verified 1280x800x32 in QEMU)
- GDT + TSS with double-fault IST
- IDT with CPU exception + timer + keyboard IRQ handlers
- PIC 8259 remap, 100 Hz PIT, IRQ0 + IRQ1 unmasked
- Local APIC discovery and uncached xAPIC MMIO / x2APIC software enable
- ACPI RSDP/XSDT validation with firmware-table checksum checks, including
  MADT processor/IOAPIC/IRQ-override discovery and MCFG region counting
- Limine SMP handoff: per-CPU GDT/TSS/IDT setup, AP acknowledgement, and
  locked kernel-work dispatch before safe AP halt
- Concurrent AP user-mode scheduling with isolated per-CPU run queues, global
  PIDs, and local-APIC timer preemption verified in two-vCPU QEMU
- PS/2 keyboard via `pc-keyboard` scancode decoder
- Limine memory-map accounting and bounded physical-frame allocator
- HHDM translation and page-table inspection of Limine's active mappings
- Mapped reclaiming Rust kernel heap with `Vec`/`Box` allocation self-checks
- ELF64 PT_LOAD loading into an isolated address space with user/NX flags
- Reclaimable process mappings and a four-page user stack
- Ring-3 entry through `iretq`, with a TSS privilege stack and syscall test
- Per-CPU `syscall`/`sysretq` ABI with `open`, `read`, `write`, `close`,
  `lseek`, `dup`, `getpid`, `yield`, `exit`, `socket`, and `connect` calls
- Per-process descriptor tables with Linux-style shared open-file offsets for
  duplicated descriptors
- Conventional descriptor numbers: `0` stdin, `1` stdout, `2` stderr, and
  VFS opens beginning at `3`
- `SYS_SPAWN` loads a VFS-backed ELF into a new address space with a distinct
  child PID and recorded parent PID
- `SYS_WAITPID` blocks the calling parent until its selected child exits, then
  wakes it with the child exit code
- Native pipe reads block the calling task in the kernel until a writer adds
  data or closes the pipe; writers wake matching blocked readers
- `SYS_EXEC` replaces the current process with a VFS-backed ELF; the old image
  is reclaimed before control enters the replacement
- Timer-preemptive round-robin scheduler with complete callee-saved user
  context across timer and voluntary syscall switches
- Per-CPU syscall stacks and return state selected through the kernel GS base
- Per-CPU round-robin address-space switching and per-process exit reclamation
- User process exit switches back to the kernel address space and reclaims it
- Read-only CPIO `newc` initramfs with `/bin/init` and `/etc/motd` lookup
- Filesystem-backed `/bin/init` loading through the ELF/process path
- Sector block-device abstraction with RAM and legacy VirtIO PCI drivers
- Legacy VirtIO-net polling driver with Ethernet ARP, IPv4 ICMP echo, UDP DNS,
  and a bounded TCP stream path through QEMU's NAT services
- VFS-backed `/etc/network.conf`, created on first boot and used for the guest
  address, gateway, DNS server, and TCP probe target
- Ring-3 TCP socket regression: `socket(AF_INET, SOCK_STREAM, 0)`,
  `connect(sockaddr_in)`, `write("ping")`, `read("pong")`, and `close` with a
  host-side FIN observation in QEMU
- Legacy PCI configuration-space enumeration shared by platform diagnostics and
  the VirtIO block probe
- Writable VantaFS root mount with remount/persistence self-checks
- Persistent VantaFS auto-format/mount on an attached legacy VirtIO disk,
  verified through an attached-disk write/read round trip and a second boot
- Persistent GPT RedoxFS root with ownership, group, mode, traversal, and umask
  enforcement for the `vanta` developer account
- Native `/sbin/init`, `/bin/vsh`, and static Rust base commands installed in
  the GPT image
- Native shell execution with command arguments, `<`, `>`, `>>`, `2>`, a real
  `echo | cat` pipeline, child waits, and foreground Ctrl-C targeting
- `libvanta` bootstrap static library, C header, freestanding allocator/CRT
  entry, and reproducible `cargo xtask sdk` output including `hello-vanta.elf`
- ABI v0 contract vectors for syscall numbers, feature bits, errno decoding,
  capability boundaries, credentials, signal layout, and directory records;
  see [`abi/README.md`](abi/README.md)
- Native `GetAbiInfo` query through the kernel, Rust userland, and `libvanta`;
  the GPT C hello acceptance program validates the returned version and size
- `libvanta` wrappers for the currently implemented descriptor, directory,
  pipe, process, scheduling, signal, and path-mutation syscalls; the GPT
  `c-sdk-smoke` program exercises that surface end to end
- Linux syscall translation and restartable-service contract crates as the
  foundation for later compatibility personalities

## TCP user ABI

Vanta currently supports one synchronous IPv4 stream operation at a time per
socket descriptor. `socket(2, 1, 0)` creates a descriptor, and
`connect(fd, sockaddr_in*, 16)` accepts the conventional 16-byte `sockaddr_in`
layout. Existing `read`, `write`, `dup`, and `close` operate on that descriptor;
`close` sends FIN when the last duplicate is closed. TCP payloads are limited to
64 bytes, with no retransmission, receive queue, fragmentation, or listener API
yet.

The default configuration is:

```text
address=10.0.2.15
gateway=10.0.2.2
dns=10.0.2.3
tcp_host=10.0.2.2
tcp_port=18080
```

## Current limitations

- No copy-on-write `fork` yet
- No slab allocator yet; the bootstrap free-list has fixed metadata capacity
- No modern VirtIO PCI transport or filesystem journaling
- No TCP retransmission, listener/accept path, UDP socket ABI, DHCP, or native
  DNS resolver; the QEMU DNS regression path itself is passing
- `sigaction` currently supports default and ignore dispositions; custom user
  handler delivery and full POSIX process groups are not implemented
- The native C runtime is only the bootstrap profile; full stdio, directories,
  environment, and relibc compatibility remain Track B work
- ABI v1 negotiation is not implemented; the current native query reports the
  frozen ABI v0 contract and rejects no unknown mandatory bits implicitly
- No mouse, no windowing — terminal only
- No SMP task migration, load balancing, or idle-CPU wake IPIs yet

## Verifying the keyboard pipeline without a GUI

`HEADLESS=1 ./run.sh` redirects serial to stdout. The kernel logs the first 8
keyboard IRQs with their scancodes so you can confirm the chain works without
needing the QEMU window.
