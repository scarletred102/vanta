# VantaOS Design Document

> *"Absorbs everything good. Reflects nothing bad."*

**Version:** 0.1.0-draft
**Status:** Active Development
**Language:** Zig (kernel + core userspace)
**Architecture:** Capability-based microkernel, clean break from POSIX

---

## Table of Contents

1. [Vision & Philosophy](#1-vision--philosophy)
2. [Architecture Overview](#2-architecture-overview)
3. [Kernel Design](#3-kernel-design)
4. [Capability System](#4-capability-system)
5. [IPC — Inter-Process Communication](#5-ipc--inter-process-communication)
6. [Memory Management](#6-memory-management)
7. [Process & Thread Model](#7-process--thread-model)
8. [Scheduler](#8-scheduler)
9. [Resource Model (Filesystem Replacement)](#9-resource-model)
10. [Display & Compositor](#10-display--compositor)
11. [Audio System](#11-audio-system)
12. [Networking](#12-networking)
13. [Device Driver Model](#13-device-driver-model)
14. [Package Management](#14-package-management)
15. [Security Architecture](#15-security-architecture)
16. [Portability Strategy](#16-portability-strategy)
17. [Feature Matrix: Best of Every OS](#17-feature-matrix)
18. [User Experience](#18-user-experience)
19. [Developer Experience](#19-developer-experience)
20. [Roadmap](#20-roadmap)
21. [Appendices](#appendices)

---

## 1. Vision & Philosophy

### 1.1 Why VantaOS Exists

Every mainstream OS carries decades of design debt. POSIX was designed for 1970s timesharing. Windows carries Win32 from 1993. macOS grafts modern UX onto Mach/BSD from the 80s. Linux has 400+ syscalls, half redundant.

VantaOS starts from zero. No POSIX. No Win32. No legacy.

### 1.2 Core Principles

| Principle | Meaning |
|---|---|
| **Capability-first** | Every resource is a handle. No ambient authority. No "root". |
| **Least privilege** | Programs get exactly what they need, nothing more. |
| **Typed everything** | IPC is structured. Resources have schemas. Errors have types. |
| **Crash isolation** | Microkernel: any driver, server, or app crash is contained. |
| **Stable ABI** | Apps built for VantaOS 1.0 run on VantaOS 10.0. Forever. |
| **Open ecosystem** | Sideloading encouraged. No app store gatekeeping. No signing requirement. |
| **Performance by design** | Zero-copy IPC. GPU-direct I/O. Energy-aware scheduling. |
| **Portability pragmatism** | Native apps optimal. Ported apps supported via compatibility shims. |

### 1.3 Anti-Goals

- NOT a Linux distribution
- NOT POSIX compatible (compatibility layers exist for ported apps, not for native code)
- NOT a research toy — this ships
- NOT closed — everything is open source, permissive license
- NOT cloud-dependent — all features work fully offline

---

## 2. Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                     USER APPLICATIONS                     │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │ Native   │ │ WASM     │ │ Compat   │ │ Games       │ │
│  │ Apps     │ │ Apps     │ │ Pods     │ │ (Vulkan)    │ │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └──────┬──────┘ │
├───────┼─────────────┼───────────┼───────────────┼────────┤
│       │     SYSTEM SERVERS (Userspace)          │        │
│  ┌────┴─────┐ ┌─────┴────┐ ┌────┴─────┐ ┌─────┴──────┐ │
│  │ Display  │ │ Resource │ │ Network  │ │ Audio      │ │
│  │ Server   │ │ Server   │ │ Server   │ │ Server     │ │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └─────┬──────┘ │
│       │             │            │              │        │
│  ┌────┴─────┐ ┌─────┴────┐ ┌────┴─────┐ ┌─────┴──────┐ │
│  │ GPU      │ │ Storage  │ │ NIC      │ │ Audio HW   │ │
│  │ Driver   │ │ Driver   │ │ Driver   │ │ Driver     │ │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └─────┬──────┘ │
├═══════╪═════════════╪════════════╪═════════════╪════════╡
│       │         VANTA MICROKERNEL              │        │
│  ┌────┴──────────────────────────────────┬─────┘        │
│  │  Capabilities │ IPC │ Memory │ Sched  │              │
│  └───────────────────────────────────────┘              │
└──────────────────────────────────────────────────────────┘
                        HARDWARE
```

### 2.1 What Lives in the Kernel

The kernel is **tiny**. It handles exactly five things:

1. **Capability management** — creating, deriving, revoking, checking handles
2. **IPC** — synchronous and async message passing between processes
3. **Memory management** — physical pages, virtual address spaces, shared mappings
4. **Scheduling** — thread scheduling with priority and deadline support
5. **Interrupt routing** — forwarding hardware interrupts to userspace drivers

**Everything else** is a userspace server: filesystem, networking, display, audio, USB, GPU.

### 2.2 Why Microkernel

| Concern | Monolithic (Linux) | VantaOS Microkernel |
|---|---|---|
| Driver crash | Kernel panic, reboot | Server restarts, system continues |
| Attack surface | Millions of LOC in ring 0 | ~10K LOC in ring 0 |
| Hot update | Requires reboot (usually) | Restart individual servers |
| Modularity | Compile-time config | Runtime server swapping |
| IPC overhead | N/A (in-kernel calls) | Mitigated: zero-copy, shared memory |

### 2.3 Addressing the IPC Tax

Microkernels are historically slower due to IPC overhead. VantaOS mitigates this:

1. **Zero-copy transfers** — Sender maps pages into receiver's address space. No memcpy.
2. **Shared memory regions** — Bulk data (framebuffers, audio buffers) uses shared mappings.
3. **Batched syscalls** — Multiple operations in a single kernel transition.
4. **Direct capability grants** — Hardware access can be delegated directly to userspace (MMIO regions, I/O ports).
5. **Fast-path IPC** — Small messages (<64 bytes) pass in registers, never touch memory.

---

## 3. Kernel Design

### 3.1 Kernel Objects

Everything in the kernel is an **object**. Objects are never accessed directly — only through capabilities.

| Object Type | Description |
|---|---|
| `AddressSpace` | Virtual address space for a process |
| `Thread` | Schedulable execution context |
| `Port` | IPC endpoint (send or receive) |
| `Channel` | Bidirectional IPC pair (two ports) |
| `MemoryObject` | Physical memory region |
| `Interrupt` | Hardware interrupt source |
| `IoRegion` | I/O port or MMIO range |
| `Notification` | Lightweight signaling (like futex) |
| `Timer` | Kernel timer for deadlines |

### 3.2 Syscall Interface

VantaOS has **23 syscalls**. That's it. Compare: Linux has 450+.

```
CAPABILITY OPERATIONS (core of everything)
  cap_send(port, msg)           → Send typed message
  cap_recv(port)                → Receive typed message
  cap_call(port, msg)           → Send + wait for reply (RPC)
  cap_derive(cap, rights_mask)  → Create restricted child capability
  cap_revoke(cap)               → Destroy cap and all children
  cap_inspect(cap)              → Query cap type and rights

MEMORY OPERATIONS
  mem_create(size, flags)       → Create memory object
  mem_map(space, mem, addr, prot) → Map memory object into address space
  mem_unmap(space, addr, size)  → Unmap region
  mem_share(mem, rights)        → Create shareable memory handle

PROCESS & THREAD OPERATIONS
  proc_create(space, caps[])    → Create process with initial capabilities
  thread_create(proc, entry, stack, arg) → Create thread in process
  thread_exit(code)             → Terminate current thread
  thread_yield()                → Voluntary preemption
  thread_sleep(ns)              → Sleep for duration

INTERRUPT & I/O (for userspace drivers)
  irq_create(irq_num)          → Bind to hardware interrupt → cap
  irq_wait(irq_cap)            → Wait for interrupt
  irq_ack(irq_cap)             → Acknowledge interrupt
  io_map(phys, size)           → Map MMIO region → cap

SYSTEM
  sys_info(what)                → Query system information
  sys_log(msg)                  → Kernel debug log
  sys_time()                    → High-resolution monotonic time
  sys_shutdown(action)          → Reboot/poweroff
```

### 3.3 Syscall ABI

- **Calling convention:** `syscall` instruction on x86_64
- **Registers:**
  - `rax` = syscall number
  - `rdi, rsi, rdx, r10, r8, r9` = arguments (up to 6)
  - `rax` = return value
  - `rdx` = error code (0 = success)
- **Guaranteed stable** across versions. New syscalls may be added; existing ones never change semantics.

---

## 4. Capability System

### 4.1 What is a Capability

A **capability** is an unforgeable token that grants specific rights to a specific object. Think of it as a key that only opens one door, and you can make copies with fewer teeth (fewer rights) but never more.

```
Capability = (Object Reference, Rights Bitmask, Generation)
```

- **Object Reference**: Kernel pointer to the object
- **Rights**: What operations are permitted
- **Generation**: Prevents use-after-revoke (stale handles fail safely)

### 4.2 Rights

```
READ        (1 << 0)   — Read data/state from object
WRITE       (1 << 1)   — Modify data/state
EXECUTE     (1 << 2)   — Execute (for memory: run code; for ports: invoke)
GRANT       (1 << 3)   — Transfer this capability to another process
DERIVE      (1 << 4)   — Create child capabilities with subset of rights
REVOKE      (1 << 5)   — Destroy this cap and all derived children
MAP         (1 << 6)   — Map into address space (for memory objects)
CONNECT     (1 << 7)   — Create connections (for ports)
MANAGE      (1 << 8)   — Administrative operations (resize, configure)
INSPECT     (1 << 9)   — Query metadata without accessing content
```

### 4.3 Capability Derivation

```
Parent cap:   (MemoryObject, READ|WRITE|DERIVE)
                  │
Child cap:    (MemoryObject, READ)          ← can only read
                  │
Grandchild:   (MemoryObject, READ)          ← still only read, can't add WRITE back
```

**Monotonic restriction**: derived capabilities can only have equal or fewer rights. You can never escalate.

### 4.4 Revocation

Revoking a capability invalidates it AND all its descendants. The kernel maintains a **derivation tree** for each object.

```
cap_revoke(parent_cap) →
    invalidate(parent_cap)
    for each child in parent_cap.children:
        cap_revoke(child)   // recursive
```

This is O(n) in the number of descendants. For pathological cases, the kernel can defer revocation using **lazy revocation** (generation numbers).

### 4.5 Per-Process Capability Table

Each process has a **capability table** — a flat array mapping handle numbers (integers) to kernel capability objects.

```
Process A's Capability Table:
  Handle 0: (IPC Port to Display Server, SEND|RECV)
  Handle 1: (MemoryObject 0x1000-0x5000, READ|WRITE|MAP)
  Handle 2: (Thread self, READ|MANAGE)
  Handle 3: (IPC Port to Resource Server, SEND)
  Handle 4: NULL (revoked or unused)
  ...
```

Handle 0 is always NULL. Handles are process-local. Transferring a capability between processes creates a new handle in the receiver's table.

---

## 5. IPC — Inter-Process Communication

### 5.1 Message Structure

Messages are **typed** and **fixed-format**. No raw byte streams for IPC (that's what shared memory is for).

```
Message {
    msg_type:  u32      — Operation code (defined per protocol)
    flags:     u32      — Delivery flags
    payload:   [64]u8   — Inline data (small messages, register-passed)
    caps:      [4]Handle — Capability transfers (up to 4 per message)
    buffer:    ?MemCap  — Optional shared memory for bulk data
}
```

### 5.2 IPC Patterns

**Synchronous call (RPC):**
```
client                          server
  │                               │
  ├── cap_call(port, request) ──→ │
  │   (client blocks)             ├── process request
  │                               │
  │ ←── reply ────────────────────┤
  │   (client resumes)            │
```

**Asynchronous send:**
```
sender                          receiver
  │                               │
  ├── cap_send(port, msg) ──→     │  (queued)
  │   (sender continues)         │
  │                               ├── cap_recv(port) → msg
```

**Notification (lightweight signal):**
```
notifier                        waiter
  │                               │
  ├── notify(cap, bits) ───→      │  (bitmap OR'd)
  │                               ├── wait(cap) → bits
```

### 5.3 Protocols

IPC messages follow **protocols** — typed interfaces defined in a schema language (VantaIDL, similar to Fuchsia's FIDL or Cap'n Proto).

```
protocol Display {
    create_surface(width: u32, height: u32) → (surface: Cap<Surface>, buffer: Cap<Memory>);
    present(surface: Cap<Surface>) → ();
    resize(surface: Cap<Surface>, width: u32, height: u32) → (buffer: Cap<Memory>);
    destroy_surface(surface: Cap<Surface>) → ();
}
```

The IDL compiler generates:
- Client stubs (send typed messages)
- Server dispatch (receive and decode)
- Wire format documentation

---

## 6. Memory Management

### 6.1 Physical Memory Manager (PMM)

**Phase 0**: Bitmap allocator — simple, correct, easy to debug.
**Phase 1**: Buddy allocator — O(log n) alloc/free, handles fragmentation.
**Phase 2**: Slab allocator on top of buddy — efficient for kernel objects.

### 6.2 Virtual Memory Manager (VMM)

Each process has its own `AddressSpace` object containing a page table hierarchy.

**Address space layout (x86_64):**
```
0x0000_0000_0000_0000 ─┬─ User space (lower half)
                        │  Applications, libraries, stacks
0x0000_7FFF_FFFF_FFFF ─┘
                        ... (non-canonical hole)
0xFFFF_8000_0000_0000 ─┬─ Higher Half Direct Map (HHDM)
                        │  All physical memory mapped here
0xFFFF_FFFF_8000_0000 ─┬─ Kernel image
                        │  Code, data, BSS
0xFFFF_FFFF_FFFF_FFFF ─┘
```

### 6.3 Memory Objects

Applications don't call `mmap` on file descriptors (there are no file descriptors). Instead:

1. Request a `MemoryObject` capability (from kernel or a server)
2. Map it into your address space with `mem_map`
3. Optionally share it by deriving a handle and sending it via IPC

```
// Allocate 4 pages of anonymous memory
mem_cap = mem_create(4 * PAGE_SIZE, MEM_ANON | MEM_ZEROED);

// Map it read-write at a hint address
mem_map(my_address_space, mem_cap, 0x1_0000_0000, PROT_READ | PROT_WRITE);

// Share read-only with another process
shared = cap_derive(mem_cap, RIGHT_READ | RIGHT_MAP);
cap_send(other_process_port, msg_with_cap(shared));
```

### 6.4 Copy-on-Write

Memory objects support CoW for efficient cloning:
- `proc_create` with CoW flag shares parent's memory until written
- Great for spawn() (our replacement for fork+exec)

### 6.5 Demand Paging

Pages are allocated lazily. A mapped memory object doesn't consume physical pages until accessed. Page faults are handled by:
1. Kernel (for anonymous memory — just allocate a zeroed page)
2. Userspace pager server (for file-backed memory — the resource server handles it)

---

## 7. Process & Thread Model

### 7.1 No fork()

`fork()` is a terrible API. It copies the entire process state, which is:
- Expensive (even with CoW)
- Semantically broken (what happens to threads? locks? file handles?)
- A security nightmare (child inherits everything)

VantaOS uses **`proc_create()`**:

```
proc_create(
    address_space,     // Pre-configured address space
    initial_caps[],    // EXPLICIT list of capabilities the new process gets
) → (process_cap, main_thread_cap)
```

The parent must **explicitly grant** every capability. No inheritance by default. No ambient authority leaking.

### 7.2 Thread Model

- Threads are kernel objects, scheduled preemptively
- Each thread belongs to exactly one process
- Threads within a process share the address space
- Thread-local storage via dedicated per-thread pages

### 7.3 Process Lifecycle

```
CREATED → RUNNING → EXITED
              │
              ├── SUSPENDED (by parent or debugger)
              │
              └── CRASHED (unhandled fault)
                     │
                     └── Crash report sent to parent via IPC
```

Crash isolation: a crashing process sends its crash report (registers, backtrace, faulting address) to its parent via IPC, then is cleaned up by the kernel. The parent decides whether to restart it, log it, or propagate the error.

---

## 8. Scheduler

### 8.1 Design: Hybrid Priority + Deadline

The scheduler supports two modes:

**Priority scheduling** (default):
- 64 priority levels (0 = idle, 63 = real-time)
- Round-robin within each priority level
- Anti-starvation: threads that haven't run get temporary priority boost

**Deadline scheduling** (opt-in, for real-time):
- EDF (Earliest Deadline First) for threads with deadlines
- Used by audio server, compositor, game mode
- Guaranteed CPU time if admitted

### 8.2 Game Mode

When activated:
1. Scheduler switches to **low-latency mode** (smaller time slices)
2. Game process gets priority 60+ (near real-time)
3. Background services get limited to 10% CPU
4. Audio server gets deadline scheduling (no glitches)
5. GPU compositor disabled — game gets exclusive GPU access
6. Memory: large pages for game, aggressive swapping for background

### 8.3 Energy-Aware Scheduling

On battery:
- Fewer CPU cores active
- Lower frequency
- Background tasks batched (execute together, then sleep)
- Compositor reduces refresh rate
- Network polling intervals increased

---

## 9. Resource Model

### 9.1 Not "Everything is a File"

POSIX: everything is a file (bytes stream through a file descriptor).
Plan 9: everything is a file (but with better semantics).
VantaOS: **everything is a typed resource accessed through capabilities**.

```
POSIX:      fd = open("/dev/gpu0", O_RDWR);
Plan 9:     fd = open("/dev/gpu0/render", OWRITE);
VantaOS:    gpu = request_cap(display_server, GPU_RENDER, my_needs);
```

### 9.2 Resource Servers

The **Resource Server** replaces the traditional filesystem. It's a userspace server that manages a hierarchical namespace of typed resources.

Resources have:
- **Type** — file, directory, device, stream, sensor, etc.
- **Schema** — structured metadata (not just name/size/time)
- **Capabilities** — access controlled per-handle
- **Queries** — find resources by metadata, not just path

```
// Find all images larger than 1MB, modified this week
results = resource_query(root_cap, Query{
    .type = "image/*",
    .min_size = 1_000_000,
    .modified_after = this_week_start,
    .sort = .modified_desc,
});
```

This is inspired by BeOS attributes + Spotlight + WinFS (the filesystem Microsoft tried and failed to build).

### 9.3 Namespace Structure

There is **no global filesystem namespace**. Each process has a **namespace** — a set of capability bindings:

```
Process A's namespace:
  "storage"  → Cap<ResourceServer> (home directory subtree)
  "display"  → Cap<DisplayServer>
  "network"  → Cap<NetworkServer>
  "audio"    → Cap<AudioServer>
  "system"   → Cap<SystemInfo> (read-only system queries)
```

A sandboxed app might only get:
```
  "storage"  → Cap<ResourceServer> (app-specific directory ONLY)
  "display"  → Cap<DisplayServer> (single surface)
```

No `/etc/passwd`. No `/dev/sda`. No `/proc/self`. Each process sees only what it was granted.

### 9.4 Copy-on-Write Filesystem

The storage backend supports:
- **Snapshots** — instant, space-efficient (like ZFS/Btrfs)
- **Branching** — fork the filesystem state, merge later
- **Checksums** — every block verified (silent corruption detected)
- **Compression** — transparent, per-file or per-directory
- **Deduplication** — block-level, async

---

## 10. Display & Compositor

### 10.1 Architecture

The display server is a userspace process that:
1. Owns the GPU capability (received from the GPU driver)
2. Provides surface capabilities to applications
3. Composites surfaces into the final output
4. Handles input routing (keyboard, mouse, touch)

### 10.2 Scene Graph Compositor

Not just a window stacker. The compositor maintains a **scene graph**:

```
Screen
├── Wallpaper (background surface)
├── Desktop
│   ├── Window: Browser (surface + shadow + corner radius)
│   ├── Window: Terminal (surface + transparency)
│   └── Window: File Manager (surface)
├── Dock (surface with blur backdrop)
├── Notifications (overlay surfaces with animations)
└── Cursor (hardware or software)
```

Rendering pipeline:
1. Each app renders into its surface (Vulkan, software, whatever)
2. Compositor builds scene graph
3. GPU composites all surfaces with effects (blur, shadow, animation)
4. Output to display

### 10.3 Capabilities

- `Surface` — Render target (app gets this)
- `Display` — Physical display (compositor gets this from GPU driver)
- `InputSink` — Receives input events (focused surface gets this)
- `Clipboard` — System clipboard (shared via capability)
- `Screenshot` — Capture screen (must be explicitly granted)

### 10.4 Key Features

- **120fps** native (adaptive sync support)
- **HDR & Wide Color Gamut** — P3/Rec.2020, HDR10/Dolby Vision
- **Variable refresh rate** — FreeSync/G-Sync via GPU driver
- **Per-surface color profiles** — each app can specify its color space
- **Tiling and floating** — both modes, user switchable (i3 + macOS hybrid)
- **Animations** — spring-based physics, 120fps, GPU-accelerated
- **Multi-monitor** — each monitor is a display capability, independently configurable

---

## 11. Audio System

### 11.1 Architecture: Audio Graph

Inspired by PipeWire, CoreAudio, and JACK. System-level audio routing as a directed graph:

```
                 ┌─────────────┐
                 │  App: Music │
                 └──────┬──────┘
                        │ (stereo PCM)
                        ▼
┌────────────┐   ┌─────────────┐   ┌────────────────┐
│ App: Voice │──→│  Mixer      │──→│ Output: Speaker│
│ Chat       │   │  (system)   │   └────────────────┘
└────────────┘   └──────┬──────┘
                        │
                        ▼
                 ┌─────────────────┐
                 │ Output: Headset │
                 └─────────────────┘
```

### 11.2 Key Features

- **Low latency** — <10ms round-trip for pro audio
- **Per-app volume** — each app is a node in the graph
- **Spatial audio** — 3D positional audio for games and VR
- **System-wide EQ** — DSP pipeline configurable
- **Bluetooth codec selection** — AAC, LDAC, aptX, etc.
- **Audio routing** — any output to any input (loopback, virtual cables)
- **Sample rate conversion** — transparent, high-quality

---

## 12. Networking

### 12.1 Architecture

Network stack is a userspace server. The kernel only provides:
- IRQ capability (for NIC driver)
- MMIO capability (for NIC registers)
- Shared memory (for DMA ring buffers)

### 12.2 Protocol Stack

```
┌──────────────────────────┐
│ Applications             │
├──────────────────────────┤
│ Socket API (cap-based)   │
├──────────────────────────┤
│ TLS (built-in, default)  │
├──────────────────────────┤
│ TCP / UDP / QUIC         │
├──────────────────────────┤
│ IP (v4 + v6)             │
├──────────────────────────┤
│ NIC Driver (userspace)   │
└──────────────────────────┘
```

### 12.3 Key Features

- **TLS by default** — unencrypted connections require explicit opt-in
- **QUIC native** — modern transport, not bolted on
- **Per-app firewall** — network capability can restrict destinations
- **DNS over HTTPS** — default resolver
- **mDNS/DNS-SD** — zero-config local network discovery
- **Mesh networking** — device-to-device capability sharing (Section 18.4)

---

## 13. Device Driver Model

### 13.1 Userspace Drivers

All drivers run in userspace. The kernel provides:
1. `irq_create(n)` — bind to hardware interrupt
2. `io_map(phys, size)` — map MMIO region
3. `mem_create` with DMA flag — allocate DMA-capable memory

The driver is a normal process with elevated capabilities. If it crashes, the kernel cleans up its resources and the driver manager can restart it.

### 13.2 Driver Discovery

1. Kernel provides ACPI/device tree info via `sys_info`
2. **Driver Manager** (userspace server) matches hardware IDs to drivers
3. Drivers are loaded from the package system
4. Hot-plug: Driver Manager watches for device events and loads/unloads drivers

### 13.3 GPU Driver Architecture

```
┌──────────────────────────┐
│ Application (Vulkan API) │
├──────────────────────────┤
│ Vulkan ICD (userspace)   │  ← Vendor-specific
├──────────────────────────┤
│ GPU Kernel Interface     │  ← Shared memory + capabilities
├──────────────────────────┤
│ GPU Driver (userspace)   │  ← Manages command submission
├──────────────────────────┤
│ Hardware                 │
└──────────────────────────┘
```

- Vulkan is the primary GPU API (no OpenGL — it's legacy)
- GPU driver manages command queue submission
- Applications get Vulkan surfaces via display server capability
- DirectStorage-style: assets load directly from storage to GPU memory

---

## 14. Package Management

### 14.1 Content-Addressed Store (Nix-inspired)

Packages are stored by the **hash of their contents**, not by name/version.

```
/vanta/store/sha256-a1b2c3d4.../bin/firefox
/vanta/store/sha256-e5f6a7b8.../bin/firefox   ← different version, different hash
```

Benefits:
- **Multiple versions coexist** — no dependency conflicts
- **Atomic updates** — switch a symlink, done
- **Instant rollback** — switch the symlink back
- **Reproducible** — same inputs always produce same hash
- **Deduplication** — identical files across packages share storage

### 14.2 Declarative System Configuration

System state is defined in a configuration file:

```
system {
    hostname = "vanta-dev"
    timezone = "America/New_York"
    
    packages = [
        "core/base",
        "dev/zig",
        "apps/browser",
        "games/steam-compat",
    ]
    
    services = [
        { name = "display-server", autostart = true },
        { name = "audio-server", autostart = true },
        { name = "network-manager", autostart = true },
    ]
    
    display {
        resolution = "auto"
        refresh = 120
        hdr = true
    }
}
```

`vanta system apply config.vanta` atomically transitions to the new state.
`vanta system rollback` goes back to the previous state.

### 14.3 Package Format

```
package.vanta {
    name = "my-app"
    version = "1.2.0"
    
    depends = ["vstd >= 1.0"]
    
    capabilities_required = [
        "display:surface",
        "audio:playback",
        "storage:app-data",
    ]
    
    capabilities_optional = [
        "network:internet",      // App works offline
        "camera:capture",        // For QR scanning
    ]
    
    binary = "bin/my-app"        // Native ELF
    // OR
    wasm = "lib/my-app.wasm"     // Universal WASM
}
```

### 14.4 Update Strategy

- **Delta updates** — only download changed blocks
- **Background download** — updates download silently
- **Staged apply** — new version prepared alongside old one
- **Instant switch** — atomic symlink swap, no downtime
- **Rollback always works** — old version kept until confirmed

---

## 15. Security Architecture

### 15.1 Threat Model

| Threat | Mitigation |
|---|---|
| Malicious app | Capability sandbox — can only access granted resources |
| Driver exploit | Userspace drivers — crash can't escalate to kernel |
| Supply chain attack | Content-addressed packages + reproducible builds |
| Use-after-free | Zig's safety checks + capability generation numbers |
| Privilege escalation | No root/admin — capabilities are the only authority |
| Side channels | Separate address spaces, KPTI, microarch mitigations |

### 15.2 No Root / No Admin

There is no superuser. There is no "admin mode." There are only capabilities.

The **system manager** process has capabilities to start/stop services and manage hardware. But even it can't read your files unless it has a capability to your storage.

### 15.3 Verified Boot

1. Firmware verifies bootloader signature
2. Bootloader verifies kernel hash
3. Kernel verifies system server hashes
4. Each component verified before execution

**Optional, not mandatory.** You can disable it — it's YOUR computer. But it's on by default.

### 15.4 App Sandboxing

Every app runs in a sandbox by default:
- Own address space (enforced by hardware)
- Own namespace (only sees granted capabilities)
- Resource limits (CPU, memory, I/O quotas)
- No network access unless granted
- No filesystem access beyond app's own data unless granted

Users see a permission prompt on first use:
```
"Browser wants to access: Internet, File Downloads"
[Allow] [Allow Once] [Deny]
```

### 15.5 Disposable Environments (Qubes-inspired)

Create **temporary sandboxes** for risky operations:

```
vanta sandbox --disposable --caps=display,network -- run suspicious-app
```

Everything inside the sandbox is destroyed when it closes. No persistence.

---

## 16. Portability Strategy

### 16.1 The Layering

```
Layer 3: Application Frameworks (UI, media, etc.)
Layer 2: vstd — Vanta Standard Library
Layer 1: Syscall ABI (stable, versioned, guaranteed)
Layer 0: Kernel
```

**Native apps** use Layers 1-3 directly.
**Ported apps** use compatibility shims that translate to Layer 1.

### 16.2 Native Apps — Vanta SDK

The Vanta SDK provides:
- **vstd**: Standard library (memory, strings, collections, crypto, etc.)
- **vui**: UI framework (widgets, layout, rendering)
- **vaudio**: Audio API
- **vnet**: Networking API
- **vgfx**: Graphics API (Vulkan wrapper + 2D canvas)

SDK is C-ABI compatible. Usable from Zig, C, C++, Rust, or any language with C FFI.

### 16.3 Universal Apps — WASM

WebAssembly as a universal binary format:

- Apps compiled to WASM run on any VantaOS architecture (x86, ARM, RISC-V)
- WASM has a natural capability model (WASI)
- JIT compiled for near-native performance
- Sandboxed by default (WASM can't access memory outside its linear memory)

```
package.vanta {
    name = "universal-app"
    wasm = "lib/app.wasm"      // Runs on ALL architectures
    caps = ["display", "audio"]
}
```

### 16.4 Compatibility Pods

For running existing Linux/Windows apps without modification:

**Linux Compatibility Pod:**
```
┌─────────────────────────────────────┐
│ Linux App (unmodified ELF binary)   │
├─────────────────────────────────────┤
│ musl libc (or glibc)                │
├─────────────────────────────────────┤
│ Linux Syscall Shim                  │  ← Translates Linux syscalls
│ (open→resource_request,             │     to VantaOS capabilities
│  read→cap_recv,                     │
│  write→cap_send, etc.)              │
├─────────────────────────────────────┤
│ VantaOS Process (sandboxed)         │
└─────────────────────────────────────┘
```

**Windows Compatibility Pod (future):**
- Wine-style Win32 translation
- DirectX → Vulkan translation (like DXVK)
- Registry emulation

### 16.5 Game Porting — The Priority Path

Games are the killer app. Easy game porting strategy:

1. **Vulkan games**: Just need a Vulkan ICD (driver). Almost zero porting work.
2. **SDL games**: Port SDL to VantaOS. SDL abstracts input, audio, windowing. Thousands of games instantly compatible.
3. **OpenGL games**: Zink (OpenGL-on-Vulkan translation layer).
4. **DirectX games**: DXVK (DX9/10/11→Vulkan) + VKD3D (DX12→Vulkan).
5. **Unity/Unreal/Godot**: Port the engine runtime. All games on that engine follow.

**Priority order:**
1. Vulkan ICD → native Vulkan games
2. SDL backend → SDL-based games (huge catalog)
3. Linux compat pod + DXVK/VKD3D → Steam/Proton games
4. Engine ports → Unity/Unreal/Godot native

### 16.6 Stable ABI Guarantee

**Promise:** Any binary built for VantaOS syscall ABI v1 will run on all future versions.

How:
- Syscall numbers never change meaning
- New syscalls get new numbers
- Deprecated syscalls remain functional (thin wrapper to new implementation)
- vstd library is backward-compatible (new functions added, old ones never removed)

---

## 17. Feature Matrix: Best of Every OS

### From macOS
- [ ] Spotlight-class system search (instant, indexed, semantic)
- [ ] Quick Look (preview any file with spacebar)
- [ ] Time Machine (versioned backups → our CoW snapshots)
- [ ] Continuity / Handoff (seamless device-to-device)
- [ ] Universal Clipboard (copy on one device, paste on another)
- [ ] AirDrop-style proximity sharing
- [ ] Keychain (secure credential storage via capability)
- [ ] Automator/Shortcuts (visual automation)
- [ ] Smooth 120fps compositor
- [ ] Color management (system-wide P3/HDR)
- [ ] PDF as first-class citizen
- [ ] Consistent UI guidelines (VantaHIG)
- [ ] Drag & Drop everywhere
- [ ] Services menu (app-to-app actions via capabilities)
- [ ] System-wide undo/redo
- [ ] Accessibility (screen reader, voice control, switch control)

### From Windows
- [ ] Broad hardware support (x86_64 first, ARM64 next)
- [ ] Backward compatibility (stable ABI guarantee)
- [ ] Gaming excellence (Vulkan native, DX compat, Game Mode)
- [ ] Plug and Play (hardware auto-detection)
- [ ] Remote Desktop (built-in, capability-secured)
- [ ] Task Manager (deep system visibility)
- [ ] Snap layouts (window tiling)
- [ ] Multi-monitor excellence
- [ ] Clipboard history
- [ ] PowerToys-class utilities (built-in)
- [ ] Full disk encryption (default, transparent)
- [ ] WSL concept → our compatibility pods
- [ ] Game Bar (overlay, recording, performance HUD)
- [ ] HDR + Auto HDR
- [ ] DirectStorage-class GPU-direct I/O

### From Linux
- [ ] Content-addressed package manager (Nix-inspired)
- [ ] Everything configurable (declarative system config)
- [ ] Container-class isolation (capabilities + namespaces)
- [ ] cgroups-class resource control (per-process quotas)
- [ ] PipeWire-class audio system
- [ ] Btrfs/ZFS-class copy-on-write storage
- [ ] Service management (dependency-based, socket-activated)
- [ ] Flatpak-class sandboxed apps (native to our design)
- [ ] Live patching (microkernel hot-swap servers)
- [ ] Tiling window management (built-in, not bolted on)
- [ ] Powerful CLI (Vanta Shell with structured data)
- [ ] DTrace/eBPF-class system tracing
- [ ] SSH-class remote shell (capability-secured)

### From Plan 9
- [ ] Per-process namespaces (core to our design)
- [ ] Network transparency (remote resources look local)
- [ ] Resource protocol (our typed IPC)
- [ ] Plumber (context-aware data routing → our Intent system)
- [ ] Factotum-class auth agent (credentials via capability)

### From Haiku/BeOS
- [ ] Attribute-rich resources (queryable metadata)
- [ ] Query-based virtual folders
- [ ] Responsive under load (pervasive multithreading)
- [ ] Media Kit (system-level audio/video routing)
- [ ] Script teams (inter-app scripting via capabilities)
- [ ] Fast boot

### From iOS/Android
- [ ] Permission model (camera, mic, location, contacts)
- [ ] App lifecycle management
- [ ] Push notifications (system-level)
- [ ] Background task scheduling
- [ ] Sandboxed by default
- [ ] OTA delta updates
- [ ] Biometric auth
- [ ] Adaptive UI (responsive to screen size)
- [ ] Sideloading (not gatekept!)

### From ChromeOS
- [ ] Sub-3-second boot
- [ ] Verified boot chain
- [ ] Auto-update (atomic, background)
- [ ] Web apps as first-class citizens (via WASM)

### From Fuchsia
- [ ] Capability-based kernel (direct inspiration)
- [ ] Component model
- [ ] Typed IPC (FIDL → our VantaIDL)
- [ ] No global filesystem namespace (direct adoption)
- [ ] Structured logs

### From FreeBSD
- [ ] Jails-class isolation (our capability sandboxes)
- [ ] Capsicum capabilities (direct inspiration)
- [ ] DTrace-class tracing
- [ ] Clean kernel/userland separation

### From Qubes OS
- [ ] Security compartmentalization
- [ ] Per-activity isolation
- [ ] Disposable environments

### Unique to VantaOS
- [ ] Capability-first design (everything is a handle)
- [ ] Typed IPC with VantaIDL (not byte streams)
- [ ] WASM as universal binary format
- [ ] AI-native system services (local inference engine)
- [ ] Zero-copy IPC (register-passed small messages)
- [ ] Structured audit trail (every cap operation logged)
- [ ] Intent system (apps declare capabilities, system routes user intent)
- [ ] Declarative system configuration (NixOS-inspired)
- [ ] Built-in profiler, debugger, tracer
- [ ] Crash-proof (microkernel isolates all failures)
- [ ] Energy-aware scheduling
- [ ] Game Mode (kernel-level, not a user script)
- [ ] Mesh networking (device-to-device resource sharing)
- [ ] No root, no admin (capabilities only)

---

## 18. User Experience

### 18.1 First Boot

1. Language / timezone / keyboard
2. Create user profile (no "admin account" — just a profile with system management caps)
3. Network setup
4. System theme (dark by default — we're Vanta Black)
5. Import from previous OS (optional — detect and migrate from Windows/macOS/Linux)
6. Done. Under 3 minutes.

### 18.2 Desktop

Default desktop is a **hybrid tiling/floating compositor**:

- Windows float by default (macOS-like)
- Keyboard shortcuts instantly tile (i3-like)
- Touchpad gestures for window management
- Global search (Cmd/Super+Space) — searches everything: apps, files, settings, content, web
- Dock/taskbar (customizable position)
- System tray (notification area with capability-controlled access)
- Dark theme default with full theming support

### 18.3 Shell — Vanta Shell (vsh)

Not bash. Not PowerShell. A new shell that works with **structured data**.

```vsh
# Commands output typed data, not text
> processes | where cpu > 10 | sort mem desc | take 5
╭───┬─────────────┬───────┬─────────╮
│ # │ name        │ cpu % │ mem MB  │
├───┼─────────────┼───────┼─────────┤
│ 1 │ browser     │ 45.2  │ 2048.0  │
│ 2 │ compositor  │ 12.1  │ 512.0   │
│ 3 │ compiler    │ 11.8  │ 1024.0  │
│ 4 │ game        │ 11.2  │ 4096.0  │
│ 5 │ ide         │ 10.5  │ 768.0   │
╰───┴─────────────┴───────┴─────────╯

# Pipe to different formats
> processes | where cpu > 10 | to json > high-cpu.json

# Query resources by metadata
> resources "~/photos" | where type = "image/jpeg" and width > 4000 | count
42

# Capabilities are visible
> my-caps
╭───┬──────────────────────┬────────────────╮
│ # │ object               │ rights         │
├───┼──────────────────────┼────────────────┤
│ 0 │ Display Server       │ send,recv      │
│ 1 │ Home Storage         │ read,write     │
│ 2 │ Network (internet)   │ send           │
│ 3 │ Audio Server         │ send,recv      │
╰───┴──────────────────────┴────────────────╯
```

Inspired by Nushell but integrated with VantaOS capabilities.

### 18.4 Mesh Networking — Device Continuity

Devices running VantaOS discover each other and share capabilities:

- **Universal Clipboard** — Copy on laptop, paste on desktop
- **File Transfer** — Drag file to device icon, it appears on that device
- **Session Handoff** — Start working on phone, continue on laptop
- **Remote Display** — Use tablet as second monitor
- **Shared Input** — Mouse flows between devices (like Logitech Flow / Universal Control)
- **Capability Delegation** — "Let my phone use my desktop's GPU for this render"

Protocol: mDNS discovery + TLS + mutual capability authentication.

### 18.5 AI-Native Features

On-device, privacy-preserving AI:

- **Smart Search** — Semantic search across all resources ("find that email about the project deadline")
- **Smart Organization** — Auto-tag photos, documents, music
- **Smart Automation** — "Every time I connect to my work WiFi, open Slack and email"
- **Code Completion** — System-wide, not just in IDEs
- **Voice Control** — Natural language system control ("open browser and go to github")
- **Accessibility** — Real-time image descriptions, caption generation

All inference runs **locally**. No cloud. No telemetry. No "requires internet."

---

## 19. Developer Experience

### 19.1 SDK

```
vanta sdk install         # Install dev tools
vanta new my-app          # Scaffold new application
vanta build               # Build for current platform
vanta build --wasm        # Build universal WASM binary
vanta run                 # Run in sandboxed environment
vanta debug               # Attach debugger
vanta package             # Create distributable package
vanta publish             # Publish to repository
```

### 19.2 Built-in Developer Tools

- **System Profiler** — CPU, memory, IPC, capability usage
- **IPC Tracer** — See all messages between processes in real-time
- **Capability Inspector** — Visualize capability trees
- **Memory Visualizer** — See address space layout, page faults, allocations
- **Kernel Console** — Direct kernel debug interface
- **Hot Reload** — For system servers during development

### 19.3 Language Support

The SDK is C-ABI compatible. First-class support for:
- **Zig** — Primary language, best integration
- **C/C++** — Full support via C ABI
- **Rust** — Via C ABI + custom vstd bindings
- **WASM languages** — Any language that compiles to WASM (Go, Swift, Kotlin, etc.)

### 19.4 VantaIDL — Interface Definition Language

Define typed IPC protocols:

```
@version(1)
protocol ResourceServer {
    /// Open a resource by path
    open(path: string, mode: OpenMode) -> result<Cap<Resource>, Error>;
    
    /// Read structured metadata
    stat(resource: Cap<Resource>) -> result<ResourceInfo, Error>;
    
    /// Query resources by metadata
    query(root: Cap<Resource>, filter: QueryFilter) -> result<list<ResourceInfo>, Error>;
    
    /// Watch for changes
    watch(resource: Cap<Resource>) -> Cap<Notification>;
}
```

Generates client/server code in Zig, C, and WASM.

---

## 20. Roadmap

### Phase 0 — Foundation (Current)
- [x] Project structure
- [x] Limine bootloader integration
- [x] Serial debug output
- [x] GDT setup
- [x] IDT setup (minimal)
- [x] Physical memory manager (bitmap)
- [x] Capability type definitions
- [x] IPC message types
- [x] Syscall table design
- [ ] Boots on QEMU with "VantaOS" output

### Phase 1 — Kernel Core
- [ ] Virtual memory manager (VMM)
- [ ] Page fault handler
- [ ] Kernel heap (slab allocator)
- [ ] Capability table implementation
- [ ] IPC port send/receive
- [ ] Thread creation and scheduling
- [ ] Context switching
- [ ] Syscall handler (ring 3 → ring 0 transition)
- [ ] Basic timer (PIT or HPET)

### Phase 2 — Userspace
- [ ] Ring 3 transition (first userspace process)
- [ ] ELF loader
- [ ] Process creation with capabilities
- [ ] User-mode serial console server
- [ ] System call interface validation
- [ ] Basic test suite (kernel self-tests)

### Phase 3 — Essential Services
- [ ] PS/2 keyboard driver (userspace)
- [ ] Framebuffer display driver (userspace)
- [ ] Simple text compositor (framebuffer console)
- [ ] Resource server (in-memory filesystem)
- [ ] PCI bus enumeration
- [ ] AHCI/NVMe storage driver
- [ ] Persistent filesystem (basic)

### Phase 4 — Interactive System
- [ ] Vanta Shell (vsh) — basic commands
- [ ] GPU driver (basic — framebuffer mode)
- [ ] Display compositor (scene graph)
- [ ] Input handling (keyboard + mouse)
- [ ] Window management (floating + tiling)
- [ ] Basic system apps (file manager, text editor, terminal)

### Phase 5 — Ecosystem
- [ ] Package manager (vpm)
- [ ] Content-addressed store
- [ ] Vanta SDK
- [ ] VantaIDL compiler
- [ ] Developer documentation
- [ ] WASM runtime
- [ ] vstd (standard library)

### Phase 6 — Compatibility & Performance
- [ ] Linux compatibility pod (POSIX shim)
- [ ] SDL backend for VantaOS
- [ ] Vulkan ICD (GPU-specific)
- [ ] Audio server (graph-based)
- [ ] Network stack (TCP/IP)
- [ ] DHCP, DNS
- [ ] OpenGL → Vulkan translation (Zink)

### Phase 7 — Polish & Features
- [ ] Game Mode
- [ ] HDR display support
- [ ] Bluetooth stack
- [ ] WiFi driver
- [ ] USB stack
- [ ] AI inference engine
- [ ] System-wide search
- [ ] Mesh networking
- [ ] Accessibility framework
- [ ] Security audit

---

## Appendices

### Appendix A: Syscall Quick Reference

| # | Name | Args | Returns |
|---|---|---|---|
| 0 | cap_send | port, msg_ptr | error |
| 1 | cap_recv | port, buf_ptr | error, msg_size |
| 2 | cap_call | port, msg_ptr, reply_ptr | error |
| 3 | cap_derive | cap, rights_mask | new_handle, error |
| 4 | cap_revoke | cap | error |
| 5 | cap_inspect | cap, info_ptr | error |
| 10 | mem_create | size, flags | mem_cap, error |
| 11 | mem_map | space, mem, addr, prot | error |
| 12 | mem_unmap | space, addr, size | error |
| 13 | mem_share | mem, rights | shared_cap, error |
| 20 | proc_create | space, caps_ptr, caps_len | proc_cap, thread_cap, error |
| 21 | thread_create | proc, entry, stack, arg | thread_cap, error |
| 22 | thread_exit | code | — |
| 23 | thread_yield | — | — |
| 24 | thread_sleep | nanoseconds | error |
| 30 | irq_create | irq_num | irq_cap, error |
| 31 | irq_wait | irq_cap | error |
| 32 | irq_ack | irq_cap | error |
| 33 | io_map | phys_addr, size | io_cap, error |
| 40 | sys_info | query_type, buf_ptr | error |
| 41 | sys_log | msg_ptr, msg_len | error |
| 42 | sys_time | — | nanoseconds |
| 43 | sys_shutdown | action | — |

### Appendix B: Capability Rights Bitmap

```
Bit 0:  READ      — Read data/state
Bit 1:  WRITE     — Modify data/state
Bit 2:  EXECUTE   — Execute code / invoke operations
Bit 3:  GRANT     — Transfer capability to another process
Bit 4:  DERIVE    — Create restricted child capabilities
Bit 5:  REVOKE    — Destroy capability and all children
Bit 6:  MAP       — Map into address space
Bit 7:  CONNECT   — Create connections
Bit 8:  MANAGE    — Administrative operations
Bit 9:  INSPECT   — Query metadata
Bits 10-31: Reserved for future use
```

### Appendix C: IPC Message Wire Format

```
Offset  Size  Field
0       4     msg_type (operation code)
4       4     flags
8       64    inline payload (register-passable)
72      4×4   capability handles (up to 4 transfers)
88      8     shared memory capability (optional, for bulk data)
                Total: 96 bytes per message
```

### Appendix D: Building & Running

```bash
# Prerequisites
# - Zig 0.13+ (https://ziglang.org)
# - QEMU (qemu-system-x86_64)
# - xorriso (for ISO creation)
# - git (for Limine download)

# Build kernel
zig build

# Create bootable ISO (downloads Limine on first run)
./scripts/build-iso.sh

# Run in QEMU
./scripts/run-qemu.sh

# Or manually:
qemu-system-x86_64 -cdrom vanta.iso -serial stdio -m 256M -no-reboot
```

---

*This document is a living specification. It will evolve as VantaOS develops.*
*Last updated: Phase 0 — Foundation*
