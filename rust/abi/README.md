# Vanta ABI v0

This crate is the single source of truth for the native Vanta ABI contract.
It is `no_std` and uses fixed-width, explicitly laid-out values so the kernel,
native programs, services, and later compatibility personalities can share
stable representations.

## Frozen Contract

- `ABI_VERSION` is `0`.
- Native syscall numbers are Vanta-owned. Linux syscall numbers do not enter
  the native dispatch table.
- Syscall errors are returned as negative `isize` values containing a positive
  `i32` errno. Non-negative values and values that cannot represent an `i32`
  errno are not decoded as errors.
- `CapabilityId` stores a `u32` slot in the low half and a `u32` generation in
  the high half. `CapabilityId::INVALID` is zero.
- `Rights` is a `u32` bitset. Rights compose explicitly; possessing one right
  does not grant unrelated authority.
- `Credentials` uses fixed `uid`, `gid`, supplementary groups, group count,
  and umask fields. The shipped identities are root (`0:0`) and vanta
  (`1000:1000`).
- `SignalAction` and `DirectoryRecord` use `repr(C)` layouts covered by the
  ABI tests. `DirectoryRecord` reserves 256 bytes for a bounded name.

## Feature Discovery

`FeatureSet` is a transparent `u64` bitset. Current stable feature bits are:

| Feature | Bit |
|---|---:|
| `FEATURE_NATIVE_TERMINAL` | 0 |
| `FEATURE_REDOXFS_ROOT` | 1 |
| `FEATURE_PIPE_WAKEUP` | 2 |
| `FEATURE_C_ABI_BOOTSTRAP` | 3 |

`FeatureSet::unknown_mandatory_bits` returns the required bits that are not
supported. Callers must reject unknown mandatory bits rather than silently
falling back. The current feature set is a host/kernel contract constant; a
native feature-query syscall is separate follow-up work.

## Verification

Run the focused contract suite from `rust/`:

```powershell
cargo test -p vanta-abi
```

The suite freezes syscall vectors, feature bits, errno behavior, capability
boundaries, rights, credentials, signal layout, and directory-record layout.
