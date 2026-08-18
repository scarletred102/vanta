use vanta_abi::Syscall;
use vanta_linuxd::{
    broker, is_static_elf_supported, translate, BrokerDecision, ElfError, LinuxOp,
    LinuxSyscallRequest, StaticElf, UnsupportedSyscall,
};

fn make_base_elf(elf_type: u16, phnum: u16, phentsize: u16) -> Vec<u8> {
    let mut image = vec![0u8; 64 + (phnum as usize) * (phentsize as usize) + 256];
    image[..4].copy_from_slice(b"\x7fELF");
    image[4] = 2; // ELFCLASS64
    image[5] = 1; // ELFDATA2LSB
    image[6] = 1; // EV_CURRENT
    image[16..18].copy_from_slice(&elf_type.to_le_bytes());
    image[18..20].copy_from_slice(&0x3eu16.to_le_bytes()); // EM_X86_64
    image[24..32].copy_from_slice(&0x401000u64.to_le_bytes()); // entry
    image[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
    image[52..54].copy_from_slice(&64u16.to_le_bytes()); // ehsize
    image[54..56].copy_from_slice(&phentsize.to_le_bytes());
    image[56..58].copy_from_slice(&phnum.to_le_bytes());
    image
}

fn write_phdr(
    image: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
) {
    let phentsize = 56;
    let base = 64 + index * phentsize;
    image[base..base + 4].copy_from_slice(&kind.to_le_bytes());
    image[base + 4..base + 8].copy_from_slice(&flags.to_le_bytes());
    image[base + 8..base + 16].copy_from_slice(&offset.to_le_bytes());
    image[base + 16..base + 24].copy_from_slice(&vaddr.to_le_bytes());
    image[base + 24..base + 32].copy_from_slice(&vaddr.to_le_bytes()); // paddr
    image[base + 32..base + 40].copy_from_slice(&filesz.to_le_bytes());
    image[base + 40..base + 48].copy_from_slice(&memsz.to_le_bytes());
    image[base + 48..base + 56].copy_from_slice(&align.to_le_bytes());
}

#[test]
fn test_valid_static_exec_elf() {
    let mut image = make_base_elf(2 /* ET_EXEC */, 1, 56);
    write_phdr(&mut image, 0, 1 /* PT_LOAD */, 5 /* R-X */, 0, 0x400000, 100, 100, 0x1000);
    let elf = StaticElf::parse(&image).expect("Valid static ELF must parse successfully");
    assert_eq!(elf.entry, 0x401000);
    assert_eq!(elf.segment_count, 1);
    assert_eq!(elf.interpreter_offset, None);
    assert_eq!(elf.interpreter_size, None);
    assert_eq!(elf.interpreter(&image), None);
    assert!(is_static_elf_supported(None));
}

#[test]
fn test_valid_dynamic_pie_elf_with_pt_interp_null_terminated() {
    let mut image = make_base_elf(3 /* ET_DYN */, 2, 56);
    let interp_path = b"/lib/ld-musl-x86_64.so.1\0";
    let interp_offset = 200u64;
    image[interp_offset as usize..interp_offset as usize + interp_path.len()]
        .copy_from_slice(interp_path);

    // PT_INTERP
    write_phdr(&mut image, 0, 3 /* PT_INTERP */, 4 /* R */, interp_offset, 0x200, interp_path.len() as u64, interp_path.len() as u64, 1);
    // PT_LOAD
    write_phdr(&mut image, 1, 1 /* PT_LOAD */, 5 /* R-X */, 0, 0x400000, 100, 100, 0x1000);

    let elf = StaticElf::parse(&image).expect("Valid dynamic ELF must parse");
    assert_eq!(elf.segment_count, 1);
    assert_eq!(elf.interpreter_offset, Some(interp_offset));
    assert_eq!(elf.interpreter_size, Some(interp_path.len() as u64));
    assert_eq!(elf.interpreter(&image), Some("/lib/ld-musl-x86_64.so.1"));
}

#[test]
fn test_valid_dynamic_elf_with_pt_interp_non_null_terminated() {
    let mut image = make_base_elf(3 /* ET_DYN */, 2, 56);
    let interp_path = b"/compat/linux/lib/ld-linux-x86-64.so.2";
    let interp_offset = 200u64;
    image[interp_offset as usize..interp_offset as usize + interp_path.len()]
        .copy_from_slice(interp_path);

    // PT_INTERP without trailing null
    write_phdr(&mut image, 0, 3 /* PT_INTERP */, 4, interp_offset, 0x200, interp_path.len() as u64, interp_path.len() as u64, 1);
    // PT_LOAD
    write_phdr(&mut image, 1, 1 /* PT_LOAD */, 5, 0, 0x400000, 100, 100, 0x1000);

    let elf = StaticElf::parse(&image).expect("Valid dynamic ELF without null terminator must parse");
    assert_eq!(elf.interpreter(&image), Some("/compat/linux/lib/ld-linux-x86-64.so.2"));
}

#[test]
fn test_rejects_truncated_elf_headers() {
    assert_eq!(StaticElf::parse(&[]), Err(ElfError::NotX86_64));
    assert_eq!(StaticElf::parse(&[0x7f, b'E', b'L', b'F']), Err(ElfError::NotX86_64));
    let mut truncated = make_base_elf(2, 1, 56);
    truncated.truncate(63);
    assert_eq!(StaticElf::parse(&truncated), Err(ElfError::NotX86_64));
}

#[test]
fn test_rejects_corrupted_elf_magics_and_classes() {
    let mut image = make_base_elf(2, 1, 56);
    write_phdr(&mut image, 0, 1, 5, 0, 0x400000, 50, 50, 0x1000);

    // Bad magic
    image[0] = 0x00;
    assert_eq!(StaticElf::parse(&image), Err(ElfError::NotX86_64));
    image[0] = 0x7f;

    // Bad class (32-bit instead of 64-bit)
    image[4] = 1;
    assert_eq!(StaticElf::parse(&image), Err(ElfError::NotX86_64));
    image[4] = 2;

    // Bad endianness (Big-endian instead of Little-endian)
    image[5] = 2;
    assert_eq!(StaticElf::parse(&image), Err(ElfError::NotX86_64));
    image[5] = 1;

    // Unsupported ELF type (ET_REL = 1, ET_CORE = 4)
    image[16..18].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(StaticElf::parse(&image), Err(ElfError::UnsupportedType));
    image[16..18].copy_from_slice(&4u16.to_le_bytes());
    assert_eq!(StaticElf::parse(&image), Err(ElfError::UnsupportedType));
}

#[test]
fn test_rejects_malformed_program_headers_table() {
    // phentsize < 56
    let image_small_phent = make_base_elf(2, 1, 40);
    assert_eq!(StaticElf::parse(&image_small_phent), Err(ElfError::InvalidProgramTable));

    // phnum > 16
    let image_too_many_phdrs = make_base_elf(2, 17, 56);
    assert_eq!(StaticElf::parse(&image_too_many_phdrs), Err(ElfError::InvalidProgramTable));

    // phoff beyond EOF
    let mut image_phoff_oob = make_base_elf(2, 1, 56);
    image_phoff_oob[32..40].copy_from_slice(&999999u64.to_le_bytes());
    assert_eq!(StaticElf::parse(&image_phoff_oob), Err(ElfError::InvalidProgramTable));

    // phoff arithmetic overflow
    let mut image_overflow = make_base_elf(2, 2, 56);
    image_overflow[32..40].copy_from_slice(&(u64::MAX - 10).to_le_bytes());
    assert_eq!(StaticElf::parse(&image_overflow), Err(ElfError::InvalidProgramTable));
}

#[test]
fn test_rejects_out_of_bounds_pt_load_and_pt_interp() {
    // PT_LOAD file_offset + file_size > image length
    let mut image1 = make_base_elf(2, 1, 56);
    write_phdr(&mut image1, 0, 1, 5, 200, 0x400000, 5000, 5000, 0x1000);
    assert_eq!(StaticElf::parse(&image1), Err(ElfError::InvalidProgramTable));

    // PT_LOAD memory_size < file_size
    let mut image2 = make_base_elf(2, 1, 56);
    write_phdr(&mut image2, 0, 1, 5, 0, 0x400000, 100, 50, 0x1000);
    assert_eq!(StaticElf::parse(&image2), Err(ElfError::InvalidProgramTable));

    // PT_INTERP file_offset + file_size > image length
    let mut image3 = make_base_elf(3, 2, 56);
    write_phdr(&mut image3, 0, 3, 4, 100, 0x200, 5000, 5000, 1);
    write_phdr(&mut image3, 1, 1, 5, 0, 0x400000, 50, 50, 0x1000);
    assert_eq!(StaticElf::parse(&image3), Err(ElfError::InvalidProgramTable));

    // PT_INTERP integer overflow in file_offset + file_size
    let mut image4 = make_base_elf(3, 2, 56);
    write_phdr(&mut image4, 0, 3, 4, u64::MAX - 5, 0x200, 100, 100, 1);
    write_phdr(&mut image4, 1, 1, 5, 0, 0x400000, 50, 50, 0x1000);
    assert_eq!(StaticElf::parse(&image4), Err(ElfError::InvalidProgramTable));
}

#[test]
fn test_rejects_elf_without_load_segments() {
    let mut image = make_base_elf(2, 1, 56);
    // Phdr is PT_NOTE (kind = 4), not PT_LOAD
    write_phdr(&mut image, 0, 4, 4, 0, 0, 10, 10, 1);
    assert_eq!(StaticElf::parse(&image), Err(ElfError::NoLoadSegments));
}

#[test]
fn test_invalid_utf8_interpreter_returns_none() {
    let mut image = make_base_elf(3, 2, 56);
    let invalid_utf8 = [0xff, 0xfe, 0xfd, 0x00];
    let interp_offset = 200u64;
    image[interp_offset as usize..interp_offset as usize + invalid_utf8.len()]
        .copy_from_slice(&invalid_utf8);

    write_phdr(&mut image, 0, 3, 4, interp_offset, 0x200, invalid_utf8.len() as u64, invalid_utf8.len() as u64, 1);
    write_phdr(&mut image, 1, 1, 5, 0, 0x400000, 50, 50, 0x1000);

    let elf = StaticElf::parse(&image).expect("Parses even if string is invalid utf8");
    assert_eq!(elf.interpreter(&image), None);
}

#[test]
fn test_milestone1_syscall_translations() {
    // mmap (9) -> LinuxOp::MMap, native Some(Syscall::MMap)
    let mmap_tr = translate(9).unwrap();
    assert_eq!(mmap_tr.operation, LinuxOp::MMap);
    assert_eq!(mmap_tr.native, Some(Syscall::MMap));

    // mprotect (10) -> LinuxOp::MProtect, native None (process primitive)
    let mprotect_tr = translate(10).unwrap();
    assert_eq!(mprotect_tr.operation, LinuxOp::MProtect);
    assert_eq!(mprotect_tr.native, None);

    // munmap (11) -> LinuxOp::MUnmap, native Some(Syscall::MUnmap)
    let munmap_tr = translate(11).unwrap();
    assert_eq!(munmap_tr.operation, LinuxOp::MUnmap);
    assert_eq!(munmap_tr.native, Some(Syscall::MUnmap));

    // brk (12) -> LinuxOp::Brk, native Some(Syscall::Brk)
    let brk_tr = translate(12).unwrap();
    assert_eq!(brk_tr.operation, LinuxOp::Brk);
    assert_eq!(brk_tr.native, Some(Syscall::Brk));
}

#[test]
fn test_broker_routing_for_memory_operations() {
    // Test mmap request brokering
    let mmap_req = LinuxSyscallRequest {
        number: 9,
        args: [0x400000, 4096, 3, 0x22, u64::MAX, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(mmap_req),
        BrokerDecision::Native {
            syscall: Syscall::MMap,
            args: [0x400000, 4096, 3, 0x22],
        }
    );

    // Test mprotect request brokering
    let mprotect_req = LinuxSyscallRequest {
        number: 10,
        args: [0x400000, 8192, 1, 0, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(mprotect_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::MProtect,
        }
    );

    // Test unsupported syscall numbers
    assert_eq!(translate(99999), Err(UnsupportedSyscall { number: 99999 }));
    let unsupp_req = LinuxSyscallRequest {
        number: 99999,
        args: [0; 6],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(broker(unsupp_req), BrokerDecision::Unsupported { number: 99999 });
}

#[test]
fn test_inspect_dynamic_samples() {
    let bytes = std::fs::read("../target/compat/linux/dynamic-threads").unwrap();
    let parsed = StaticElf::parse(&bytes).unwrap();
    println!("dynamic-threads: entry={:#x} segments={:?}", parsed.entry, parsed.segments);
    // Find offset for virtual address 0x1002e50
    // LoadSegment 2: virtual_address = 0x1001000.. or similar
    for seg in parsed.segments.iter().flatten() {
        if 0x1002e50 >= seg.virtual_address && 0x1002e50 < seg.virtual_address + seg.memory_size {
            let file_off = (0x1002e40 - seg.virtual_address + seg.file_offset) as usize;
            println!("bytes at 0x1002e40: {:02x?}", &bytes[file_off..file_off + 48]);
        }
    }
}

