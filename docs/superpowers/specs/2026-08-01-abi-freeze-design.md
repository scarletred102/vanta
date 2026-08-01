# Vanta ABI Freeze Package Design

## Goal

Make the existing Vanta ABI v0 explicit and testable enough that later native
runtime, service, and compatibility work can depend on stable contracts rather
than inferred Rust implementation details.

## Scope

This bundle freezes the current ABI surface without changing its syscall
numbers or changing kernel behavior:

- ABI version and feature discovery constants;
- syscall number golden vectors and reserved ranges;
- negative errno return encoding, including invalid-value rejection;
- capability handle slot/generation encoding and invalid-handle semantics;
- rights bit assignments and composition behavior;
- credential and directory-record wire layouts;
- signal action wire layout;
- host-side contract tests for values, sizes, alignments, and round trips;
- a short ABI contract document referenced by the Rust README.

It does not implement ABI v1, change syscall dispatch, add new syscalls, add a
C header generator, or implement feature negotiation in the kernel. Those are
follow-up bundles after the frozen v0 contract has evidence behind it.

## Approach

`rust/abi/src/lib.rs` remains the single source of truth. Public wire-facing
types use `#[repr(C)]` or `#[repr(transparent)]`, fixed-width integer fields,
and explicit constants. The test suite records the exact values and layout
properties that consumers may rely on. Feature discovery is represented as a
fixed bitset with mandatory-feature rejection helpers so future callers can
distinguish supported optional features from unknown mandatory requirements.

Golden-vector tests are preferable to a second schema file at this stage:
they keep the contract close to the types, avoid generator drift, and are easy
to run on the Windows host and in the kernel build.

## Contract Details

### Version and features

- `ABI_VERSION` remains `0`.
- A `FeatureSet` is a fixed-width `u64` bitset.
- Features have stable bit positions and no implicit fallback behavior.
- A helper reports unknown mandatory bits as unsupported.

### Wire layouts

The following types receive explicit layout assertions:

- `SignalAction`;
- `Credentials`;
- a directory record type containing inode, file type, record length, and a
  bounded name representation.

The directory record is a contract type only in this bundle; kernel directory
serialization changes are out of scope unless an existing implementation
already consumes the same shape.

### Error and handle safety

- `Errno::from_return_value` rejects zero and positive values.
- Negative encoding uses the documented signed return convention.
- Capability slot and generation round trips are tested at zero, ordinary, and
  maximum `u32` values.
- Reserved or invalid capability values remain distinguishable.

## Verification

The focused ABI test suite must verify exact syscall vectors, feature behavior,
errno behavior, handle behavior, rights behavior, credentials, and all public
wire layouts. The existing focused package tests must remain passing. Full
workspace tests and QEMU regressions remain separate gates because the current
Windows host cannot run the vendored RedoxFS host dependency unchanged.

## Documentation

After tests pass, update:

- `rust/abi/README.md` with the v0 contract and compatibility rules;
- `rust/README.md` to identify the ABI freeze evidence and remaining ABI work;
- both active Vanta roadmap files to mark only this bundle complete and leave
  ABI v1 negotiation and service extraction as pending work.

## Definition of Done

The bundle is complete when the ABI crate contains the explicit v0 contract,
focused tests pass, no syscall number changes occur, the documentation matches
the tested surface, and the implementation commit records the verification
commands and known Windows workspace-test limitation.
