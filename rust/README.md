# Vanta — Rust rewrite, session 1

Minimal x86_64 kernel. Boots via UEFI + Limine, draws an in-kernel terminal to
the framebuffer, echoes PS/2 keystrokes.

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
      serial.rs              # COM1 logger
      gdt.rs                 # GDT + TSS + double-fault IST
      interrupts.rs          # IDT, PIC, exception + IRQ handlers
      framebuffer.rs         # Limine framebuffer + bitmap font
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
- In-kernel shell: prompt, echo, backspace, newline

## What does not (yet)

- No userspace, no syscalls, no ELF loader
- No heap; everything is static / `heapless`
- No filesystem, no networking
- No mouse, no windowing — terminal only
- Single-CPU only; SMP/APIC come later

## Verifying the keyboard pipeline without a GUI

`HEADLESS=1 ./run.sh` redirects serial to stdout. The kernel logs the first 8
keyboard IRQs with their scancodes so you can confirm the chain works without
needing the QEMU window.
