// ============================================================================
// VantaOS — Slab Allocator (Phase 5)
// ============================================================================

const std = @import("std");
const pmm = @import("pmm.zig");
const vmm = @import("vmm.zig");
const serial = @import("../arch/x86_64/serial.zig");
const builtin = @import("builtin");

const Thread = @import("../sched/thread.zig").Thread;
const CapEntry = @import("../cap/handle.zig").CapEntry;
const IpcPort = @import("../ipc/port.zig").Port;
const VmaEntry = @import("../proc/process.zig").Vma;
const cpu_local = @import("../arch/x86_64/cpu_local.zig");

pub fn SlabCache(comptime T: type, comptime alignment: ?usize) type {
    return struct {
        const Self = @This();
        const obj_size = @sizeOf(T);
        const obj_align = if (alignment) |a| a else @alignOf(T);

        pub const slots_per_page = calculateSlots();
        pub const bitmap_words = (slots_per_page + 63) / 64;

        pub const Header = struct {
            next: ?*Header = null,
            free_count: u32 = slots_per_page,
            bitmap: [bitmap_words]u64 = [_]u64{0} ** bitmap_words,
        };

        const header_size = @sizeOf(Header);
        const slots_start = (header_size + obj_align - 1) & ~(obj_align - 1);

        head: ?*Header = null,

        fn calculateSlots() comptime_int {
            var slots: comptime_int = 1;
            while (true) {
                const words = (slots + 63) / 64;
                const h_size = 16 + words * 8;
                const start = (h_size + obj_align - 1) & ~(obj_align - 1);
                if (start + (slots + 1) * obj_size > 4096) {
                    return slots;
                }
                slots += 1;
            }
        }

        pub fn init(self: *Self) void {
            self.head = null;
        }

        pub fn alloc(self: *Self) ?*T {
            var curr = self.head;
            while (curr) |h| {
                if (h.free_count > 0) {
                    return self.allocInSlab(h);
                }
                curr = h.next;
            }

            const phys = pmm.allocPage() orelse return null;
            const virt = vmm.phys2virt(phys);
            const h = @as(*Header, @ptrFromInt(virt));

            h.next = self.head;
            h.free_count = slots_per_page;
            @memset(&h.bitmap, 0);

            self.head = h;

            return self.allocInSlab(h);
        }

        fn allocInSlab(self: *Self, h: *Header) *T {
            _ = self;
            var word_idx: usize = 0;
            while (word_idx < bitmap_words) : (word_idx += 1) {
                const word = h.bitmap[word_idx];
                if (word != 0xFFFF_FFFF_FFFF_FFFF) {
                    const bit_idx = @ctz(~word);
                    const slot_idx = word_idx * 64 + bit_idx;
                    std.debug.assert(slot_idx < slots_per_page);

                    h.bitmap[word_idx] |= (@as(u64, 1) << @intCast(bit_idx));
                    h.free_count -= 1;

                    const ptr = @as(*T, @ptrFromInt(@intFromPtr(h) + slots_start + slot_idx * obj_size));

                    if (builtin.mode == .Debug or builtin.mode == .ReleaseSafe) {
                        const uaf_ptr = @as(*const u64, @ptrCast(@alignCast(ptr)));
                        if ((uaf_ptr.* >> 32) == 0xDEADC0DE) {
                            @panic("SlabCache: Use-after-free or corruption detected!");
                        }
                    }

                    @memset(@as([*]u8, @ptrCast(ptr))[0..obj_size], 0);
                    return ptr;
                }
            }
            @panic("SlabCache: Inconsistency - free_count > 0 but bitmap full!");
        }

        pub fn free(self: *Self, ptr: *T) void {
            const h = @as(*Header, @ptrFromInt(@intFromPtr(ptr) & ~@as(u64, 4095)));
            const slot_idx = (@intFromPtr(ptr) - (@intFromPtr(h) + slots_start)) / obj_size;

            if (slot_idx >= slots_per_page) {
                @panic("SlabCache: free pointer out of bounds!");
            }

            const word_idx = slot_idx / 64;
            const bit_idx = slot_idx % 64;
            const mask = @as(u64, 1) << @intCast(bit_idx);

            if ((h.bitmap[word_idx] & mask) == 0) {
                @panic("SlabCache: Double free detected!");
            }

            if (builtin.mode == .Debug or builtin.mode == .ReleaseSafe) {
                const uaf_ptr = @as(*u64, @ptrCast(@alignCast(ptr)));
                uaf_ptr.* = 0xDEADC0DEDEADBEEF;
            }

            h.bitmap[word_idx] &= ~mask;
            h.free_count += 1;

            if (h.free_count == slots_per_page) {
                self.freeSlabPage(h);
            }
        }

        fn freeSlabPage(self: *Self, h: *Header) void {
            if (self.head == h) {
                self.head = h.next;
            } else {
                var prev = self.head;
                while (prev) |p| {
                    if (p.next == h) {
                        p.next = h.next;
                        break;
                    }
                    prev = p.next;
                }
            }
            pmm.freePage(vmm.virt2phys_hhdm(@intFromPtr(h)));
        }
    };
}

pub var thread_cache: SlabCache(Thread, 64) = .{};
pub var cap_cache: SlabCache(CapEntry, 8) = .{};
pub var port_cache: SlabCache(IpcPort, 16) = .{};
pub var vma_cache: SlabCache(VmaEntry, 8) = .{};

pub fn init() void {
    thread_cache.init();
    cap_cache.init();
    port_cache.init();
    vma_cache.init();
    serial.puts("[SLAB]  Slab allocator online\n");
}

// ── Magazine-aware alloc/free for Thread and CapEntry ──────────────────────
// On free: deposit into per-CPU magazine (skip global slab if not full).
// On alloc: drain from per-CPU magazine first (skip global slab if available).

pub fn alloc_thread() ?*Thread {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.thread_mag_count > 0) {
        cpu.thread_mag_count -= 1;
        const ptr: *Thread = @ptrFromInt(cpu.thread_magazine[cpu.thread_mag_count]);
        @memset(@as([*]u8, @ptrCast(ptr))[0..@sizeOf(Thread)], 0);
        return ptr;
    }
    return thread_cache.alloc();
}

pub fn free_thread(ptr: *Thread) void {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.thread_mag_count < 64) {
        cpu.thread_magazine[cpu.thread_mag_count] = @intFromPtr(ptr);
        cpu.thread_mag_count += 1;
        // Write sentinel so magazine slots aren't confused with live objects
        if (builtin.mode == .Debug or builtin.mode == .ReleaseSafe) {
            const uaf_ptr = @as(*u64, @ptrCast(@alignCast(ptr)));
            uaf_ptr.* = 0xDEADC0DEDEADBEEF;
        }
        return;
    }
    thread_cache.free(ptr);
}

pub fn alloc_cap() ?*CapEntry {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.cap_mag_count > 0) {
        cpu.cap_mag_count -= 1;
        const ptr: *CapEntry = @ptrFromInt(cpu.cap_magazine[cpu.cap_mag_count]);
        @memset(@as([*]u8, @ptrCast(ptr))[0..@sizeOf(CapEntry)], 0);
        return ptr;
    }
    return cap_cache.alloc();
}

pub fn free_cap(ptr: *CapEntry) void {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.cap_mag_count < 64) {
        cpu.cap_magazine[cpu.cap_mag_count] = @intFromPtr(ptr);
        cpu.cap_mag_count += 1;
        if (builtin.mode == .Debug or builtin.mode == .ReleaseSafe) {
            const uaf_ptr = @as(*u64, @ptrCast(@alignCast(ptr)));
            uaf_ptr.* = 0xDEADC0DEDEADBEEF;
        }
        return;
    }
    cap_cache.free(ptr);
}
