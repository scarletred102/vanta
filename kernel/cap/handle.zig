// ============================================================================
// VantaOS — Capability System Core Types
//
// A capability is an unforgeable token granting specific rights to an object.
// Processes access ALL resources through capabilities — there is no ambient
// authority, no root, no admin mode.
//
// Phase 0: Type definitions only.
// Phase 1: Capability table implementation with derive/revoke.
// ============================================================================

// ── Handle ──────────────────────────────────────────────────────
// A handle is a process-local index into the capability table.
// Handle 0 is always NULL (invalid).

pub const Handle = u32;
pub const NULL_HANDLE: Handle = 0;

// ── Object Types ────────────────────────────────────────────────

pub const ObjectType = enum(u8) {
    null = 0,
    memory, // Physical or virtual memory region
    ipc_port, // IPC send/receive endpoint
    channel, // Bidirectional IPC (pair of ports)
    process, // Process control
    thread, // Thread control
    address_space, // Virtual address space
    interrupt, // Hardware interrupt
    io_region, // I/O port or MMIO range
    notification, // Lightweight signaling
    timer, // Kernel timer
};

// ── Rights Bitmap ───────────────────────────────────────────────
// 32-bit bitmask. Derived capabilities can only REMOVE rights, never add.

pub const Rights = packed struct(u32) {
    read: bool = false,
    write: bool = false,
    execute: bool = false,
    grant: bool = false, // Can transfer this cap to another process
    derive: bool = false, // Can create child caps with fewer rights
    revoke: bool = false, // Can destroy this cap and all descendants
    map: bool = false, // Can map into address space (memory objects)
    connect: bool = false, // Can create connections (ports)
    manage: bool = false, // Administrative operations
    inspect: bool = false, // Query metadata without accessing content
    _reserved: u22 = 0,

    pub const ALL = Rights{
        .read = true,
        .write = true,
        .execute = true,
        .grant = true,
        .derive = true,
        .revoke = true,
        .map = true,
        .connect = true,
        .manage = true,
        .inspect = true,
    };

    pub const READ_ONLY = Rights{ .read = true, .inspect = true };

    pub const READ_WRITE = Rights{ .read = true, .write = true, .inspect = true };

    /// Returns the intersection of two rights sets.
    pub fn intersect(self: Rights, mask: Rights) Rights {
        const a: u32 = @bitCast(self);
        const b: u32 = @bitCast(mask);
        return @bitCast(a & b);
    }

    /// Check if this rights set contains all rights in `required`.
    pub fn contains(self: Rights, required: Rights) bool {
        const a: u32 = @bitCast(self);
        const b: u32 = @bitCast(required);
        return (a & b) == b;
    }
};

// ── Capability ──────────────────────────────────────────────────
// Kernel-side representation. Processes never see this directly.

pub const Capability = struct {
    obj_type: ObjectType,
    rights: Rights,
    object: u64, // Kernel pointer or object ID
    parent: ?Handle, // Parent in derivation tree (null for root caps)
    generation: u32, // Incremented on revoke to detect stale handles
    owner: u32, // Process ID that owns this cap

    pub fn isValid(self: *const Capability) bool {
        return self.obj_type != .null;
    }
};

// ── Per-Process Capability Table ────────────────────────────────
// Fixed-size for Phase 0. Phase 1 will use dynamic allocation.

pub const MAX_CAPS: usize = 256;

pub const CapabilityTable = struct {
    entries: [MAX_CAPS]?Capability = [_]?Capability{null} ** MAX_CAPS,
    count: usize = 0,

    /// Allocate a slot and insert a capability. Returns the handle.
    pub fn insert(self: *CapabilityTable, cap: Capability) ?Handle {
        // Slot 0 is always NULL
        for (1..MAX_CAPS) |i| {
            if (self.entries[i] == null) {
                self.entries[i] = cap;
                self.count += 1;
                return @intCast(i);
            }
        }
        return null; // Table full
    }

    /// Look up a capability by handle.
    pub fn get(self: *const CapabilityTable, handle: Handle) ?*const Capability {
        if (handle == 0 or handle >= MAX_CAPS) return null;
        if (self.entries[handle]) |*cap| {
            return cap;
        }
        return null;
    }

    /// Get a mutable reference.
    pub fn getMut(self: *CapabilityTable, handle: Handle) ?*Capability {
        if (handle == 0 or handle >= MAX_CAPS) return null;
        if (self.entries[handle]) |*cap| {
            return cap;
        }
        return null;
    }

    /// Derive a child capability with restricted rights.
    /// The child has the intersection of parent's rights and the mask.
    pub fn derive(self: *CapabilityTable, parent_handle: Handle, mask: Rights) ?Handle {
        const parent = self.get(parent_handle) orelse return null;

        // Parent must have DERIVE right
        if (!parent.rights.derive) return null;

        const child = Capability{
            .obj_type = parent.obj_type,
            .rights = parent.rights.intersect(mask),
            .object = parent.object,
            .parent = parent_handle,
            .generation = parent.generation,
            .owner = parent.owner,
        };

        return self.insert(child);
    }

    /// Revoke a capability and ALL descendants (depth-first via parent links).
    pub fn revoke(self: *CapabilityTable, handle: Handle) void {
        if (handle == 0 or handle >= MAX_CAPS) return;
        if (self.entries[handle] == null) return;

        // First pass: bump generation on the target (invalidate stale refs)
        if (self.entries[handle]) |*c| c.generation +%= 1;

        // Iteratively revoke any cap whose .parent chain ends at `handle`.
        // Simple O(n²) for Phase 1 — fine for 256-slot table.
        var changed = true;
        while (changed) {
            changed = false;
            for (1..MAX_CAPS) |i| {
                if (self.entries[i]) |cap| {
                    if (cap.parent) |p| {
                        if (p == handle or self.entries[p] == null) {
                            self.entries[i] = null;
                            self.count -= 1;
                            changed = true;
                        }
                    }
                }
            }
        }

        self.entries[handle] = null;
        self.count -= 1;
    }
};
