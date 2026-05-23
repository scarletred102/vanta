import sys
import struct

def find_symbol(elf_path, target_vaddr):
    print(f"Searching symbol for VAddr: {hex(target_vaddr)} in {elf_path}")
    with open(elf_path, "rb") as f:
        elf_header = f.read(64)
        if elf_header[:4] != b"\x7fELF":
            print("Not a valid ELF file")
            return
        
        # Read section headers
        sh_off, = struct.unpack("<Q", elf_header[40:48])
        sh_num = struct.unpack("<H", elf_header[60:62])[0]
        sh_size = struct.unpack("<H", elf_header[58:60])[0]
        sh_strndx = struct.unpack("<H", elf_header[62:64])[0]
        
        # Read section string table header
        f.seek(sh_off + sh_strndx * sh_size)
        sh_str_data = f.read(sh_size)
        sh_str_offset, sh_str_size = struct.unpack("<QQ", sh_str_data[24:40])
        
        # Read section string table
        f.seek(sh_str_offset)
        sh_str_tab = f.read(sh_str_size)
        
        # Find SYMTAB and STRTAB sections
        symtab_off = 0
        symtab_sz = 0
        symtab_entsz = 0
        strtab_off = 0
        strtab_sz = 0
        
        f.seek(sh_off)
        for i in range(sh_num):
            sh_data = f.read(sh_size)
            sh_name_idx = struct.unpack("<I", sh_data[:4])[0]
            sh_type = struct.unpack("<I", sh_data[4:8])[0]
            sh_offset, sh_size_val = struct.unpack("<QQ", sh_data[24:40])
            sh_entsize = struct.unpack("<Q", sh_data[56:64])[0]
            
            name = sh_str_tab[sh_name_idx:].split(b"\x00")[0].decode()
            if sh_type == 2: # SHT_SYMTAB
                symtab_off = sh_offset
                symtab_sz = sh_size_val
                symtab_entsz = sh_entsize
            elif sh_type == 3 and name == ".strtab": # SHT_STRTAB
                strtab_off = sh_offset
                strtab_sz = sh_size_val
        
        if not symtab_off or not strtab_off:
            print("Symbol table not found")
            return
        
        # Read string table
        f.seek(strtab_off)
        str_tab = f.read(strtab_sz)
        
        # Read symbol table entries
        f.seek(symtab_off)
        num_syms = symtab_sz // symtab_entsz
        best_name = "unknown"
        best_diff = 0xFFFFFFFFFFFFFFFF
        best_start = 0
        
        for idx in range(num_syms):
            sym_data = f.read(symtab_entsz)
            st_name = struct.unpack("<I", sym_data[:4])[0]
            st_info = sym_data[4]
            st_value, st_size = struct.unpack("<QQ", sym_data[8:24])
            
            name = str_tab[st_name:].split(b"\x00")[0].decode()
            if st_value <= target_vaddr and target_vaddr < st_value + st_size:
                print(f"EXACT MATCH: {name} (Value: {hex(st_value)}, Size: {st_size})")
                return
            elif st_value <= target_vaddr and st_value > 0:
                diff = target_vaddr - st_value
                if diff < best_diff:
                    best_diff = diff
                    best_name = name
                    best_start = st_value
                    
        print(f"CLOSEST SYMBOL: {best_name} (Start: {hex(best_start)}, Offset: +{hex(best_diff)})")

if __name__ == "__main__":
    find_symbol("zig-out/bin/vanta", 0xFFFFFFFF80028c40)
