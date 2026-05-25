// ============================================================================
// VantaOS — Capability System Core (Phase 3)
//
// A capability is an unforgeable token granting specific rights to an object.
// Processes access ALL resources through capabilities — there is no ambient
// authority, no root, no admin mode.
// ============================================================================

const std = @import("std");
const table_orig = @import("../syscall/table.zig");
const port_mod = @import("../ipc/port.zig");
const notif_mod = @import("../ipc/notification.zig");
const shm_mod = @import("../ipc/shm.zig");
const Thread = @import("../sched/thread.zig").Thread;

// ── Handle ──────────────────────────────────────────────────────
// A handle is a process-local 64-bit index and generation counter.
// Top 16 bits = generation counter, bottom 48 bits = index.
// Handle 0 is always NULL (invalid).

pub const Handle = u64;
pub const NULL_HANDLE: Handle = 0;

// ── Capability Types ────────────────────────────────────────────

pub const CapType = enum(u4) {
    Null = 0,
    Memory = 1,
    Endpoint = 2,
    Thread = 3,
    Notification = 4,
    DeviceIRQ = 5,
    PageTable = 6,
    SharedMemory = 7,
};

// ── Rights ──────────────────────────────────────────────────────
// Rights are represented as a u8 bitmask.
// Type-specific rights are defined as constants.

pub const Rights = struct {
    // Memory
    pub const MemoryRead: u8 = 1 << 0;
    pub const MemoryWrite: u8 = 1 << 1;
    pub const MemoryExec: u8 = 1 << 2;
    pub const MemoryMap: u8 = 1 << 3;

    // Endpoint
    pub const EndpointSend: u8 = 1 << 0;
    pub const EndpointRecv: u8 = 1 << 1;
    pub const EndpointGrant: u8 = 1 << 2;

    // Thread
    pub const ThreadControl: u8 = 1 << 0;
    pub const ThreadInspect: u8 = 1 << 1;

    // Notification
    pub const NotificationSignal: u8 = 1 << 0;
    pub const NotificationWait: u8 = 1 << 1;

    // DeviceIRQ
    pub const DeviceIRQBind: u8 = 1 << 0;

    // PageTable
    pub const PageTableMap: u8 = 1 << 0;
    pub const PageTableUnmap: u8 = 1 << 1;

    // SharedMemory
    pub const ShmRead: u8 = 1 << 0;
    pub const ShmWrite: u8 = 1 << 1;
};

// ── CapEntry (Embedded Tree Links) ──────────────────────────────
// Embedded singly-linked lists are kept on each kernel object to track
// child capabilities for O(derived_count) transitive invalidation.

pub const CapEntry = struct {
    type: u4 = 0,
    rights: u8 = 0,
    generation: u16 = 1, // Start at 1 so handle 0 is always invalid
    kernel_object_ptr: u48 = 0,

    // Singly-linked list of derived capabilities pointing to the same kernel object.
    next_derived_table: ?*CapTable = null,
    next_derived_index: u16 = 0,

    // Parent details for ancestry verification
    parent_table: ?*CapTable = null,
    parent_index: u16 = 0,
    parent_generation: u16 = 0,

    // Temporal fields used ONLY during IPC transfer (in-transit tracking)
    old_table: ?*CapTable = null,
    old_index: u16 = 0,
};

// ── CapListHead ─────────────────────────────────────────────────
// Embedded inside kernel objects (Port, Thread) to track the head of
// the object's capability list.

pub const CapListHead = struct {
    table: ?*CapTable = null,
    index: u16 = 0,
};

// ── CapTable (1024 flat entries) ────────────────────────────────

pub const MAX_CAPS: usize = 1024;

pub const CapTable = struct {
    entries: [MAX_CAPS]CapEntry = [_]CapEntry{.{}} ** MAX_CAPS,
    count: usize = 0,
};

// ── Handle Encoding Helpers ─────────────────────────────────────

pub fn encodeHandle(index: u16, generation: u16) u64 {
    return (@as(u64, generation) << 48) | @as(u64, index);
}

pub fn decodeHandle(handle: u64) struct { index: u16, generation: u16 } {
    return .{
        .index = @as(u16, @truncate(handle & 0xFFFFFFFFFFFF)),
        .generation = @as(u16, @truncate(handle >> 48)),
    };
}

// ── CapTable Core Operations ────────────────────────────────────

/// Insert a new capability into a free slot in the capability table.
pub fn cap_table_insert(table: *CapTable, obj_ptr: u64, cap_type: u4, rights: u8) ?Handle {
    // Slot 0 is reserved for Null / NULL_HANDLE
    var i: usize = 1;
    while (i < MAX_CAPS) : (i += 1) {
        const entry = &table.entries[i];
        if (entry.type == 0) { // Free slot
            entry.type = cap_type;
            entry.rights = rights;
            entry.kernel_object_ptr = @truncate(obj_ptr);
            entry.parent_table = null;
            entry.parent_index = 0;
            entry.parent_generation = 0;
            entry.old_table = null;
            entry.old_index = 0;
            // entry.generation is preserved/incremented on free to prevent handle reuse
            
            table.count += 1;

            // Link into the kernel object's capability list
            linkEntry(table, @intCast(i));

            return encodeHandle(@intCast(i), entry.generation);
        }
    }
    return null; // Table full
}

/// Look up a capability entry by handle, validating generation.
pub fn cap_table_lookup(table: *CapTable, handle: Handle) ?*CapEntry {
    if (handle == NULL_HANDLE) return null;
    const decoded = decodeHandle(handle);

    if (decoded.index >= MAX_CAPS) return null;
    const entry = &table.entries[decoded.index];

    if (entry.type == 0 or entry.generation != decoded.generation) {
        return null; // Generation mismatch or slot empty
    }
    return entry;
}

// ── Derivation Tree & Invalidation ──────────────────────────────

/// Safely retrieve the 64-bit object pointer from a 48-bit kernel_object_ptr using sign-extension.
pub fn getObjectPtr(entry: *const CapEntry) u64 {
    const signed_val = @as(i48, @bitCast(entry.kernel_object_ptr));
    const extended_val = @as(i64, signed_val);
    return @as(u64, @bitCast(extended_val));
}

/// Get a pointer to the capability list head on a specific kernel object.
fn getCapListHead(cap_type: u4, obj_ptr: u48) ?*CapListHead {
    const type_val: CapType = @enumFromInt(cap_type);
    const signed_val = @as(i48, @bitCast(obj_ptr));
    const extended_val = @as(i64, signed_val);
    const obj_u64 = @as(u64, @bitCast(extended_val));
    return switch (type_val) {
        .Endpoint => {
            const port = @as(*port_mod.Port, @ptrFromInt(obj_u64));
            return &port.cap_list;
        },
        .Thread => {
            const thread_ptr = @as(*Thread, @ptrFromInt(obj_u64));
            return &thread_ptr.cap_list;
        },
        .Notification => {
            const notif = @as(*notif_mod.Notification, @ptrFromInt(obj_u64));
            return &notif.cap_list;
        },
        .SharedMemory => {
            const shm = @as(*shm_mod.ShmObject, @ptrFromInt(obj_u64));
            return &shm.cap_list;
        },
        else => null,
    };
}

/// Link a capability entry into its kernel object's capability list.
pub fn linkEntry(table: *CapTable, index: u16) void {
    const entry = &table.entries[index];
    if (getCapListHead(entry.type, entry.kernel_object_ptr)) |head| {
        entry.next_derived_table = head.table;
        entry.next_derived_index = head.index;
        head.table = table;
        head.index = index;
    }
}

/// Unlink a capability entry from its kernel object's capability list.
pub fn unlinkEntry(table: *CapTable, index: u16) void {
    const entry = &table.entries[index];
    if (getCapListHead(entry.type, entry.kernel_object_ptr)) |head| {
        if (head.table == table and head.index == index) {
            head.table = entry.next_derived_table;
            head.index = entry.next_derived_index;
        } else {
            var curr_table = head.table;
            var curr_idx = head.index;
            while (curr_table) |c_tab| {
                const curr = &c_tab.entries[curr_idx];
                if (curr.next_derived_table == table and curr.next_derived_index == index) {
                    curr.next_derived_table = entry.next_derived_table;
                    curr.next_derived_index = entry.next_derived_index;
                    break;
                }
                curr_table = curr.next_derived_table;
                curr_idx = curr.next_derived_index;
            }
        }
    }
    entry.next_derived_table = null;
    entry.next_derived_index = 0;
}

/// Revoke a capability and transitively invalidate all its descendants.
pub fn cap_revoke(table: *CapTable, handle: u64) void {
    const decoded = decodeHandle(handle);

    if (decoded.index >= MAX_CAPS) return;
    const entry = &table.entries[decoded.index];
    if (entry.type == 0 or entry.generation != decoded.generation) return;

    invalidateDescendants(table, decoded.index, decoded.generation);
}

/// Invalidate descendants transitively in O(derived_count).
fn invalidateDescendants(start_table: *CapTable, start_idx: u16, start_generation: u16) void {
    const start_entry = &start_table.entries[start_idx];
    const obj_ptr = start_entry.kernel_object_ptr;

    // Unlink the start entry first
    unlinkEntry(start_table, start_idx);

    // Walk the specific kernel object's capability list to invalidate all descendants
    if (getCapListHead(start_entry.type, obj_ptr)) |head| {
        var curr_table = head.table;
        var curr_idx = head.index;

        while (curr_table) |c_tab| {
            const entry = &c_tab.entries[curr_idx];
            const next_tab = entry.next_derived_table;
            const next_idx = entry.next_derived_index;

            if (isDescendantOf(c_tab, curr_idx, start_table, start_idx, start_generation)) {
                // Unlink first before invalidating to preserve list traversal integrity
                unlinkEntry(c_tab, curr_idx);

                // Invalidate slot
                entry.type = 0;
                entry.rights = 0;
                entry.kernel_object_ptr = 0;
                entry.generation +%= 1; // Prevent handle reuse
                entry.parent_table = null;
                entry.parent_index = 0;
                entry.parent_generation = 0;

                c_tab.count -= 1;
            }

            curr_table = next_tab;
            curr_idx = next_idx;
        }
    }

    // Invalidate the start entry itself
    start_entry.type = 0;
    start_entry.rights = 0;
    start_entry.kernel_object_ptr = 0;
    start_entry.generation +%= 1;
    start_entry.parent_table = null;
    start_entry.parent_index = 0;
    start_entry.parent_generation = 0;

    start_table.count -= 1;
}

/// Check if a capability is a descendant of a specific ancestor.
fn isDescendantOf(
    child_table: *CapTable, child_idx: u16,
    ancestor_table: *CapTable, ancestor_idx: u16, ancestor_generation: u16
) bool {
    var curr_table = child_table;
    var curr_idx = child_idx;

    while (true) {
        const entry = &curr_table.entries[curr_idx];
        const p_table = entry.parent_table orelse return false;
        const p_idx = entry.parent_index;
        const p_gen = entry.parent_generation;

        if (p_table == ancestor_table and p_idx == ancestor_idx and p_gen == ancestor_generation) {
            return true;
        }

        // Move up the parent chain, validating each parent link is alive
        const parent_entry = &p_table.entries[p_idx];
        if (parent_entry.type == 0 or parent_entry.generation != p_gen) {
            return false; // Parent was already revoked
        }

        curr_table = p_table;
        curr_idx = p_idx;
    }
}

// ── IPC Move Transaction Semantics ──────────────────────────────

/// Prepare a message for sending: validates all handles, copies them to message transit,
/// unlinks them, and zeros the sender's slots. Transaction-safe.
pub fn prepareMessageForSend(sender_table: *CapTable, msg: *port_mod.Message) table_orig.Error {
    // 1. Transaction Validation: verify all handles are valid first
    if (msg.buffer_cap != NULL_HANDLE) {
        _ = cap_table_lookup(sender_table, msg.buffer_cap) orelse return .invalid_handle;
    }
    var i: usize = 0;
    while (i < port_mod.MAX_CAP_TRANSFERS) : (i += 1) {
        const handle = msg.caps[i];
        if (handle != NULL_HANDLE) {
            _ = cap_table_lookup(sender_table, handle) orelse return .invalid_handle;
        }
    }

    // 2. Perform the Moves (Now guaranteed to succeed)
    if (msg.buffer_cap != NULL_HANDLE) {
        const idx = decodeHandle(msg.buffer_cap).index;
        const entry = &sender_table.entries[idx];
        
        msg.transferred_buffer_cap = entry.*;
        msg.transferred_buffer_cap.old_table = sender_table;
        msg.transferred_buffer_cap.old_index = idx;
        
        unlinkEntry(sender_table, idx);
        entry.type = 0;
        entry.rights = 0;
        entry.kernel_object_ptr = 0;
        entry.generation +%= 1;
        sender_table.count -= 1;
        
        msg.buffer_cap = NULL_HANDLE;
    }

    i = 0;
    while (i < port_mod.MAX_CAP_TRANSFERS) : (i += 1) {
        const handle = msg.caps[i];
        if (handle != NULL_HANDLE) {
            const idx = decodeHandle(handle).index;
            const entry = &sender_table.entries[idx];
            
            msg.transferred_caps[i] = entry.*;
            msg.transferred_caps[i].old_table = sender_table;
            msg.transferred_caps[i].old_index = idx;
            
            unlinkEntry(sender_table, idx);
            entry.type = 0;
            entry.rights = 0;
            entry.kernel_object_ptr = 0;
            entry.generation +%= 1;
            sender_table.count -= 1;
            
            msg.caps[i] = NULL_HANDLE;
        }
    }

    return .success;
}

/// Receive transferred capabilities: checks parent validity, allocates free slots,
/// links them, updates descendant child parent links, and outputs new handles.
pub fn receiveMessageCaps(receiver_table: *CapTable, msg: *port_mod.Message) void {
    // 1. Receive buffer_cap
    if (msg.transferred_buffer_cap.type != 0) {
        var cap_entry = msg.transferred_buffer_cap;

        // Check if parent has been revoked while in transit
        if (cap_entry.parent_table) |p_table| {
            const p_entry = &p_table.entries[cap_entry.parent_index];
            if (p_entry.type == 0 or p_entry.generation != cap_entry.parent_generation) {
                cap_entry.type = 0; // Parent was revoked
            }
        }

        if (cap_entry.type != 0) {
            if (findFreeSlot(receiver_table)) |dst_idx| {
                const entry = &receiver_table.entries[dst_idx];
                entry.type = cap_entry.type;
                entry.rights = cap_entry.rights;
                entry.kernel_object_ptr = cap_entry.kernel_object_ptr;
                entry.parent_table = cap_entry.parent_table;
                entry.parent_index = cap_entry.parent_index;
                entry.parent_generation = cap_entry.parent_generation;
                entry.old_table = null;
                entry.old_index = 0;

                receiver_table.count += 1;
                linkEntry(receiver_table, dst_idx);

                // Update parent links for any children that were derived from this cap
                updateChildrenParent(cap_entry.old_table, cap_entry.old_index, cap_entry.generation, receiver_table, dst_idx);

                msg.buffer_cap = encodeHandle(dst_idx, entry.generation);
            } else {
                msg.buffer_cap = NULL_HANDLE;
            }
        } else {
            msg.buffer_cap = NULL_HANDLE;
        }
    }

    // 2. Receive up to 4 capabilities
    var i: usize = 0;
    while (i < port_mod.MAX_CAP_TRANSFERS) : (i += 1) {
        var cap_entry = msg.transferred_caps[i];
        if (cap_entry.type != 0) {
            // Check if parent was revoked while in transit
            if (cap_entry.parent_table) |p_table| {
                const p_entry = &p_table.entries[cap_entry.parent_index];
                if (p_entry.type == 0 or p_entry.generation != cap_entry.parent_generation) {
                    cap_entry.type = 0; // Parent revoked
                }
            }
        }

        if (cap_entry.type != 0) {
            if (findFreeSlot(receiver_table)) |dst_idx| {
                const entry = &receiver_table.entries[dst_idx];
                entry.type = cap_entry.type;
                entry.rights = cap_entry.rights;
                entry.kernel_object_ptr = cap_entry.kernel_object_ptr;
                entry.parent_table = cap_entry.parent_table;
                entry.parent_index = cap_entry.parent_index;
                entry.parent_generation = cap_entry.parent_generation;
                entry.old_table = null;
                entry.old_index = 0;

                receiver_table.count += 1;
                linkEntry(receiver_table, dst_idx);

                // Update parent links for any children that were derived from this cap
                updateChildrenParent(cap_entry.old_table, cap_entry.old_index, cap_entry.generation, receiver_table, dst_idx);

                msg.caps[i] = encodeHandle(dst_idx, entry.generation);
            } else {
                msg.caps[i] = NULL_HANDLE;
            }
        } else {
            msg.caps[i] = NULL_HANDLE;
        }
    }
}

fn findFreeSlot(table: *CapTable) ?u16 {
    var i: usize = 1;
    while (i < MAX_CAPS) : (i += 1) {
        if (table.entries[i].type == 0) {
            return @intCast(i);
        }
    }
    return null;
}

fn updateChildrenParent(
    old_table: ?*CapTable, old_index: u16, generation: u16,
    new_table: *CapTable, new_index: u16
) void {
    const entry = &new_table.entries[new_index];
    if (getCapListHead(entry.type, entry.kernel_object_ptr)) |head| {
        var curr_table = head.table;
        var curr_idx = head.index;
        while (curr_table) |c_tab| {
            const child = &c_tab.entries[curr_idx];
            if (child.parent_table == old_table and child.parent_index == old_index and child.parent_generation == generation) {
                child.parent_table = new_table;
                child.parent_index = new_index;
            }
            curr_table = child.next_derived_table;
            curr_idx = child.next_derived_index;
        }
    }
}
