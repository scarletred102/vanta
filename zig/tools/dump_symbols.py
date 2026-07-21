import sys
import struct

def dump_symbols(elf_path):
    with open(elf_path, 'rb') as f:
        data = f.read()
    
    if data[:4] != b'\x7fELF':
        print("Not an ELF file")
        return
    e_shoff = struct.unpack_from('<Q', data, 40)[0]
    e_shentsize = struct.unpack_from('<H', data, 58)[0]
    e_shnum = struct.unpack_from('<H', data, 60)[0]
    e_shstrndx = struct.unpack_from('<H', data, 62)[0]
    
    sections = []
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        sh_name, sh_type, sh_flags, sh_addr, sh_offset, sh_size, sh_link = struct.unpack_from('<IIQQQQI', data, off)[:7]
        sections.append({
            'name_off': sh_name, 'type': sh_type, 'addr': sh_addr, 'offset': sh_offset, 'size': sh_size, 'link': sh_link
        })
        
    shstr_off = sections[e_shstrndx]['offset']
    for s in sections:
        name_bytes = []
        o = shstr_off + s['name_off']
        while o < len(data) and data[o] != 0:
            name_bytes.append(chr(data[o]))
            o += 1
        s['name'] = "".join(name_bytes)
        
    symtab = next((s for s in sections if s['type'] == 2), None)
    if not symtab:
        print("No symtab found")
        return
        
    strtab = sections[symtab['link']]
    strtab_offset = strtab['offset']
    
    sym_offset = symtab['offset']
    sym_size = symtab['size']
    num_syms = sym_size // 24
    
    symbols = []
    for i in range(num_syms):
        o = sym_offset + i * 24
        st_name = struct.unpack_from('<I', data, o)[0]
        st_value = struct.unpack_from('<Q', data, o + 8)[0]
        
        name_bytes = []
        no = strtab_offset + st_name
        while no < len(data) and data[no] != 0:
            name_bytes.append(chr(data[no]))
            no += 1
        name = "".join(name_bytes)
        
        symbols.append((st_value, name))
        
    symbols.sort()
    for val, name in symbols:
        if val > 0:
            print(f"{val:016x} {name}")

if __name__ == '__main__':
    dump_symbols('zig-out/bin/vanta')
