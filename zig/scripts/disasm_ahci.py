import os
import struct

target_addr = 0x40073e
filepath = 'kernel/bin/ahci'

with open(filepath, 'rb') as f:
    elf_data = f.read()

# Find segment containing target_addr
phoff = struct.unpack_from('<Q', elf_data, 32)[0]
phentsize = struct.unpack_from('<H', elf_data, 54)[0]
phnum = struct.unpack_from('<H', elf_data, 56)[0]

p_offset = None
p_vaddr = None
for i in range(phnum):
    ph_offset = phoff + i * phentsize
    p_type = struct.unpack_from('<I', elf_data, ph_offset)[0]
    if p_type == 1:
        vo = struct.unpack_from('<Q', elf_data, ph_offset + 8)[0]
        va = struct.unpack_from('<Q', elf_data, ph_offset + 16)[0]
        sz = struct.unpack_from('<Q', elf_data, ph_offset + 32)[0]
        if va <= target_addr < va + sz:
            p_offset = vo
            p_vaddr = va
            break

if p_offset is not None:
    file_offset = p_offset + (target_addr - p_vaddr)
    # Print 64 bytes before and 64 bytes after
    start = file_offset - 64
    end = file_offset + 64
    
    # Also find all symbols in ahci to help locate where this is
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

    symbols = []
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
            symbols.append((name, st_value, st_size))
            
    symbols.sort(key=lambda x: x[1])
    
    print("SURROUNDING BYTES:")
    for byte_off in range(start, end, 16):
        va_addr = p_vaddr + (byte_off - p_offset)
        hex_s = " ".join(f"{b:02x}" for b in elf_data[byte_off:byte_off+16])
        # Find if any symbol starts here
        sym_name = ""
        for name, val, size in symbols:
            if val == va_addr:
                sym_name = f" <{name}>"
                break
        print(f"0x{va_addr:08x}:{sym_name}   {hex_s}")
        
    print("\nCLOSEST SYMBOLS:")
    for name, val, size in symbols:
        if val - 256 <= target_addr <= val + size + 256:
            print(f"0x{val:08x} - 0x{val+size:08x} (size: {size}): {name}")
