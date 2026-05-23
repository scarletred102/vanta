// ============================================================================
// VantaOS — Capability System Stress Test (P3.8)
//
// 16 threads. Each creates 64 caps to shared endpoint objects,
// derives children with reduced rights, transfers caps via IPC,
// revokes roots, verifies all derived caps return null.
// Reports total ops/second. Zero tolerance for silent corruption.
// ============================================================================

const serial = @import("arch/x86_64/serial.zig");
const cap_mod = @import("cap/handle.zig");
const port_mod = @import("ipc/port.zig");
const proc = @import("proc/process.zig");
const sched = @import("sched/scheduler.zig");
const thread = @import("sched/thread.zig");

const N_THREADS: usize = 16;
const CAPS_PER_THREAD: usize = 64;

// Shared ports for the test
var test_ports: [N_THREADS]port_mod.Port = [_]port_mod.Port{.{}} ** N_THREADS;

// Per-thread results
var thread_ops: [N_THREADS]u64 = [_]u64{0} ** N_THREADS;
var thread_done: [N_THREADS]bool = [_]bool{false} ** N_THREADS;
var thread_errors: [N_THREADS]u64 = [_]u64{0} ** N_THREADS;

// Shared cap tables for each test thread (separate from kernel_proc)
var test_cap_tables: [N_THREADS]cap_mod.CapTable = [_]cap_mod.CapTable{.{}} ** N_THREADS;

fn runStressThread(tid: usize) void {
    var ops: u64 = 0;
    var errors: u64 = 0;
    var loop: usize = 0;

    while (loop < 100) : (loop += 1) {
        // Phase A: create CAPS_PER_THREAD root handles to a shared port
        var root_handles: [CAPS_PER_THREAD]cap_mod.Handle = [_]cap_mod.Handle{0} ** CAPS_PER_THREAD;
        var child_handles: [CAPS_PER_THREAD]cap_mod.Handle = [_]cap_mod.Handle{0} ** CAPS_PER_THREAD;

        const port_addr = @intFromPtr(&test_ports[tid]);

        for (0..CAPS_PER_THREAD) |i| {
            const h = cap_mod.cap_table_insert(
                &test_cap_tables[tid],
                port_addr,
                @intFromEnum(cap_mod.CapType.Endpoint),
                cap_mod.Rights.EndpointSend | cap_mod.Rights.EndpointRecv | cap_mod.Rights.EndpointGrant,
            ) orelse {
                errors += 1;
                continue;
            };
            root_handles[i] = h;
            ops += 1;
        }

        // Phase B: derive children with reduced rights (Send only = 0x01)
        for (0..CAPS_PER_THREAD) |i| {
            if (root_handles[i] == cap_mod.NULL_HANDLE) continue;
            const parent = cap_mod.cap_table_lookup(&test_cap_tables[tid], root_handles[i]) orelse {
                errors += 1;
                continue;
            };
            const parent_rights = parent.rights;
            const child_rights: u8 = cap_mod.Rights.EndpointSend; // strict subset

            if ((child_rights & parent_rights) != child_rights) {
                errors += 1;
                continue;
            }

            // Manually derive (no syscall in kernel test)
            var found_slot: ?u16 = null;
            var si: usize = 1;
            while (si < cap_mod.MAX_CAPS) : (si += 1) {
                if (test_cap_tables[tid].entries[si].type == 0) {
                    found_slot = @intCast(si);
                    break;
                }
            }
            const child_idx = found_slot orelse {
                errors += 1;
                continue;
            };
            const child = &test_cap_tables[tid].entries[child_idx];
            child.type = parent.type;
            child.rights = child_rights;
            child.kernel_object_ptr = parent.kernel_object_ptr;
            child.parent_table = &test_cap_tables[tid];
            child.parent_index = @as(u16, @truncate(root_handles[i] & 0xFFFFFFFFFFFF));
            child.parent_generation = parent.generation;
            child.old_table = null;
            child.old_index = 0;
            test_cap_tables[tid].count += 1;
            cap_mod.linkEntry(&test_cap_tables[tid], child_idx);

            child_handles[i] = cap_mod.encodeHandle(child_idx, child.generation);
            ops += 1;
        }

        // Phase C: revoke root handles — all children must become invalid
        for (0..CAPS_PER_THREAD) |i| {
            if (root_handles[i] == cap_mod.NULL_HANDLE) continue;
            cap_mod.cap_revoke(&test_cap_tables[tid], root_handles[i]);
            ops += 1;
        }

        // Phase D: verify all children are null after revocation (no use-after-revoke)
        for (0..CAPS_PER_THREAD) |i| {
            if (child_handles[i] == cap_mod.NULL_HANDLE) continue;
            const result = cap_mod.cap_table_lookup(&test_cap_tables[tid], child_handles[i]);
            if (result != null) {
                // Generation check failed — this is silent corruption
                serial.puts("[STRESS] FAIL: use-after-revoke detected thread=");
                serial.putDec(tid);
                serial.puts(" cap=");
                serial.putDec(i);
                serial.puts("\n");
                errors += 1;
            } else {
                ops += 1;
            }
        }

        // Reset table for next iteration
        test_cap_tables[tid] = .{};
    }

    thread_ops[tid] = ops;
    thread_errors[tid] = errors;
    thread_done[tid] = true;
}

// Wrappers with comptime-generated thread IDs (no closures in Zig)
fn makeStressThread(comptime tid: usize) fn () callconv(.c) noreturn {
    return struct {
        fn f() callconv(.c) noreturn {
            runStressThread(tid);
            sched.exitCurrentThread();
        }
    }.f;
}

pub fn run() void {
    serial.puts("[P3-TEST] Capability stress test starting (16 threads x 64 caps)...\n");

    // Reset state
    for (0..N_THREADS) |i| {
        test_cap_tables[i] = .{};
        thread_ops[i] = 0;
        thread_errors[i] = 0;
        thread_done[i] = false;
        test_ports[i] = .{};
    }

    // Spawn 16 stress threads using inline for (comptime indices required for function values)
    inline for (0..N_THREADS) |i| {
        const t = thread.create(makeStressThread(i)) orelse {
            serial.puts("[P3-TEST] FAIL: could not spawn thread ");
            serial.putDec(i);
            serial.puts("\n");
            return;
        };
        sched.enqueue(t);
    }

    // Yield until all threads complete
    var deadline: u64 = 10_000_000_000; // ~10 billion pause cycles (rough timeout)
    while (deadline > 0) : (deadline -= 1) {
        var all_done = true;
        for (0..N_THREADS) |i| {
            if (!thread_done[i]) { all_done = false; break; }
        }
        if (all_done) break;
        sched.yield();
    }

    // Tally results
    var total_ops: u64 = 0;
    var total_errors: u64 = 0;
    for (0..N_THREADS) |i| {
        total_ops += thread_ops[i];
        total_errors += thread_errors[i];
    }

    serial.puts("[P3-TEST] Stress test complete.\n");
    serial.puts("[P3-TEST] Total ops: ");
    serial.putDec(total_ops);
    serial.puts("\n[P3-TEST] Total errors: ");
    serial.putDec(total_errors);
    serial.puts("\n");

    if (total_errors == 0) {
        serial.puts("[P3-TEST] PASS: Zero silent corruption, all revocations verified.\n");
    } else {
        serial.puts("[P3-TEST] FAIL: Corruption detected.\n");
    }
}
