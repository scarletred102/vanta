# ABI Freeze Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Vanta ABI v0 an explicit, layout-tested contract for future native runtimes, services, and compatibility personalities.

**Architecture:** Keep `rust/abi/src/lib.rs` as the single source of truth. Add a fixed-width feature bitset and a directory-record wire type beside the existing syscall, errno, capability, rights, credential, and signal types. Encode every promise in host tests, then document only the behavior covered by those tests.

**Tech Stack:** Rust 2021, `no_std` ABI crate, Cargo package tests, fixed-width `repr(C)` and `repr(transparent)` types.

## Global Constraints

- `ABI_VERSION` remains `0`; this bundle does not introduce ABI v1.
- Existing syscall numbers remain unchanged and Linux syscall numbers are not added to native dispatch.
- `rust/abi/src/lib.rs` is the only ABI source of truth; do not add a parallel schema or generator.
- Wire-facing structs use fixed-width integer fields and explicit `repr(C)` or `repr(transparent)` layouts.
- Unknown mandatory feature bits are reported as unsupported; no silent fallback is allowed.
- The cloned `starling/` directory is unrelated and must not be staged or modified.
- Focused package tests are required; the known Windows full-workspace RedoxFS dependency limitation remains documented.

---

### Task 1: Add Feature Discovery Contract

**Files:**
- Modify: `rust/abi/src/lib.rs`
- Modify: `rust/abi/tests/abi_contract.rs`

**Interfaces:**
- Produces `FeatureSet(u64)` with `const EMPTY`, `const fn from_bits(u64)`, `const fn bits() -> u64`, `const fn contains(FeatureSet) -> bool`, and `const fn unknown_mandatory_bits(FeatureSet) -> FeatureSet`.
- Produces stable feature constants `FEATURE_NATIVE_TERMINAL`, `FEATURE_REDOXFS_ROOT`, `FEATURE_PIPE_WAKEUP`, and `FEATURE_C_ABI_BOOTSTRAP`.
- Produces `SUPPORTED_FEATURES: FeatureSet` containing only features already verified in the current native scope.

- [ ] **Step 1: Write the failing feature-vector tests**

Add tests that assert the exact bit values, the supported aggregate, containment, and mandatory-bit rejection:

```rust
#[test]
fn feature_vectors_and_mandatory_bits_are_stable() {
    assert_eq!(FEATURE_NATIVE_TERMINAL.bits(), 1 << 0);
    assert_eq!(FEATURE_REDOXFS_ROOT.bits(), 1 << 1);
    assert!(SUPPORTED_FEATURES.contains(FEATURE_NATIVE_TERMINAL));
    assert_eq!(FeatureSet::EMPTY.unknown_mandatory_bits(FEATURE_NATIVE_TERMINAL), FEATURE_NATIVE_TERMINAL);
    assert_eq!(SUPPORTED_FEATURES.unknown_mandatory_bits(SUPPORTED_FEATURES), FeatureSet::EMPTY);
}
```

- [ ] **Step 2: Run the focused ABI test to verify it fails**

Run: `cargo test -p vanta-abi --test abi_contract feature_vectors_and_mandatory_bits_are_stable`

Expected: FAIL because `FeatureSet`, feature constants, and `SUPPORTED_FEATURES` do not exist.

- [ ] **Step 3: Implement the minimal feature contract**

Add the transparent `FeatureSet` wrapper and bit constants to `rust/abi/src/lib.rs`. Implement `unknown_mandatory_bits` as `FeatureSet(required.bits() & !self.bits())`, and define `SUPPORTED_FEATURES` from the four features documented as implemented in `rust/README.md`.

- [ ] **Step 4: Run the focused feature tests**

Run: `cargo test -p vanta-abi --test abi_contract feature_vectors_and_mandatory_bits_are_stable`

Expected: PASS.

- [ ] **Step 5: Commit the feature contract**

Run: `git add rust/abi/src/lib.rs rust/abi/tests/abi_contract.rs; git commit -m "feat: add ABI feature discovery contract"`

### Task 2: Add Wire Layout Types

**Files:**
- Modify: `rust/abi/src/lib.rs`
- Modify: `rust/abi/tests/abi_contract.rs`

**Interfaces:**
- Produces `MAX_DIRECTORY_NAME: usize = 256`.
- Produces `#[repr(C)] struct DirectoryRecord { inode: u64, file_type: u8, name_len: u8, record_len: u16, name: [u8; MAX_DIRECTORY_NAME] }`.
- Existing `SignalAction` and `Credentials` layouts become explicit test contracts without changing their fields.

- [ ] **Step 1: Write failing layout and round-trip tests**

Add tests for `repr(C)` sizes and alignments, plus a zeroed directory record with a bounded name:

```rust
#[test]
fn wire_layouts_are_stable() {
    assert_eq!(size_of::<SignalAction>(), 16);
    assert_eq!(align_of::<SignalAction>(), 8);
    assert_eq!(size_of::<Credentials>(), 44);
    assert_eq!(align_of::<Credentials>(), 4);
    assert_eq!(size_of::<DirectoryRecord>(), 272);
    assert_eq!(align_of::<DirectoryRecord>(), 8);
}

#[test]
fn directory_record_preserves_bounded_name_data() {
    let mut record = DirectoryRecord::empty(42, 8);
    record.set_name(b"hello");
    assert_eq!(record.inode, 42);
    assert_eq!(record.name_len, 5);
    assert_eq!(&record.name[..5], b"hello");
}
```

- [ ] **Step 2: Run the focused layout test to verify it fails**

Run: `cargo test -p vanta-abi --test abi_contract wire_layouts_are_stable`

Expected: FAIL because `DirectoryRecord` does not exist and the layout assertions are not present.

- [ ] **Step 3: Implement the minimal wire type**

Add `DirectoryRecord::empty(inode, file_type)` and `set_name(&mut self, name: &[u8])`. `set_name` must copy at most `MAX_DIRECTORY_NAME` bytes and set `name_len` to the copied length; it must not write past the fixed array. Set `record_len` to `size_of::<DirectoryRecord>() as u16` in `empty`.

- [ ] **Step 4: Run the focused layout tests**

Run: `cargo test -p vanta-abi --test abi_contract`

Expected: PASS, including both layout assertions and the directory-name test.

- [ ] **Step 5: Commit the wire layout contract**

Run: `git add rust/abi/src/lib.rs rust/abi/tests/abi_contract.rs; git commit -m "feat: define ABI wire layouts"`

### Task 3: Harden Golden Encoding Vectors

**Files:**
- Modify: `rust/abi/src/lib.rs`
- Modify: `rust/abi/tests/abi_contract.rs`

**Interfaces:**
- Existing `Syscall`, `Errno`, `Rights`, and `CapabilityId` public values remain source-compatible.
- `Errno::from_return_value` rejects `0` and positive values and only accepts the representable negative errno range.
- Capability round trips cover zero, ordinary, and maximum `u32` slot/generation values.

- [ ] **Step 1: Add failing edge-vector tests**

Add tests for every syscall number currently assigned, errno zero/positive rejection, and capability boundary values:

```rust
#[test]
fn errno_rejects_non_error_returns() {
    assert_eq!(Errno::from_return_value(0), None);
    assert_eq!(Errno::from_return_value(1), None);
}

#[test]
fn capability_boundaries_round_trip() {
    for (slot, generation) in [(0, 0), (42, 7), (u32::MAX, u32::MAX)] {
        let id = CapabilityId::from_parts(slot, generation);
        assert_eq!((id.slot(), id.generation()), (slot, generation));
    }
}
```

- [ ] **Step 2: Run the edge-vector tests to verify the errno test fails**

Run: `cargo test -p vanta-abi --test abi_contract errno_rejects_non_error_returns`; then run `cargo test -p vanta-abi --test abi_contract capability_boundaries_round_trip`

Expected: FAIL because `Errno::from_return_value(0)` currently returns `Some(Errno(0))`.

- [ ] **Step 3: Implement the minimal safety fix and complete syscall vectors**

Change `Errno::from_return_value` to return `None` for non-negative values. Add one table-driven assertion for each current `Syscall` discriminant and assert that the reserved gaps remain unused by checking the known sequence explicitly.

- [ ] **Step 4: Run the full ABI contract test**

Run: `cargo test -p vanta-abi --test abi_contract`

Expected: PASS with all existing and new vectors.

- [ ] **Step 5: Commit the hardened vectors**

Run: `git add rust/abi/src/lib.rs rust/abi/tests/abi_contract.rs; git commit -m "test: freeze ABI encoding vectors"`

### Task 4: Document the Frozen Contract

**Files:**
- Create: `rust/abi/README.md`
- Modify: `rust/README.md`
- Modify: `.vanta-local/plans/2026-07-21-maximizing.md`
- Modify: `.vanta-local/plans/2026-07-30-universal-compatibility-roadmap.md`

**Interfaces:**
- Documentation names the tested ABI v0 values and feature-discovery behavior without claiming ABI v1 or Linux compatibility.
- The active roadmaps mark the ABI v0 freeze package complete and retain ABI v1 negotiation, service extraction, and the Linux personality as pending work.

- [ ] **Step 1: Write the ABI crate README**

Document the version, syscall ownership rule, negative errno convention, capability encoding, rights, credentials, feature discovery, directory record, and exact focused test command.

- [ ] **Step 2: Update the Rust README status**

Add the ABI freeze package to the verified “What works” section and clarify that ABI v1 negotiation and generated C bindings are not part of this bundle.

- [ ] **Step 3: Update both active roadmaps**

Add a dated implementation-status bullet for the tested ABI v0 contract. Do not mark Track B or Linux compatibility complete.

- [ ] **Step 4: Check documentation consistency**

Run: `rg -n "ABI_VERSION|FeatureSet|DirectoryRecord|ABI v1|linuxd|Track B" rust/abi/README.md rust/README.md .vanta-local/plans/2026-07-21-maximizing.md .vanta-local/plans/2026-07-30-universal-compatibility-roadmap.md`

Expected: every new public contract is described consistently, and no document claims a working Linux personality.

- [ ] **Step 5: Commit documentation**

Run: `git add rust/abi/README.md rust/README.md .vanta-local/plans/2026-07-21-maximizing.md .vanta-local/plans/2026-07-30-universal-compatibility-roadmap.md; git commit -m "docs: record frozen Vanta ABI v0"`

### Task 5: Run Bundle Verification

**Files:**
- Modify: none unless verification exposes a contract mismatch.

- [ ] **Step 1: Run the focused ABI package tests**

Run: `cargo test -p vanta-abi`

Expected: PASS.

- [ ] **Step 2: Run dependent contract tests**

Run: `cargo test -p vanta-gpt -p vanta-redoxfs-adapter -p vanta-linuxd -p vanta-services`

Expected: PASS, proving the ABI changes did not break dependent crates.

- [ ] **Step 3: Check the diff and repository scope**

Run: `git diff --check; git status --short --branch`

Expected: no whitespace errors; only intended Vanta files are changed; `starling/` remains untracked and unstaged.

- [ ] **Step 4: Record the known full-workspace limitation**

If `cargo test --workspace` is attempted and fails for the documented Windows `libfuse`/`dlltool` dependency, report that exact limitation rather than weakening the ABI verification claim.

- [ ] **Step 5: Commit the verification checkpoint**

Run: `git status --short --branch; git log -5 --oneline`

Expected: the ABI implementation and documentation commits are present, with no unrelated staged files.
