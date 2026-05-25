import os
import struct

target_addr = 0x40073e
bin_dir = 'kernel/bin'

def inspect_elf_bytes(filepath, addr):
    with open(filepath, 'rb') as f:
        elf_data = f.read()
    
    if elf_data[:4] != b'\x7fELF':
        return
        
    # Find segment containing addr
    phoff = struct.unpack_from('<Q', elf_data, 32)[0]
    phentsize = struct.unpack_from('<H', elf_data, 54)[0]
    phnum = struct.unpack_from('<H', elf_data, 56)[0]
    
    for i in range(phnum):
        ph_offset = phoff + i * phentsize
        p_type = struct.unpack_from('<I', elf_data, ph_offset)[0]
        # PT_LOAD = 1
        if p_type == 1:
            p_offset = struct.unpack_from('<Q', elf_data, ph_offset + 8)[0]
            p_vaddr = struct.unpack_from('<Q', elf_data, ph_offset + 16)[0]
            p_filesz = struct.unpack_from('<Q', elf_data, ph_offset + 32)[0]
            
            if p_vaddr <= addr < p_vaddr + p_filesz:
                file_offset = p_offset + (addr - p_vaddr)
                bytes_at = elf_data[file_offset : file_offset + 16]
                hex_bytes = " ".join(f"{b:02x}" for b in bytes_at)
                print(f"{os.path.basename(filepath)}: 0x{addr:x} -> bytes: {hex_bytes}")
                return

for filename in os.listdir(bin_dir):
    filepath = os.path.join(bin_dir, filename)
    if os.path.isfile(filepath):
        inspect_elf_bytes(filepath, target_addr)
