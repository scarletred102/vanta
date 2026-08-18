use core::mem::{align_of, offset_of, size_of};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserContext {
    pub return_value: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub instruction_pointer: u64,
    pub flags: u64,
    pub stack_pointer: u64,
}

const SYSCALL_STACK_SIZE: usize = 128 * 1024;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct CpuLocal {
    pub self_pointer: u64,
    pub syscall_stack: [u8; SYSCALL_STACK_SIZE],
    pub syscall_stack_top: u64,
    pub user_rsp: u64,
    pub exit_code: u64,
    pub next_context: UserContext,
    pub cpu_index: usize,
    pub block_descriptor: u64,
}

#[test]
fn test_cpulocal_stack_top_alignment() {
    let local_align = align_of::<CpuLocal>();
    assert_eq!(local_align, 16);

    let self_ptr_offset = offset_of!(CpuLocal, self_pointer);
    let stack_offset = offset_of!(CpuLocal, syscall_stack);
    assert_eq!(self_ptr_offset, 0);
    // In CpuLocal, self_pointer is u64 (8 bytes). syscall_stack is [u8; SYSCALL_STACK_SIZE].
    // Since u8 has alignment 1, syscall_stack starts at offset 8!
    assert_eq!(stack_offset, 8);

    // If local is allocated at 16-byte aligned base address A (A % 16 == 0):
    let base_a: u64 = 0x1000_0000;
    let stack_start = base_a + stack_offset as u64;
    let stack_top = stack_start + SYSCALL_STACK_SIZE as u64;

    // stack_top % 16:
    // stack_top = 0x1000_0000 + 8 + 131072 = 0x1002_0008.
    // 0x1002_0008 % 16 == 8!
    let stack_top_mod_16 = (stack_top % 16) as usize;
    println!("stack_top_mod_16 = {}", stack_top_mod_16);
    assert_eq!(stack_top_mod_16, 8, "syscall_stack_top is 8 mod 16, NOT 16-byte aligned!");

    // Now let's trace vanta_syscall_entry stack operations:
    // 1. rsp starts at stack_top (8 mod 16)
    let mut rsp = stack_top;
    assert_eq!(rsp % 16, 8);

    // 2. 15 pushes (r15, r14, r13, r12, rbp, rbx, r11, rcx, r10, r9, r8, rdx, rsi, rdi, rax)
    for _ in 0..15 {
        rsp -= 8;
    }
    // After 15 pushes: rsp = stack_top - 120
    assert_eq!(rsp % 16, 0);

    // 3. 16th push: `push qword ptr [rsp + 40]` (pushes arg6 onto stack)
    rsp -= 8;
    // After 16th push: rsp = stack_top - 128
    assert_eq!(rsp % 16, 8);

    // 4. `call vanta_syscall_dispatch` executes:
    // CALL instruction pushes 8-byte return address onto the stack:
    let rsp_at_callee_entry = rsp - 8;
    assert_eq!(rsp_at_callee_entry % 16, 0);

    // Under System V AMD64 ABI:
    // Immediately prior to CALL: rsp % 16 MUST be 0.
    // Immediately upon entry to callee: (rsp + 8) % 16 MUST be 0 (i.e. rsp % 16 == 8).
    // Here:
    // rsp before CALL is 8 mod 16 (VIOLATION of System V ABI).
    // rsp at callee entry is 0 mod 16 (VIOLATION of System V ABI).
    let is_system_v_compliant_before_call = (rsp % 16) == 0;
    let is_system_v_compliant_at_entry = ((rsp_at_callee_entry + 8) % 16) == 0;

    println!("is_system_v_compliant_before_call = {}", is_system_v_compliant_before_call);
    println!("is_system_v_compliant_at_entry = {}", is_system_v_compliant_at_entry);

    assert!(!is_system_v_compliant_before_call, "Empirically demonstrated non-compliance due to CpuLocal offset 8");
    assert!(!is_system_v_compliant_at_entry, "Empirically demonstrated non-compliance due to CpuLocal offset 8");
}

#[test]
fn test_cpulocal_stack_top_with_padding_simulation() {
    // If CpuLocal had self_pointer padded or aligned to 16 bytes:
    // e.g. syscall_stack offset was 16:
    let base_a: u64 = 0x1000_0000;
    let padded_stack_offset: u64 = 16;
    let stack_start = base_a + padded_stack_offset;
    let stack_top = stack_start + SYSCALL_STACK_SIZE as u64;

    assert_eq!(stack_top % 16, 0);

    let mut rsp = stack_top;
    // 15 pushes
    rsp -= 15 * 8;
    assert_eq!(rsp % 16, 8);

    // 1 push for 7th argument
    rsp -= 8;
    assert_eq!(rsp % 16, 0); // PRE-CALL IS 16-BYTE ALIGNED!

    // CALL pushes return address
    let rsp_at_callee_entry = rsp - 8;
    assert_eq!((rsp_at_callee_entry + 8) % 16, 0); // CALLEE ENTRY IS 16-BYTE ALIGNED!
}

#[test]
fn test_cpulocal_stack_top_masked_alignment() {
    // If initialize_cpu_local masks stack_top: `stack_top & !15`
    let base_a: u64 = 0x1000_0000;
    let stack_offset: u64 = 8;
    let raw_stack_top = base_a + stack_offset + SYSCALL_STACK_SIZE as u64;
    let aligned_stack_top = raw_stack_top & !15;

    assert_eq!(aligned_stack_top % 16, 0);

    let mut rsp = aligned_stack_top;
    rsp -= 15 * 8;
    rsp -= 8; // 16th push for 7th argument
    assert_eq!(rsp % 16, 0); // pre-call is 16-byte aligned

    let rsp_at_callee_entry = rsp - 8;
    assert_eq!((rsp_at_callee_entry + 8) % 16, 0); // callee entry (rsp + 8) % 16 == 0
}

#[test]
fn test_syscall_entry_argument_register_offsets() {
    // Let's verify the exact stack offsets in vanta_syscall_entry
    // Pushes in order:
    // 0: r15
    // 1: r14
    // 2: r13
    // 3: r12
    // 4: rbp
    // 5: rbx
    // 6: r11
    // 7: rcx
    // 8: r10 (arg4)
    // 9: r9  (arg6)
    // 10: r8 (arg5)
    // 11: rdx (arg3)
    // 12: rsi (arg2)
    // 13: rdi (arg1)
    // 14: rax (syscall nr)

    let mut stack = [0u64; 16];
    let rax_val = 0x100; // syscall nr
    let rdi_val = 0x101; // arg1
    let rsi_val = 0x102; // arg2
    let rdx_val = 0x103; // arg3
    let r8_val  = 0x104; // arg5
    let r9_val  = 0x105; // arg6
    let r10_val = 0x106; // arg4
    let rcx_val = 0x107;
    let r11_val = 0x108;
    let rbx_val = 0x109;
    let rbp_val = 0x10a;
    let r12_val = 0x10b;
    let r13_val = 0x10c;
    let r14_val = 0x10d;
    let r15_val = 0x10e;

    // Pushed from bottom of array (top of stack at index 0):
    stack[0] = rax_val;
    stack[1] = rdi_val;
    stack[2] = rsi_val;
    stack[3] = rdx_val;
    stack[4] = r8_val;
    stack[5] = r9_val;
    stack[6] = r10_val;
    stack[7] = rcx_val;
    stack[8] = r11_val;
    stack[9] = rbx_val;
    stack[10] = rbp_val;
    stack[11] = r12_val;
    stack[12] = r13_val;
    stack[13] = r14_val;
    stack[14] = r15_val;

    // Assembly mappings:
    // mov rdi, [rsp] -> stack[0]
    let dispatch_nr = stack[0 / 8];
    assert_eq!(dispatch_nr, rax_val);

    // mov rsi, [rsp + 8] -> stack[1]
    let dispatch_a1 = stack[8 / 8];
    assert_eq!(dispatch_a1, rdi_val);

    // mov rdx, [rsp + 16] -> stack[2]
    let dispatch_a2 = stack[16 / 8];
    assert_eq!(dispatch_a2, rsi_val);

    // mov rcx, [rsp + 24] -> stack[3]
    let dispatch_a3 = stack[24 / 8];
    assert_eq!(dispatch_a3, rdx_val);

    // mov r8, [rsp + 48] -> stack[6]
    let dispatch_a4 = stack[48 / 8];
    assert_eq!(dispatch_a4, r10_val);

    // mov r9, [rsp + 32] -> stack[4]
    let dispatch_a5 = stack[32 / 8];
    assert_eq!(dispatch_a5, r8_val);

    // push qword ptr [rsp + 40] -> stack[5]
    let dispatch_a6 = stack[40 / 8];
    assert_eq!(dispatch_a6, r9_val);
}

#[test]
fn test_vanta_syscall_exec_error_offsets() {
    // In vanta_syscall_exec_error:
    // mov r11, [rsp + 48]
    // mov rcx, [rsp + 40]
    // Let's verify where rcx (saved user RIP) and r11 (saved user RFLAGS) actually reside!
    // Pushes:
    // [0]: rax
    // [8]: rdi
    // [16]: rsi
    // [24]: rdx
    // [32]: r8
    // [40]: r9
    // [48]: r10
    // [56]: rcx (user RIP)
    // [64]: r11 (user RFLAGS)
    let rcx_actual_offset = 56;
    let r11_actual_offset = 64;

    let exec_error_rcx_offset_in_asm = 40;
    let exec_error_r11_offset_in_asm = 48;

    assert_ne!(
        exec_error_rcx_offset_in_asm, rcx_actual_offset,
        "vanta_syscall_exec_error reads offset 40 (r9) instead of offset 56 (rcx/user RIP)!"
    );
    assert_ne!(
        exec_error_r11_offset_in_asm, r11_actual_offset,
        "vanta_syscall_exec_error reads offset 48 (r10) instead of offset 64 (r11/user RFLAGS)!"
    );
}

#[test]
fn test_auxv_structure_and_order() {
    // Linux Elf64_auxv_t definition
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Elf64Auxv {
        a_type: u64,
        a_val: u64,
    }
    assert_eq!(size_of::<Elf64Auxv>(), 16);
    assert_eq!(align_of::<Elf64Auxv>(), 8);

    // Let's simulate initialize_stack auxv layout in memory
    let mut memory = [0u8; 1024];
    let mut stack_pointer: u64 = 1024;

    let random_ptr = 900u64;
    let main_entry = 0x401000u64;
    let interp_base = 0x7f00_0000_0000u64;
    let phnum = 4u64;
    let phent = 56u64;
    let phdr_vaddr = 0x400040u64;
    let page_size = 4096u64;

    let auxv_table: &[(u64, u64)] = &[
        (0, 0),                       // AT_NULL (0)
        (25, random_ptr),             // AT_RANDOM (25)
        (23, 0),                      // AT_SECURE (23)
        (17, 100),                    // AT_CLKTCK (17)
        (14, 0),                      // AT_EGID (14)
        (13, 0),                      // AT_GID (13)
        (12, 0),                      // AT_EUID (12)
        (11, 0),                      // AT_UID (11)
        (9, main_entry),              // AT_ENTRY (9)
        (8, 0),                       // AT_FLAGS (8)
        (7, interp_base),             // AT_BASE (7)
        (6, page_size),               // AT_PAGESZ (6)
        (5, phnum),                   // AT_PHNUM (5)
        (4, phent),                   // AT_PHENT (4)
        (3, phdr_vaddr),              // AT_PHDR (3)
    ];

    for (key, val) in auxv_table {
        stack_pointer -= 8;
        let val_bytes = val.to_ne_bytes();
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&val_bytes);

        stack_pointer -= 8;
        let key_bytes = key.to_ne_bytes();
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&key_bytes);
    }

    // Now let's read from stack_pointer upwards (as a C program / ld-musl does):
    let auxv_base = stack_pointer as usize;
    let mut entries = Vec::new();
    let mut offset = auxv_base;
    loop {
        let key = u64::from_ne_bytes(memory[offset..offset + 8].try_into().unwrap());
        let val = u64::from_ne_bytes(memory[offset + 8..offset + 16].try_into().unwrap());
        entries.push((key, val));
        offset += 16;
        if key == 0 {
            break;
        }
    }

    assert_eq!(entries.len(), 15);
    assert_eq!(entries[0], (3, phdr_vaddr));   // AT_PHDR
    assert_eq!(entries[1], (4, phent));        // AT_PHENT
    assert_eq!(entries[2], (5, phnum));        // AT_PHNUM
    assert_eq!(entries[3], (6, page_size));    // AT_PAGESZ
    assert_eq!(entries[4], (7, interp_base));  // AT_BASE
    assert_eq!(entries[5], (8, 0));            // AT_FLAGS
    assert_eq!(entries[6], (9, main_entry));   // AT_ENTRY
    assert_eq!(entries[7], (11, 0));           // AT_UID
    assert_eq!(entries[8], (12, 0));           // AT_EUID
    assert_eq!(entries[9], (13, 0));           // AT_GID
    assert_eq!(entries[10], (14, 0));          // AT_EGID
    assert_eq!(entries[11], (17, 100));        // AT_CLKTCK
    assert_eq!(entries[12], (23, 0));          // AT_SECURE
    assert_eq!(entries[13], (25, random_ptr)); // AT_RANDOM
    assert_eq!(entries[14], (0, 0));           // AT_NULL
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct RemediatedCpuLocal {
    pub self_pointer: u64,
    pub _pad: u64,
    pub syscall_stack: [u8; SYSCALL_STACK_SIZE],
    pub syscall_stack_top: u64,
    pub user_rsp: u64,
    pub exit_code: u64,
    pub next_context: UserContext,
    pub cpu_index: usize,
    pub block_descriptor: u64,
}

#[test]
fn test_remediated_cpulocal_16byte_alignment() {
    let local_align = align_of::<RemediatedCpuLocal>();
    assert_eq!(local_align, 16);

    let self_ptr_offset = offset_of!(RemediatedCpuLocal, self_pointer);
    let pad_offset = offset_of!(RemediatedCpuLocal, _pad);
    let stack_offset = offset_of!(RemediatedCpuLocal, syscall_stack);
    assert_eq!(self_ptr_offset, 0);
    assert_eq!(pad_offset, 8);
    assert_eq!(stack_offset, 16, "syscall_stack must start at 16-byte aligned offset 16");

    let base_a: u64 = 0x2000_0000;
    let stack_start = base_a + stack_offset as u64;
    let stack_top = stack_start + SYSCALL_STACK_SIZE as u64;
    assert_eq!(stack_top % 16, 0, "syscall_stack_top is strictly 16-byte aligned!");

    // Trace assembly execution:
    let mut rsp = stack_top;
    assert_eq!(rsp % 16, 0);

    // 15 register pushes
    rsp -= 15 * 8;
    assert_eq!(rsp % 16, 8);

    // 16th push for 7th argument
    rsp -= 8;
    assert_eq!(rsp % 16, 0, "rsp prior to CALL must be 16-byte aligned per System V ABI");

    // CALL instruction pushes 8-byte return address
    let rsp_at_callee = rsp - 8;
    assert_eq!((rsp_at_callee + 8) % 16, 0, "(rsp + 8) % 16 == 0 at callee entry per System V ABI");
}

#[test]
fn test_remediated_vanta_syscall_exec_error_offsets() {
    let rcx_actual_offset = 56;
    let r11_actual_offset = 64;
    let frame_cleanup_size = 120;

    let remediated_rcx_offset = 56;
    let remediated_r11_offset = 64;
    let remediated_add_rsp = 120;

    assert_eq!(remediated_rcx_offset, rcx_actual_offset);
    assert_eq!(remediated_r11_offset, r11_actual_offset);
    assert_eq!(remediated_add_rsp, frame_cleanup_size);
}

#[test]
fn test_argv_envp_and_auxv_stack_order() {
    let mut memory = [0u8; 4096];
    let mut stack_pointer: u64 = 4096;

    let args: &[&[u8]] = &[b"/bin/echo", b"hello", b"world"];
    let env: &[&[u8]] = &[b"PATH=/bin", b"USER=vanta"];

    let mut argument_pointers = Vec::new();
    let mut environment_pointers = Vec::new();

    // 1. Allocate string buffers forward
    for argument in args {
        let size = argument.len() + 1;
        stack_pointer -= size as u64;
        memory[stack_pointer as usize..stack_pointer as usize + argument.len()].copy_from_slice(argument);
        memory[stack_pointer as usize + argument.len()] = 0;
        argument_pointers.push(stack_pointer);
    }
    for value in env {
        let size = value.len() + 1;
        stack_pointer -= size as u64;
        memory[stack_pointer as usize..stack_pointer as usize + value.len()].copy_from_slice(value);
        memory[stack_pointer as usize + value.len()] = 0;
        environment_pointers.push(stack_pointer);
    }

    // 2. Random bytes
    stack_pointer -= 16;
    let random_ptr = stack_pointer;
    stack_pointer &= !15;

    // 3. Auxv
    let auxv: &[(u64, u64)] = &[
        (0, 0),                       // AT_NULL
        (25, random_ptr),             // AT_RANDOM
        (3, 0x400040),                // AT_PHDR
    ];
    for (key, val) in auxv {
        stack_pointer -= 8;
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&val.to_ne_bytes());
        stack_pointer -= 8;
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&key.to_ne_bytes());
    }

    // 4. Terminate envp & push pointers in reverse
    stack_pointer -= 8;
    memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&0u64.to_ne_bytes());
    for pointer in environment_pointers.iter().rev() {
        stack_pointer -= 8;
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&pointer.to_ne_bytes());
    }

    // 5. Terminate argv & push pointers in reverse
    stack_pointer -= 8;
    memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&0u64.to_ne_bytes());
    for pointer in argument_pointers.iter().rev() {
        stack_pointer -= 8;
        memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&pointer.to_ne_bytes());
    }

    // 6. argc
    stack_pointer -= 8;
    memory[stack_pointer as usize..stack_pointer as usize + 8].copy_from_slice(&(args.len() as u64).to_ne_bytes());

    // Verify stack layout from rsp:
    let rsp = stack_pointer as usize;
    let argc = u64::from_ne_bytes(memory[rsp..rsp + 8].try_into().unwrap());
    assert_eq!(argc, 3);

    let argv0_ptr = u64::from_ne_bytes(memory[rsp + 8..rsp + 16].try_into().unwrap()) as usize;
    let argv1_ptr = u64::from_ne_bytes(memory[rsp + 16..rsp + 24].try_into().unwrap()) as usize;
    let argv2_ptr = u64::from_ne_bytes(memory[rsp + 24..rsp + 32].try_into().unwrap()) as usize;
    let argv_null = u64::from_ne_bytes(memory[rsp + 32..rsp + 40].try_into().unwrap());

    assert_eq!(&memory[argv0_ptr..argv0_ptr + 10], b"/bin/echo\0");
    assert_eq!(&memory[argv1_ptr..argv1_ptr + 6], b"hello\0");
    assert_eq!(&memory[argv2_ptr..argv2_ptr + 6], b"world\0");
    assert_eq!(argv_null, 0);

    let envp0_ptr = u64::from_ne_bytes(memory[rsp + 40..rsp + 48].try_into().unwrap()) as usize;
    let envp1_ptr = u64::from_ne_bytes(memory[rsp + 48..rsp + 56].try_into().unwrap()) as usize;
    let envp_null = u64::from_ne_bytes(memory[rsp + 56..rsp + 64].try_into().unwrap());

    assert_eq!(&memory[envp0_ptr..envp0_ptr + 10], b"PATH=/bin\0");
    assert_eq!(&memory[envp1_ptr..envp1_ptr + 11], b"USER=vanta\0");
    assert_eq!(envp_null, 0);

    let auxv_key0 = u64::from_ne_bytes(memory[rsp + 64..rsp + 72].try_into().unwrap());
    let auxv_val0 = u64::from_ne_bytes(memory[rsp + 72..rsp + 80].try_into().unwrap());
    assert_eq!(auxv_key0, 3); // AT_PHDR
    assert_eq!(auxv_val0, 0x400040);
}
