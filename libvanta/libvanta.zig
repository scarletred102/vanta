// ============================================================================
// libvanta — Minimal Userspace Runtime Library
// ============================================================================

const std = @import("std");

pub const Handle = u64;
pub const NULL_HANDLE: Handle = 0;

pub export var global_auxv: [*]const struct { ty: u64, val: u64 } = undefined;
var heap_cursor: u64 = 0x10000000;

pub fn syscall(num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64, arg6: u64) struct { val: u64, err: u64 } {
    var val: u64 = undefined;
    var err: u64 = undefined;
    asm volatile (
        \\ syscall
        : [val] "={rax}" (val),
          [err] "={rdx}" (err),
        : [num] "{rax}" (num),
          [arg1] "{rdi}" (arg1),
          [arg2] "{rsi}" (arg2),
          [arg3] "{rdx}" (arg3),
          [arg4] "{r10}" (arg4),
          [arg5] "{r8}" (arg5),
          [arg6] "{r9}" (arg6),
        : .{ .rcx = true, .r11 = true, .memory = true }
    );
    return .{ .val = val, .err = err };
}

pub fn vanta_cap_send(port: u64, msg_ptr: u64) u64 {
    return syscall(0, port, msg_ptr, 0, 0, 0, 0).err;
}

pub fn vanta_cap_recv(port: u64, msg_ptr: u64) u64 {
    return syscall(1, port, msg_ptr, 0, 0, 0, 0).err;
}

pub fn vanta_cap_call(port: u64, msg_ptr: u64, reply_ptr: u64) u64 {
    return syscall(2, port, msg_ptr, reply_ptr, 0, 0, 0).err;
}

pub fn vanta_cap_derive(parent: u64, mask: u64, child_ptr: u64) u64 {
    return syscall(3, parent, mask, child_ptr, 0, 0, 0).err;
}

pub fn vanta_cap_revoke(handle: u64) u64 {
    return syscall(4, handle, 0, 0, 0, 0, 0).err;
}

pub fn vanta_debug_print(msg: []const u8) void {
    _ = syscall(5, @intFromPtr(msg.ptr), msg.len, 0, 0, 0, 0);
}

pub fn vanta_exit(code: u64) noreturn {
    _ = syscall(6, code, 0, 0, 0, 0, 0);
    while (true) {}
}

pub fn vanta_mem_create(n_pages: u64) struct { handle: u64, err: u64 } {
    const res = syscall(11, n_pages, 0, 0, 0, 0, 0);
    return .{ .handle = res.val, .err = res.err };
}

pub fn vanta_mem_map(mem_cap: u64, target_vaddr: u64, n_pages: u64) u64 {
    return syscall(7, mem_cap, target_vaddr, n_pages, 0, 0, 0).err;
}

pub fn vanta_mem_unmap(target_vaddr: u64) u64 {
    return syscall(8, target_vaddr, 0, 0, 0, 0, 0).err;
}

pub fn vanta_thread_spawn(mem_cap: u64) struct { handle: u64, err: u64 } {
    const res = syscall(9, mem_cap, 0, 0, 0, 0, 0);
    return .{ .handle = res.val, .err = res.err };
}

pub fn vanta_notif_create() struct { handle: u64, err: u64 } {
    const res = syscall(13, 0, 0, 0, 0, 0, 0);
    return .{ .handle = res.val, .err = res.err };
}

pub fn vanta_cap_notify(notif_handle: u64, bits: u64) u64 {
    return syscall(12, notif_handle, bits, 0, 0, 0, 0).err;
}

pub fn vanta_cap_wait(notif_handle: u64, mask: u64) struct { matched: u64, err: u64 } {
    const res = syscall(10, notif_handle, mask, 0, 0, 0, 0);
    return .{ .matched = res.val, .err = res.err };
}

pub fn vanta_getauxval(ty: u64) u64 {
    var i: usize = 0;
    while (true) {
        const entry = global_auxv[i];
        if (entry.ty == 0) break; // AT_NULL
        if (entry.ty == ty) return entry.val;
        i += 1;
    }
    return 0;
}

pub fn vanta_alloc_pages(n_pages: u64) ?u64 {
    const mem_res = vanta_mem_create(n_pages);
    if (mem_res.err != 0) return null;
    const vaddr = heap_cursor;
    const map_err = vanta_mem_map(mem_res.handle, vaddr, n_pages);
    if (map_err != 0) return null;
    heap_cursor += n_pages * 4096;
    return vaddr;
}

pub extern fn main() void;

pub export fn _start() callconv(.naked) noreturn {
    asm volatile (
        \\ movq (%rsp), %rdi         // argc
        \\ leaq 8(%rsp), %rsi        // argv
        \\ leaq 16(%rsp,%rdi,8), %rdx // envp (after NULL in argv)
        \\ movq %rdx, %rcx
        \\1:
        \\ cmpq $0, (%rcx)           // find envp NULL terminator
        \\ leaq 8(%rcx), %rcx
        \\ jne 1b
        \\ // rcx now points to auxv!
        \\ movq %rcx, global_auxv(%rip)
        \\ andq $-16, %rsp
        \\ callq libvanta_main
        :
        :
        : .{ .memory = true }
    );
}

pub export fn libvanta_main() callconv(.c) noreturn {
    main();
    vanta_exit(0);
}
