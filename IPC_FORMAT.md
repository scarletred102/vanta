# IPC_FORMAT.md — VantaOS IPC Message Format

All userspace servers in VantaOS MUST use this message format for IPC communication.

---

## Message Layout

### Header

Every message begins with an 8-byte header:

```
MessageHeader {
    msg_code:    u32,   // identifies the operation or reply type
    payload_len: u32,   // number of valid bytes in the inline payload
    cap_count:   u8,    // number of valid capability slots (0–4)
    reserved:    [3]u8, // must be zero
}
```

### Inline Payload Words

Immediately following the header are up to 4 inline u64 words (32 bytes total).
These carry small, fixed-size arguments (paths, flags, integer values).
`payload_len` tells the receiver how many bytes are meaningful.

### Capability Transfer Slots

After the inline words are up to 4 capability transfer slots (one u64 handle each).
`cap_count` tells the receiver how many slots are valid.
Unused slots MUST be zero.

### Full Wire Layout (summary)

```
Offset  Size  Field
------  ----  -----
0       4     msg_code
4       4     payload_len
8       1     cap_count
9       3     reserved (zero)
12      4     (padding to align payload)
16      32    inline payload (up to 4 × u64 words)
48      32    capability slots (up to 4 × u64 handles)
```
Total fixed message size: 80 bytes.

---

## Bulk Data

When the payload exceeds 32 bytes, the sender must:

1. Allocate a SharedMemory capability (syscall 16 `ShmCreate`).
2. Map it (syscall 17 `ShmMap`) and write the data there.
3. Pass the SharedMemory cap handle in `caps[0]` (or the first free cap slot).
4. Set `cap_count` accordingly.

The receiver maps the SharedMemory cap to read the data, then revokes it when done.

---

## Standard Error Codes

All servers return one of these i64 values in `payload[0..8]` (little-endian) when
`msg_code` is set to `MSG_ERROR` (0x0003):

| Name      | Value | Meaning                          |
|-----------|-------|----------------------------------|
| OK        |  0    | Success                          |
| EPERM     | -1    | Permission denied                |
| ENOENT    | -2    | Name or object not found         |
| EBUSY     | -3    | Resource busy / port full        |
| ETIMEOUT  | -4    | Operation timed out              |
| EINVAL    | -5    | Invalid argument                 |
| ENOSYS    | -6    | Syscall or operation not implemented |

---

## Registry-Specific Message Codes

The registry server (`sys.registry`) uses `msg_code` values in the `0x10` range.

### RegistryRegister — 0x10

Register a named service endpoint.

- `msg_code`: `0x10`
- Inline payload `[0..]`: service name as a null-terminated UTF-8 string (max 31 bytes + null).
- `caps[0]`: the endpoint capability to register (caller transfers ownership).
- `cap_count`: 1
- Reply `msg_code`: `0x10` on success, `MSG_ERROR` on failure.

### RegistryLookup — 0x11

Look up a registered service by name.

- `msg_code`: `0x11`
- Inline payload `[0..]`: service name as a null-terminated UTF-8 string.
- `cap_count`: 0
- Reply `msg_code`: `0x11` on success, `MSG_ERROR` (ENOENT) if not found.
- Reply `caps[0]`: a send-only derived EndpointCap (rights = `EndpointSend` = 1).
- Reply `cap_count`: 1

### RegistryList — 0x12

List all registered service names.

- `msg_code`: `0x12`
- No payload, no caps.
- Reply uses a SharedMemory capability (in `caps[0]`, `cap_count` = 1) containing all
  registered names as a newline-separated UTF-8 string.
- Reply `payload[0..8]`: total byte length of the list (little-endian u64).
