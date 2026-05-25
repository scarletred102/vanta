# VantaOS Virtual File System (VFS) Protocol (Phase 7)

This document is the absolute specification for the VantaOS VFS protocol. Every filesystem provider (e.g., `tmpfs`, `VantaFS`) and the namespace mount server MUST implement this exact protocol. Deviation is a bug.

---

## 1. IPC Protocol Message Codes

The VFS protocol operates on stable IPC message types defined as follows:

- **`MSG_FS_OPEN` (`0x0100`)**: Open a file or directory, returning a session-scoped file descriptor endpoint capability (`FdCap`).
- **`MSG_FS_READ` (`0x0101`)**: Read block data from a session `FdCap` using zero-copy shared memory.
- **`MSG_FS_WRITE` (`0x0102`)**: Write block data to a session `FdCap` using zero-copy shared memory.
- **`MSG_FS_CLOSE` (`0x0103`)**: Close a file session, terminating the `FdCap` session endpoint.
- **`MSG_FS_STAT` (`0x0104`)**: Query file/directory metadata.
- **`MSG_FS_READDIR` (`0x0105`)**: Read directory entries from a directory session `FdCap`.
- **`MSG_FS_MKDIR` (`0x0106`)**: Create a directory.
- **`MSG_FS_UNLINK` (`0x0107`)**: Delete a file or empty directory.
- **`MSG_FS_RENAME` (`0x0108`)**: Rename or move a filesystem entry.

---

## 2. Session Endpoint Design (`FdCap`)

To prevent global file descriptor tables and maintain strict security, `MSG_FS_OPEN` returns a **derived Endpoint capability (`FdCap`)** representing an active, stateful open file session. 

* All subsequent data operations (`MSG_FS_READ`, `MSG_FS_WRITE`, `MSG_FS_READDIR`, `MSG_FS_CLOSE`) are sent **directly to the `FdCap` endpoint handle** rather than the main filesystem service port.
* Stateful properties (such as current file read/write offset or inode bounds) are tracked inside the server and associated with the respective active session.
* When `MSG_FS_CLOSE` is issued, the server terminates the stateful session, revokes/invalidates all derived child capabilities, and frees the session port.

---

## 3. Message Payload Offsets & Formats

The standard VantaOS `Message` payload is exactly 64 bytes. Offsets and data packing must adhere to the following layouts:

### `MSG_FS_OPEN` (0x0100)
- **Request Payload**:
  - `0..4`: `flags` (u32, where `1 = O_RDONLY`, `2 = O_WRONLY`, `4 = O_RDWR`, `8 = O_CREAT`)
  - `4..64`: `path` (null-terminated UTF-8 string, max 59 bytes)
- **Response Payload / Capabilities**:
  - `caps[0]`: The newly derived `FdCap` endpoint capability.

### `MSG_FS_READ` (0x0101) & `MSG_FS_WRITE` (0x0102)
- **Request Payload**:
  - `0..8`: `offset` (u64 logical byte offset in the file)
  - `8..16`: `len` (u64 byte length to transfer)
- **Request Capabilities**:
  - `buffer_cap`: Shared Memory (`MemoryCap`) capability containing/receiving the data.
- **Response Payload**:
  - `0..8`: `bytes_transferred` (u64 number of bytes successfully read/written)

### `MSG_FS_CLOSE` (0x0103)
- **Request**: Sent directly to the stateful `FdCap` endpoint. No extra payload needed.
- **Response**: Terminates/revokes the capability.

### `MSG_FS_STAT` (0x0104)
- **Request Payload**:
  - `0..64`: `path` (null-terminated UTF-8 string, max 63 bytes)
- **Response Payload**:
  - `0..8`: `size` (u64 size in bytes)
  - `8`: `is_dir` (u8, `1 = directory`, `0 = file`)
  - `9..64`: `reserved`

### `MSG_FS_READDIR` (0x0105)
- **Request Payload**:
  - `0..8`: `offset` (u64 directory entry index offset)
- **Request Capabilities**:
  - `buffer_cap`: Shared Memory (`MemoryCap`) capability receiving the packed list of `DirEntry` structures.
- **Response Payload**:
  - `0..8`: `entry_count` (u64 number of directory entries written to the buffer)

### `MSG_FS_MKDIR` (0x0106) & `MSG_FS_UNLINK` (0x0107)
- **Request Payload**:
  - `0..64`: `path` (null-terminated UTF-8 string, max 63 bytes)
- **Response**: IPC reply indicating success or `MSG_ERROR`.

### `MSG_FS_RENAME` (0x0108)
- **Request Payload**:
  - `0..32`: `src_path` (null-terminated UTF-8 string, max 31 bytes)
  - `32..64`: `dst_path` (null-terminated UTF-8 string, max 31 bytes)
- **Response**: IPC reply indicating success or `MSG_ERROR`.

---

## 4. Directory Entry Layout (`DirEntry`)

When calling `MSG_FS_READDIR`, the list of directory entries returned in the `buffer_cap` shared memory must be packed sequentially as a series of `DirEntry` structs:

```zig
pub const DirEntry = extern struct {
    ino: u64,            // Inode number
    is_dir: u8,          // 1 = directory, 0 = file
    name_len: u8,        // Length of the entry name
    name: [62]u8,        // Null-terminated entry name
};
```
* Each `DirEntry` occupies exactly 72 bytes.
* Entries are packed back-to-back inside the shared memory page.

---

## 5. Zero-Allocation Zero-Copy Bulk I/O Invariant

**Reads and writes larger than 32 bytes MUST use the `buffer_cap` shared memory capability.** 
Inline data transfers inside the 64-byte payload are strictly prohibited for file block content. Filesystem servers map the caller-provided `buffer_cap` to their own virtual range, execute the required physical hardware sector transfers or memory operations directly, and immediately unmap/revoke the buffer capability to maintain maximum execution performance and prevent address space pollution.
