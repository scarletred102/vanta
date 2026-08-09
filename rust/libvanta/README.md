# libvanta bootstrap profile

`libvanta` is the first external C ABI proof for Vanta. It is a `no_std`
static library exposing the native syscall convention through stable C names.
It includes a freestanding `_start` that forwards the native initial stack to
`main`, plus a bounded bootstrap allocator for early C programs.

The current bootstrap profile exposes the implemented native syscall families:
file I/O, descriptors, directory reads, metadata, pipes, process identity and
waiting, scheduling, signals, path mutation, ABI discovery, and a bounded
single-process allocator. It provides unbuffered stream wrappers plus a bounded
buffered `vanta_file_t` layer over Vanta descriptors. It does not claim to be a
complete libc, thread runtime, environment, or POSIX implementation yet.

The header and sample are the contract surface:

- `include/vanta.h`
- `examples/hello.c`
- `examples/sdk_smoke.c`
- `examples/stdio_smoke.c`
- `examples/dir_smoke.c`
- `examples/env_smoke.c`
- `examples/process_smoke.c`
- `examples/exec_smoke.c`

`cargo xtask sdk` builds the C samples. The generated GPT image runs the SDK
and buffered stdio plus directory smoke samples during native acceptance and
requires their success markers. The stdio sample exercises buffered `putc`,
bulk write, `getc`, EOF, flush, close, and file removal behavior. The directory
sample exercises the directory handle wrapper and bounded allocator. The
environment sample exposes the stable bootstrap environment, and the process
sample verifies native launch plus zero/nonzero child exit status through
`vanta_spawn` and `vanta_waitpid`.
The exec sample verifies replacement of the current image through
`vanta_exec`.

The next SDK steps are environment, broader process runtime support, and the
eventual `FILE`/relibc compatibility surface beyond these bounded wrappers.
