# Vanta

Can AI break the barrier of making another universally compatible and usable OS?

Vanta is an experimental operating-systems project pursuing that question. It is not yet a universally compatible, production-ready operating system.

## Architecture

Vanta OS is a 100% Rust-native microkernel operating system featuring capability-based IPC, persistent transactional RedoxFS storage, an interactive Orbital window compositor, a 300+ command BusyBox Unix environment, embedded Lua 5.4 scripting, and dual-personality binary compatibility (native Vanta ABI and Linux x86_64 ELF personality).

> [!NOTE]
> The early experimental Zig kernel prototype has been archived on the [`archive/zig-track`](https://github.com/scarletred102/vanta/tree/archive/zig-track) branch. The active development and release tree on `main` is completely Rust-native.

## Build and verification

From `rust/`, use the nightly pinned in `kernel/rust-toolchain.toml`:

```powershell
.\test-qemu.ps1
```

For an interactive boot instead of the checked QEMU regression:

```powershell
.\run.ps1
```

The Rust QEMU checks need QEMU with edk2 UEFI firmware. Its separate Linux
reference helper is run from `rust/` as `..\scripts\fetch-linux.ps1`.

## Current status
 
Vanta OS has verified Gate A, Gate B, Gate C, and Gate D execution foundations and full operating system functionality:
- **Gate A**: Partitioned GPT disk, writable RedoxFS persistent root mount, permission and umask enforcement, and freestanding C SDK (`libvanta`).
- **Gate B**: Microkernel IPC services (`procd`, `auditd`, `vfsd`) with capability passing, generation bumps, and authority revocation.
- **Gate C**: Static Linux personality via `vanta-linuxd` broker with argv/envp/auxv, memory mappings, and system call translation.
- **Gate D**: Dynamic ELF interpreter loading (`ld-musl`), POSIX signals with stack frame injection (`rt_sigframe`), multi-threading (`clone`/`futex`), and complete VirtIO-Net TCP/IP networking (ARP, ICMP, UDP, TCP client/server stream).
- **Unix Shell & BusyBox 300+ Suite**: Static BusyBox suite with 50 standard Unix links (`sh`, `vi`, `grep`, `sed`, `awk`, `tar`, `find`, `wget`, `top`, `ps`, etc.) backed by zero-overhead RedoxFS hard links, full termios ioctl emulation, process groups, and session IDs.
- **Redox Orbital Windowing System**: Complete `orbclient` 2D graphics engine, `orbital` window compositor with dynamic z-ordering, draggable decorated windows with close/minimize/maximize buttons, and `orbterm` graphical terminal emulator running `/bin/sh`.
- **Scripting Runtime**: Embedded Lua 5.4 scripting engine runtime (`/bin/lua`) for scripting and automation.
- **Package Management**: Native package manager `vpkg` (`/bin/vpkg`) with package database tracking and deployment.
- **Master Acceptance**: Master acceptance harness (`test-gpt-qemu.ps1`) verifies all gates and OS subsystems across first boot, reboot persistence, and corrupt-root recovery.
 
- Development and verification target QEMU; Vanta makes no hardware-compatibility promise.
- It is an experimental microkernel OS with a Linux binary compatibility broker.
- Do not use for production workloads or depend on it for data safety or security isolation.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution expectations and
[.github/SECURITY.md](.github/SECURITY.md) for private vulnerability reporting.

## License

Licensed under the [Apache License 2.0](LICENSE).
