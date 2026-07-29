# libvanta bootstrap profile

`libvanta` is the first external C ABI proof for Vanta. It is a `no_std`
static library exposing the native syscall convention through stable C names.
It includes a freestanding `_start` that forwards the native initial stack to
`main`, plus a bounded bootstrap allocator for early C programs.

The current bootstrap profile intentionally contains only direct file and
process operations and a bounded single-process allocator. It does not claim
to be a complete libc, thread runtime, or POSIX implementation yet.

The header and sample are the contract surface:

- `include/vanta.h`
- `examples/hello.c`

The next SDK step is a Vanta CRT/startup object and a freestanding C build
recipe that links this library into the generated RedoxFS image.
