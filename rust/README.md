# Vanta — Rust kernel rewrite

Rust x86_64 kernel foundation. It boots via UEFI + Limine, draws an in-kernel
terminal to the framebuffer, and echoes PS/2 keystrokes. The rewrite follows
Linux-style subsystem boundaries while retaining Vanta's own microkernel and
capability model; it is not a copy of Linux.

## Linux reference source

The pinned reference is Linux 6.18.39, the current long-term branch selected
for a stable architecture baseline. Fetch it outside git with:

```powershell
.\scripts\fetch-linux.ps1
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
      vfs.rs                 # writable VantaFS volume + root mount
      serial.rs              # COM1 logger
      gdt.rs                 # GDT + TSS + ring-3 entry + double-fault IST
      interrupts.rs          # IDT, PIC, exception + IRQ handlers
      framebuffer.rs         # Limine framebuffer + bitmap font
      memory.rs              # memory-map accounting + physical frames
      paging.rs              # HHDM translation + mutable page-table manager
      heap.rs                # mapped free-list + global Rust allocator
      process.rs              # ELF PT_LOAD mapping + user stack lifecycle
      scheduler.rs           # cooperative task table + round-robin switching
      syscall.rs              # syscall/sysret entry + write/exit ABI
      keyboard.rs            # IRQ1 scancode queue
      shell.rs               # decoder + echo loop
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
- PIC 8259 remap, IRQ0 + IRQ1 unmasked
- PS/2 keyboard via `pc-keyboard` scancode decoder
- Limine memory-map accounting and bounded physical-frame allocator
- HHDM translation and page-table inspection of Limine's active mappings
- Mapped reclaiming Rust kernel heap with `Vec`/`Box` allocation self-checks
- ELF64 PT_LOAD loading into an isolated address space with user/NX flags
- Reclaimable process mappings and a four-page user stack
- Ring-3 entry through `iretq`, with a TSS privilege stack and syscall test
- Single-CPU `syscall`/`sysretq` ABI with user `write` and `exit` calls
- Cooperative single-CPU scheduler with two user processes and `yield`
- Round-robin address-space switching and per-process exit reclamation
- User process exit switches back to the kernel address space and reclaims it
- Read-only CPIO `newc` initramfs with `/bin/init` and `/etc/motd` lookup
- Filesystem-backed `/bin/init` loading through the ELF/process path
- Sector block-device abstraction with a writable RAM-disk implementation
- Writable VantaFS root mount with remount/persistence self-checks
- In-kernel shell: prompt, echo, backspace, newline

## What does not (yet)

- No preemptive scheduling, fork/exec, or broader syscall surface yet
- No slab allocator yet; the bootstrap free-list has fixed metadata capacity
- No VirtIO/persistent block driver, filesystem journaling, or networking
- No mouse, no windowing — terminal only
- Single-CPU only; SMP/APIC come later

## Verifying the keyboard pipeline without a GUI

`HEADLESS=1 ./run.sh` redirects serial to stdout. The kernel logs the first 8
keyboard IRQs with their scancodes so you can confirm the chain works without
needing the QEMU window.
