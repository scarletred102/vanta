//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use vanta_abi::{CapabilityId, Credentials, Rights, SignalAction};

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
    pid: u64,
    parent_pid: Option<u64>,
    state: TaskState,
    process: Option<Box<Process>>,
    context: UserContext,
    interrupt_context: InterruptContext,
    descriptors: Vec<Option<FileDescriptor>>,
    credentials: Credentials,
    signal_actions: [SignalAction; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Waiting { child_pid: u64 },
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

struct Pipe;

struct PipeReader {
    state: Arc<Mutex<PipeState>>,
}

struct PipeWriter {
    state: Arc<Mutex<PipeState>>,
}

struct PipeState {
    bytes: Vec<u8>,
    writer_open: bool,
}

impl Pipe {
    fn new() -> (PipeReader, PipeWriter) {
        let state = Arc::new(Mutex::new(PipeState {
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
    fn write(&mut self, bytes: &[u8]) {
        let mut state = self.state.lock();
        if state.writer_open {
            state.bytes.extend_from_slice(bytes);
        }
    }

    fn close(&mut self) {
        self.state.lock().writer_open = false;
    }
}

fn close_pipe_writer(writer: Arc<Mutex<PipeWriter>>) {
    if Arc::strong_count(&writer) == 1 {
        writer.lock().close();
    }
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
    crate::syscall::set_native_abi(native_tty);
    let kernel_space = paging::current_address_space();
    if processes.is_empty() {
        crate::shell::run();
    }
    let tasks = processes
        .into_iter()
        .map(|process| {
            new_task(
                allocate_pid(),
                None,
                Credentials::vanta(),
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
        (task.context, process.address_space(), previous, next)
    };
    if previous != next {
        crate::serial_println!(
            "[sched] yield pid={} -> {}",
            scheduler_pid(previous),
            scheduler_pid(next)
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
            instruction_pointer: scheduler.tasks[previous]
                .interrupt_context
                .instruction_pointer,
            flags: scheduler.tasks[previous].interrupt_context.flags,
            stack_pointer: scheduler.tasks[previous].interrupt_context.stack_pointer,
        };
        scheduler.current = next;
        let previous_pid = scheduler.tasks[previous].pid;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        (
            &task.interrupt_context as *const InterruptContext,
            process.address_space(),
            previous_pid,
            task.pid,
        )
    };

    unsafe {
        paging::activate(next.1);
    }
    crate::serial_println!("[sched] preempt pid={} -> {}", next.2, next.3);
    next.0
}

pub fn exit_current(code: u64) -> *const UserContext {
    let (next, remaining, parent_pid, exited_process) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("process exit without scheduler");
        let current = scheduler.current;
        let parent_pid = scheduler.tasks[current].parent_pid;
        let process = scheduler.tasks[current]
            .process
            .take()
            .expect("current task already exited");
        scheduler.tasks[current].state = TaskState::Zombie { exit_code: code };
        let exited_pid = scheduler.tasks[current].pid;
        if FOREGROUND_PID.load(AtomicOrdering::Relaxed) == exited_pid {
            FOREGROUND_PID.store(0, AtomicOrdering::Relaxed);
        }
        if let Some(parent) = scheduler.tasks.iter_mut().find(|task| {
            task.pid == parent_pid.unwrap_or(0)
                && task.state
                    == TaskState::Waiting {
                        child_pid: exited_pid,
                    }
        }) {
            parent.state = TaskState::Runnable;
            parent.context.return_value = code;
            parent.interrupt_context.rax = code;
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
            (task.context, process.address_space(), index)
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
    crate::serial_println!("[sched] continue pid={}", scheduler_pid(next));
    crate::syscall::prepare_user_return(context, space)
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
    (task.context, process.address_space())
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
        .map(|scheduler| scheduler.tasks[scheduler.current].pid)
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

pub fn signal_action(signal: u64) -> Option<SignalAction> {
    let scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_ref()?;
    if signal == 0 || signal > 31 {
        return None;
    }
    Some(scheduler.tasks[scheduler.current].signal_actions[signal as usize])
}

pub fn set_signal_action(signal: u64, action: SignalAction) -> Result<SignalAction, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    if signal == 0 || signal > 31 {
        return Err(());
    }
    let slot = &mut scheduler.tasks[scheduler.current].signal_actions[signal as usize];
    let old = *slot;
    *slot = action;
    Ok(old)
}

pub fn kill_process(pid: u64, signal: u64) -> Result<(), ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let current_pid = scheduler.tasks[scheduler.current].pid;
    if pid == current_pid {
        return Err(());
    }
    let (process, parent_pid) = {
        let target = scheduler
            .tasks
            .iter_mut()
            .find(|task| task.pid == pid && task.process.is_some())
            .ok_or(())?;
        if target.signal_actions[signal as usize].handler == 1 {
            return Ok(());
        }
        let process = target.process.take();
        let parent_pid = target.parent_pid;
        target.state = TaskState::Zombie {
            exit_code: 128 + signal,
        };
        if FOREGROUND_PID.load(AtomicOrdering::Relaxed) == pid {
            FOREGROUND_PID.store(0, AtomicOrdering::Relaxed);
        }
        (process, parent_pid)
    };
    if let Some(parent_pid) = parent_pid {
        if let Some(parent) = scheduler.tasks.iter_mut().find(|task| {
            task.pid == parent_pid && task.state == TaskState::Waiting { child_pid: pid }
        }) {
            parent.state = TaskState::Runnable;
            parent.context.return_value = 128 + signal;
            parent.interrupt_context.rax = 128 + signal;
        }
    }
    drop(process);
    Ok(())
}

pub fn interrupt_current(signal: u64) {
    let process = {
        let mut scheduler = current_scheduler().lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return;
        };
        let target_pid = {
            let foreground = FOREGROUND_PID.load(AtomicOrdering::Relaxed);
            if foreground == 0 {
                scheduler.tasks[scheduler.current].pid
            } else {
                foreground
            }
        };
        let Some(target) = scheduler
            .tasks
            .iter_mut()
            .find(|task| task.pid == target_pid && task.process.is_some())
        else {
            return;
        };
        if signal > 31 || target.signal_actions[signal as usize].handler == 1 {
            return;
        }
        let process = target.process.take();
        if process.is_none() {
            return;
        }
        let pid = target.pid;
        target.state = TaskState::Zombie {
            exit_code: 128 + signal,
        };
        let parent_pid = target.parent_pid;
        FOREGROUND_PID.store(0, AtomicOrdering::Relaxed);
        if let Some(parent_pid) = parent_pid {
            if let Some(parent) = scheduler.tasks.iter_mut().find(|task| {
                task.pid == parent_pid && task.state == TaskState::Waiting { child_pid: pid }
            }) {
                parent.state = TaskState::Runnable;
                parent.context.return_value = 128 + signal;
                parent.interrupt_context.rax = 128 + signal;
            }
        }
        process
    };
    drop(process);
    crate::serial_println!("[signal] pid={} signal={}", current_pid(), signal);
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
    const MAX_TASKS: usize = 8;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    if scheduler.tasks.len() == MAX_TASKS {
        return Err(());
    }
    let parent_pid = scheduler.tasks[scheduler.current].pid;
    let credentials = scheduler.tasks[scheduler.current].credentials;
    let descriptors = scheduler.tasks[scheduler.current].descriptors.clone();
    let pid = allocate_pid();
    scheduler.tasks.push(new_task(
        pid,
        Some(parent_pid),
        credentials,
        process,
        descriptors,
    ));
    FOREGROUND_PID.store(pid, AtomicOrdering::Relaxed);
    Ok(pid)
}

pub fn spawn_with_stdio_current(
    process: Box<Process>,
    stdin: u64,
    stdout: u64,
    stderr: u64,
) -> Result<u64, ()> {
    const MAX_TASKS: usize = 8;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    if scheduler.tasks.len() == MAX_TASKS {
        return Err(());
    }
    let parent_pid = scheduler.tasks[scheduler.current].pid;
    let credentials = scheduler.tasks[scheduler.current].credentials;
    let mut descriptors = scheduler.tasks[scheduler.current].descriptors.clone();
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
    scheduler.tasks.push(new_task(
        pid,
        Some(parent_pid),
        credentials,
        process,
        descriptors,
    ));
    FOREGROUND_PID.store(pid, AtomicOrdering::Relaxed);
    Ok(pid)
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
            instruction_pointer: process.entry(),
            flags: 0x202,
            stack_pointer: process.user_stack_top(),
        };
        let interrupt_context =
            InterruptContext::initial(process.entry(), process.user_stack_top());
        let task = &mut scheduler.tasks[current];
        let old = core::mem::replace(&mut task.process, Some(process));
        task.context = context;
        task.interrupt_context = interrupt_context;
        let space = task
            .process
            .as_mut()
            .expect("exec lost process")
            .address_space();
        (context, space, old)
    };
    drop(previous);
    crate::serial_println!("[sched] exec pid={}", current_pid());
    crate::syscall::prepare_user_return(context, space)
}

pub fn wait_child_current(pid: u64) -> Result<Option<u64>, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let parent_pid = scheduler.tasks[scheduler.current].pid;
    let child = scheduler
        .tasks
        .iter_mut()
        .find(|task| task.pid == pid && task.parent_pid == Some(parent_pid))
        .ok_or(())?;
    let TaskState::Zombie { exit_code } = child.state else {
        return Ok(None);
    };
    child.state = TaskState::Reaped;
    Ok(Some(exit_code))
}

pub fn wait_current(pid: u64, context: UserContext) -> *const UserContext {
    let (next_context, next_space, previous, next) = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().expect("wait without scheduler");
        let previous = scheduler.current;
        let parent_pid = scheduler.tasks[previous].pid;
        let child = scheduler
            .tasks
            .iter()
            .find(|task| task.pid == pid && task.parent_pid == Some(parent_pid))
            .expect("wait selected an invalid child");
        assert_eq!(
            child.state,
            TaskState::Runnable,
            "wait selected a dead child"
        );

        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous].interrupt_context.rbx = context.rbx;
        scheduler.tasks[previous].interrupt_context.rbp = context.rbp;
        scheduler.tasks[previous].interrupt_context.r12 = context.r12;
        scheduler.tasks[previous].interrupt_context.r13 = context.r13;
        scheduler.tasks[previous].interrupt_context.r14 = context.r14;
        scheduler.tasks[previous].interrupt_context.r15 = context.r15;
        scheduler.tasks[previous].interrupt_context.rax = context.return_value;
        scheduler.tasks[previous]
            .interrupt_context
            .instruction_pointer = context.instruction_pointer;
        scheduler.tasks[previous].interrupt_context.flags = context.flags;
        scheduler.tasks[previous].interrupt_context.stack_pointer = context.stack_pointer;
        scheduler.tasks[previous].state = TaskState::Waiting { child_pid: pid };

        let next = next_alive(scheduler, previous).expect("wait left no runnable task");
        scheduler.current = next;
        scheduler.slice_ticks = 0;
        let task = &mut scheduler.tasks[next];
        let process = task
            .process
            .as_mut()
            .expect("scheduler selected an exited task");
        (task.context, process.address_space(), previous, next)
    };
    crate::serial_println!(
        "[sched] wait pid={} child={} -> {}",
        scheduler_pid(previous),
        pid,
        scheduler_pid(next)
    );
    crate::syscall::prepare_user_return(next_context, next_space)
}

fn new_task(
    pid: u64,
    parent_pid: Option<u64>,
    credentials: Credentials,
    process: Box<Process>,
    descriptors: Vec<Option<FileDescriptor>>,
) -> Task {
    Task {
        pid,
        parent_pid,
        state: TaskState::Runnable,
        context: UserContext {
            return_value: 0,
            rbx: 0,
            rbp: 0,
            r12: process.user_stack_top(),
            r13: 0,
            r14: 0,
            r15: 0,
            instruction_pointer: process.entry(),
            flags: 0x202,
            stack_pointer: process.user_stack_top(),
        },
        interrupt_context: InterruptContext::initial(process.entry(), process.user_stack_top()),
        process: Some(process),
        descriptors,
        credentials,
        signal_actions: [SignalAction::default(); 32],
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
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    let initial_offset = if append { contents.len() } else { 0 };
    let mut rights = Rights::READ | Rights::TRANSFER;
    if writable {
        rights |= Rights::WRITE;
    }
    install_descriptor(
        descriptors,
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
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    install_descriptor(
        descriptors,
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
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    install_descriptor(
        descriptors,
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
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    let duplicate = descriptors
        .get(index)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())?;
    if duplicate.capability.is_invalid() || !duplicate.rights.contains(Rights::TRANSFER) {
        return Err(());
    }
    install_descriptor(descriptors, duplicate)
}

pub fn open_pipe_current() -> Result<(u64, u64), ()> {
    let (reader, writer) = Pipe::new();
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    let reader = install_descriptor(
        descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: DescriptorResource::PipeRead(Arc::new(Mutex::new(reader))),
        },
    )?;
    let writer = install_descriptor(
        descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::WRITE | Rights::TRANSFER,
            resource: DescriptorResource::PipeWrite(Arc::new(Mutex::new(writer))),
        },
    )?;
    Ok((reader, writer))
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
        _ => false,
    }
}

pub fn close_current(descriptor: u64) -> Result<(), ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let descriptor = {
        let mut scheduler = current_scheduler().lock();
        let scheduler = scheduler.as_mut().ok_or(())?;
        scheduler.tasks[scheduler.current]
            .descriptors
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
        | DescriptorResource::PipeRead(_) => {}
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
            writer.lock().write(bytes);
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
        DescriptorResource::Directory(_) | DescriptorResource::PipeRead(_) => Err(()),
    }
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
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.tasks[scheduler.current].descriptors.get(index))
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())
}

fn scheduler_pid(index: usize) -> u64 {
    current_scheduler()
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks[index].pid)
        .unwrap_or(0)
}

fn task_count() -> usize {
    current_scheduler()
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks.len())
        .unwrap_or(0)
}
