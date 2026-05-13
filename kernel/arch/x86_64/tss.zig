// ============================================================================
// VantaOS — Task State Segment (TSS) for x86_64
//
// In 64-bit mode, the TSS is used only for:
//   - RSP0: stack pointer used on ring 3 → ring 0 transitions (interrupts/syscalls)
//   - IST1..IST7: dedicated stacks for specific exception vectors
//   - I/O permission bitmap base offset
//
// IST usage in VantaOS:
//   IST1: Double Fault (#DF)  — safe stack to handle stack corruption
//   IST2: NMI                  — non-maskable interrupts
//   IST3: Machine Check (#MC)  — hardware errors
// ============================================================================

const std = @import("std");

// ── TSS Structure (104 bytes minimum, +IOPB) ────────────────────

pub const Tss = extern struct {
    reserved0: u32 = 0,
    rsp0: u64 align(1) = 0,   // Ring 0 stack pointer
    rsp1: u64 align(1) = 0,
    rsp2: u64 align(1) = 0,
    reserved1: u64 align(1) = 0,
    ist1: u64 align(1) = 0,    // Double Fault stack
    ist2: u64 align(1) = 0,    // NMI stack
    ist3: u64 align(1) = 0,    // Machine Check stack
    ist4: u64 align(1) = 0,
    ist5: u64 align(1) = 0,
    ist6: u64 align(1) = 0,
    ist7: u64 align(1) = 0,
    reserved2: u64 align(1) = 0,
    reserved3: u16 = 0,
    iopb_offset: u16 = @sizeOf(@This()), // No I/O permission bitmap → point past end
};

comptime {
    if (@sizeOf(Tss) != 104) @compileError("TSS must be exactly 104 bytes");
}

// ── IST Stacks ──────────────────────────────────────────────────
// Statically reserved 16KB stacks for each critical exception.

const IST_STACK_SIZE: usize = 16 * 1024;

var ist1_stack: [IST_STACK_SIZE]u8 align(16) = [_]u8{0} ** IST_STACK_SIZE;
var ist2_stack: [IST_STACK_SIZE]u8 align(16) = [_]u8{0} ** IST_STACK_SIZE;
var ist3_stack: [IST_STACK_SIZE]u8 align(16) = [_]u8{0} ** IST_STACK_SIZE;

// Kernel stack used for ring 3 → ring 0 transitions (RSP0).
// In Phase 2 this becomes per-thread; for now, a single global stack is fine
// since we don't have userspace yet.
var rsp0_stack: [IST_STACK_SIZE]u8 align(16) = [_]u8{0} ** IST_STACK_SIZE;

// ── TSS Instance ────────────────────────────────────────────────

pub var tss: Tss = .{};

pub fn init() void {
    tss = .{};
    // Stack pointers point to the TOP (high end) of each stack — they grow down.
    tss.rsp0 = @intFromPtr(&rsp0_stack) + IST_STACK_SIZE;
    tss.ist1 = @intFromPtr(&ist1_stack) + IST_STACK_SIZE;
    tss.ist2 = @intFromPtr(&ist2_stack) + IST_STACK_SIZE;
    tss.ist3 = @intFromPtr(&ist3_stack) + IST_STACK_SIZE;
    tss.iopb_offset = @sizeOf(Tss);
}

pub fn address() u64 {
    return @intFromPtr(&tss);
}

pub fn size() u32 {
    return @sizeOf(Tss);
}

/// Update RSP0 — called by scheduler on thread switch in Phase 2.
pub fn setRsp0(rsp: u64) void {
    tss.rsp0 = rsp;
}
