//! Cooperative single-CPU scheduler for user processes.

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;

use crate::paging::{self, AddressSpace};
use crate::process::Process;
use crate::syscall::UserContext;

struct Task {
    process: Option<Box<Process>>,
    context: UserContext,
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
        .map(|process| Task {
            context: UserContext {
                instruction_pointer: process.entry(),
                flags: 0x202,
                stack_pointer: process.user_stack_top(),
            },
            process: Some(process),
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
        crate::serial_println!("[sched] yield task={} -> {}", previous, next);
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
    crate::serial_println!("[sched] continue task={}", next);
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

fn task_count() -> usize {
    SCHEDULER
        .lock()
        .as_ref()
        .map(|scheduler| scheduler.tasks.len())
        .unwrap_or(0)
}
