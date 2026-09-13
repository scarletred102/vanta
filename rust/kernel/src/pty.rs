//! Pseudo-Terminal (PTY) subsystem and in-kernel line discipline.
//!
//! Provides master/slave PTY pair multiplexing, canonical line editing,
//! raw pass-through, signal generation (SIGINT, SIGTSTP, SIGQUIT),
//! window resizing (TIOCSWINSZ -> SIGWINCH), and controlling terminal management.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

/// Standard termios flags and constants.
pub const ICRNL: u32 = 0x0100;
pub const IXON: u32 = 0x0400;
pub const OPOST: u32 = 0x0001;
pub const ONLCR: u32 = 0x0004;
pub const CS8: u32 = 0x0030;
pub const CREAD: u32 = 0x0080;
pub const B38400: u32 = 0x000f;
pub const ISIG: u32 = 0x0001;
pub const ICANON: u32 = 0x0002;
pub const ECHO: u32 = 0x0008;
pub const ECHOE: u32 = 0x0010;
pub const ECHOK: u32 = 0x0020;
pub const ECHOCTL: u32 = 0x0200;
pub const ECHOKE: u32 = 0x0800;
pub const IEXTEN: u32 = 0x8000;

/// Linux termios structure (60 bytes on Linux x86_64).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; 32],
    pub __c_ispeed: u32,
    pub __c_ospeed: u32,
}

impl Default for Termios {
    fn default() -> Self {
        let mut cc = [0u8; 32];
        cc[0] = 3;   // VINTR (^C)
        cc[1] = 28;  // VQUIT (^\)
        cc[2] = 127; // VERASE (DEL/Backspace)
        cc[3] = 21;  // VKILL (^U)
        cc[4] = 4;   // VEOF (^D)
        cc[5] = 0;   // VTIME
        cc[6] = 1;   // VMIN
        cc[7] = 0;   // VSWTC
        cc[8] = 17;  // VSTART (^Q)
        cc[9] = 19;  // VSTOP (^S)
        cc[10] = 26; // VSUSP (^Z)
        cc[11] = 0;  // VEOL
        cc[12] = 18; // VREPRINT (^R)
        cc[13] = 15; // VDISCARD (^O)
        cc[14] = 23; // VWERASE (^W)
        cc[15] = 22; // VLNEXT (^V)
        cc[16] = 0;  // VEOL2

        Self {
            c_iflag: ICRNL | IXON,
            c_oflag: OPOST | ONLCR,
            c_cflag: B38400 | CS8 | CREAD,
            c_lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | ECHOKE | IEXTEN,
            c_line: 0,
            c_cc: cc,
            __c_ispeed: 38400,
            __c_ospeed: 38400,
        }
    }
}

/// Linux winsize structure (8 bytes).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct WinSize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl Default for WinSize {
    fn default() -> Self {
        Self {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 1280,
            ws_ypixel: 800,
        }
    }
}

/// In-kernel state for a single PTY pair.
pub struct PtyState {
    pub id: u32,
    pub master_open: bool,
    pub slave_open: bool,
    pub locked: bool,
    pub master_to_slave: Vec<u8>,
    pub line_buf: Vec<u8>,
    pub slave_to_master: Vec<u8>,
    pub termios: Termios,
    pub winsize: WinSize,
    pub fg_pgrp: u32,
    pub session_id: u32,
}

impl PtyState {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            master_open: true,
            slave_open: false,
            locked: true,
            master_to_slave: Vec::new(),
            line_buf: Vec::new(),
            slave_to_master: Vec::new(),
            termios: Termios::default(),
            winsize: WinSize::default(),
            fg_pgrp: 0,
            session_id: 0,
        }
    }

    /// Process input written to the master by the terminal emulator.
    /// Applies line discipline: ISIG signals, canonical line buffering, backspace editing, and echo.
    pub fn write_master(&mut self, bytes: &[u8]) {
        let isig = self.termios.c_lflag & ISIG != 0;
        let icanon = self.termios.c_lflag & ICANON != 0;
        let echo = self.termios.c_lflag & ECHO != 0;
        let echoe = self.termios.c_lflag & ECHOE != 0;
        let onlcr = self.termios.c_oflag & ONLCR != 0;
        let icrnl = self.termios.c_iflag & ICRNL != 0;

        let vintr = self.termios.c_cc[0];
        let vquit = self.termios.c_cc[1];
        let verase = self.termios.c_cc[2];
        let veof = self.termios.c_cc[4];
        let vsusp = self.termios.c_cc[10];

        for &raw_byte in bytes {
            // Signal characters (ISIG)
            if isig {
                if raw_byte == vintr && vintr != 0 {
                    // Ctrl+C -> SIGINT (2)
                    self.line_buf.clear();
                    if echo {
                        self.slave_to_master.extend_from_slice(b"^C\n");
                    }
                    if self.fg_pgrp != 0 {
                        let _ = crate::scheduler::signal_pgrp(self.fg_pgrp, 2);
                    }
                    continue;
                } else if raw_byte == vsusp && vsusp != 0 {
                    // Ctrl+Z -> SIGTSTP (20)
                    if echo {
                        self.slave_to_master.extend_from_slice(b"^Z\n");
                    }
                    if self.fg_pgrp != 0 {
                        let _ = crate::scheduler::signal_pgrp(self.fg_pgrp, 20);
                    }
                    continue;
                } else if raw_byte == vquit && vquit != 0 {
                    // Ctrl+\ -> SIGQUIT (3)
                    if echo {
                        self.slave_to_master.extend_from_slice(b"^\\\n");
                    }
                    if self.fg_pgrp != 0 {
                        let _ = crate::scheduler::signal_pgrp(self.fg_pgrp, 3);
                    }
                    continue;
                }
            }

            if icanon {
                // Canonical line buffering
                if (raw_byte == verase && verase != 0) || raw_byte == 0x7f || raw_byte == 0x08 {
                    // Backspace
                    if !self.line_buf.is_empty() {
                        self.line_buf.pop();
                        if echo && echoe {
                            // Backspace-space-backspace to visually erase in terminal
                            self.slave_to_master.extend_from_slice(b"\x08 \x08");
                        }
                    }
                } else if raw_byte == veof && veof != 0 {
                    // Ctrl+D / EOF
                    if !self.line_buf.is_empty() {
                        self.master_to_slave.extend(self.line_buf.drain(..));
                    }
                } else if raw_byte == b'\r' || raw_byte == b'\n' {
                    let ch = if raw_byte == b'\r' && icrnl { b'\n' } else { raw_byte };
                    self.line_buf.push(ch);
                    if echo {
                        if ch == b'\n' && onlcr {
                            self.slave_to_master.extend_from_slice(b"\r\n");
                        } else {
                            self.slave_to_master.push(ch);
                        }
                    }
                    self.master_to_slave.extend(self.line_buf.drain(..));
                } else {
                    self.line_buf.push(raw_byte);
                    if echo {
                        self.slave_to_master.push(raw_byte);
                    }
                }
            } else {
                // Raw mode: immediate pass-through
                let ch = if raw_byte == b'\r' && icrnl { b'\n' } else { raw_byte };
                self.master_to_slave.push(ch);
                if echo {
                    if ch == b'\n' && onlcr {
                        self.slave_to_master.extend_from_slice(b"\r\n");
                    } else {
                        self.slave_to_master.push(ch);
                    }
                }
            }
        }
    }

    /// Read available data on the slave side (e.g. from the shell).
    pub fn read_slave(&mut self, length: usize) -> Result<Vec<u8>, ()> {
        if self.master_to_slave.is_empty() {
            if !self.master_open {
                // Master closed: return EOF (0 bytes)
                return Ok(Vec::new());
            }
            return Err(()); // WouldBlock
        }
        let count = self.master_to_slave.len().min(length);
        let bytes = self.master_to_slave.drain(..count).collect();
        Ok(bytes)
    }

    /// Write data from the slave side (e.g. shell output).
    pub fn write_slave(&mut self, bytes: &[u8]) -> Result<usize, ()> {
        if !self.master_open {
            return Err(()); // EIO / broken pipe
        }
        let onlcr = self.termios.c_oflag & ONLCR != 0;
        for &byte in bytes {
            if byte == b'\n' && onlcr {
                self.slave_to_master.extend_from_slice(b"\r\n");
            } else {
                self.slave_to_master.push(byte);
            }
        }
        Ok(bytes.len())
    }

    /// Read data from the master side (terminal emulator reading shell output).
    pub fn read_master(&mut self, length: usize) -> Result<Vec<u8>, ()> {
        if self.slave_to_master.is_empty() {
            if !self.slave_open {
                // Slave closed: return EOF
                return Ok(Vec::new());
            }
            return Err(()); // WouldBlock
        }
        let count = self.slave_to_master.len().min(length);
        let bytes = self.slave_to_master.drain(..count).collect();
        Ok(bytes)
    }
}

static PTY_TABLE: Mutex<BTreeMap<u32, Arc<Mutex<PtyState>>>> = Mutex::new(BTreeMap::new());
static NEXT_PTY_ID: Mutex<u32> = Mutex::new(0);

/// Allocate a new master/slave PTY pair.
pub fn create_pty() -> (u32, Arc<Mutex<PtyState>>) {
    let mut table = PTY_TABLE.lock();
    let mut next_id = NEXT_PTY_ID.lock();
    let id = *next_id;
    *next_id = next_id.wrapping_add(1);

    let pty = Arc::new(Mutex::new(PtyState::new(id)));
    table.insert(id, Arc::clone(&pty));
    (id, pty)
}

/// Retrieve an active PTY by slave index.
pub fn get_pty(id: u32) -> Option<Arc<Mutex<PtyState>>> {
    PTY_TABLE.lock().get(&id).cloned()
}

/// Check whether a slave index exists.
pub fn has_pty(id: u32) -> bool {
    PTY_TABLE.lock().contains_key(&id)
}

/// List all active slave indices (for `/dev/pts` directory enumeration).
pub fn list_ptys() -> Vec<u32> {
    PTY_TABLE.lock().keys().copied().collect()
}

/// Close master side of PTY.
pub fn close_master(id: u32) {
    let mut table = PTY_TABLE.lock();
    let should_remove = if let Some(pty) = table.get(&id) {
        let mut p = pty.lock();
        p.master_open = false;
        !p.slave_open
    } else {
        false
    };
    if should_remove {
        table.remove(&id);
    }
}

/// Close slave side of PTY.
pub fn close_slave(id: u32) {
    let mut table = PTY_TABLE.lock();
    let should_remove = if let Some(pty) = table.get(&id) {
        let mut p = pty.lock();
        p.slave_open = false;
        !p.master_open
    } else {
        false
    };
    if should_remove {
        table.remove(&id);
    }
}
