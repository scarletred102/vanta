# Vanta Rust-Native Developer OS Roadmap

## Summary

Vanta is currently a strong kernel prototype, but not yet a usable OS: the shell and filesystem are kernel-resident, standard descriptors are incomplete, and there is no native userland/runtime. The usable-release gate is: boot a persistent GPT disk, mount RedoxFS, start a native `/sbin/init`, log into a regular developer account, and use a terminal with files, pipelines, redirection, signals, and cross-built tools.

Use Linux 6.18.39 as the behavioral reference for x86_64 process, ELF, TTY, VFS, and error semantics. Use the [Redox kernel](https://gitlab.redox-os.org/redox-os/kernel), [syscall](https://gitlab.redox-os.org/redox-os/syscall), [RedoxFS](https://gitlab.redox-os.org/redox-os/redoxfs), and [relibc](https://gitlab.redox-os.org/redox-os/relibc) as Rust-native architectural references—without adopting Linux’s kernel ABI or copying code without provenance.

## Long-term compatibility architecture

The current document remains the near-term native terminal roadmap. The broader
compatibility strategy—Linux, Win32, Android, guests, service extraction,
phase gates, and Codex execution rules—is maintained in
[2026-07-30-universal-compatibility-roadmap.md](2026-07-30-universal-compatibility-roadmap.md).

## Implementation status — 2026-08-10

### Track B SDK and process-context bundle — 2026-08-10

The next post-Gate-A SDK bundle is complete. `libvanta` now provides bounded
directory handles, a stable bootstrap environment (`VANTA_ABI_VERSION=0`),
process launch/wait wrappers, and native `exec` coverage. Generated SDK smoke
programs verify directory access, allocation, environment lookup, successful
and nonzero child exit statuses, and image replacement via `exec`. GPT
acceptance runs these alongside the existing file and stdio smokes across
first boot, reboot persistence, and corrupt-root recovery; legacy, VirtIO,
network, and focused host/cross-target checks remain green.

Full environment propagation/storage, broader process-runtime behavior, and
`FILE`/relibc compatibility are not claimed by this change.

### Immediate roadmap deliverables — 2026-07-30

### ABI v0 contract update — 2026-08-01

The ABI crate now has tested golden vectors for every current Vanta syscall,
errno decoding including signed-boundary rejection, capability slot/generation
boundaries, feature discovery bits, and the `repr(C)` signal, credential, and
directory-record layouts. This freezes the host-side v0 contract without
claiming ABI v1 negotiation. `GetAbiInfo` now exposes the frozen version and
feature bits to native callers, and the GPT C hello acceptance path validates
that query before printing its success marker.

The first external SDK slice is also complete: `libvanta` now wraps the
implemented descriptor, directory, pipe, process, scheduling, signal, and
path-mutation syscalls. A generated `/bin/c-sdk-smoke` program exercises the
surface in GPT QEMU. The next stdio slice is also complete: unbuffered stream
wrappers and a generated `/bin/c-stdio-smoke` program create, write, reopen,
read, and remove a file during GPT native acceptance. Buffered `FILE`
semantics, environment, threading, and relibc remain later runtime work.

### Buffered stdio bootstrap — 2026-08-07

The stdio slice now includes a bounded buffered `vanta_file_t` object over
Vanta descriptors. The GPT `/bin/c-stdio-smoke` acceptance program verifies
buffered `putc`, bulk write, `getc`, EOF, explicit flush, close, and file
removal. This is a runtime bootstrap contract, not full global `FILE` streams,
formatting, environment handling, or relibc compatibility.

The implementation worktree now includes the first `libvanta` static-library
bootstrap, a reproducible `cargo xtask sdk` artifact, the initial `linuxd`
static syscall translation contract, and capability-bearing service request
headers. The generated GPT image now executes the linked `/bin/c-hello`
program during native acceptance. These are foundation deliverables, not
completion of the full C runtime or Linux personality. The broader status and remaining gates are in
[2026-07-30-universal-compatibility-roadmap.md](2026-07-30-universal-compatibility-roadmap.md).

**Current state:** the real Gate A native developer OS milestone is verified on
`main` as of 2026-08-10.
The GPT image boots RedoxFS, starts native `/sbin/init` and `/bin/vsh`, enforces
developer ownership/modes, blocks pipe readers in the kernel until wakeup,
executes real shell pipeline/redirection paths, and passes the native,
legacy, VirtIO, network, and GPT acceptance gates. Vanta is not yet a full
developer-platform release: the C runtime remains a bootstrap profile,
`linuxd` is still a translation contract, and full POSIX signal handlers,
process groups, `fork`, and dynamic runtime support remain later work.

Gate A is now closed by one generated-image workflow: `/sbin/init` starts as
root and demotes spawned developer programs to `vanta` (`1000:1000`), the
developer gate rejects `/etc` creation while allowing `/home/vanta` file
creation/write/removal, the same GPT image passes first boot and reboot
persistence checks, and a truncated root image enters the kernel recovery
shell without launching native tasks. Legacy, VirtIO, network, focused Rust,
formatting, kernel, and userland checks also pass.

Completed foundation work:

- Rust workspace baseline with `vanta-abi`, `vanta-gpt`,
  `vanta-redoxfs-adapter`, `vanta-image`, and host `xtask` crates.
- Vanta ABI v0 numbers, errno encoding, credentials, capability IDs, rights,
  and capability-backed descriptor-table groundwork.
- Pinned RedoxFS `99bc185bf8ad8bd6f4d2562c424d800c2a3d310b`, provenance,
  license/update notes, and a bounded 4 KiB-to-512 byte sector adapter with
  `EIO` translation tests.
- Validated GPT root discovery, including a kernel VirtIO probe that preserves
  the legacy VantaFS path when no Vanta root partition exists.
- `cargo xtask image`, which emits `target/vanta-gpt.img` with a FAT ESP,
  Limine/kernel payload, formatted RedoxFS root, and source-revision manifest.
- Existing ABI, GPT, RedoxFS-adapter, kernel-release, legacy QEMU, VirtIO,
  and VirtIO-network QEMU regressions were last verified passing on 2026-08-07.
- Native `/sbin/init`, `/bin/vsh`, and static base commands are installed into
  the generated RedoxFS image; GPT QEMU boots the native-init path.
- Descriptor-backed TTY, pipe resources, capability rights, descriptor close
  lifetime handling, native spawn stdio plumbing, and native userland syscall
  wrappers are present, with host pipe/TTY lifetime tests.
- Native-only open flags now support read/write/create/truncate/append file
  descriptors backed by the mounted root, and native mkdir/unlink/rename
  entrypoints are wired through the RedoxFS VFS boundary.
- Native directory descriptors now expose directory entries through
  `getdents`-compatible reads, and `fstat` reports file/directory modes.
- Native pipe readers enter a kernel wait state and writers/close wake matching
  readers; the userland wrapper retries after the wakeup instead of busy-looping.
- Native mutation checks now restrict the developer account to `/home/vanta`
  and `/tmp`, while root retains system mutation authority.
- GPT boot now performs and verifies a RedoxFS write/remount/read persistence
  check, and the GPT harness requires the successful marker.
- TTY input recognizes Ctrl-C when Control is held, targets the foreground
  child instead of the shell, and exposes signal delivery through `kill`;
  terminated children become waitable zombies with status `128 + signal`.
- Native ELF startup now receives a constructed `argc/argv` stack, and the
  native spawn path can copy argv strings into child processes for argumented
  commands and redirection-aware shell execution.
- The image bundles `ls`, `mkdir`, `rm`, `mv`, `pwd`, and `stat` alongside the
  original init, shell, and stream commands.
- The legacy synthetic ELF fixture remains compatible after isolating its
  historical syscall register convention; non-VirtIO, VirtIO, and GPT QEMU
  gates pass. Native syscall clients now declare the x86_64 syscall clobbers,
  while the kernel keeps the legacy and native register-return paths separate.

Gate A is complete. The explicit remaining work is full custom signal-handler
delivery, process groups/job control, and copy-on-write `fork`; release work
now moves to `libvanta`/relibc expansion, the static C toolchain, and the Linux
personality.

## Architecture and interface contract

- Convert `rust/` into a Rust workspace: kernel, `vanta-abi`, `libvanta`, userland programs, host `xtask`, and versioned third-party forks.
- Publish a versioned Vanta ABI v0. Native syscalls use Vanta-owned numbers and POSIX-shaped results; Linux syscall numbers are handled only by the later compatibility personality.
- Replace special-cased stdin/stdout with descriptor-backed objects:
  `File`, `Directory`, `Tty`, `PipeRead`, `PipeWrite`, `Socket`, `Device`, and internal capability handles.
- Expose native operations for `openat`, `read`, `write`, `close`, `dup3`, `pipe2`, `lseek`, `fstat`, directory reads, mkdir/unlink/rename, `ioctl` for TTY, `spawnve`/`execve`, `waitpid`, `exit`, `kill`, `sigaction`, `brk`, `mmap`, and `munmap`.
- Keep authority kernel-owned: every descriptor resolves to an opaque capability with generation and rights checks. `init` receives mount/device/process-admin authority; normal applications receive only inherited terminal and filesystem descriptors.
- Define `Credentials { uid, gid, groups, umask }`. Ship `root` (`0:0`) and `vanta` (`1000:1000`); enforce RedoxFS UID/GID/mode metadata during path traversal and mutation.
- Load static ELF binaries only at first. The kernel builds `argc`, `argv`, `envp`, and a minimal aux vector; `PT_INTERP`, native threads, and dynamic linking return a clear unsupported error until later.

## Development path

1. **Freeze the reference baseline and build system**
   - Preserve current QEMU boot, SMP, VirtIO, network, and persistence tests while replacing ad-hoc scripts with Rust `xtask` commands wrapped by the existing PowerShell entrypoints.
   - Track Linux 6.18.39 and exact Redox source revisions in the reference document.
   - Vendor two pinned MIT forks: RedoxFS `99bc185bf8ad8bd6f4d2562c424d800c2a3d310b` and relibc `4b2cc549cd22634509ff3572cf9bb59ef6285f22`; retain upstream licenses, commit provenance, patch notes, and an update procedure.

2. **Make user processes and descriptors real**
   - Replace fixture-only ELF execution and the kernel shell fallback with process credentials, per-process address spaces, real descriptor tables, blocking/wakeup queues, and child lifecycle management.
   - Implement `spawnve` first for native shell pipelines; add copy-on-write `fork` before the Linux personality and broader POSIX shell support.
   - Route standard descriptors to a kernel TTY object, not direct serial output. Preserve serial as an early-boot and recovery console.

3. **Adopt RedoxFS in a kernel adapter**
   - Create a narrow `RedoxFsBackend` boundary over Vanta block I/O, VFS requests, credentials, and Vanta errno translation. It must not depend directly on scheduler internals, enabling later extraction into `vfsd`.
   - Adapt RedoxFS’s 4 KiB `Disk` contract to the existing 512-byte block driver, enforce partition bounds, serialize filesystem mutations, and translate device failures to `EIO` rather than panicking.
   - Replace VantaFS as the writable root. Keep the existing in-memory filesystem only as a recovery/bootstrap facility during the transition.

4. **Build one persistent UEFI/GPT disk**
   - Generate `vanta-gpt.img` with a FAT ESP containing Limine and the kernel, plus a RedoxFS root partition using Vanta GPT type GUID `5d2f0d4e-9cff-4b2f-a9b6-6bf9eaa4d201`.
   - Boot this single image through QEMU VirtIO. Add the required transitional/modern VirtIO transport support before switching the default test path if OVMF cannot boot the existing legacy device configuration.
   - `xtask image` formats the root, installs binaries/configuration, assigns ownership and modes, and emits a manifest containing kernel, fork, and image-build revisions.
   - Kernel boot sequence: find validated GPT root → mount RedoxFS → execute `/sbin/init`; a missing/corrupt root enters a minimal serial recovery console rather than the normal shell.

5. **Deliver the terminal-first native base system**
   - Implement Rust `/sbin/init`, `/bin/vsh`, and essential tools. `init` owns root authority, mounts pseudo-devices, starts the TTY, and launches `vsh` as `vanta`.
   - `vsh` supports command execution, exit status, `<`, `>`, `>>`, `2>`, `|`, Ctrl-C, and foreground child cleanup. Background jobs, full job control, terminal multiplexers, and a graphical UI remain out of scope.
   - Bundle `echo`, `cat`, `ls`, `mkdir`, `rm`, `mv`, `pwd`, `true`, `false`, and a file/status inspection tool. Native Rust tools may remain `no_std` initially; the externally supported application surface is the C/POSIX ABI.

6. **Port relibc as Vanta’s C ABI**
   - Create `libvanta`, modeled on Redox `libredox`, as the Rust client/runtime layer over Vanta ABI v0.
   - Port relibc’s bootstrap profile: CRT startup, allocator, errno, stdio, files/directories, environment, process launch/wait, signals, and static C linking. Defer pthreads, TLS-heavy programs, dynamic linking, and a full Rust `std` target.
   - Provide a host cross-toolchain path that builds static `x86_64-vanta` C applications into the RedoxFS image. Keep all Vanta kernel/runtime implementation Rust.

7. **Add the Linux userspace personality**
   - Add a privileged Rust `linuxd` service. ELF loading marks Linux x86_64 binaries as a separate personality; their syscall traps are brokered to `linuxd`, which translates them to Vanta ABI operations under the caller’s credentials and capabilities.
   - First target: static musl Linux utilities and simple command-line programs. Support file I/O, directories, process identity, memory setup, signals, `exec`, and static process creation needed by the selected test tools.
   - Reject dynamic glibc binaries and unsupported syscalls explicitly at first. Add `fork`/clone coverage, `PT_INTERP`, TLS, and dynamic loader support only after static-musl regression coverage is stable.

8. **Harden and extract services after usability**
   - Keep RedoxFS kernel-resident for the first release, but use the established backend boundary to move it into a `vfsd` Rust service after the ABI is proven.
   - Move network and additional drivers toward similarly capability-scoped services. QEMU VirtIO remains the release target; AHCI/NVMe, USB HID, real-hardware boot, package management, self-hosted Rust, and graphical work follow the stable terminal release.

## Test plan and usable-release gate

- Host tests cover ABI encoding, descriptor rights/lifetimes, path permission checks, GPT validation, RedoxFS block adaptation, corruption handling, ELF argument stack construction, pipes, and signal delivery.
- QEMU tests boot the generated GPT image, verify `/sbin/init` and `vsh`, create files as `vanta`, reject unauthorized writes, reboot and confirm persistence, and recover safely from absent/corrupt root media.
- Terminal acceptance: `echo hello | cat > /home/vanta/out`, `cat < /home/vanta/out`, stderr redirection, child exit status, and Ctrl-C terminating a blocked foreground child.
- C ABI acceptance: compile and run static C programs using stdio, directories, allocation, fork/exec where enabled, and mode checks through relibc.
- Linux personality acceptance: execute a static-musl Linux hello program, `cat`, and `ls` through `linuxd`; unsupported dynamic binaries fail deterministically without destabilizing the kernel.
- The current boot/SMP/VirtIO/network tests remain mandatory throughout; no phase replaces an existing regression with weaker coverage.

## Assumptions

- The first release is QEMU x86_64/UEFI-first, with one GPT VirtIO disk and no self-hosted compiler.
- The console starts directly as `vanta`; root is reserved for init/recovery initially. Account authentication, remote login, and privilege elevation are deferred, while ownership and mode enforcement are active.
- Vanta’s native ABI is stable only after the terminal, relibc bootstrap profile, and persistence tests pass. RedoxFS starts in the kernel by explicit choice, then moves to `vfsd` through the same backend interface.
