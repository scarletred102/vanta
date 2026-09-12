//! AF_UNIX / AF_LOCAL Domain Sockets for Vanta OS
//!
//! Provides high-throughput local inter-process communication:
//! - SOCK_STREAM (connection-oriented, bidirectional stream byte pipe)
//! - SOCK_DGRAM (connectionless datagram preserving message boundaries)
//! - socketpair() creation of bidirectional connected socket pairs
//! - SCM_RIGHTS file descriptor passing across process boundaries
//! - Filesystem path binding (e.g. /tmp/afunix.sock)

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use crate::scheduler::FileDescriptor;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnixAddress {
    Path(String),
    Abstract(Vec<u8>),
}

static NEXT_UNIX_SOCKET_ID: AtomicU64 = AtomicU64::new(0x6000_0000);
static UNIX_REGISTRY: Mutex<BTreeMap<UnixAddress, Weak<Mutex<AfUnixSocket>>>> = Mutex::new(BTreeMap::new());

pub struct UnixDatagram {
    pub sender_addr: Option<UnixAddress>,
    pub data: Vec<u8>,
    pub(crate) fds: Vec<FileDescriptor>,
}

pub struct StreamEndpoint {
    pub peer: Option<Weak<Mutex<AfUnixSocket>>>,
    pub rx_buf: Vec<u8>,
    pub(crate) rx_fds: Vec<FileDescriptor>,
    pub closed: bool,
    pub peer_closed: bool,
}

pub struct DatagramEndpoint {
    pub bound_addr: Option<UnixAddress>,
    pub peer: Option<Weak<Mutex<AfUnixSocket>>>,
    pub rx_queue: Vec<UnixDatagram>,
}

pub struct ListenerEndpoint {
    pub bound_addr: UnixAddress,
    pub backlog: usize,
    pub accept_queue: Vec<Arc<Mutex<AfUnixSocket>>>,
}

pub enum AfUnixSocketKind {
    Stream(StreamEndpoint),
    Datagram(DatagramEndpoint),
    Listener(ListenerEndpoint),
}

pub struct AfUnixSocket {
    pub id: u64,
    pub kind: AfUnixSocketKind,
    pub nonblocking: bool,
}

impl AfUnixSocket {
    pub fn new_stream() -> Self {
        Self {
            id: NEXT_UNIX_SOCKET_ID.fetch_add(1, Ordering::Relaxed),
            kind: AfUnixSocketKind::Stream(StreamEndpoint {
                peer: None,
                rx_buf: Vec::new(),
                rx_fds: Vec::new(),
                closed: false,
                peer_closed: false,
            }),
            nonblocking: false,
        }
    }

    pub fn new_datagram() -> Self {
        Self {
            id: NEXT_UNIX_SOCKET_ID.fetch_add(1, Ordering::Relaxed),
            kind: AfUnixSocketKind::Datagram(DatagramEndpoint {
                bound_addr: None,
                peer: None,
                rx_queue: Vec::new(),
            }),
            nonblocking: false,
        }
    }

    pub fn has_pending_data(&self) -> bool {
        match &self.kind {
            AfUnixSocketKind::Stream(s) => !s.rx_buf.is_empty() || s.peer_closed,
            AfUnixSocketKind::Datagram(d) => !d.rx_queue.is_empty(),
            AfUnixSocketKind::Listener(l) => !l.accept_queue.is_empty(),
        }
    }

    pub fn can_write(&self) -> bool {
        match &self.kind {
            AfUnixSocketKind::Stream(s) => !s.closed && !s.peer_closed,
            AfUnixSocketKind::Datagram(_) => true,
            AfUnixSocketKind::Listener(_) => false,
        }
    }

    pub fn close(&mut self) {
        let peer_opt = match &mut self.kind {
            AfUnixSocketKind::Stream(s) => {
                s.closed = true;
                s.peer.as_ref().and_then(|w| w.upgrade())
            }
            AfUnixSocketKind::Datagram(d) => {
                if let Some(addr) = &d.bound_addr {
                    UNIX_REGISTRY.lock().remove(addr);
                }
                None
            }
            AfUnixSocketKind::Listener(l) => {
                UNIX_REGISTRY.lock().remove(&l.bound_addr);
                None
            }
        };

        if let Some(peer_arc) = peer_opt {
            let mut peer = peer_arc.lock();
            if let AfUnixSocketKind::Stream(ps) = &mut peer.kind {
                ps.peer_closed = true;
            }
            crate::scheduler::wake_pipe_waiters(peer.id);
        }

        crate::scheduler::wake_pipe_waiters(self.id);
    }
}

pub fn create_socket(socket_type: u32) -> Result<Arc<Mutex<AfUnixSocket>>, ()> {
    match socket_type & 0xf {
        1 => Ok(Arc::new(Mutex::new(AfUnixSocket::new_stream()))),
        2 => Ok(Arc::new(Mutex::new(AfUnixSocket::new_datagram()))),
        _ => Err(()),
    }
}

pub fn create_socketpair(socket_type: u32) -> Result<(Arc<Mutex<AfUnixSocket>>, Arc<Mutex<AfUnixSocket>>), ()> {
    match socket_type & 0xf {
        1 => {
            // SOCK_STREAM
            let sock_a = Arc::new(Mutex::new(AfUnixSocket::new_stream()));
            let sock_b = Arc::new(Mutex::new(AfUnixSocket::new_stream()));

            if let AfUnixSocketKind::Stream(ref mut s_a) = sock_a.lock().kind {
                s_a.peer = Some(Arc::downgrade(&sock_b));
            }
            if let AfUnixSocketKind::Stream(ref mut s_b) = sock_b.lock().kind {
                s_b.peer = Some(Arc::downgrade(&sock_a));
            }

            Ok((sock_a, sock_b))
        }
        2 => {
            // SOCK_DGRAM
            let sock_a = Arc::new(Mutex::new(AfUnixSocket::new_datagram()));
            let sock_b = Arc::new(Mutex::new(AfUnixSocket::new_datagram()));

            if let AfUnixSocketKind::Datagram(ref mut d_a) = sock_a.lock().kind {
                d_a.peer = Some(Arc::downgrade(&sock_b));
            }
            if let AfUnixSocketKind::Datagram(ref mut d_b) = sock_b.lock().kind {
                d_b.peer = Some(Arc::downgrade(&sock_a));
            }

            Ok((sock_a, sock_b))
        }
        _ => Err(()),
    }
}

pub(crate) fn send_af_unix(
    sock: &Arc<Mutex<AfUnixSocket>>,
    data: &[u8],
    passed_fds: Vec<FileDescriptor>,
) -> Result<usize, ()> {
    let (peer_arc, is_stream) = {
        let sock_guard = sock.lock();
        match &sock_guard.kind {
            AfUnixSocketKind::Stream(s) => {
                if s.closed || s.peer_closed {
                    return Err(());
                }
                let weak = s.peer.as_ref().ok_or(())?;
                (weak.upgrade().ok_or(())?, true)
            }
            AfUnixSocketKind::Datagram(d) => {
                let weak = d.peer.as_ref().ok_or(())?;
                (weak.upgrade().ok_or(())?, false)
            }
            AfUnixSocketKind::Listener(_) => return Err(()),
        }
    };

    let peer_id = {
        let mut peer = peer_arc.lock();
        if is_stream {
            if let AfUnixSocketKind::Stream(ps) = &mut peer.kind {
                if ps.closed {
                    return Err(());
                }
                ps.rx_buf.extend_from_slice(data);
                ps.rx_fds.extend(passed_fds);
            } else {
                return Err(());
            }
        } else {
            if let AfUnixSocketKind::Datagram(pd) = &mut peer.kind {
                pd.rx_queue.push(UnixDatagram {
                    sender_addr: None,
                    data: data.to_vec(),
                    fds: passed_fds,
                });
            } else {
                return Err(());
            }
        }
        peer.id
    };
    crate::scheduler::wake_pipe_waiters(peer_id);
    Ok(data.len())
}

pub(crate) fn recv_af_unix(
    sock: &Arc<Mutex<AfUnixSocket>>,
    limit: usize,
) -> Result<(Vec<u8>, Vec<FileDescriptor>), ()> {
    let mut sock_guard = sock.lock();
    match &mut sock_guard.kind {
        AfUnixSocketKind::Stream(s) => {
            if !s.rx_buf.is_empty() {
                let take_len = s.rx_buf.len().min(limit);
                let out = s.rx_buf.drain(..take_len).collect();
                let fds = core::mem::take(&mut s.rx_fds);
                Ok((out, fds))
            } else if s.peer_closed {
                Ok((Vec::new(), Vec::new())) // EOF
            } else {
                Err(()) // WouldBlock
            }
        }
        AfUnixSocketKind::Datagram(d) => {
            if !d.rx_queue.is_empty() {
                let dg = d.rx_queue.remove(0);
                let take_len = dg.data.len().min(limit);
                let out = dg.data[..take_len].to_vec();
                Ok((out, dg.fds))
            } else {
                Err(()) // WouldBlock
            }
        }
        AfUnixSocketKind::Listener(_) => Err(()),
    }
}

pub fn bind_af_unix(sock: &Arc<Mutex<AfUnixSocket>>, addr: &UnixAddress) -> Result<(), ()> {
    let mut reg = UNIX_REGISTRY.lock();
    if let Some(existing) = reg.get(addr) {
        if existing.upgrade().is_some() {
            return Err(());
        }
    }
    let mut s = sock.lock();
    match &mut s.kind {
        AfUnixSocketKind::Stream(_) => {
            s.kind = AfUnixSocketKind::Listener(ListenerEndpoint {
                bound_addr: addr.clone(),
                backlog: 128,
                accept_queue: Vec::new(),
            });
            reg.insert(addr.clone(), Arc::downgrade(sock));
            Ok(())
        }
        AfUnixSocketKind::Datagram(d) => {
            d.bound_addr = Some(addr.clone());
            reg.insert(addr.clone(), Arc::downgrade(sock));
            Ok(())
        }
        AfUnixSocketKind::Listener(_) => Err(()),
    }
}

pub fn listen_af_unix(sock: &Arc<Mutex<AfUnixSocket>>, backlog: usize) -> Result<(), ()> {
    let mut s = sock.lock();
    if let AfUnixSocketKind::Listener(l) = &mut s.kind {
        l.backlog = if backlog == 0 { 128 } else { backlog.min(128) };
        Ok(())
    } else {
        Err(())
    }
}

pub fn connect_af_unix(sock: &Arc<Mutex<AfUnixSocket>>, addr: &UnixAddress) -> Result<(), ()> {
    let target = {
        let reg = UNIX_REGISTRY.lock();
        reg.get(addr).and_then(|w| w.upgrade()).ok_or(())?
    };
    let mut target_guard = target.lock();
    match &mut target_guard.kind {
        AfUnixSocketKind::Listener(listener) => {
            let server_sock = Arc::new(Mutex::new(AfUnixSocket::new_stream()));
            {
                let mut client = sock.lock();
                if let AfUnixSocketKind::Stream(ref mut client_stream) = client.kind {
                    client_stream.peer = Some(Arc::downgrade(&server_sock));
                } else {
                    return Err(());
                }
            }
            {
                let mut server = server_sock.lock();
                if let AfUnixSocketKind::Stream(ref mut server_stream) = server.kind {
                    server_stream.peer = Some(Arc::downgrade(sock));
                }
            }
            listener.accept_queue.push(server_sock);
            let target_id = target_guard.id;
            drop(target_guard);
            crate::scheduler::wake_pipe_waiters(target_id);
            Ok(())
        }
        AfUnixSocketKind::Datagram(_) => {
            if let AfUnixSocketKind::Datagram(ref mut d) = sock.lock().kind {
                d.peer = Some(Arc::downgrade(&target));
                Ok(())
            } else {
                Err(())
            }
        }
        _ => Err(()),
    }
}

pub fn accept_af_unix(
    sock: &Arc<Mutex<AfUnixSocket>>,
    nonblocking: bool,
) -> Result<Arc<Mutex<AfUnixSocket>>, ()> {
    let mut s = sock.lock();
    if let AfUnixSocketKind::Listener(l) = &mut s.kind {
        if !l.accept_queue.is_empty() {
            let conn = l.accept_queue.remove(0);
            if nonblocking {
                conn.lock().nonblocking = true;
            }
            Ok(conn)
        } else {
            Err(())
        }
    } else {
        Err(())
    }
}
