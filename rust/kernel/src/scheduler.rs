//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use crate::paging::{self, AddressSpace};
use crate::process::Process;
use crate::syscall::UserContext;

const TIMER_TICKS_PER_SLICE: u64 = 3;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Runnable,
    Zombie { exit_code: u64 },
}

#[derive(Clone)]
struct FileDescriptor {
    file: Arc<Mutex<OpenFile>>,
}

struct OpenFile {
    contents: Vec<u8>,
    offset: usize,
}

struct Scheduler {
    tasks: Vec<Task>,
    current: usize,
    kernel_space: AddressSpace,
    ticks: u64,
    slice_ticks: u64,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

pub unsafe fn start(processes: Vec<Box<Process>>) -> ! {
    start_on_current_cpu(processes, "started")
}

pub unsafe fn start_ap_test(process: Box<Process>) -> ! {
    let mut processes = Vec::with_capacity(1);
    processes.push(process);
    start_on_current_cpu(processes, "AP user test started")
}

unsafe fn start_on_current_cpu(processes: Vec<Box<Process>>, label: &str) -> ! {
    let kernel_space = paging::current_address_space();
    if processes.is_empty() {
        crate::shell::run();
    }
    let tasks = processes
        .into_iter()
        .enumerate()
        .map(|(index, process)| Task {
            pid: (index + 1) as u64,
            parent_pid: None,
            state: TaskState::Runnable,
            context: UserContext {
                instruction_pointer: process.entry(),
                flags: 0x202,
                stack_pointer: process.user_stack_top(),
            },
            interrupt_context: InterruptContext::initial(process.entry(), process.user_stack_top()),
            process: Some(process),
            descriptors: alloc::vec![None, None, None],
        })
        .collect();
    *SCHEDULER.lock() = Some(Scheduler {
        tasks,
        current: 0,
        kernel_space,
        ticks: 0,
        slice_ticks: 0,
    });

    let (context, space) = current_target();
    crate::serial_println!("[sched] {} tasks={}", label, task_count());
    crate::syscall::prepare_user_return(context, space);
    unsafe { crate::gdt::enter_user(context.instruction_pointer, context.stack_pointer) }
}

pub fn yield_current(context: UserContext) -> *const UserContext {
    let (next_context, next_space, previous, next) = {
        let mut scheduler = SCHEDULER.lock();
        let scheduler = scheduler.as_mut().expect("yield without scheduler");
        let previous = scheduler.current;
        scheduler.tasks[previous].context = context;
        scheduler.tasks[previous]
            .interrupt_context
            .instruction_pointer = context.instruction_pointer;
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
        let mut scheduler = SCHEDULER.lock();
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
        let mut scheduler = SCHEDULER.lock();
        let scheduler = scheduler.as_mut().expect("process exit without scheduler");
        let current = scheduler.current;
        let parent_pid = scheduler.tasks[current].parent_pid;
        let process = scheduler.tasks[current]
            .process
            .take()
            .expect("current task already exited");
        scheduler.tasks[current].state = TaskState::Zombie { exit_code: code };
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
        *SCHEDULER.lock() = None;
        if crate::smp::ap_user_task_active() {
            crate::smp::finish_user_task();
        }
        x86_64::instructions::interrupts::enable();
        crate::shell::run()
    };
    crate::serial_println!("[sched] continue pid={}", scheduler_pid(next));
    crate::syscall::prepare_user_return(context, space)
}

fn current_target() -> (UserContext, AddressSpace) {
    let mut scheduler = SCHEDULER.lock();
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
    let scheduler = SCHEDULER.lock();
    scheduler
        .as_ref()
        .map(|scheduler| scheduler.tasks[scheduler.current].pid)
        .unwrap_or(0)
}

pub fn open_current(contents: Vec<u8>) -> Result<u64, ()> {
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    install_descriptor(
        descriptors,
        FileDescriptor {
            file: Arc::new(Mutex::new(OpenFile {
                contents,
                offset: 0,
            })),
        },
    )
}

pub fn duplicate_current(descriptor: u64) -> Result<u64, ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    let duplicate = descriptors
        .get(index)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or(())?;
    install_descriptor(descriptors, duplicate)
}

pub fn seek_current(descriptor: u64, offset: i64, whence: u64) -> Result<u64, ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptor = scheduler.tasks[scheduler.current]
        .descriptors
        .get_mut(index)
        .and_then(Option::as_mut)
        .ok_or(())?;
    let mut file = descriptor.file.lock();
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
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptor = scheduler.tasks[scheduler.current]
        .descriptors
        .get_mut(index)
        .and_then(Option::as_mut)
        .ok_or(())?;
    let mut file = descriptor.file.lock();
    let end = file.offset.saturating_add(length).min(file.contents.len());
    let bytes = file.contents[file.offset..end].to_vec();
    file.offset = end;
    Ok(bytes)
}

pub fn close_current(descriptor: u64) -> Result<(), ()> {
    let index: usize = descriptor.try_into().map_err(|_| ())?;
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptor = scheduler.tasks[scheduler.current]
        .descriptors
        .get_mut(index)
        .ok_or(())?;
    if descriptor.is_none() {
        return Err(());
    }
    *descriptor = None;
    Ok(())
}

fn scheduler_pid(index: usize) -> u64 {
    SCHEDULER
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks[index].pid)
        .unwrap_or(0)
}

fn task_count() -> usize {
    SCHEDULER
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks.len())
        .unwrap_or(0)
}
