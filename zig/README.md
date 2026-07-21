# Vanta — Zig capability kernel

This is Vanta's capability-oriented Zig kernel track. It is an experimental
QEMU-targeted kernel, not a universally compatible or production-ready
operating system. For the project overview and the Rust-native rewrite, see
the [repository README](../README.md).

## Verification status

This source track is preserved for exploration, but it is not part of Vanta's
current release-verification matrix. Its existing build and run scripts remain
in the track for development work; they are not a supported or
release-verified boot path. Use the [`rust/`](../rust/README.md) track for the
reproducible QEMU verification workflow.

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
