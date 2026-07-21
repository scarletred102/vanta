# Vanta

Can AI break the barrier of making another universally compatible and usable OS?

Vanta is an experimental operating-systems project pursuing that question. It is not yet a universally compatible, production-ready operating system.

## Tracks

- [`zig/`](zig/README.md) is the capability-oriented Zig kernel track.
- [`rust/`](rust/README.md) is the active Rust-native rewrite.

The tracks are separate experiments with different implementations and maturity.
Neither is a general-purpose replacement for Linux or another established
operating system.

## Build and verification

### Zig capability kernel

The Zig source is preserved as a separate experimental track. It is not part
of the current release-verification matrix; see the [Zig README](zig/README.md)
for its scope and prerequisites. Do not treat it as a supported or
release-verified boot path.

### Rust-native rewrite

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

## Current limitations

- Development and verification currently target QEMU; Vanta makes no
  hardware-compatibility promise.
- It is not Linux-compatible or Linux-equivalent, and does not promise a
  stable application, driver, filesystem, or binary interface.
- The implemented subsystems are incomplete and may change substantially.
- Do not use either track for production workloads or depend on it for data
  safety, security isolation, or device support.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution expectations and
[.github/SECURITY.md](.github/SECURITY.md) for private vulnerability reporting.

## License

Licensed under the [Apache License 2.0](LICENSE).
