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
const sched = @import("../sched/scheduler.zig");
const Thread = @import("../sched/thread.zig").Thread;

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

    // Transferred capability slots in-transit (kernel-only)
    transferred_caps: [MAX_CAP_TRANSFERS]cap.CapEntry = [_]cap.CapEntry{.{}} ** MAX_CAP_TRANSFERS,
    transferred_buffer_cap: cap.CapEntry = .{},

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

    /// Spinlock to protect queue operations under SMP
    lock: @import("../arch/x86_64/cpu_local.zig").TicketLock = .{},

    /// Capability handle of the owning process (for access checks)
    owner_cap: cap.Handle = cap.NULL_HANDLE,

    /// Capability list head registered on this port for transitive invalidations
    cap_list: cap.CapListHead = .{},

    /// Port state
    state: PortState = .open,

    /// Threads blocked on recv (singly-linked via Thread.next)
    recv_waiters: ?*Thread = null,
    /// Threads blocked on send-when-full
    send_waiters: ?*Thread = null,

    pub const PortState = enum { open, closed };

    /// Non-blocking send. Returns false if queue full or port closed.
    pub fn send(self: *Port, msg: *const Message) bool {
        const flags = self.lock.lock_irqsave();
        if (self.state == .closed or self.count >= PORT_QUEUE_CAPACITY) {
            self.lock.unlock_irqrestore(flags);
            return false;
        }

        self.queue[self.tail] = msg.*;
        self.tail = (self.tail + 1) % PORT_QUEUE_CAPACITY;
        self.count += 1;

        // Wake one recv waiter if any
        if (self.recv_waiters) |w| {
            self.recv_waiters = w.next;
            w.next = null;
            self.lock.unlock_irqrestore(flags);
            sched.wake(w);
            return true;
        }
        self.lock.unlock_irqrestore(flags);
        return true;
    }

    /// Blocking send. Parks current thread on send_waiters until queue has room.
    pub fn sendBlocking(self: *Port, msg: *const Message) bool {
        while (true) {
            const flags = self.lock.lock_irqsave();
            if (self.state == .closed) {
                self.lock.unlock_irqrestore(flags);
                return false;
            }
            if (self.count < PORT_QUEUE_CAPACITY) {
                self.queue[self.tail] = msg.*;
                self.tail = (self.tail + 1) % PORT_QUEUE_CAPACITY;
                self.count += 1;

                if (self.recv_waiters) |w| {
                    self.recv_waiters = w.next;
                    w.next = null;
                    self.lock.unlock_irqrestore(flags);
                    sched.wake(w);
                    return true;
                }
                self.lock.unlock_irqrestore(flags);
                return true;
            }

            // Park
            const cur = @import("../arch/x86_64/cpu_local.zig").get_cpu_local().current_thread orelse {
                self.lock.unlock_irqrestore(flags);
                return false;
            };
            var already_in = false;
            var curr_w = self.send_waiters;
            while (curr_w) |w| {
                if (w == cur) {
                    already_in = true;
                    break;
                }
                curr_w = w.next;
            }
            if (!already_in) {
                cur.next = self.send_waiters;
                self.send_waiters = cur;
            }
            @atomicStore(bool, &cur.yielded, false, .release);
            self.lock.unlock_irqrestore(flags);
            sched.block();
        }
    }

    /// Non-blocking recv. Returns null if empty.
    pub fn recv(self: *Port) ?Message {
        const flags = self.lock.lock_irqsave();
        if (self.count == 0) {
            self.lock.unlock_irqrestore(flags);
            return null;
        }
        const msg = self.queue[self.head];
        self.head = (self.head + 1) % PORT_QUEUE_CAPACITY;
        self.count -= 1;

        // Wake one send waiter
        if (self.send_waiters) |w| {
            self.send_waiters = w.next;
            w.next = null;
            self.lock.unlock_irqrestore(flags);
            sched.wake(w);
            return msg;
        }
        self.lock.unlock_irqrestore(flags);
        return msg;
    }

    /// Blocking recv. Parks until a message arrives.
    pub fn recvBlocking(self: *Port) ?Message {
        while (true) {
            const flags = self.lock.lock_irqsave();
            if (self.count > 0) {
                const msg = self.queue[self.head];
                self.head = (self.head + 1) % PORT_QUEUE_CAPACITY;
                self.count -= 1;

                if (self.send_waiters) |w| {
                    self.send_waiters = w.next;
                    w.next = null;
                    self.lock.unlock_irqrestore(flags);
                    sched.wake(w);
                    return msg;
                }
                self.lock.unlock_irqrestore(flags);
                return msg;
            }
            if (self.state == .closed) {
                self.lock.unlock_irqrestore(flags);
                return null;
            }

            const cur = @import("../arch/x86_64/cpu_local.zig").get_cpu_local().current_thread orelse {
                self.lock.unlock_irqrestore(flags);
                return null;
            };
            var already_in = false;
            var curr_w = self.recv_waiters;
            while (curr_w) |w| {
                if (w == cur) {
                    already_in = true;
                    break;
                }
                curr_w = w.next;
            }
            if (!already_in) {
                cur.next = self.recv_waiters;
                self.recv_waiters = cur;
            }
            @atomicStore(bool, &cur.yielded, false, .release);
            self.lock.unlock_irqrestore(flags);
            sched.block();
        }
    }

    /// Park on recv_waiters until at least one message is present, without consuming it.
    /// Used by cap_poll to block without dequeuing.
    pub fn waitReady(self: *Port) void {
        while (true) {
            const flags = self.lock.lock_irqsave();
            if (self.count > 0 or self.state == .closed) {
                self.lock.unlock_irqrestore(flags);
                return;
            }
            const cur = @import("../arch/x86_64/cpu_local.zig").get_cpu_local().current_thread orelse {
                self.lock.unlock_irqrestore(flags);
                return;
            };
            var already_in = false;
            var curr_w = self.recv_waiters;
            while (curr_w) |w| {
                if (w == cur) { already_in = true; break; }
                curr_w = w.next;
            }
            if (!already_in) {
                cur.next = self.recv_waiters;
                self.recv_waiters = cur;
            }
            @atomicStore(bool, &cur.yielded, false, .release);
            self.lock.unlock_irqrestore(flags);
            sched.block();
            // Woken by a send — return so cap_poll can re-check readiness.
            return;
        }
    }

    /// Check if the port has pending messages.
    pub fn hasPending(self: *const Port) bool {
        return self.count > 0;
    }

    /// Check if the port's queue is full.
    pub fn isFull(self: *const Port) bool {
        return self.count >= PORT_QUEUE_CAPACITY;
    }

    /// Close the port. Pending messages are discarded. Wake all waiters.
    pub fn close(self: *Port) void {
        const flags = self.lock.lock_irqsave();
        self.state = .closed;
        self.count = 0;
        self.head = 0;
        self.tail = 0;
        
        var rw = self.recv_waiters;
        self.recv_waiters = null;
        var sw = self.send_waiters;
        self.send_waiters = null;
        self.lock.unlock_irqrestore(flags);

        while (rw) |w| {
            const nxt = w.next;
            w.next = null;
            sched.wake(w);
            rw = nxt;
        }
        while (sw) |w| {
            const nxt = w.next;
            w.next = null;
            sched.wake(w);
            sw = nxt;
        }
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
