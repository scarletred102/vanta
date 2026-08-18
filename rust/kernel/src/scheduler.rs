//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use vanta_abi::{CapabilityId, Credentials, Rights};
use vanta_linuxd::{self, LinuxSigAction};

use crate::paging::{self, AddressSpace};
use crate::process::Process;
use crate::syscall::UserContext;

const TIMER_TICKS_PER_SLICE: u64 = 3;
const MAX_CPUS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InterruptContext {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    instruction_pointer: u64,
    code_segment: u64,
    flags: u64,
    stack_pointer: u64,
    stack_segment: u64,
}

impl InterruptContext {
    pub fn new(
        r15: u64,
        r14: u64,
        r13: u64,
        r12: u64,
        r11: u64,
        r10: u64,
        r9: u64,
        r8: u64,
        rdi: u64,
        rsi: u64,
        rbp: u64,
        rdx: u64,
        rcx: u64,
        rbx: u64,
        rax: u64,
        instruction_pointer: u64,
        code_segment: u64,
        flags: u64,
        stack_pointer: u64,
        stack_segment: u64,
    ) -> Self {
        Self {
            r15,
            r14,
            r13,
            r12,
            r11,
            r10,
            r9,
            r8,
            rdi,
            rsi,
            rbp,
            rdx,
            rcx,
            rbx,
            rax,
            instruction_pointer,
            code_segment,
            flags,
            stack_pointer,
            stack_segment,
        }
    }

    fn initial(entry: u64, stack_pointer: u64) -> Self {
        let (code_segment, stack_segment) = crate::gdt::user_interrupt_selectors();
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: 0,
            rsi: 0,
            rbp: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            instruction_pointer: entry,
            code_segment,
            flags: 0x202,
            stack_pointer,
            stack_segment,
        }
    }

    pub fn interrupted_user_mode(&self) -> bool {
        self.code_segment & 3 == 3
    }
}

struct Task {
    tid: u64,
    tgid: u64,
    parent_pid: Option<u64>,
    state: TaskState,
    process: Option<Arc<Mutex<Process>>>,
    context: UserContext,
    interrupt_context: InterruptContext,
    descriptors: Arc<Mutex<Vec<Option<FileDescriptor>>>>,
    credentials: Credentials,
    signal_actions: Arc<Mutex<[LinuxSigAction; 65]>>,
    fs_base: u64,
    blocked_mask: u64,
    pending_signals: u64,
    clear_child_tid: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Waiting { child_pid: u64, status_ptr: u64 },
    PipeWaiting { pipe_id: u64 },
    FutexWait { uaddr: u64, bitset: u32 },
    Zombie { exit_code: u64 },
    Reaped,
}

#[derive(Clone)]
struct FileDescriptor {
    capability: CapabilityId,
    rights: Rights,
    resource: DescriptorResource,
}

#[derive(Clone)]
enum DescriptorResource {
    File(Arc<Mutex<OpenFile>>),
    Directory(Arc<Mutex<OpenDirectory>>),
    Serial,
    Tty,
    PipeRead(Arc<Mutex<PipeReader>>),
    PipeWrite(Arc<Mutex<PipeWriter>>),
    Socket(Arc<Mutex<OpenSocket>>),
    Ipc(Arc<Mutex<IpcEndpoint>>),
    Epoll(Arc<Mutex<EpollInstance>>),
    EventFd(Arc<Mutex<EventFdInstance>>),
    PtyMaster(Arc<Mutex<PtyState>>),
    PtySlave(Arc<Mutex<PtyState>>),
}

pub struct EpollItem {
    pub fd: u64,
    pub events: u32,
    pub data: u64,
}

pub struct EpollInstance {
    pub items: Vec<EpollItem>,
}

pub struct EventFdInstance {
    pub counter: u64,
    pub flags: u32,
}

pub struct PtyState {
    pub master_to_slave: Vec<u8>,
    pub slave_to_master: Vec<u8>,
    pub rows: u16,
    pub cols: u16,
    pub raw: bool,
}

struct OpenFile {
    path: String,
    contents: Vec<u8>,
    offset: usize,
    writable: bool,
}

struct OpenDirectory {
    entries: Vec<String>,
    offset: usize,
}

struct OpenSocket {
    connection: Option<crate::network::TcpConnection>,
}

struct IpcEndpoint {
    state: Arc<Mutex<IpcState>>,
    send: bool,
}

struct IpcState {
    id: u64,
    queue: Vec<IpcMessage>,
    revoked: bool,
}

struct IpcMessage {
    sender_pid: u64,
    bytes: Vec<u8>,
}

const IPC_QUEUE_LIMIT: usize = 8;
const IPC_MESSAGE_LIMIT: usize = 256;

struct Pipe;

struct PipeReader {
    state: Arc<Mutex<PipeState>>,
}

struct PipeWriter {
    state: Arc<Mutex<PipeState>>,
}

struct PipeState {
    id: u64,
    bytes: Vec<u8>,
    writer_open: bool,
}

impl Pipe {
    fn new() -> (PipeReader, PipeWriter) {
        let state = Arc::new(Mutex::new(PipeState {
            id: NEXT_PIPE_ID.fetch_add(1, Ordering::Relaxed),
            bytes: Vec::new(),
            writer_open: true,
        }));
        (
            PipeReader {
                state: Arc::clone(&state),
            },
            PipeWriter { state },
        )
    }
}

impl PipeReader {
    fn read(&mut self, length: usize) -> Vec<u8> {
        let mut state = self.state.lock();
        let length = length.min(state.bytes.len());
        state.bytes.drain(..length).collect()
    }
}

impl PipeWriter {
    fn write(&mut self, bytes: &[u8]) -> u64 {
        let mut state = self.state.lock();
        if state.writer_open {
            state.bytes.extend_from_slice(bytes);
        }
        state.id
    }

    fn close(&mut self) -> u64 {
        let mut state = self.state.lock();
        state.writer_open = false;
        state.id
    }
}

fn close_pipe_writer(writer: Arc<Mutex<PipeWriter>>) {
    if Arc::strong_count(&writer) == 1 {
        let pipe_id = writer.lock().close();
        wake_pipe_waiters(pipe_id);
    }
}

fn wake_pipe_waiters(pipe_id: u64) {
    let mut scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return;
    };
    for task in &mut scheduler.tasks {
        if task.state == (TaskState::PipeWaiting { pipe_id }) {
            task.state = TaskState::Runnable;
        }
    }
}

fn futex_wake_unlocked(scheduler: &mut Scheduler, uaddr: u64, count: u32, bitset: u32) -> u64 {
    let mut woken = 0u64;
    for task in &mut scheduler.tasks {
        if woken >= count as u64 {
            break;
        }
        if let TaskState::FutexWait {
            uaddr: w_uaddr,
            bitset: w_bitset,
        } = task.state
        {
            if w_uaddr == uaddr && (w_bitset & bitset) != 0 {
                task.state = TaskState::Runnable;
                woken += 1;
            }
        }
    }
    crate::serial_println!(
        "[futex] wake uaddr={:#x} count={} bitset={:#x} woken={}",
        uaddr,
        count,
        bitset,
        woken
    );
    woken
}

pub fn futex_wake(uaddr: u64, count: u32, bitset: u32) -> u64 {
    let mut scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return 0;
    };
    futex_wake_unlocked(scheduler, uaddr, count, bitset)
}

struct Scheduler {
    tasks: Vec<Task>,
    current: usize,
    kernel_space: AddressSpace,
    ticks: u64,
    slice_ticks: u64,
}

static SCHEDULERS: [Mutex<Option<Scheduler>>; MAX_CPUS] = [const { Mutex::new(None) }; MAX_CPUS];
static NEXT_PID: AtomicU64 = AtomicU64::new(1);
static NEXT_PIPE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_IPC_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_CAPABILITY_SLOT: AtomicU64 = AtomicU64::new(1);
static TTY_CTRL_HELD: AtomicBool = AtomicBool::new(false);
// The terminal has one foreground process for now.  This is deliberately
// narrower than a full POSIX process-group implementation, but prevents
// Ctrl-C from killing the shell while it is waiting for a foreground child.
static FOREGROUND_PID: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::{close_pipe_writer, decode_tty_scancode, Pipe};
    use alloc::sync::Arc;
    use spin::Mutex;

    #[test]
    fn pipe_preserves_order_then_reports_eof_after_writer_close() {
        let (mut reader, mut writer) = Pipe::new();
        writer.write(b"hello");
        assert_eq!(reader.read(2), b"he");
        writer.close();
        assert_eq!(reader.read(8), b"llo");
        assert!(reader.read(8).is_empty());
    }

    #[test]
    fn duplicated_pipe_writer_keeps_pipe_open_until_last_descriptor_closes() {
        let (_, writer) = Pipe::new();
        let first = Arc::new(Mutex::new(writer));
        let second = Arc::clone(&first);
        let state = Arc::clone(&first.lock().state);

        close_pipe_writer(first);
        assert!(state.lock().writer_open);
        close_pipe_writer(second);
        assert!(!state.lock().writer_open);
    }

    #[test]
    fn tty_decodes_printable_keys_and_line_endings() {
        assert_eq!(decode_tty_scancode(0x23), Some(b'h'));
        assert_eq!(decode_tty_scancode(0x1c), Some(b'\n'));
        assert_eq!(decode_tty_scancode(0xa3), None);
    }
}

fn current_scheduler() -> &'static Mutex<Option<Scheduler>> {
    let index = crate::syscall::current_cpu_index();
    &SCHEDULERS[index.min(MAX_CPUS - 1)]
}

fn allocate_pid() -> u64 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

fn allocate_capability() -> CapabilityId {
    CapabilityId::from_parts(
        NEXT_CAPABILITY_SLOT.fetch_add(1, Ordering::Relaxed) as u32,
        1,
    )
}

pub unsafe fn start(processes: Vec<Box<Process>>) -> ! {
    start_on_current_cpu(processes, "started", false)
}

pub unsafe fn start_native(processes: Vec<Box<Process>>) -> ! {
    start_on_current_cpu(processes, "native started", true)
}

pub unsafe fn start_ap(processes: Vec<Box<Process>>) -> ! {
    start_on_current_cpu(processes, "AP run queue started", false)
}

unsafe fn start_on_current_cpu(processes: Vec<Box<Process>>, label: &str, native_tty: bool) -> ! {
    let kernel_space = paging::current_address_space();
    if processes.is_empty() {
        crate::shell::run();
    }
    let tasks = processes
        .into_iter()
        .map(|process| {
            let pid = allocate_pid();
            new_task(
                pid,
                pid,
                None,
                Credentials::root(),
                process,
                standard_descriptors(native_tty),
            )
        })
        .collect();
    *current_scheduler().lock() = Some(Scheduler {
        tasks,
        current: 0,
        kernel_space,
        ticks: 0,
        slice_ticks: 0,
    });

    let (context, space) = current_target();
    crate::serial_println!(
        "[sched] cpu={} {} tasks={}",
        crate::syscall::current_cpu_index(),
        label,
        task_count()
    );
    crate::syscall::prepare_user_return(context, space);
    unsafe { crate::gdt::enter_user(context.instruction_pointer, context.stack_pointer) }
}

pub fn yield_current(context: UserContext) -> *const UserContext {
    let (next_context, next_space, previous, next) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("yield without scheduler");
        let previous = scheduler.current;
        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous]
            .interrupt_context
            .instruction_pointer = context.instruction_pointer;
        scheduler.tasks[previous].interrupt_context.rbx = context.rbx;
        scheduler.tasks[previous].interrupt_context.rbp = context.rbp;
        scheduler.tasks[previous].interrupt_context.r12 = context.r12;
        scheduler.tasks[previous].interrupt_context.r13 = context.r13;
        scheduler.tasks[previous].interrupt_context.r14 = context.r14;
        scheduler.tasks[previous].interrupt_context.r15 = context.r15;
        scheduler.tasks[previous].interrupt_context.rax = context.return_value;
        scheduler.tasks[previous].interrupt_context.flags = context.flags;
        scheduler.tasks[previous].interrupt_context.stack_pointer = context.stack_pointer;
        let next = next_alive(scheduler, previous).unwrap_or(previous);
        scheduler.current = next;
        scheduler.slice_ticks = 0;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        let space = process.lock().address_space();
        (task.context, space, previous, next)
    };
    if previous != next {
        crate::serial_println!(
            "[sched] yield tid={} -> {}",
            scheduler_tid(previous),
            scheduler_tid(next)
        );
    }
    crate::syscall::prepare_user_return(next_context, next_space)
}

pub fn timer_tick(context: *mut InterruptContext) -> *const InterruptContext {
    let interrupted = unsafe { &*context };
    if !interrupted.interrupted_user_mode() {
        return context;
    }

    let next = {
        let mut scheduler = current_scheduler().lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return context;
        };
        scheduler.ticks = scheduler.ticks.wrapping_add(1);
        scheduler.slice_ticks += 1;
        if scheduler.slice_ticks < TIMER_TICKS_PER_SLICE {
            return context;
        }

        let previous = scheduler.current;
        let next = next_alive(scheduler, previous).unwrap_or(previous);
        scheduler.slice_ticks = 0;
        if previous == next {
            return context;
        }
        scheduler.tasks[previous].interrupt_context = unsafe { *context };
        scheduler.tasks[previous].context = UserContext {
            return_value: scheduler.tasks[previous].interrupt_context.rax,
            rbx: scheduler.tasks[previous].interrupt_context.rbx,
            rbp: scheduler.tasks[previous].interrupt_context.rbp,
            r12: scheduler.tasks[previous].interrupt_context.r12,
            r13: scheduler.tasks[previous].interrupt_context.r13,
            r14: scheduler.tasks[previous].interrupt_context.r14,
            r15: scheduler.tasks[previous].interrupt_context.r15,
            rdi: scheduler.tasks[previous].interrupt_context.rdi,
            rsi: scheduler.tasks[previous].interrupt_context.rsi,
            rdx: scheduler.tasks[previous].interrupt_context.rdx,
            r8: scheduler.tasks[previous].interrupt_context.r8,
            r9: scheduler.tasks[previous].interrupt_context.r9,
            r10: scheduler.tasks[previous].interrupt_context.r10,
            instruction_pointer: scheduler.tasks[previous]
                .interrupt_context
                .instruction_pointer,
            flags: scheduler.tasks[previous].interrupt_context.flags,
            stack_pointer: scheduler.tasks[previous].interrupt_context.stack_pointer,
        };
        scheduler.current = next;
        let previous_tid = scheduler.tasks[previous].tid;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        let space = process.lock().address_space();
        let target_rip = task.interrupt_context.instruction_pointer;
        let target_cs = task.interrupt_context.code_segment;
        let target_rsp = task.interrupt_context.stack_pointer;
        (
            &task.interrupt_context as *const InterruptContext,
            space,
            previous_tid,
            task.tid,
            target_rip,
            target_cs,
            target_rsp,
        )
    };

    unsafe {
        paging::activate(next.1);
    }
    crate::syscall::set_user_fs_base(current_fs_base());
    crate::serial_println!(
        "[sched] preempt tid={} -> {} rip={:#x} cs={:#x} rsp={:#x}",
        next.2,
        next.3,
        next.4,
        next.5,
        next.6
    );
    next.0
}

pub fn exit_current(code: u64) -> *const UserContext {
    let (next, remaining, parent_pid, exited_process) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("process exit without scheduler");
        let current = scheduler.current;
        let parent_pid = scheduler.tasks[current].parent_pid;
        let exited_tid = scheduler.tasks[current].tid;
        let exited_tgid = scheduler.tasks[current].tgid;

        // 1. Thread termination: clear_child_tid handling
        if scheduler.tasks[current].clear_child_tid != 0 {
            let clear_addr = scheduler.tasks[current].clear_child_tid;
            scheduler.tasks[current].clear_child_tid = 0;
            if let Some(ref proc_arc) = scheduler.tasks[current].process {
                let space = proc_arc.lock().address_space();
                let _ = crate::process::write_user_u32_in(space, clear_addr, 0);
            }
            futex_wake_unlocked(scheduler, clear_addr, 1, vanta_linuxd::FUTEX_BITSET_MATCH_ANY);
        }

        let process = scheduler.tasks[current]
            .process
            .take()
            .expect("current task already exited");
        scheduler.tasks[current].state = TaskState::Zombie { exit_code: code };

        if FOREGROUND_PID.load(AtomicOrdering::Relaxed) == exited_tgid
            || FOREGROUND_PID.load(AtomicOrdering::Relaxed) == exited_tid
        {
            FOREGROUND_PID.store(0, AtomicOrdering::Relaxed);
        }

        // Check if other threads in the same TGID are still alive
        let other_threads_alive = scheduler
            .tasks
            .iter()
            .any(|t| t.tgid == exited_tgid && t.process.is_some());

        if !other_threads_alive {
            // Last thread in thread group exited: wake parent waiting for this TGID
            if let Some(parent) = scheduler.tasks.iter_mut().find(|task| {
                task.tgid == parent_pid.unwrap_or(0)
                    && (matches!(task.state, TaskState::Waiting { child_pid, .. } if child_pid == exited_tgid || child_pid == u64::MAX || child_pid == 0))
            }) {
                let is_linux = parent.process.as_ref().map(|p| p.lock().personality() != crate::process::ProcessPersonality::NativeVanta).unwrap_or(false);
                if let TaskState::Waiting { status_ptr, .. } = parent.state {
                    if status_ptr != 0 {
                        if let Some(ref parent_proc) = parent.process {
                            let parent_space = parent_proc.lock().address_space();
                            let status: i32 = ((code as i32) & 0xff) << 8;
                            let _ = crate::process::write_user_u32_in(parent_space, status_ptr, status as u32);
                        }
                    }
                }
                let return_val = if is_linux { exited_tgid } else { code };
                parent.state = TaskState::Runnable;
                parent.context.return_value = return_val;
                parent.interrupt_context.rax = return_val;
            }
        }

        let kernel_space = scheduler.kernel_space;
        unsafe {
            paging::activate(kernel_space);
        }
        let remaining = scheduler
            .tasks
            .iter()
            .filter(|task| task.process.is_some())
            .count();
        let next = next_alive(scheduler, current).map(|index| {
            scheduler.current = index;
            let task = &mut scheduler.tasks[index];
            let process = task
                .process
                .as_mut()
                .expect("scheduler selected an exited task");
            let space = process.lock().address_space();
            (task.context, space, index)
        });
        (next, remaining, parent_pid, process)
    };

    drop(exited_process);

    crate::serial_println!(
        "[sched] task exited: code={} parent={} remaining={}",
        code,
        parent_pid.unwrap_or(0),
        remaining
    );
    let Some((context, space, next)) = next else {
        *current_scheduler().lock() = None;
        if crate::smp::is_application_processor() {
            crate::smp::on_user_task_complete();
            crate::smp::ap_idle();
        }
        x86_64::instructions::interrupts::enable();
        crate::shell::run()
    };
    crate::serial_println!("[sched] continue tid={}", scheduler_tid(next));
    crate::syscall::prepare_user_return(context, space)
}

pub fn exit_group_current(code: u64) -> *const UserContext {
    {
        let mut scheduler = current_scheduler().lock();
        if let Some(ref mut sched) = *scheduler {
            let current_tgid = sched.tasks[sched.current].tgid;
            let current_tid = sched.tasks[sched.current].tid;
            let mut clear_addrs = Vec::new();
            for task in &mut sched.tasks {
                if task.tgid == current_tgid && task.process.is_some() && task.tid != current_tid {
                    if task.clear_child_tid != 0 {
                        let clear_addr = task.clear_child_tid;
                        task.clear_child_tid = 0;
                        if let Some(ref proc_arc) = task.process {
                            let space = proc_arc.lock().address_space();
                            let _ = crate::process::write_user_u32_in(space, clear_addr, 0);
                        }
                        clear_addrs.push(clear_addr);
                    }
                    let proc = task.process.take();
                    task.state = TaskState::Zombie { exit_code: code };
                    drop(proc);
                }
            }
            for clear_addr in clear_addrs {
                futex_wake_unlocked(sched, clear_addr, 1, vanta_linuxd::FUTEX_BITSET_MATCH_ANY);
            }
        }
    }
    exit_current(code)
}

fn current_target() -> (UserContext, AddressSpace) {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().expect("scheduler not initialized");
    let current = scheduler.current;
    let task = &mut scheduler.tasks[current];
    let process = task
        .process
        .as_mut()
        .expect("scheduler current task is empty");
    let space = process.lock().address_space();
    (task.context, space)
}

fn next_alive(scheduler: &Scheduler, current: usize) -> Option<usize> {
    for offset in 1..=scheduler.tasks.len() {
        let index = (current + offset) % scheduler.tasks.len();
        if scheduler.tasks[index].state == TaskState::Runnable {
            return Some(index);
        }
    }
    None
}

pub fn current_pid() -> u64 {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].tgid)
        .unwrap_or(0)
}

pub fn current_tid() -> u64 {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].tid)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn current_tgid() -> u64 {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].tgid)
        .unwrap_or(0)
}

pub fn current_parent_pid() -> u64 {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.tasks[scheduler.current].parent_pid)
        .unwrap_or(0)
}

pub fn current_credentials() -> Credentials {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].credentials)
        .unwrap_or_else(Credentials::root)
}

pub const UNBLOCKABLE_SIGNALS_MASK: u64 = (1 << (9 - 1)) | (1 << (19 - 1));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalDefaultAction {
    Terminate,
    CoreDump,
    Ignore,
    Stop,
    Continue,
}

pub fn default_signal_action(signo: u64) -> SignalDefaultAction {
    match signo {
        17 /* SIGCHLD */ | 23 /* SIGURG */ | 28 /* SIGWINCH */ => SignalDefaultAction::Ignore,
        19 /* SIGSTOP */ | 20 /* SIGTSTP */ | 21 /* SIGTTIN */ | 22 /* SIGTTOU */ => SignalDefaultAction::Stop,
        18 /* SIGCONT */ => SignalDefaultAction::Continue,
        3 /* SIGQUIT */ | 4 /* SIGILL */ | 5 /* SIGTRAP */ | 6 /* SIGABRT */ | 7 /* SIGBUS */ | 8 /* SIGFPE */ | 11 /* SIGSEGV */ | 31 /* SIGSYS */ => SignalDefaultAction::CoreDump,
        _ => SignalDefaultAction::Terminate,
    }
}

pub fn current_blocked_mask() -> u64 {
    let scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return 0;
    };
    scheduler.tasks[scheduler.current].blocked_mask
}

pub fn set_current_blocked_mask(mask: u64) {
    let mut scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return;
    };
    scheduler.tasks[scheduler.current].blocked_mask = mask & !UNBLOCKABLE_SIGNALS_MASK;
}

#[allow(dead_code)]
pub fn current_pending_signals() -> u64 {
    let scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return 0;
    };
    scheduler.tasks[scheduler.current].pending_signals
}

pub fn signal_action(signal: u64) -> Option<LinuxSigAction> {
    let scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_ref()?;
    if signal == 0 || signal > 64 {
        return None;
    }
    let act = scheduler.tasks[scheduler.current].signal_actions.lock()[signal as usize];
    Some(act)
}

pub fn set_signal_action(signal: u64, action: LinuxSigAction) -> Result<LinuxSigAction, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    if signal == 0 || signal > 64 {
        return Err(());
    }
    if signal == 9 || signal == 19 {
        return Err(());
    }
    let mut slot = scheduler.tasks[scheduler.current].signal_actions.lock();
    let old = slot[signal as usize];
    slot[signal as usize] = action;
    Ok(old)
}

pub fn reset_signal_action(signal: u64) {
    let mut scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return;
    };
    if signal >= 1 && signal <= 64 {
        scheduler.tasks[scheduler.current].signal_actions.lock()[signal as usize] = LinuxSigAction::default();
    }
}

pub fn check_pending_signal_to_deliver() -> Option<(u64, LinuxSigAction)> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut()?;
    let task = &mut scheduler.tasks[scheduler.current];
    if task.process.is_none() {
        return None;
    }
    let deliverable = task.pending_signals & !task.blocked_mask;
    if deliverable == 0 {
        return None;
    }
    let bit = deliverable.trailing_zeros();
    let signo = (bit + 1) as u64;
    task.pending_signals &= !(1 << bit);
    let action = task.signal_actions.lock()[signo as usize];
    Some((signo, action))
}

pub fn kill_process(pid: u64, signal: u64) -> Result<(), ()> {
    if signal > 64 {
        return Err(());
    }
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let current_tgid = scheduler.tasks[scheduler.current].tgid;
    let target_tgid = if pid == 0 { current_tgid } else { pid };

    let matching_indices: Vec<usize> = scheduler
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.tgid == target_tgid && task.process.is_some())
        .map(|(idx, _)| idx)
        .collect();

    if matching_indices.is_empty() {
        return Err(());
    }

    if signal == 0 {
        return Ok(());
    }

    let default_act = default_signal_action(signal);
    let first_idx = matching_indices[0];
    let action = scheduler.tasks[first_idx].signal_actions.lock()[signal as usize];

    if signal == 9 || (action.sa_handler == 0 && (default_act == SignalDefaultAction::Terminate || default_act == SignalDefaultAction::CoreDump)) {
        let parent_pid = scheduler.tasks[first_idx].parent_pid;
        let mut clear_addrs = Vec::new();
        for &idx in &matching_indices {
            let task = &mut scheduler.tasks[idx];
            if task.clear_child_tid != 0 {
                let clear_addr = task.clear_child_tid;
                task.clear_child_tid = 0;
                if let Some(ref proc_arc) = task.process {
                    let space = proc_arc.lock().address_space();
                    let _ = crate::process::write_user_u32_in(space, clear_addr, 0);
                }
                clear_addrs.push(clear_addr);
            }
            let proc = task.process.take();
            task.state = TaskState::Zombie {
                exit_code: 128 + signal,
            };
            drop(proc);
        }
        for clear_addr in clear_addrs {
            futex_wake_unlocked(scheduler, clear_addr, 1, vanta_linuxd::FUTEX_BITSET_MATCH_ANY);
        }

        if FOREGROUND_PID.load(AtomicOrdering::Relaxed) == target_tgid {
            FOREGROUND_PID.store(0, AtomicOrdering::Relaxed);
        }

        if let Some(parent_pid) = parent_pid {
            if let Some(parent) = scheduler.tasks.iter_mut().find(|task| {
                task.tgid == parent_pid && (matches!(task.state, TaskState::Waiting { child_pid, .. } if child_pid == target_tgid || child_pid == u64::MAX || child_pid == 0))
            }) {
                let is_linux = parent.process.as_ref().map(|p| p.lock().personality() != crate::process::ProcessPersonality::NativeVanta).unwrap_or(false);
                if let TaskState::Waiting { status_ptr, .. } = parent.state {
                    if status_ptr != 0 {
                        if let Some(ref parent_proc) = parent.process {
                            let parent_space = parent_proc.lock().address_space();
                            let status: i32 = ((128 + signal) as i32) & 0x7f;
                            let _ = crate::process::write_user_u32_in(parent_space, status_ptr, status as u32);
                        }
                    }
                }
                let return_val = if is_linux { target_tgid } else { 128 + signal };
                parent.state = TaskState::Runnable;
                parent.context.return_value = return_val;
                parent.interrupt_context.rax = return_val;
            }
        }
        return Ok(());
    }

    if action.sa_handler == 1 || (action.sa_handler == 0 && default_act == SignalDefaultAction::Ignore) {
        return Ok(());
    }

    let target = &mut scheduler.tasks[first_idx];
    target.pending_signals |= 1 << (signal - 1);
    if matches!(target.state, TaskState::PipeWaiting { .. } | TaskState::FutexWait { .. }) {
        target.state = TaskState::Runnable;
    }
    Ok(())
}

pub fn kill_thread(tid: u64, signal: u64) -> Result<(), ()> {
    if signal > 64 {
        return Err(());
    }
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let target_idx = scheduler
        .tasks
        .iter()
        .position(|task| task.tid == tid && task.process.is_some())
        .ok_or(())?;

    if signal == 0 {
        return Ok(());
    }

    let action = scheduler.tasks[target_idx].signal_actions.lock()[signal as usize];
    let default_act = default_signal_action(signal);
    let target = &mut scheduler.tasks[target_idx];
    if signal == 9 {
        let clear_addr = if target.clear_child_tid != 0 {
            let addr = target.clear_child_tid;
            target.clear_child_tid = 0;
            if let Some(ref proc_arc) = target.process {
                let space = proc_arc.lock().address_space();
                let _ = crate::process::write_user_u32_in(space, addr, 0);
            }
            Some(addr)
        } else {
            None
        };
        let process = target.process.take();
        target.state = TaskState::Zombie {
            exit_code: 128 + signal,
        };
        drop(process);
        if let Some(addr) = clear_addr {
            futex_wake_unlocked(scheduler, addr, 1, vanta_linuxd::FUTEX_BITSET_MATCH_ANY);
        }
        return Ok(());
    }

    if action.sa_handler == 1 || (action.sa_handler == 0 && default_act == SignalDefaultAction::Ignore) {
        return Ok(());
    }
    if action.sa_handler == 0 && (default_act == SignalDefaultAction::Terminate || default_act == SignalDefaultAction::CoreDump) {
        let clear_addr = if target.clear_child_tid != 0 {
            let addr = target.clear_child_tid;
            target.clear_child_tid = 0;
            if let Some(ref proc_arc) = target.process {
                let space = proc_arc.lock().address_space();
                let _ = crate::process::write_user_u32_in(space, addr, 0);
            }
            Some(addr)
        } else {
            None
        };
        let process = target.process.take();
        target.state = TaskState::Zombie {
            exit_code: 128 + signal,
        };
        drop(process);
        if let Some(addr) = clear_addr {
            futex_wake_unlocked(scheduler, addr, 1, vanta_linuxd::FUTEX_BITSET_MATCH_ANY);
        }
        return Ok(());
    }

    target.pending_signals |= 1 << (signal - 1);
    if matches!(target.state, TaskState::PipeWaiting { .. } | TaskState::FutexWait { .. }) {
        target.state = TaskState::Runnable;
    }
    Ok(())
}

pub fn interrupt_current(signal: u64) {
    if signal == 0 || signal > 64 {
        return;
    }
    let target_pid = {
        let foreground = FOREGROUND_PID.load(AtomicOrdering::Relaxed);
        if foreground == 0 {
            current_pid()
        } else {
            foreground
        }
    };
    let _ = kill_process(target_pid, signal);
    crate::serial_println!("[signal] pid={} signal={}", target_pid, signal);
}

pub fn can_mutate_path(path: &str) -> bool {
    let scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return false;
    };
    let credentials = scheduler.tasks[scheduler.current].credentials;
    credentials.is_root()
        || path == "/tmp"
        || path.starts_with("/tmp/")
        || path == "/home/vanta"
        || path.starts_with("/home/vanta/")
}

pub fn spawn_current(process: Box<Process>) -> Result<u64, ()> {
    const MAX_TASKS: usize = 256;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let parent_tgid = scheduler.tasks[scheduler.current].tgid;
    let parent_credentials = scheduler.tasks[scheduler.current].credentials;
    let credentials = if parent_credentials.is_root() {
        Credentials::vanta()
    } else {
        parent_credentials
    };
    let descriptors = scheduler.tasks[scheduler.current].descriptors.lock().clone();
    let pid = allocate_pid();
    let task = new_task(
        pid,
        pid,
        Some(parent_tgid),
        credentials,
        process,
        descriptors,
    );
    if let Some(index) = scheduler
        .tasks
        .iter()
        .position(|t| t.state == TaskState::Reaped)
    {
        scheduler.tasks[index] = task;
    } else if scheduler.tasks.len() < MAX_TASKS {
        scheduler.tasks.push(task);
    } else {
        return Err(());
    }
    FOREGROUND_PID.store(pid, AtomicOrdering::Relaxed);
    Ok(pid)
}

pub fn spawn_with_stdio_current(
    process: Box<Process>,
    stdin: u64,
    stdout: u64,
    stderr: u64,
) -> Result<u64, ()> {
    const MAX_TASKS: usize = 256;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let parent_tgid = scheduler.tasks[scheduler.current].tgid;
    let parent_credentials = scheduler.tasks[scheduler.current].credentials;
    let credentials = if parent_credentials.is_root() {
        Credentials::vanta()
    } else {
        parent_credentials
    };
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock().clone();
    for (target, source) in [(0usize, stdin), (1usize, stdout), (2usize, stderr)] {
        if source == u64::MAX {
            continue;
        }
        let source: usize = source.try_into().map_err(|_| ())?;
        let descriptor = descriptors
            .get(source)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or(())?;
        if target >= descriptors.len() {
            descriptors.resize_with(target + 1, || None);
        }
        descriptors[target] = Some(descriptor);
    }
    let pid = allocate_pid();
    let task = new_task(
        pid,
        pid,
        Some(parent_tgid),
        credentials,
        process,
        descriptors,
    );
    if let Some(index) = scheduler
        .tasks
        .iter()
        .position(|t| t.state == TaskState::Reaped)
    {
        scheduler.tasks[index] = task;
    } else if scheduler.tasks.len() < MAX_TASKS {
        scheduler.tasks.push(task);
    } else {
        return Err(());
    }
    FOREGROUND_PID.store(pid, AtomicOrdering::Relaxed);
    Ok(pid)
}

pub fn clone_task_current(
    flags: u64,
    child_stack: u64,
    parent_tidptr: u64,
    child_tidptr: u64,
    tls: u64,
    context: UserContext,
    interrupt_context: InterruptContext,
) -> Result<u64, ()> {
    const MAX_TASKS: usize = 256;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let current = scheduler.current;
    let parent_task = &scheduler.tasks[current];
    let parent_process_arc = parent_task.process.as_ref().ok_or(())?;

    let child_tid = allocate_pid();
    let parent_tid = parent_task.tid;
    let (child_tgid, child_parent_pid) = if flags & vanta_linuxd::CLONE_THREAD != 0 {
        (parent_task.tgid, parent_task.parent_pid)
    } else {
        (child_tid, Some(parent_task.tgid))
    };

    let child_process = if flags & vanta_linuxd::CLONE_VM != 0 {
        Arc::clone(parent_process_arc)
    } else {
        let parent_proc = parent_process_arc.lock();
        let cloned_space = crate::paging::clone_user_address_space(parent_proc.address_space())
            .map_err(|_| ())?;
        let new_proc = parent_proc.clone_process(cloned_space);
        Arc::new(Mutex::new(new_proc))
    };

    let child_descriptors = if flags & vanta_linuxd::CLONE_FILES != 0 {
        Arc::clone(&parent_task.descriptors)
    } else {
        Arc::new(Mutex::new(parent_task.descriptors.lock().clone()))
    };

    let child_signal_actions = if flags & (vanta_linuxd::CLONE_SIGHAND | vanta_linuxd::CLONE_THREAD) != 0 {
        Arc::clone(&parent_task.signal_actions)
    } else {
        Arc::new(Mutex::new(*parent_task.signal_actions.lock()))
    };

    let child_fs_base = if flags & vanta_linuxd::CLONE_SETTLS != 0 {
        tls
    } else {
        parent_task.fs_base
    };

    let child_sp = if child_stack != 0 {
        child_stack
    } else {
        context.stack_pointer
    };

    let child_clear_child_tid = if flags & vanta_linuxd::CLONE_CHILD_CLEARTID != 0 {
        if parent_tidptr != 0 && parent_tidptr >= 0x1000_0000 {
            parent_tidptr
        } else if child_tidptr != 0 {
            child_tidptr
        } else {
            parent_tidptr
        }
    } else {
        0
    };

    let child_rip = interrupt_context.instruction_pointer;
    let mut child_context = context;
    child_context.return_value = 0; // Child returns 0
    child_context.instruction_pointer = child_rip;
    child_context.flags = context.flags | 0x202;
    child_context.stack_pointer = child_sp;

    let mut child_interrupt_context = interrupt_context;
    child_interrupt_context.rax = 0;
    child_interrupt_context.instruction_pointer = child_rip;
    child_interrupt_context.flags = context.flags | 0x202;
    child_interrupt_context.stack_pointer = child_sp;

    let child_credentials = parent_task.credentials;
    let blocked_mask = parent_task.blocked_mask;

    let child_task = Task {
        tid: child_tid,
        tgid: child_tgid,
        parent_pid: child_parent_pid,
        state: TaskState::Runnable,
        context: child_context,
        interrupt_context: child_interrupt_context,
        process: Some(child_process),
        descriptors: child_descriptors,
        credentials: child_credentials,
        signal_actions: child_signal_actions,
        fs_base: child_fs_base,
        blocked_mask,
        pending_signals: 0,
        clear_child_tid: child_clear_child_tid,
    };

    let space = parent_process_arc.lock().address_space();

    if flags & vanta_linuxd::CLONE_PARENT_SETTID != 0 && parent_tidptr != 0 {
        let _ = crate::process::write_user_u32_in(space, parent_tidptr, child_tid as u32);
    }
    if flags & vanta_linuxd::CLONE_CHILD_SETTID != 0 && child_tidptr != 0 {
        let _ = crate::process::write_user_u32_in(space, child_tidptr, child_tid as u32);
    }

    if let Some(index) = scheduler.tasks.iter().position(|t| t.state == TaskState::Reaped) {
        scheduler.tasks[index] = child_task;
    } else if scheduler.tasks.len() < MAX_TASKS {
        scheduler.tasks.push(child_task);
    } else {
        return Err(());
    }

    crate::serial_println!(
        "[sched] clone parent_tid={} child_tid={} tgid={} flags={:#x} tls={:#x} rip={:#x} sp={:#x}",
        parent_tid,
        child_tid,
        child_tgid,
        flags,
        tls,
        context.instruction_pointer,
        child_sp
    );

    Ok(child_tid)
}

pub fn exec_current(process: Box<Process>) -> *const UserContext {
    let (context, space, previous) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("exec without scheduler");
        let current = scheduler.current;
        let context = UserContext {
            return_value: 0,
            rbx: 0,
            rbp: 0,
            r12: process.user_stack_top(),
            r13: 0,
            r14: 0,
            r15: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            instruction_pointer: process.entry(),
            flags: 0x202,
            stack_pointer: process.user_stack_top(),
        };
        let interrupt_context =
            InterruptContext::initial(process.entry(), process.user_stack_top());
        let descriptors = Arc::new(Mutex::new(standard_descriptors(false)));
        let space = process.address_space();
        let new_proc = Arc::new(Mutex::new(*process));
        let task = &mut scheduler.tasks[current];
        let old = core::mem::replace(&mut task.process, Some(new_proc));
        task.context = context;
        task.interrupt_context = interrupt_context;
        task.descriptors = descriptors;
        task.signal_actions = Arc::new(Mutex::new([LinuxSigAction::default(); 65]));
        task.blocked_mask = 0;
        task.pending_signals = 0;
        task.fs_base = 0;
        task.clear_child_tid = 0;
        (context, space, old)
    };
    drop(previous);
    crate::serial_println!("[sched] exec pid={}", current_pid());
    crate::syscall::prepare_user_return(context, space)
}

pub fn wait_child_current(child_target: u64) -> Result<Option<(u64, u64)>, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let parent_tgid = scheduler.tasks[scheduler.current].tgid;

    // Check if any matching child exists
    let has_children = scheduler.tasks.iter().any(|t| {
        t.parent_pid == Some(parent_tgid)
            && (child_target == u64::MAX || child_target == 0 || t.tgid == child_target)
    });
    if !has_children {
        return Err(());
    }

    // Check for reaped/zombie children
    if let Some(index) = scheduler.tasks.iter().position(|t| {
        t.parent_pid == Some(parent_tgid)
            && (child_target == u64::MAX || child_target == 0 || t.tgid == child_target)
            && matches!(t.state, TaskState::Zombie { .. })
    }) {
        let child_tgid = scheduler.tasks[index].tgid;
        let exit_code = match scheduler.tasks[index].state {
            TaskState::Zombie { exit_code } => exit_code,
            _ => 0,
        };
        scheduler.tasks[index].state = TaskState::Reaped;
        return Ok(Some((child_tgid, exit_code)));
    }

    Ok(None)
}

pub fn pipe_wait_key(descriptor: u64) -> Option<u64> {
    let descriptor = current_descriptor(descriptor).ok()?;
    match descriptor.resource {
        DescriptorResource::PipeRead(reader) => Some(reader.lock().state.lock().id),
        DescriptorResource::Ipc(endpoint) => Some(endpoint.lock().state.lock().id),
        _ => None,
    }
}

pub fn current_personality() -> crate::process::ProcessPersonality {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.tasks[scheduler.current].process.as_ref())
        .map(|process| process.lock().personality())
        .unwrap_or(crate::process::ProcessPersonality::NativeVanta)
}

pub fn current_fs_base() -> u64 {
    let scheduler = current_scheduler().lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].fs_base)
        .unwrap_or(0)
}

pub fn set_current_fs_base(fs_base: u64) -> Result<(), ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    scheduler.tasks[scheduler.current].fs_base = fs_base;
    Ok(())
}

pub fn set_current_clear_child_tid(addr: u64) -> u64 {
    let mut scheduler = current_scheduler().lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return 0;
    };
    let task = &mut scheduler.tasks[scheduler.current];
    task.clear_child_tid = addr;
    task.tid
}

pub fn brk_current(new_brk: u64) -> u64 {
    let proc_arc = {
        let scheduler = current_scheduler().lock();
        let Some(scheduler) = scheduler.as_ref() else {
            return 0;
        };
        let Some(process) = scheduler.tasks[scheduler.current].process.as_ref() else {
            return 0;
        };
        Arc::clone(process)
    };
    let res = proc_arc.lock().brk(new_brk);
    res
}

pub fn mmap_current(addr: u64, length: u64, prot: u64, flags: u64) -> Result<u64, ()> {
    let proc_arc = {
        let scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_ref().ok_or(())?;
        let process = scheduler.tasks[scheduler.current].process.as_ref().ok_or(())?;
        Arc::clone(process)
    };
    let res = proc_arc.lock().mmap_anonymous(addr, length, prot, flags);
    res
}

pub fn munmap_current(addr: u64, length: u64) -> Result<(), ()> {
    let proc_arc = {
        let scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_ref().ok_or(())?;
        let process = scheduler.tasks[scheduler.current].process.as_ref().ok_or(())?;
        Arc::clone(process)
    };
    let res = proc_arc.lock().munmap(addr, length);
    res
}

pub fn block_pipe_current(descriptor: u64, context: UserContext) -> *const UserContext {
    let Some(pipe_id) = pipe_wait_key(descriptor) else {
        return crate::syscall::prepare_user_return(context, current_target().1);
    };
    let mut context = context;
    context.return_value = crate::syscall::SYSCALL_WOULD_BLOCK;
    let (next_context, next_space, previous, next) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("pipe block without scheduler");
        let previous = scheduler.current;
        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous].interrupt_context.rbx = context.rbx;
        scheduler.tasks[previous].interrupt_context.rbp = context.rbp;
        scheduler.tasks[previous].interrupt_context.r12 = context.r12;
        scheduler.tasks[previous].interrupt_context.r13 = context.r13;
        scheduler.tasks[previous].interrupt_context.r14 = context.r14;
        scheduler.tasks[previous].interrupt_context.r15 = context.r15;
        scheduler.tasks[previous].interrupt_context.rax = context.return_value;
        let (code_segment, stack_segment) = crate::gdt::user_interrupt_selectors();
        scheduler.tasks[previous].interrupt_context.instruction_pointer = context.instruction_pointer;
        scheduler.tasks[previous].interrupt_context.flags = context.flags | 0x202;
        scheduler.tasks[previous].interrupt_context.stack_pointer = context.stack_pointer;
        scheduler.tasks[previous].interrupt_context.code_segment = code_segment;
        scheduler.tasks[previous].interrupt_context.stack_segment = stack_segment;
        scheduler.tasks[previous].state = TaskState::PipeWaiting { pipe_id };

        let Some(next) = next_alive(scheduler, previous) else {
            scheduler.tasks[previous].state = TaskState::Runnable;
            let process = scheduler.tasks[previous]
                .process
                .as_mut()
                .expect("blocked task lost process");
            let space = process.lock().address_space();
            return crate::syscall::prepare_user_return(context, space);
        };
        scheduler.current = next;
        scheduler.slice_ticks = 0;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        let space = process.lock().address_space();
        (task.context, space, previous, next)
    };
    crate::serial_println!(
        "[sched] pipe block tid={} pipe={} -> {}",
        scheduler_tid(previous),
        pipe_id,
        scheduler_tid(next)
    );
    crate::syscall::prepare_user_return(next_context, next_space)
}

pub fn futex_wait_current(uaddr: u64, bitset: u32, context: UserContext) -> *const UserContext {
    let mut context = context;
    context.return_value = 0; // return 0 on successful wake
    let (next_context, next_space, previous, next) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("futex_wait without scheduler");
        let previous = scheduler.current;
        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous].interrupt_context.rbx = context.rbx;
        scheduler.tasks[previous].interrupt_context.rbp = context.rbp;
        scheduler.tasks[previous].interrupt_context.r12 = context.r12;
        scheduler.tasks[previous].interrupt_context.r13 = context.r13;
        scheduler.tasks[previous].interrupt_context.r14 = context.r14;
        scheduler.tasks[previous].interrupt_context.r15 = context.r15;
        scheduler.tasks[previous].interrupt_context.rax = 0;
        let (code_segment, stack_segment) = crate::gdt::user_interrupt_selectors();
        scheduler.tasks[previous].interrupt_context.instruction_pointer = context.instruction_pointer;
        scheduler.tasks[previous].interrupt_context.flags = context.flags | 0x202;
        scheduler.tasks[previous].interrupt_context.stack_pointer = context.stack_pointer;
        scheduler.tasks[previous].interrupt_context.code_segment = code_segment;
        scheduler.tasks[previous].interrupt_context.stack_segment = stack_segment;
        scheduler.tasks[previous].state = TaskState::FutexWait { uaddr, bitset };

        let next = next_alive(scheduler, previous).unwrap_or(previous);
        scheduler.current = next;
        scheduler.slice_ticks = 0;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        let space = process.lock().address_space();
        (task.context, space, previous, next)
    };
    crate::serial_println!(
        "[futex] wait tid={} uaddr={:#x} bitset={:#x} -> switch to tid={}",
        scheduler_tid(previous),
        uaddr,
        bitset,
        scheduler_tid(next)
    );
    crate::syscall::prepare_user_return(next_context, next_space)
}

pub fn wait_current(pid: u64, status_ptr: u64, context: UserContext) -> *const UserContext {
    let (next_context, next_space, previous, next) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("wait without scheduler");
        let previous = scheduler.current;

        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous].interrupt_context.rbx = context.rbx;
        scheduler.tasks[previous].interrupt_context.rbp = context.rbp;
        scheduler.tasks[previous].interrupt_context.r12 = context.r12;
        scheduler.tasks[previous].interrupt_context.r13 = context.r13;
        scheduler.tasks[previous].interrupt_context.r14 = context.r14;
        scheduler.tasks[previous].interrupt_context.r15 = context.r15;
        scheduler.tasks[previous].interrupt_context.rax = context.return_value;
        let (code_segment, stack_segment) = crate::gdt::user_interrupt_selectors();
        scheduler.tasks[previous].interrupt_context.instruction_pointer = context.instruction_pointer;
        scheduler.tasks[previous].interrupt_context.flags = context.flags | 0x202;
        scheduler.tasks[previous].interrupt_context.stack_pointer = context.stack_pointer;
        scheduler.tasks[previous].interrupt_context.code_segment = code_segment;
        scheduler.tasks[previous].interrupt_context.stack_segment = stack_segment;
        scheduler.tasks[previous].state = TaskState::Waiting { child_pid: pid, status_ptr };

        let next = next_alive(scheduler, previous).expect("wait left no runnable task");
        scheduler.current = next;
        scheduler.slice_ticks = 0;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        let space = process.lock().address_space();
        (task.context, space, previous, next)
    };
    crate::serial_println!(
        "[sched] wait tid={} target={} -> {}",
        scheduler_tid(previous),
        pid,
        scheduler_tid(next)
    );
    crate::syscall::prepare_user_return(next_context, next_space)
}

fn new_task(
    tid: u64,
    tgid: u64,
    parent_pid: Option<u64>,
    credentials: Credentials,
    process: Box<Process>,
    descriptors: Vec<Option<FileDescriptor>>,
) -> Task {
    let entry = process.entry();
    let stack_top = process.user_stack_top();
    Task {
        tid,
        tgid,
        parent_pid,
        state: TaskState::Runnable,
        context: UserContext {
            return_value: 0,
            rbx: 0,
            rbp: 0,
            r12: stack_top,
            r13: 0,
            r14: 0,
            r15: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            instruction_pointer: entry,
            flags: 0x202,
            stack_pointer: stack_top,
        },
        interrupt_context: InterruptContext::initial(entry, stack_top),
        process: Some(Arc::new(Mutex::new(*process))),
        descriptors: Arc::new(Mutex::new(descriptors)),
        credentials,
        signal_actions: Arc::new(Mutex::new([LinuxSigAction::default(); 65])),
        fs_base: 0,
        blocked_mask: 0,
        pending_signals: 0,
        clear_child_tid: 0,
    }
}

fn standard_descriptors(native_tty: bool) -> Vec<Option<FileDescriptor>> {
    let resource = if native_tty {
        DescriptorResource::Tty
    } else {
        DescriptorResource::Serial
    };
    alloc::vec![
        Some(FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: resource.clone(),
        }),
        Some(FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::WRITE | Rights::TRANSFER,
            resource: resource.clone(),
        }),
        Some(FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::WRITE | Rights::TRANSFER,
            resource,
        }),
    ]
}

pub fn open_current(contents: Vec<u8>) -> Result<u64, ()> {
    open_native_current(String::new(), contents, false, false)
}

pub fn open_native_current(
    path: String,
    contents: Vec<u8>,
    writable: bool,
    append: bool,
) -> Result<u64, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    let initial_offset = if append { contents.len() } else { 0 };
    let mut rights = Rights::READ | Rights::TRANSFER;
    if writable {
        rights |= Rights::WRITE;
    }
    install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights,
            resource: DescriptorResource::File(Arc::new(Mutex::new(OpenFile {
                path,
                contents,
                offset: initial_offset,
                writable,
            }))),
        },
    )
}

pub fn open_directory_current(entries: Vec<String>) -> Result<u64, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: DescriptorResource::Directory(Arc::new(Mutex::new(OpenDirectory {
                entries,
                offset: 0,
            }))),
        },
    )
}

pub fn open_socket_current() -> Result<u64, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::WRITE | Rights::TRANSFER | Rights::CONNECT,
            resource: DescriptorResource::Socket(Arc::new(Mutex::new(OpenSocket {
                connection: None,
            }))),
        },
    )
}

pub fn connect_socket_current(
    descriptor: u64,
    remote_ip: crate::net::Ipv4Address,
    remote_port: u16,
) -> Result<(), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::CONNECT) {
        return Err(());
    }
    let DescriptorResource::Socket(socket) = descriptor.resource else {
        return Err(());
    };
    let mut socket = socket.lock();
    if socket.connection.is_some() {
        return Err(());
    }
    socket.connection = Some(crate::network::tcp_connect(remote_ip, remote_port).map_err(|_| ())?);
    Ok(())
}

pub fn duplicate_current(descriptor: u64) -> Result<u64, ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    let duplicate = descriptors
        .get(index)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())?;
    if duplicate.capability.is_invalid() || !duplicate.rights.contains(Rights::TRANSFER) {
        return Err(());
    }
    install_descriptor(&mut descriptors, duplicate)
}

pub fn duplicate_to_current(old_fd: u64, new_fd: u64) -> Result<u64, ()> {
    let old_index: usize = old_fd.try_into().map_err(|_| ())?;
    let new_index: usize = new_fd.try_into().map_err(|_| ())?;
    if new_index >= 256 {
        return Err(());
    }
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    let duplicate = descriptors
        .get(old_index)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())?;
    if old_index == new_index {
        return Ok(new_fd);
    }
    if new_index >= descriptors.len() {
        descriptors.resize_with(new_index + 1, || None);
    }
    descriptors[new_index] = Some(duplicate);
    Ok(new_fd)
}

pub fn stat_linux_current(descriptor: u64) -> Result<[u8; 144], ()> {
    let descriptor = current_descriptor(descriptor)?;
    let mut stat = [0u8; 144];
    let (mode, size, is_char) = match descriptor.resource {
        DescriptorResource::File(file) => (0o100644u32, file.lock().contents.len() as i64, false),
        DescriptorResource::Directory(_) => (0o040755u32, 4096i64, false),
        DescriptorResource::Tty | DescriptorResource::Serial => (0o020666u32, 0i64, true),
        DescriptorResource::PipeRead(_) | DescriptorResource::PipeWrite(_) => {
            (0o010600u32, 0i64, false)
        }
        DescriptorResource::Socket(_) => (0o140666u32, 0i64, false),
        DescriptorResource::Ipc(_) | DescriptorResource::Epoll(_) | DescriptorResource::EventFd(_) => {
            (0o010600u32, 0i64, false)
        }
        DescriptorResource::PtyMaster(_) | DescriptorResource::PtySlave(_) => (0o020666u32, 0i64, true),
    };
    let dev = 1u64;
    let ino = 1u64;
    let nlink = 1u64;
    let uid = 1000u32;
    let gid = 1000u32;
    let rdev = if is_char { 5u64 } else { 0u64 };
    let blksize = 4096i64;
    let blocks = (size + 511) / 512;
    stat[0..8].copy_from_slice(&dev.to_ne_bytes());
    stat[8..16].copy_from_slice(&ino.to_ne_bytes());
    stat[16..24].copy_from_slice(&nlink.to_ne_bytes());
    stat[24..28].copy_from_slice(&mode.to_ne_bytes());
    stat[28..32].copy_from_slice(&uid.to_ne_bytes());
    stat[32..36].copy_from_slice(&gid.to_ne_bytes());
    stat[40..48].copy_from_slice(&rdev.to_ne_bytes());
    stat[48..56].copy_from_slice(&size.to_ne_bytes());
    stat[56..64].copy_from_slice(&blksize.to_ne_bytes());
    stat[64..72].copy_from_slice(&blocks.to_ne_bytes());
    Ok(stat)
}

pub fn read_dir_linux_current(descriptor: u64, max_length: usize) -> Result<Vec<u8>, ()> {
    let descriptor = current_descriptor(descriptor)?;
    let DescriptorResource::Directory(directory) = descriptor.resource else {
        return Err(());
    };
    let mut directory = directory.lock();
    let mut bytes = Vec::new();
    while directory.offset < directory.entries.len() {
        let entry_name = directory.entries[directory.offset].as_bytes();
        let name_len = entry_name.len();
        let raw_len = 19 + name_len + 1;
        let reclen = (raw_len + 7) & !7;
        if bytes.len() + reclen > max_length && !bytes.is_empty() {
            break;
        }
        let ino = (directory.offset + 1) as u64;
        let off = (directory.offset + 1) as i64;
        let file_type: u8 = if directory.entries[directory.offset].ends_with('/') {
            4
        } else {
            8
        };
        let mut record = Vec::with_capacity(reclen);
        record.extend_from_slice(&ino.to_ne_bytes());
        record.extend_from_slice(&off.to_ne_bytes());
        record.extend_from_slice(&(reclen as u16).to_ne_bytes());
        record.push(file_type);
        record.extend_from_slice(entry_name);
        record.push(0);
        while record.len() < reclen {
            record.push(0);
        }
        bytes.extend_from_slice(&record);
        directory.offset += 1;
    }
    Ok(bytes)
}

pub fn open_pipe_current() -> Result<(u64, u64), ()> {
    let (reader, writer) = Pipe::new();
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    let reader = install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: DescriptorResource::PipeRead(Arc::new(Mutex::new(reader))),
        },
    )?;
    let writer = install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::WRITE | Rights::TRANSFER,
            resource: DescriptorResource::PipeWrite(Arc::new(Mutex::new(writer))),
        },
    )?;
    Ok((reader, writer))
}

pub fn open_ipc_pair_current() -> Result<(u64, u64), ()> {
    let state = Arc::new(Mutex::new(IpcState {
        id: NEXT_IPC_ID.fetch_add(1, Ordering::Relaxed),
        queue: Vec::new(),
        revoked: false,
    }));
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    let sender = install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::WRITE | Rights::TRANSFER,
            resource: DescriptorResource::Ipc(Arc::new(Mutex::new(IpcEndpoint {
                state: Arc::clone(&state),
                send: true,
            }))),
        },
    )?;
    let receiver = install_descriptor(
        &mut descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: DescriptorResource::Ipc(Arc::new(Mutex::new(IpcEndpoint {
                state,
                send: false,
            }))),
        },
    )?;
    Ok((sender, receiver))
}

pub fn ipc_send_current(descriptor: u64, bytes: &[u8]) -> Result<(), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::WRITE) || bytes.len() > IPC_MESSAGE_LIMIT {
        return Err(());
    }
    let DescriptorResource::Ipc(endpoint) = descriptor.resource else {
        return Err(());
    };
    let endpoint = endpoint.lock();
    if !endpoint.send {
        return Err(());
    }
    let mut state = endpoint.state.lock();
    if state.revoked || state.queue.len() >= IPC_QUEUE_LIMIT {
        return Err(());
    }
    let id = state.id;
    state.queue.push(IpcMessage {
        sender_pid: current_pid(),
        bytes: bytes.to_vec(),
    });
    crate::serial_println!("[ipc] send id={} queue={}", id, state.queue.len());
    let queue_id = state.id;
    drop(state);
    drop(endpoint);
    wake_pipe_waiters(queue_id);
    Ok(())
}

pub fn ipc_receive_current(descriptor: u64) -> Result<Option<Vec<u8>>, ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::READ) {
        return Err(());
    }
    let DescriptorResource::Ipc(endpoint) = descriptor.resource else {
        return Err(());
    };
    let endpoint = endpoint.lock();
    if endpoint.send {
        return Err(());
    }
    let mut state = endpoint.state.lock();
    if state.revoked {
        return Err(());
    }
    let position = state
        .queue
        .iter()
        .position(|message| message.sender_pid != current_pid());
    let message = if let Some(position) = position {
        Some(state.queue.remove(position).bytes)
    } else {
        None
    };
    if message.is_some() {
        crate::serial_println!("[ipc] recv id={} queue={}", state.id, state.queue.len());
    }
    Ok(message)
}

pub fn ipc_revoke_current(descriptor: u64) -> Result<(), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::TRANSFER) {
        return Err(());
    }
    let DescriptorResource::Ipc(endpoint) = descriptor.resource else {
        return Err(());
    };
    let id = {
        let endpoint = endpoint.lock();
        let mut state = endpoint.state.lock();
        state.revoked = true;
        state.id
    };
    wake_pipe_waiters(id);
    Ok(())
}

pub fn seek_current(descriptor: u64, offset: i64, whence: u64) -> Result<u64, ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::READ) {
        return Err(());
    }
    let DescriptorResource::File(file) = descriptor.resource else {
        return Err(());
    };
    let mut file = file.lock();
    let base = match whence {
        0 => 0,
        1 => file.offset,
        2 => file.contents.len(),
        _ => return Err(()),
    };
    let magnitude: usize = offset.unsigned_abs().try_into().map_err(|_| ())?;
    let position = if offset.is_negative() {
        base.checked_sub(magnitude).ok_or(())?
    } else {
        base.checked_add(magnitude).ok_or(())?
    };
    if position > file.contents.len() {
        return Err(());
    }
    file.offset = position;
    Ok(position as u64)
}

pub fn stat_current(descriptor: u64) -> Result<(u64, u64), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::READ) {
        return Err(());
    }
    match descriptor.resource {
        DescriptorResource::File(file) => Ok((file.lock().contents.len() as u64, 0o100644)),
        DescriptorResource::Directory(_) => Ok((0, 0o040755)),
        _ => Err(()),
    }
}

fn install_descriptor(
    descriptors: &mut Vec<Option<FileDescriptor>>,
    descriptor: FileDescriptor,
) -> Result<u64, ()> {
    const MAX_DESCRIPTORS: usize = 256;
    if let Some((index, slot)) = descriptors
        .iter_mut()
        .enumerate()
        .find(|(_, descriptor)| descriptor.is_none())
    {
        *slot = Some(descriptor);
        return Ok(index as u64);
    }
    if descriptors.len() == MAX_DESCRIPTORS {
        return Err(());
    }
    descriptors.push(Some(descriptor));
    Ok((descriptors.len() - 1) as u64)
}

pub fn read_current(descriptor: u64, length: usize) -> Result<Vec<u8>, ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::READ) {
        return Err(());
    }
    match descriptor.resource {
        DescriptorResource::File(file) => {
            let mut file = file.lock();
            let end = file.offset.saturating_add(length).min(file.contents.len());
            let bytes = file.contents[file.offset..end].to_vec();
            file.offset = end;
            Ok(bytes)
        }
        DescriptorResource::Directory(directory) => {
            let mut directory = directory.lock();
            let mut bytes = Vec::new();
            while directory.offset < directory.entries.len() && bytes.len() < length {
                let entry = directory.entries[directory.offset].as_bytes();
                let needed = entry.len() + 1;
                if bytes.len() + needed > length && !bytes.is_empty() {
                    break;
                }
                bytes.extend_from_slice(entry);
                bytes.push(b'\n');
                directory.offset += 1;
            }
            Ok(bytes)
        }
        DescriptorResource::Socket(socket) => {
            let mut socket = socket.lock();
            let connection = socket.connection.as_mut().ok_or(())?;
            crate::network::tcp_receive(connection, length).map_err(|_| ())
        }
        DescriptorResource::Serial => Err(()),
        DescriptorResource::Tty => Ok(read_tty(length)),
        DescriptorResource::PipeRead(reader) => Ok(reader.lock().read(length)),
        DescriptorResource::PipeWrite(_) => Err(()),
        DescriptorResource::Ipc(_) => Err(()),
        DescriptorResource::EventFd(efd) => {
            let mut efd = efd.lock();
            if efd.counter == 0 {
                return Err(());
            }
            let val = if efd.flags & vanta_linuxd::EFD_SEMAPHORE != 0 {
                efd.counter = efd.counter.saturating_sub(1);
                1u64
            } else {
                let v = efd.counter;
                efd.counter = 0;
                v
            };
            Ok(val.to_ne_bytes().to_vec())
        }
        DescriptorResource::PtyMaster(pty) => {
            let mut pty = pty.lock();
            let count = pty.slave_to_master.len().min(length);
            let bytes = pty.slave_to_master.drain(..count).collect();
            Ok(bytes)
        }
        DescriptorResource::PtySlave(pty) => {
            let mut pty = pty.lock();
            let count = pty.master_to_slave.len().min(length);
            let bytes = pty.master_to_slave.drain(..count).collect();
            Ok(bytes)
        }
        DescriptorResource::Epoll(_) => Err(()),
    }
}

pub fn read_would_block(descriptor: u64) -> bool {
    let Ok(descriptor) = current_descriptor(descriptor) else {
        return false;
    };
    match descriptor.resource {
        DescriptorResource::PipeRead(reader) => {
            let state = reader.lock();
            state.state.lock().bytes.is_empty() && state.state.lock().writer_open
        }
        DescriptorResource::Ipc(endpoint) => {
            let endpoint = endpoint.lock();
            let state = endpoint.state.lock();
            !state.revoked
                && !state
                    .queue
                    .iter()
                    .any(|message| message.sender_pid != current_pid())
        }
        _ => false,
    }
}

pub fn close_current(descriptor: u64) -> Result<(), ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let descriptor = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().ok_or(())?;
        let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
        descriptors
            .get_mut(index)
            .ok_or(())?
            .take()
            .ok_or(())?
    };
    match descriptor.resource {
        DescriptorResource::Socket(socket) => {
            if Arc::strong_count(&socket) == 1 {
                if let Some(connection) = socket.lock().connection.take() {
                    crate::network::tcp_close(connection).map_err(|_| ())?;
                }
            }
        }
        DescriptorResource::PipeWrite(writer) => close_pipe_writer(writer),
        DescriptorResource::File(_)
        | DescriptorResource::Directory(_)
        | DescriptorResource::Serial
        | DescriptorResource::Tty
        | DescriptorResource::PipeRead(_)
        | DescriptorResource::Ipc(_)
        | DescriptorResource::Epoll(_)
        | DescriptorResource::EventFd(_)
        | DescriptorResource::PtyMaster(_)
        | DescriptorResource::PtySlave(_) => {}
    }
    Ok(())
}

pub fn write_current(descriptor: u64, bytes: &[u8]) -> Result<(), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::WRITE) {
        return Err(());
    }
    match descriptor.resource {
        DescriptorResource::Socket(socket) => {
            let mut socket = socket.lock();
            let connection = socket.connection.as_mut().ok_or(())?;
            crate::network::tcp_send(connection, bytes).map_err(|_| ())
        }
        DescriptorResource::PipeWrite(writer) => {
            let pipe_id = writer.lock().write(bytes);
            wake_pipe_waiters(pipe_id);
            Ok(())
        }
        DescriptorResource::Serial | DescriptorResource::Tty => {
            for byte in bytes {
                crate::serial::_print(format_args!("{}", *byte as char));
            }
            Ok(())
        }
        DescriptorResource::File(file) => {
            let mut file = file.lock();
            if !file.writable {
                return Err(());
            }
            let offset = file.offset;
            let end = offset.checked_add(bytes.len()).ok_or(())?;
            if end > file.contents.len() {
                file.contents.resize(end, 0);
            }
            file.contents[offset..end].copy_from_slice(bytes);
            file.offset = end;
            let credentials = current_credentials();
            crate::vfs::write_root_as(&file.path, &file.contents, &credentials).map_err(|_| ())
        }
        DescriptorResource::EventFd(efd) => {
            if bytes.len() < 8 {
                return Err(());
            }
            let mut val_bytes = [0u8; 8];
            val_bytes.copy_from_slice(&bytes[..8]);
            let val = u64::from_ne_bytes(val_bytes);
            let mut efd = efd.lock();
            efd.counter = efd.counter.saturating_add(val);
            Ok(())
        }
        DescriptorResource::PtyMaster(pty) => {
            let mut pty = pty.lock();
            pty.master_to_slave.extend_from_slice(bytes);
            Ok(())
        }
        DescriptorResource::PtySlave(pty) => {
            let mut pty = pty.lock();
            pty.slave_to_master.extend_from_slice(bytes);
            Ok(())
        }
        DescriptorResource::Directory(_) | DescriptorResource::PipeRead(_) | DescriptorResource::Epoll(_) => Err(()),
        DescriptorResource::Ipc(_) => Err(()),
    }
}

pub fn epoll_create1_current(_flags: u32) -> Result<u64, ()> {
    let epoll = Arc::new(Mutex::new(EpollInstance { items: Vec::new() }));
    let descriptor = FileDescriptor {
        capability: allocate_capability(),
        rights: Rights::READ | Rights::WRITE,
        resource: DescriptorResource::Epoll(epoll),
    };
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    install_descriptor(&mut descriptors, descriptor)
}

pub fn epoll_ctl_current(epfd: u64, op: u32, fd: u64, events: u32, data: u64) -> Result<(), ()> {
    let descriptor = current_descriptor(epfd)?;
    let DescriptorResource::Epoll(epoll) = descriptor.resource else {
        return Err(());
    };
    let mut epoll = epoll.lock();
    match op {
        vanta_linuxd::EPOLL_CTL_ADD => {
            if epoll.items.iter().any(|item| item.fd == fd) {
                return Err(());
            }
            epoll.items.push(EpollItem { fd, events, data });
            Ok(())
        }
        vanta_linuxd::EPOLL_CTL_MOD => {
            let item = epoll.items.iter_mut().find(|item| item.fd == fd).ok_or(())?;
            item.events = events;
            item.data = data;
            Ok(())
        }
        vanta_linuxd::EPOLL_CTL_DEL => {
            let pos = epoll.items.iter().position(|item| item.fd == fd).ok_or(())?;
            epoll.items.remove(pos);
            Ok(())
        }
        _ => Err(()),
    }
}

pub fn epoll_wait_current(epfd: u64, maxevents: usize) -> Result<Vec<(u32, u64)>, ()> {
    let descriptor = current_descriptor(epfd)?;
    let DescriptorResource::Epoll(epoll) = descriptor.resource else {
        return Err(());
    };
    let epoll = epoll.lock();
    let mut ready = Vec::new();
    for item in &epoll.items {
        if ready.len() >= maxevents {
            break;
        }
        let Ok(desc) = current_descriptor(item.fd) else {
            continue;
        };
        let mut revents = 0;
        match desc.resource {
            DescriptorResource::PipeRead(ref reader) => {
                let state = reader.lock();
                if !state.state.lock().bytes.is_empty() {
                    revents |= vanta_linuxd::EPOLLIN;
                }
                if !state.state.lock().writer_open {
                    revents |= vanta_linuxd::EPOLLHUP;
                }
            }
            DescriptorResource::PipeWrite(ref writer) => {
                let state = writer.lock();
                if state.state.lock().bytes.len() < 4096 {
                    revents |= vanta_linuxd::EPOLLOUT;
                }
            }
            DescriptorResource::Socket(ref sock) => {
                if sock.lock().connection.is_some() {
                    revents |= vanta_linuxd::EPOLLIN | vanta_linuxd::EPOLLOUT;
                }
            }
            DescriptorResource::EventFd(ref efd) => {
                if efd.lock().counter > 0 {
                    revents |= vanta_linuxd::EPOLLIN;
                }
                revents |= vanta_linuxd::EPOLLOUT;
            }
            DescriptorResource::File(_) | DescriptorResource::Serial | DescriptorResource::Tty => {
                revents |= vanta_linuxd::EPOLLIN | vanta_linuxd::EPOLLOUT;
            }
            _ => {}
        }
        if revents & item.events != 0 {
            ready.push((revents & item.events, item.data));
        }
    }
    Ok(ready)
}

pub fn eventfd_current(initval: u64, flags: u32) -> Result<u64, ()> {
    let efd = Arc::new(Mutex::new(EventFdInstance { counter: initval, flags }));
    let descriptor = FileDescriptor {
        capability: allocate_capability(),
        rights: Rights::READ | Rights::WRITE,
        resource: DescriptorResource::EventFd(efd),
    };
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    install_descriptor(&mut descriptors, descriptor)
}

fn decode_tty_scancode(scancode: u8) -> Option<u8> {
    if scancode & 0x80 != 0 {
        return None;
    }
    match scancode {
        0x02..=0x0a => Some(b"123456789"[(scancode - 0x02) as usize]),
        0x0b => Some(b'0'),
        0x0e => Some(8),
        0x10..=0x19 => Some(b"qwertyuiop"[(scancode - 0x10) as usize]),
        0x1c => Some(b'\n'),
        0x1e..=0x26 => Some(b"asdfghjkl"[(scancode - 0x1e) as usize]),
        0x27 => Some(b';'),
        0x2c..=0x32 => Some(b"zxcvbnm"[(scancode - 0x2c) as usize]),
        0x33 => Some(b','),
        0x34 => Some(b'.'),
        0x35 => Some(b'/'),
        0x39 => Some(b' '),
        _ => None,
    }
}

fn read_tty(length: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    while bytes.len() < length {
        if let Some(byte) = crate::serial::try_receive() {
            bytes.push(byte);
            continue;
        }
        let Some(scancode) = crate::keyboard::pop_scancode() else {
            break;
        };
        if scancode == 0x1d {
            TTY_CTRL_HELD.store(true, AtomicOrdering::Relaxed);
            continue;
        }
        if scancode == 0x9d {
            TTY_CTRL_HELD.store(false, AtomicOrdering::Relaxed);
            continue;
        }
        if TTY_CTRL_HELD.load(AtomicOrdering::Relaxed) && scancode == 0x2e {
            bytes.push(3);
            continue;
        }
        if let Some(byte) = decode_tty_scancode(scancode) {
            bytes.push(byte);
        }
    }
    bytes
}

fn current_descriptor(descriptor: u64) -> Result<FileDescriptor, ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_ref().ok_or(())?;
    let descriptors = scheduler.tasks[scheduler.current].descriptors.lock();
    descriptors
        .get(index)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())
}

#[allow(dead_code)]
fn scheduler_pid(index: usize) -> u64 {
    current_scheduler()
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks[index].tgid)
        .unwrap_or(0)
}

fn scheduler_tid(index: usize) -> u64 {
    current_scheduler()
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks[index].tid)
        .unwrap_or(0)
}

fn task_count() -> usize {
    current_scheduler()
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks.len())
        .unwrap_or(0)
}
