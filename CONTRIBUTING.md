# Contributing to Vanta

Vanta is an experimental operating-systems project. Keep contributions small,
focused, and candid about the maturity and limitations of the affected track.

## Before opening a pull request

- Keep each commit focused on one change. Do not combine unrelated cleanup,
  generated output, or formatting churn with the change.
- Do not commit generated artifacts. In particular, keep Zig `kernel/bin`
  programs, downloaded `zig/limine-bin` payloads, build outputs, ISO images,
  and QEMU logs out of version control.
- Update public documentation when a change alters public behavior, commands,
  requirements, or stated limitations.
- Preserve the distinction between the Zig capability kernel and the Rust
  rewrite; do not imply either is universally compatible or production-ready.

## Verify the affected track

For a Zig change, first establish and report the current build status for the
affected code. The Zig track is not part of the release-verification matrix,
so do not present an exploratory result as a supported baseline.

For a Rust change, run from `rust/` with the nightly pinned in
`kernel/rust-toolchain.toml`:

```powershell
.\test-qemu.ps1
```

Include the commands you ran and any relevant limitations in the pull request
description.
