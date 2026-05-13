// ============================================================================
// VantaOS — IPC Port & Message System
//
// Messages are TYPED, not raw byte streams. This is the fundamental
// difference from POSIX pipes/sockets.
//
// Phase 0: Type definitions and basic queue.
// Phase 1: Kernel integration with blocking/wakeup.
// Phase 2: Zero-copy for large transfers via shared memory.
// ============================================================================

const cap = @import("../cap/handle.zig");

// ── Message ─────────────────────────────────────────────────────
// Fixed-size message that can be passed in registers (small) or via
// shared memory (large). Inline payload is 64 bytes — enough for
// most control messages without touching memory.

pub const MAX_INLINE_PAYLOAD: usize = 64;
pub const MAX_CAP_TRANSFERS: usize = 4;

pub const MessageFlags = packed struct(u32) {
    /// This message expects a reply (for cap_call RPC pattern)
    expects_reply: bool = false,
    /// This message IS a reply
    is_reply: bool = false,
    /// Message has a shared memory buffer attachment
    has_buffer: bool = false,
    /// Message should be delivered with high priority
    urgent: bool = false,
    _reserved: u28 = 0,
};

pub const Message = struct {
    /// Operation code — meaning defined by the protocol
    msg_type: u32 = 0,

    /// Delivery flags
    flags: MessageFlags = .{},

    /// Inline payload (small data, register-passed for fast IPC)
    payload: [MAX_INLINE_PAYLOAD]u8 = [_]u8{0} ** MAX_INLINE_PAYLOAD,

    /// Capability handles to transfer (up to 4 per message)
    /// These are moved from sender's cap table to receiver's cap table.
    caps: [MAX_CAP_TRANSFERS]cap.Handle = [_]cap.Handle{cap.NULL_HANDLE} ** MAX_CAP_TRANSFERS,

    /// Optional shared memory capability for bulk data transfers.
    /// The receiver gets a memory capability in their table.
    buffer_cap: cap.Handle = cap.NULL_HANDLE,

    /// Write structured data into the payload at an offset.
    pub fn writePayload(self: *Message, offset: usize, data: []const u8) void {
        const end = @min(offset + data.len, MAX_INLINE_PAYLOAD);
        const write_len = end - offset;
        @memcpy(self.payload[offset..end], data[0..write_len]);
    }

    /// Read data from the payload at an offset.
    pub fn readPayload(self: *const Message, offset: usize, len: usize) []const u8 {
        const end = @min(offset + len, MAX_INLINE_PAYLOAD);
        return self.payload[offset..end];
    }
};

// ── Port ────────────────────────────────────────────────────────
// An IPC endpoint. Messages are queued in a ring buffer.
// Phase 1: Add thread wait queue for blocking recv.

pub const PORT_QUEUE_CAPACITY: usize = 16;

pub const Port = struct {
    /// Ring buffer of messages
    queue: [PORT_QUEUE_CAPACITY]Message = undefined,
    head: usize = 0,
    tail: usize = 0,
    count: usize = 0,

    /// Capability handle of the owning process (for access checks)
    owner_cap: cap.Handle = cap.NULL_HANDLE,

    /// Port state
    state: PortState = .open,

    // Phase 1 TODO:
    // waiting_threads: ThreadList  — threads blocked on recv

    pub const PortState = enum {
        open,
        closed,
    };

    /// Send a message to this port. Returns false if the queue is full.
    pub fn send(self: *Port, msg: *const Message) bool {
        if (self.state == .closed) return false;
        if (self.count >= PORT_QUEUE_CAPACITY) return false;

        self.queue[self.tail] = msg.*;
        self.tail = (self.tail + 1) % PORT_QUEUE_CAPACITY;
        self.count += 1;
        return true;

        // Phase 1 TODO: Wake up any thread blocked on recv
    }

    /// Receive a message from this port. Returns null if empty.
    pub fn recv(self: *Port) ?Message {
        if (self.count == 0) return null;

        const msg = self.queue[self.head];
        self.head = (self.head + 1) % PORT_QUEUE_CAPACITY;
        self.count -= 1;
        return msg;

        // Phase 1 TODO: If queue was full, wake up blocked senders
    }

    /// Check if the port has pending messages.
    pub fn hasPending(self: *const Port) bool {
        return self.count > 0;
    }

    /// Check if the port's queue is full.
    pub fn isFull(self: *const Port) bool {
        return self.count >= PORT_QUEUE_CAPACITY;
    }

    /// Close the port. Pending messages are discarded.
    pub fn close(self: *Port) void {
        self.state = .closed;
        self.count = 0;
        self.head = 0;
        self.tail = 0;
    }
};

// ── Channel ─────────────────────────────────────────────────────
// A bidirectional IPC connection: two ports, one for each direction.
// Created as a pair — each end gets one port.

pub const Channel = struct {
    port_a: Port = .{}, // A sends here, B receives from here
    port_b: Port = .{}, // B sends here, A receives from here

    /// Send from endpoint A to endpoint B.
    pub fn sendA(self: *Channel, msg: *const Message) bool {
        return self.port_a.send(msg);
    }

    /// Send from endpoint B to endpoint A.
    pub fn sendB(self: *Channel, msg: *const Message) bool {
        return self.port_b.send(msg);
    }

    /// Receive at endpoint A (messages sent by B).
    pub fn recvA(self: *Channel) ?Message {
        return self.port_b.recv();
    }

    /// Receive at endpoint B (messages sent by A).
    pub fn recvB(self: *Channel) ?Message {
        return self.port_a.recv();
    }

    /// Close both ends.
    pub fn close(self: *Channel) void {
        self.port_a.close();
        self.port_b.close();
    }
};

// ── Well-Known Message Types ────────────────────────────────────
// Standard message types used across all protocols.

pub const MSG_PING: u32 = 0x0001;
pub const MSG_PONG: u32 = 0x0002;
pub const MSG_ERROR: u32 = 0x0003;
pub const MSG_CLOSE: u32 = 0x0004;

// Resource server messages (Phase 3)
pub const MSG_OPEN: u32 = 0x0100;
pub const MSG_READ: u32 = 0x0101;
pub const MSG_WRITE: u32 = 0x0102;
pub const MSG_STAT: u32 = 0x0103;
pub const MSG_QUERY: u32 = 0x0104;
pub const MSG_WATCH: u32 = 0x0105;

// Display server messages (Phase 4)
pub const MSG_CREATE_SURFACE: u32 = 0x0200;
pub const MSG_PRESENT: u32 = 0x0201;
pub const MSG_RESIZE: u32 = 0x0202;
pub const MSG_INPUT_EVENT: u32 = 0x0203;

// Audio server messages (Phase 6)
pub const MSG_AUDIO_OPEN: u32 = 0x0300;
pub const MSG_AUDIO_WRITE: u32 = 0x0301;
pub const MSG_AUDIO_CONFIGURE: u32 = 0x0302;
