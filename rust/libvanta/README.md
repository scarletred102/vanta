# libvanta bootstrap profile

`libvanta` is the first external C ABI proof for Vanta. It is a `no_std`
static library exposing the native syscall convention through stable C names.
It includes a freestanding `_start` that forwards the native initial stack to
`main`, plus a bounded bootstrap allocator for early C programs.

The current bootstrap profile exposes the implemented native syscall families:
file I/O, descriptors, directory reads, metadata, pipes, process identity and
waiting, scheduling, signals, path mutation, ABI discovery, and a bounded
single-process allocator. It does not claim to be a complete libc, thread
runtime, environment, or POSIX implementation yet.

The header and sample are the contract surface:

- `include/vanta.h`
- `examples/hello.c`
- `examples/sdk_smoke.c`

`cargo xtask sdk` builds both C samples. The generated GPT image runs the
smoke sample during native acceptance and requires its success marker.

The next SDK step is stdio, environment, and broader process runtime support
suitable for relibc rather than only direct syscall wrappers.
