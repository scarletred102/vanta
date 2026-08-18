# linuxd static Linux personality

This crate is the translation contract for the first Linux x86_64 static-ELF
personality. It parses static x86_64 `ET_EXEC`/`ET_DYN` images, rejects
`PT_INTERP`, maps supported Linux syscall families to Vanta-owned syscall
numbers, and reports all other numbers as unsupported.

`StaticElf` is the loader metadata contract and `broker` is the explicit trap
decision contract. The kernel now carries `LinuxX86_64Static` process
personality metadata, routes those traps through this broker, and translates
the supported file/process subset at the foreign-ABI boundary. The native
Vanta syscall table is unchanged.

This is not a complete Linux process runtime. The current QEMU acceptance
bundles static ELF hello, cat, ls, and server samples, plus an unsupported
syscall probe and a dynamic-interpreter rejection case. The next Linux work is
broader memory, pipes, signals, wait/exec, networking, and a larger musl
corpus. The current statically linked musl hello exercises FS-base/TLS setup
and the process-startup subset in QEMU.
