# VantaOS Capability Specification (Phase 3)

This document is the absolute source of truth for the VantaOS capability-based access control system. Any code or implementation that contradicts this document is a bug.

---

## 1. Handle Encoding (64-bit)

A capability handle is a 64-bit process-local token used by userspace to reference kernel objects. It has the following layout:

* **Top 16 bits (bits 48 to 63)**: **Generation Counter**. Used to detect stale handles and prevent handle reuse vulnerabilities after revocation or release.
* **Bottom 48 bits (bits 0 to 47)**: **Slot Index**. An index into the calling process's flat capability table (`CapTable`).

```text
 63             48 47                                       0
+-----------------+------------------------------------------+
|  Generation     |                 Slot Index               |
|    (16 bits)    |                 (48 bits)                |
+-----------------+------------------------------------------+
```

* **Null Handle**: The handle `0` is always the invalid/NULL handle.

---

## 2. Capability Types

VantaOS defines the following capability types:

1. **`Null` (0)**: Empty slot.
2. **`Memory` (1)**: Grants access to physical or virtual memory regions.
3. **`Endpoint` (2)**: Represents an IPC communication endpoint (a `Port` object).
4. **`Thread` (3)**: Grants management and execution control over a single thread.
5. **`Notification` (4)**: Grants access to a lightweight signaling primitive.
6. **`DeviceIRQ` (5)**: Grants the ability to bind and manage a specific hardware interrupt vector.
7. **`PageTable` (6)**: Grants the ability to map and unmap pages in a virtual address space.

---

## 3. Rights Bitmask per Type

Derived capabilities can only restrict rights, never expand them. The following rights bitmasks are defined per capability type:

### `Memory` (Type 1)
* **`Read` (1 << 0)**: Permission to read memory contents.
* **`Write` (1 << 1)**: Permission to write memory contents.
* **`Execute` (1 << 2)**: Permission to execute instructions from this memory.
* **`Map` (1 << 3)**: Permission to map the memory region into a virtual address space.

### `Endpoint` (Type 2)
* **`Send` (1 << 0)**: Permission to send messages (non-blocking or blocking).
* **`Recv` (1 << 1)**: Permission to receive messages.
* **`Grant` (1 << 2)**: Permission to transfer capabilities through this endpoint.

### `Thread` (Type 3)
* **`Control` (1 << 0)**: Suspend, resume, or terminate execution.
* **`Inspect` (1 << 1)**: Read or write CPU registers.

### `Notification` (Type 4)
* **`Signal` (1 << 0)**: Signal/wake waiting threads.
* **`Wait` (1 << 1)**: Block until signaled.

### `DeviceIRQ` (Type 5)
* **`Bind` (1 << 0)**: Bind an interrupt line to an IPC notification or port.

### `PageTable` (Type 6)
* **`Map` (1 << 0)**: Map a physical page into the address space.
* **`Unmap` (1 << 1)**: Unmap virtual addresses.

---

## 4. Capability Derivation (`cap_derive` Contract)

Capabilities are duplicated via the `cap_derive` system call. The following invariants apply:
1. **Bitwise Subset**: The derived capability's rights (`new_rights`) must be a strict bitwise subset of the parent capability's rights (`parent_rights`):
   $$\text{new\_rights} \land \text{parent\_rights} = \text{new\_rights}$$
   If this condition is violated, the system call must fail with `-EPERM`.
2. **Object Sharing**: The derived child capability points to the exact same kernel object as the parent.
3. **Hierarchy Tracking**: The kernel tracks the parent-child relationship by recording the parent slot's stable index and generation inside the child capability entry.

---

## 5. Transitive Invalidation (`cap_revoke` Invariant)

The `cap_revoke` system call destroys a capability and recursively invalidates all capabilities derived from it (transitive invalidation).

1. **Slot Generation Bump**: The slot's generation counter is incremented when a capability is revoked/freed. This immediately invalidates any stale user handles referring to that slot.
2. **Transitive Walk**: The kernel walks the singly-linked list of capabilities registered on the specific kernel object.
3. **Ancestry Match**: For each capability on the object, the kernel walks its parent chain. If the revoked capability is found in the ancestry chain, that capability is recursively invalidated:
   * Its type is reset to `Null`.
   * Its object pointer is zeroed.
   * Its generation counter is incremented.
4. **Complexity Invariant**: The invalidation walk must run in $O(\text{derived\_count})$ time — it must scale only with the number of capabilities derived for that specific kernel object, and never with the total number of capabilities in the system ($O(\text{total\_system\_caps})$).

---

## 6. IPC Capability Transfer (Move Semantics)

To prevent Ambient Authority and maintain strict compartmentalization, capability transfer via IPC follows a strict **Move** semantic:

1. **Sender Deprivation (Move)**: When a capability handle is sent inside a message payload (up to 4 capabilities per message):
   * The kernel extracts the `CapEntry` from the sender's slot.
   * The sender's slot is completely cleared (`type = Null`), and its slot generation counter is incremented. The sender permanently loses the capability.
2. **In-Transit State**: During delivery, the raw `CapEntry` fields are stored in a kernel-safe message buffer (in-transit).
3. **Parent Validity check on Receipt**: When the receiver process receives the message:
   * The kernel checks if the transferred capability's parent is still valid.
   * If the parent has been revoked while the capability was in transit, the capability is discarded immediately (`type = Null`) and not inserted.
4. **Receiver Allocation**: If valid, the capability is inserted into a free slot in the receiver's `CapTable`, yielding a new process-local handle for the receiver.
5. **Relationship Maintenance**: Any children of the transferred capability that were left behind are updated to point to the capability's new slot location in the receiver's table.
6. **No Ambiguity**: Capability duplication without explicit `cap_derive` is strictly disallowed. The move semantic is an unbreachable invariant.
