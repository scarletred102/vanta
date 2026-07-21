# Vanta — Zig capability kernel

This is Vanta's capability-oriented Zig kernel track. It is an experimental
QEMU-targeted kernel, not a universally compatible or production-ready
operating system. For the project overview and the Rust-native rewrite, see
the [repository README](../README.md).

## Build and run

Run these commands from this `zig/` directory with Zig 0.16.0:

```powershell
zig build
./run.ps1
```

`zig build` compiles the kernel. `run.ps1` builds the userspace server
programs, creates `vanta.iso`, then starts QEMU. Use `-NoDisplay` for serial
output without the graphical QEMU window.

## Prerequisites

- Zig 0.16.0
- Python 3 for `tools/build_iso.py`
- QEMU (`qemu-system-x86_64`)
- ISO tooling: `xorriso`
- Network access the first time the ISO builder downloads Limine bootloader
  files

The checked-in source deliberately does not include generated `kernel/bin`
programs or downloaded `limine-bin` payloads. The build and ISO workflow
creates or downloads them locally; they are ignored by Git.

## Scope and limitations

This track is developed and exercised in QEMU. It makes no hardware support,
driver compatibility, application compatibility, or Linux-equivalence claim.
Its interfaces and implemented subsystems remain experimental and may change.
