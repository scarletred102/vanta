import os
import struct

filepath = 'kernel/bin/ahci'

with open(filepath, 'rb') as f:
    elf_data = f.read()

shoff = struct.unpack_from('<Q', elf_data, 40)[0]
shentsize = struct.unpack_from('<H', elf_data, 58)[0]
shnum = struct.unpack_from('<H', elf_data, 60)[0]
shstrndx = struct.unpack_from('<H', elf_data, 62)[0]

symtab_sh = None
strtab_sh = None
shstrtab_offset = struct.unpack_from('<Q', elf_data, shoff + shstrndx * shentsize + 24)[0]

for i in range(shnum):
    sh_offset = shoff + i * shentsize
    sh_type = struct.unpack_from('<I', elf_data, sh_offset + 4)[0]
    if sh_type == 2:
        symtab_sh = sh_offset
    elif sh_type == 3:
        name_offset = struct.unpack_from('<I', elf_data, sh_offset)[0]
        name = ""
        curr = shstrtab_offset + name_offset
        while elf_data[curr] != 0:
            name += chr(elf_data[curr])
            curr += 1
        if name == ".strtab":
            strtab_sh = sh_offset

if symtab_sh and strtab_sh:
    sym_offset = struct.unpack_from('<Q', elf_data, symtab_sh + 24)[0]
    sym_size = struct.unpack_from('<Q', elf_data, symtab_sh + 32)[0]
    sym_entsize = struct.unpack_from('<Q', elf_data, symtab_sh + 56)[0]
    str_offset = struct.unpack_from('<Q', elf_data, strtab_sh + 24)[0]
    
    for i in range(0, sym_size, sym_entsize):
        off = sym_offset + i
        st_name = struct.unpack_from('<I', elf_data, off)[0]
        st_value = struct.unpack_from('<Q', elf_data, off + 8)[0]
        st_size = struct.unpack_from('<Q', elf_data, off + 16)[0]
        
        name = ""
        curr = str_offset + st_name
        while curr < len(elf_data) and elf_data[curr] != 0:
            name += chr(elf_data[curr])
            curr += 1
        if st_value != 0:
            print(f"0x{st_value:08x} - 0x{st_value+st_size:08x} (size: {st_size}): {name}")
