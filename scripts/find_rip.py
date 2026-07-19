import os
import struct

def find_address_in_elf(filepath, target_addr):
    with open(filepath, 'rb') as f:
        elf_data = f.read()
    
    if elf_data[:4] != b'\x7fELF':
        return None
    
    # Read ELF header
    # e_type is at 16, e_machine at 18, e_version at 20, e_entry at 24 (64-bit)
    # e_phoff at 32, e_shoff at 40, e_flags at 48, e_ehsize at 52, e_phentsize at 54, e_phnum at 56, e_shentsize at 58, e_shnum at 60, e_shstrndx at 62
    shoff = struct.unpack_from('<Q', elf_data, 40)[0]
    shentsize = struct.unpack_from('<H', elf_data, 58)[0]
    shnum = struct.unpack_from('<H', elf_data, 60)[0]
    shstrndx = struct.unpack_from('<H', elf_data, 62)[0]
    
    # Read Section Headers to find Symbol Table
    symtab_sh = None
    strtab_sh = None
    shstrtab_offset = struct.unpack_from('<Q', elf_data, shoff + shstrndx * shentsize + 24)[0]
    
    for i in range(shnum):
        sh_offset = shoff + i * shentsize
        sh_type = struct.unpack_from('<I', elf_data, sh_offset + 4)[0]
        # SHT_SYMTAB = 2, SHT_STRTAB = 3
        if sh_type == 2:
            symtab_sh = sh_offset
        elif sh_type == 3:
            # We want the string table for symbols, not the section header string table
            name_offset = struct.unpack_from('<I', elf_data, sh_offset)[0]
            name = ""
            curr = shstrtab_offset + name_offset
            while elf_data[curr] != 0:
                name += chr(elf_data[curr])
                curr += 1
            if name == ".strtab":
                strtab_sh = sh_offset

    if not symtab_sh or not strtab_sh:
        return None
        
    sym_offset = struct.unpack_from('<Q', elf_data, symtab_sh + 24)[0]
    sym_size = struct.unpack_from('<Q', elf_data, symtab_sh + 32)[0]
    sym_entsize = struct.unpack_from('<Q', elf_data, symtab_sh + 56)[0]
    
    str_offset = struct.unpack_from('<Q', elf_data, strtab_sh + 24)[0]
    
    # Enumerate symbols
    symbols = []
    for i in range(0, sym_size, sym_entsize):
        offset = sym_offset + i
        if offset + 24 > len(elf_data):
            break
        st_name = struct.unpack_from('<I', elf_data, offset)[0]
        st_info = elf_data[offset + 4]
        st_shndx = struct.unpack_from('<H', elf_data, offset + 6)[0]
        st_value = struct.unpack_from('<Q', elf_data, offset + 8)[0]
        st_size = struct.unpack_from('<Q', elf_data, offset + 16)[0]
        
        # Read name
        name = ""
        curr = str_offset + st_name
        while curr < len(elf_data) and elf_data[curr] != 0:
            name += chr(elf_data[curr])
            curr += 1
            
        symbols.append((name, st_value, st_size))
        
    # Sort symbols by value
    symbols.sort(key=lambda x: x[1])
    
    # Find matching symbol
    best_sym = None
    for sym in symbols:
        name, val, size = sym
        if val <= target_addr < (val + size if size > 0 else val + 1):
            best_sym = sym
            break
            
    # If not found inside any symbol's sized range, find the closest one preceding it
    if not best_sym:
        preceding = [s for s in symbols if s[1] <= target_addr]
        if preceding:
            best_sym = preceding[-1]
            
    return best_sym

def print_bytes_at_addr(filepath, target_addr):
    with open(filepath, 'rb') as f:
        elf_data = f.read()
    
    shoff = struct.unpack_from('<Q', elf_data, 40)[0]
    shentsize = struct.unpack_from('<H', elf_data, 58)[0]
    shnum = struct.unpack_from('<H', elf_data, 60)[0]
    
    for i in range(shnum):
        sh_offset = shoff + i * shentsize
        sh_addr = struct.unpack_from('<Q', elf_data, sh_offset + 16)[0]
        sh_size = struct.unpack_from('<Q', elf_data, sh_offset + 32)[0]
        sh_file_offset = struct.unpack_from('<Q', elf_data, sh_offset + 24)[0]
        
        if sh_addr <= target_addr < sh_addr + sh_size:
            offset_in_section = target_addr - sh_addr
            file_offset = sh_file_offset + offset_in_section
            print(f"File offset: {file_offset} (0x{file_offset:x})")
            bytes_around = elf_data[file_offset - 16 : file_offset + 16]
            hex_str = " ".join(f"{b:02x}" for b in bytes_around)
            print(f"Bytes around: {hex_str}")
            print("Hex bytes before: " + " ".join(f"{b:02x}" for b in bytes_around[:16]))
            print("Hex bytes after:  " + " ".join(f"{b:02x}" for b in bytes_around[16:]))
            break

target_addresses = [0xffffffff80027e9f]
filepath = 'zig-out/bin/vanta'
for addr in target_addresses:
    res = find_address_in_elf(filepath, addr)
    if res:
        print(f"{filepath}: 0x{addr:x} -> {res[0]} (start: 0x{res[1]:x}, size: {res[2]} bytes)")
        print_bytes_at_addr(filepath, addr)
