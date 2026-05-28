# Rust Kernel Rewrite — Session 1 Design

## Goal

Boot a fresh Rust kernel to an in-kernel framebuffer terminal that echoes typed characters. Foundation for later layers (userspace, windowing). Existing Zig code is left untouched on the `Antigravity` branch.

## Scope (this session)

In:
- Boot via the `bootloader` crate (BIOS + UEFI disk image, no external tooling)
- Serial logger on COM1 (debug output to host)
- GDT, TSS, IDT with CPU exception handlers
- PIC remap, timer IRQ (PIT) and keyboard IRQ (PS/2)
- Framebuffer text writer using a bitmap font (8x16)
- PS/2 keyboard via `pc-keyboard` crate, line buffer, in-kernel shell that echoes input and handles backspace
- `cargo run` boots QEMU and shows a prompt

Out (later sessions):
- Userspace, ELF loader, syscalls
- Windowing / compositor / mouse
- Filesystems
- Networking
- Cross-OS binary compatibility (Linux/Win/macOS)

## Boot path

Pivoted from Limine to the `bootloader` crate because xorriso is not available on the Windows host. `bootloader` generates BIOS and UEFI disk images via a build script — no external tools required. It exposes a framebuffer and memory map equivalent to Limine for our needs.

## Layout

```
rust/
  Cargo.toml                  # workspace
  rust-toolchain.toml         # nightly pin
  kernel/
    Cargo.toml
    x86_64-vanta.json         # custom no-std target
    src/
      main.rs                 # entry_point!, panic handler, init order
      serial.rs               # uart_16550 COM1 logger
      gdt.rs                  # GDT + TSS + double-fault stack
      interrupts.rs           # IDT, exception handlers, PIC, IRQ stubs
      framebuffer.rs          # Framebuffer wrapper + bitmap font + Writer
      font.rs                 # noto-sans-mono-bitmap glyph lookup
      keyboard.rs             # IRQ1 handler, scancode -> KeyEvent queue
      shell.rs                # consume key events, draw prompt, echo
  runner/
    Cargo.toml
    src/main.rs               # build_image -> qemu-system-x86_64
```

## Crate selection (proven, off-the-shelf)

- `bootloader` / `bootloader_api` — boot + framebuffer + memory map
- `x86_64` — GDT, IDT, paging types, port I/O wrappers
- `uart_16550` — serial driver
- `pic8259` — PIC remap + EOI
- `pc-keyboard` — scancode -> KeyEvent
- `noto-sans-mono-bitmap` — pre-rasterized bitmap font glyphs
- `spin` — no_std mutex
- `lazy_static` (or `once_cell`) — global statics
- `conquer-once` — IRQ-safe lazy init for the key queue
- `crossbeam-queue` (`ArrayQueue`) — lock-free MPSC for IRQ -> shell

## Init order in `_start`

1. Init serial → log "boot"
2. Init GDT/TSS → load segments
3. Init IDT → load
4. Init PIC, remap to 32..47, mask all
5. Init framebuffer from bootloader info
6. Unmask timer (IRQ0) and keyboard (IRQ1)
7. `sti`
8. Spawn shell loop: poll key queue, render to framebuffer

## Verification

`cargo run` from `rust/` should:
- Build kernel for `x86_64-vanta` target
- Build disk image via `bootloader` build script
- Launch QEMU with `-drive format=raw`, `-serial stdio`
- Serial shows init log lines
- QEMU framebuffer shows `vanta> ` prompt
- Typing letters echoes them; backspace deletes; Enter starts new line

## Risks / known unknowns

- `bootloader` crate API has changed across major versions; pinning a known-good version (0.11.x)
- Custom target JSON has to disable SSE/MMX and enable soft-float for kernel mode
- PIC vs APIC: starting with legacy PIC (simpler, fewer moving parts). APIC later.
- PS/2 only — no USB keyboard. Sufficient for QEMU.

## Out of scope, explicitly

This is not Linux, Windows, or macOS compatibility. This is not a windowing system. This is a foundation. Anything that says "easy migration from $OTHER_OS" is a later, separate, multi-session effort.
