//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;

use crate::paging::{self, AddressSpace};
use crate::process::Process;
use crate::syscall::UserContext;

struct Task {
    pid: u64,
    process: Option<Box<Process>>,
    context: UserContext,
    descriptors: Vec<Option<FileDescriptor>>,
}

struct FileDescriptor {
    contents: Vec<u8>,
    offset: usize,
}

struct Scheduler {
    tasks: Vec<Task>,
    current: usize,
    kernel_space: AddressSpace,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

pub unsafe fn start(processes: Vec<Box<Process>>) -> ! {
    let kernel_space = paging::current_address_space();
    if processes.is_empty() {
        crate::shell::run();
    }
    let tasks = processes
        .into_iter()
        .enumerate()
        .map(|(index, process)| Task {
            pid: (index + 1) as u64,
            context: UserContext {
                instruction_pointer: process.entry(),
                flags: 0x202,
                stack_pointer: process.user_stack_top(),
            },
            process: Some(process),
            descriptors: Vec::new(),
        })
        .collect();
    *SCHEDULER.lock() = Some(Scheduler {
        tasks,
        current: 0,
        kernel_space,
    });

    let (context, space) = current_target();
    crate::serial_println!("[sched] started tasks={}", task_count());
    crate::syscall::prepare_user_return(context, space);
    unsafe { crate::gdt::enter_user(context.instruction_pointer, context.stack_pointer) }
}

pub fn yield_current(context: UserContext) -> *const UserContext {
    let (next_context, next_space, previous, next) = {
        let mut scheduler = SCHEDULER.lock();
        let scheduler = scheduler.as_mut().expect("yield without scheduler");
        let previous = scheduler.current;
        scheduler.tasks[previous].context = context;
        let next = next_alive(scheduler, previous).unwrap_or(previous);
        scheduler.current = next;
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

pub fn exit_current(code: u64) -> *const UserContext {
    let (next, remaining) = {
        let mut scheduler = SCHEDULER.lock();
        let scheduler = scheduler.as_mut().expect("process exit without scheduler");
        let current = scheduler.current;
        let process = scheduler.tasks[current]
            .process
            .take()
            .expect("current task already exited");
        let kernel_space = scheduler.kernel_space;
        unsafe {
            paging::activate(kernel_space);
        }
        drop(process);
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
        (next, remaining)
    };

    crate::serial_println!("[sched] task exited: code={} remaining={}", code, remaining);
    let Some((context, space, next)) = next else {
        *SCHEDULER.lock() = None;
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
        if scheduler.tasks[index].process.is_some() {
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
    const MAX_DESCRIPTORS: usize = 4;
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut().ok_or(())?;
    let descriptors = &mut scheduler.tasks[scheduler.current].descriptors;
    if let Some((index, descriptor)) = descriptors
        .iter_mut()
        .enumerate()
        .find(|(_, descriptor)| descriptor.is_none())
    {
        *descriptor = Some(FileDescriptor {
            contents,
            offset: 0,
        });
        return Ok(index as u64);
    }
    if descriptors.len() == MAX_DESCRIPTORS {
        return Err(());
    }
    descriptors.push(Some(FileDescriptor {
        contents,
        offset: 0,
    }));
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
    let end = descriptor
        .offset
        .saturating_add(length)
        .min(descriptor.contents.len());
    let bytes = descriptor.contents[descriptor.offset..end].to_vec();
    descriptor.offset = end;
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
