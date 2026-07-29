# linuxd first spike

This crate is the translation contract for the first Linux x86_64 static-ELF
personality. It deliberately maps only the syscall families that the current
Vanta ABI can represent and reports all other numbers as unsupported.

It is not yet a syscall-trap broker or a complete Linux process runtime. The
next implementation step is to connect this table to a Linux-personality ELF
loader and an explicit `linuxd` service request path.
