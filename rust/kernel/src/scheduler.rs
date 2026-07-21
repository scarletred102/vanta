//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use vanta_abi::{CapabilityId, Credentials, Rights};

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
    Socket(Arc<Mutex<OpenSocket>>),
}

struct OpenFile {
    contents: Vec<u8>,
    offset: usize,
}

struct OpenSocket {
    connection: Option<crate::network::TcpConnection>,
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
    start_on_current_cpu(processes, "started")
}

pub unsafe fn start_ap(processes: Vec<Box<Process>>) -> ! {
    start_on_current_cpu(processes, "AP run queue started")
}

unsafe fn start_on_current_cpu(processes: Vec<Box<Process>>, label: &str) -> ! {
    let kernel_space = paging::current_address_space();
    if processes.is_empty() {
        crate::shell::run();
    }
    let tasks = processes
        .into_iter()
        .map(|process| new_task(allocate_pid(), None, Credentials::vanta(), process))
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

pub fn spawn_current(process: Box<Process>) -> Result<u64, ()> {
    const MAX_TASKS: usize = 8;
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    if scheduler.tasks.len() == MAX_TASKS {
        return Err(());
    }
    let parent_pid = scheduler.tasks[scheduler.current].pid;
    let credentials = scheduler.tasks[scheduler.current].credentials;
    let pid = allocate_pid();
    scheduler
        .tasks
        .push(new_task(pid, Some(parent_pid), credentials, process));
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
            r12: 0,
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
) -> Task {
    Task {
        pid,
        parent_pid,
        state: TaskState::Runnable,
        context: UserContext {
            return_value: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            instruction_pointer: process.entry(),
            flags: 0x202,
            stack_pointer: process.user_stack_top(),
        },
        interrupt_context: InterruptContext::initial(process.entry(), process.user_stack_top()),
        process: Some(process),
        descriptors: alloc::vec![None, None, None],
        credentials,
    }
}

pub fn open_current(contents: Vec<u8>) -> Result<u64, ()> {
    let mut scheduler = current_scheduler().lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    install_descriptor(
        descriptors,
        FileDescriptor {
            capability: allocate_capability(),
            rights: Rights::READ | Rights::TRANSFER,
            resource: DescriptorResource::File(Arc::new(Mutex::new(OpenFile {
                contents,
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

fn install_descriptor(
    descriptors: &mut Vec<Option<FileDescriptor>>,
    descriptor: FileDescriptor,
) -> Result<u64, ()> {
    const MAX_DESCRIPTORS: usize = 5;
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
        DescriptorResource::Socket(socket) => {
            let mut socket = socket.lock();
            let connection = socket.connection.as_mut().ok_or(())?;
            crate::network::tcp_receive(connection, length).map_err(|_| ())
        }
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
    let DescriptorResource::Socket(socket) = descriptor.resource else {
        return Ok(());
    };
    if Arc::strong_count(&socket) != 1 {
        return Ok(());
    }
    if let Some(connection) = socket.lock().connection.take() {
        crate::network::tcp_close(connection).map_err(|_| ())?;
    }
    Ok(())
}

pub fn write_current(descriptor: u64, bytes: &[u8]) -> Result<(), ()> {
    let descriptor = current_descriptor(descriptor)?;
    if !descriptor.rights.contains(Rights::WRITE) {
        return Err(());
    }
    let DescriptorResource::Socket(socket) = descriptor.resource else {
        return Err(());
    };
    let mut socket = socket.lock();
    let connection = socket.connection.as_mut().ok_or(())?;
    crate::network::tcp_send(connection, bytes).map_err(|_| ())
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
