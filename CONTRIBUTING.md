# Contributing to Vanta

Vanta is an experimental operating-systems project. Keep contributions small,
focused, and candid about the maturity and limitations of the affected track.

## Before opening a pull request

- Keep each commit focused on one change. Do not combine unrelated cleanup,
  generated output, or formatting churn with the change.
- Do not commit generated artifacts. In particular, keep build outputs,
  disk images, and QEMU logs out of version control.
- Update public documentation when a change alters public behavior, commands,
  requirements, or stated limitations.

## Verification

Run from `rust/` with the nightly pinned in `kernel/rust-toolchain.toml`:

```powershell
.\test-gpt-qemu.ps1
```

Include the commands you ran and test output in the pull request description.
