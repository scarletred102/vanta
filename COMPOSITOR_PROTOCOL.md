# VantaOS Compositor Protocol

## Overview

The compositor server (`sys.compositor`) manages on-screen surfaces and composites
them to the physical framebuffer at 60 Hz.  Clients hold a `SurfaceCap` (an
endpoint capability) and exchange messages with the compositor via standard VantaOS
IPC.

---

## Message Format

All messages use the standard `Message` struct:

```
msg_type  : u32
flags     : u32  (expects_reply, is_reply, …)
payload   : [64]u8
caps      : [4]u64   (outgoing cap handles)
buffer_cap: u64      (outgoing bulk-data ShmCap)
```

Integers inside `payload` are little-endian.

---

## Message Codes

### Client → Compositor

| Code   | Name                     | Payload                                       | Reply payload        |
|--------|--------------------------|-----------------------------------------------|----------------------|
| `0x30` | `CreateSurface`          | `[0..4]` width, `[4..8]` height               | `caps[0]` = SurfaceCap handle (endpoint cap) |
| `0x31` | `SwapBuffers`            | `[0..8]` surface_id; `buffer_cap` = ShmCap with pixel data (BGRA8) | none |
| `0x32` | `SetPosition`            | `[0..8]` surface_id, `[8..12]` x, `[12..16]` y | none |
| `0x33` | `SetZOrder`              | `[0..8]` surface_id, `[8..16]` z              | none |
| `0x34` | `DestroySurface`         | `[0..8]` surface_id                           | none |
| `0x35` | `QueryDisplay`           | none                                          | `[0..4]` width, `[4..8]` height |

### Compositor → Input Server (focus routing)

The compositor informs the input server which surface is focused by registering
a `NotificationCap` when creating a surface.  The input server delivers
`KeyEvent` and `MouseEvent` packets to that notification cap.

---

## Surface Pixel Format

All framebuffer data transferred via `buffer_cap` must be **BGRA8** (4 bytes per
pixel, blue in byte 0, alpha in byte 3) to match the virtio-gpu wire format and
the standard x86 VGA linear framebuffer byte order.

---

## Vsync

The compositor is self-paced at ~60 Hz using `vanta_cap_poll` with a 16 ms
timeout.  Clients that submit frames faster than 60 Hz are not throttled — the
most recently submitted buffer is composited on the next vsync boundary.

---

## Service Registry Name

`sys.compositor`

---

## Virtual Address Layout (compositor process)

| Range             | Purpose                         |
|-------------------|---------------------------------|
| `0x10000000+`     | heap (libvanta bump allocator)  |
| `0x50000000+`     | Limine/virtio-gpu framebuffer   |
| `0x60000000+`     | client surface backing buffers  |
