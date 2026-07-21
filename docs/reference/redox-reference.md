# Redox OS Reference

This document records the Redox OS sources used as an architectural reference
for Vanta's Rust rewrite. Redox source checkouts live outside Git under
`third_party/redox/` and `third_party/redox-sources/`.

## Snapshot

Fetched on 2026-07-21 from the official Redox GitLab group. The checkouts are
shallow clones pinned to their upstream `HEAD` at fetch time:

| Component | Local path | Commit | Upstream |
| --- | --- | --- | --- |
| Build system | `third_party/redox/` | `2e495efc34d0d882e16cc393fddc9efaa897d1e9` | [redox-os/redox](https://gitlab.redox-os.org/redox-os/redox) |
| Kernel | `third_party/redox-sources/kernel/` | `3a37925d8c553f8d4b0d8f7763d8b45a01a284ea` | [redox-os/kernel](https://gitlab.redox-os.org/redox-os/kernel) |
| Syscall ABI | `third_party/redox-sources/syscall/` | `1db4871e8058ee3ddc94075f5805c02859396b4d` | [redox-os/syscall](https://gitlab.redox-os.org/redox-os/syscall) |
| C/POSIX library | `third_party/redox-sources/relibc/` | `4b2cc549cd22634509ff3572cf9bb59ef6285f22` | [redox-os/relibc](https://gitlab.redox-os.org/redox-os/relibc) |
| Base services and drivers | `third_party/redox-sources/base/` | `59cf8189337bb8706ac1f52a226b24e9fa555d9c` | [redox-os/base](https://gitlab.redox-os.org/redox-os/base) |
| Filesystem | `third_party/redox-sources/redoxfs/` | `99bc185bf8ad8bd6f4d2562c424d800c2a3d310b` | [redox-os/redoxfs](https://gitlab.redox-os.org/redox-os/redoxfs) |
| Rust userspace library | `third_party/redox-sources/libredox/` | `bedf0129b9ad533c3c094b9648f4bac8936d7c99` | [redox-os/libredox](https://gitlab.redox-os.org/redox-os/libredox) |
| Shell | `third_party/redox-sources/ion/` | `1440704f7456fa4c9f873b7b17dd4f0369b0c4ab` | [redox-os/ion](https://gitlab.redox-os.org/redox-os/ion) |
| Terminal library | `third_party/redox-sources/termion/` | `c784cec0a6f8d1b02692eb58d781acf172bb5959` | [redox-os/termion](https://gitlab.redox-os.org/redox-os/termion) |

The `base` repository contains `drivers/graphics/ihdgd/src/device/aux.rs`.
Windows treats `AUX` as a reserved device name, so Git cannot materialize
that working-tree path. The object is present in the shallow clone and can be
read with `git show HEAD:drivers/graphics/ihdgd/src/device/aux.rs`; the rest of
the Redox source checkouts are ordinary working trees.

Each component carries its own license. The checked-out kernel, syscall,
relibc, RedoxFS, libredox, Ion, and Termion repositories identify themselves
as MIT-licensed. This checkout is reference material, not code imported into
Vanta.

## What Redox contributes as a reference

Redox is a full operating-system tree built from separate Rust projects, not
only a kernel. Its official build repository describes the main layers as a
microkernel, base services and drivers, RedoxFS, relibc, a shell, a terminal
library, and packaging/build tooling.

### Kernel and syscall boundary

The Redox kernel is a `no_std` Rust microkernel with architecture-specific
startup, paging, interrupts, context switching, memory management, and a
small syscall surface. The `redox_syscall` crate owns syscall numbers, data
structures, flags, and userspace wrappers.

The important boundary is the scheme system:

1. A process opens a path such as a device, filesystem namespace, process
   view, or terminal resource.
2. The kernel validates the path and descriptor and routes the operation to a
   kernel scheme or a userspace scheme server.
3. The scheme server performs the resource-specific operation and returns a
   completion result to the kernel.

This keeps resource implementations such as filesystems, networking, device
drivers, and terminals outside the core kernel while retaining kernel-owned
process, memory, descriptor, and isolation primitives.

Redox's syscall issue tracker is also a useful warning for Vanta: the syscall
crate's internal ABI is not automatically a stable application ABI. Redox
currently describes the stable compatibility layer as living primarily in
relibc, while the lower-level kernel interface may evolve.

### Userspace and terminal stack

The base repository supplies essential daemons, drivers, init scripts, and
resource servers. The shell and terminal libraries are separate projects.
Relibc supplies the C/POSIX surface and its `redox-rt` runtime supplies major
process-runtime behavior such as fork/exec and signals. This is the layer that
lets ordinary programs exist independently of the kernel implementation.

For Vanta's terminal-first goal, the sequence to study is:

```text
kernel process + fd primitives
        -> syscall/resource ABI
        -> terminal/PTY service
        -> init and shell
        -> separate user programs
        -> libc or Rust userspace runtime
```

The useful Redox source locations are:

- `kernel/src/context/` for process contexts, descriptors, blocking, and
  user/kernel transitions.
- `kernel/src/scheme/` for kernel resources and userspace scheme routing.
- `kernel/src/syscall/` for syscall implementations and user-copy rules.
- `relibc/src/platform/redox/` and `relibc/redox-rt/` for the POSIX/runtime
  boundary.
- `base/ptyd/`, `base/init/`, `base/init.d/`, and the daemon directories for
  service startup and terminal plumbing.
- `ion/` and `termion/` for shell behavior and terminal control rather than
  kernel input parsing.

### Filesystem and storage

RedoxFS is a userspace-oriented filesystem project with copy-on-write,
data/metadata checksums, transparent encryption, Unix attributes, and Linux
FUSE support. It is a reference for a later Vanta filesystem service, not a
reason to replace the current VantaFS implementation immediately.

The build system also shows an important operational detail: a usable OS needs
an image builder, init filesystem, package metadata, permissions, init scripts,
and repeatable QEMU/real-hardware workflows. A kernel that can read and write
its own VFS is not yet a user-facing operating-system distribution.

## Redox compared with Vanta

| Area | Vanta today | Redox reference lesson |
| --- | --- | --- |
| Boot and kernel | Limine boot, x86_64 kernel, paging, interrupts, SMP, scheduler | Keep kernel mechanisms small and make boundaries explicit |
| User processes | Ring-3 ELF loading and scheduler exist; test ELFs are embedded in the kernel | Move from embedded fixtures to separately built user binaries |
| Files | Writable VantaFS is mounted and tested in QEMU | Treat filesystem access as a service/ABI boundary before adding more filesystem features |
| Terminal | Keyboard input and shell commands are in-kernel | Build a terminal/PTY resource and move the shell out of the kernel |
| Syscalls | Small Vanta-specific fd/process/socket ABI | Define a deliberate Vanta ABI; borrow concepts, not Redox syscall numbers |
| Networking | QEMU VirtIO networking and a constrained TCP path work | Add userspace-facing network resources after terminal/file resources are solid |
| Drivers | Primarily QEMU-oriented VirtIO and platform code | Keep device-specific logic out of the core kernel where practical |
| Userland | No real init/libc/command package pipeline yet | Add init, a Rust runtime, shell, and basic commands as separate binaries |
| Packaging | Kernel/ESP build scripts exist | Add a userland image/manifest step that is reproducible and testable |

## Direction for the terminal-first milestone

Use Linux 6.18 as the reference for x86_64 hardware behavior and familiar
process/file semantics. Use Redox as the reference for decomposition:

1. Define a minimal Vanta resource ABI for terminal, file, and process
   operations.
2. Implement a kernel-backed terminal resource with blocking read/write and
   descriptor inheritance.
3. Build a separate `init` user program and a small Rust shell/command set.
4. Make the boot image contain those binaries instead of embedding synthetic
   ELF fixtures in kernel Rust source.
5. Add an end-to-end QEMU test that boots, starts init, runs commands, reads
   and writes VantaFS files, and exits a child process with a status code.

The first milestone should not copy Redox's scheme ABI wholesale, add POSIX
compatibility, or pull in the full Redox build system. It should establish a
small Vanta-owned boundary that can later host a terminal server, filesystem
server, network service, and compatibility layer without making the kernel
shell the permanent user interface.
