//! Input subsystem for mouse cursor motion, clicks, and keyboard events.

use alloc::collections::VecDeque;
use spin::Mutex;
use vanta_abi::InputEvent;

static EVENT_QUEUE: Mutex<VecDeque<InputEvent>> = Mutex::new(VecDeque::new());
const MAX_EVENTS: usize = 256;

pub fn push_event(event: InputEvent) {
    let mut queue = EVENT_QUEUE.lock();
    if queue.len() >= MAX_EVENTS {
        queue.pop_front();
    }
    queue.push_back(event);
}

pub fn poll_event() -> Option<InputEvent> {
    EVENT_QUEUE.lock().pop_front()
}

pub fn inject_mouse_motion(dx: i32, dy: i32, buttons: u32) {
    push_event(InputEvent {
        event_type: 1, // Mouse
        code: buttons,
        value: 0,
        x: dx,
        y: dy,
    });
}

pub fn inject_key(key_code: u32, pressed: bool) {
    push_event(InputEvent {
        event_type: 2, // Key
        code: key_code,
        value: if pressed { 1 } else { 0 },
        x: 0,
        y: 0,
    });
}
