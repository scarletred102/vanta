# linuxd static Linux personality foundation

This crate is the translation contract for the first Linux x86_64 static-ELF
personality. It parses static x86_64 `ET_EXEC`/`ET_DYN` images, rejects
`PT_INTERP`, maps supported Linux syscall families to Vanta-owned syscall
numbers, and reports all other numbers as unsupported.

`StaticElf` is the loader metadata contract and `broker` is the explicit trap
decision contract. The broker preserves the caller capability authority and
never silently turns unsupported Linux syscalls into native operations.

It is not yet a kernel Linux trap endpoint or a complete Linux process runtime.
The next integration step is to attach `LinuxSyscallRequest` to a
Linux-personality process context and route decisions through the now-booted
kernel IPC service path.
