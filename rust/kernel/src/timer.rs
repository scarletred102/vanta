//! Hierarchical Timer Wheel & Monotonic Time Subsystem for Vanta Microkernel.
//!
//! Provides O(1) timer insertion, cancellation, and expiry for concurrent tasks.
//! Level 0: 256 slots (Spans 0 to 255 ms, granular tick = 1 ms)
//! Level 1: 64 slots  (Spans 256 ms to ~16.384 s, 256 ms per slot)
//! Level 2: 64 slots  (Spans 16.384 s to ~17.47 min, 16.384 s per slot)
//! Level 3: 64 slots  (Spans 17.47 min to ~18.64 hr, 1048.576 s per slot)
//! Overflow: Auxiliary sorted list for delays > 18.64 hours.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use spin::Mutex;

const LVL0_SIZE: usize = 256;
const LVL1_SIZE: usize = 64;
const LVL2_SIZE: usize = 64;
const LVL3_SIZE: usize = 64;

const LVL0_BITS: u32 = 8;
const LVL1_BITS: u32 = 6;
const LVL2_BITS: u32 = 6;
const LVL3_BITS: u32 = 6;

const LVL0_SHIFT: u32 = 0;
const LVL1_SHIFT: u32 = LVL0_BITS;                     // 8
const LVL2_SHIFT: u32 = LVL1_SHIFT + LVL1_BITS;        // 14
const LVL3_SHIFT: u32 = LVL2_SHIFT + LVL2_BITS;        // 20

const LVL0_SPAN: u64 = 1 << LVL1_SHIFT;                // 256
const LVL1_SPAN: u64 = 1 << LVL2_SHIFT;                // 16_384
const LVL2_SPAN: u64 = 1 << LVL3_SHIFT;                // 1_048_576
const LVL3_SPAN: u64 = 1 << (LVL3_SHIFT + LVL3_BITS);  // 67_108_864

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    Sleep { tid: u64 },
    ITimerReal { pid: u64, interval_ticks: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerEntry {
    pub id: u64,
    pub target_tick: u64,
    pub kind: TimerKind,
}

pub struct TimerWheel {
    current_tick: u64,
    level0: [Vec<TimerEntry>; LVL0_SIZE],
    level1: [Vec<TimerEntry>; LVL1_SIZE],
    level2: [Vec<TimerEntry>; LVL2_SIZE],
    level3: [Vec<TimerEntry>; LVL3_SIZE],
    overflow: Vec<TimerEntry>,
    next_timer_id: u64,
}

impl TimerWheel {
    pub const fn new() -> Self {
        Self {
            current_tick: 0,
            level0: [const { Vec::new() }; LVL0_SIZE],
            level1: [const { Vec::new() }; LVL1_SIZE],
            level2: [const { Vec::new() }; LVL2_SIZE],
            level3: [const { Vec::new() }; LVL3_SIZE],
            overflow: Vec::new(),
            next_timer_id: 1,
        }
    }

    pub fn add_timer(&mut self, target_tick: u64, kind: TimerKind) -> u64 {
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1);
        let entry = TimerEntry { id, target_tick, kind };
        self.reinsert(entry);
        id
    }

    fn reinsert(&mut self, entry: TimerEntry) {
        let delta = entry.target_tick.saturating_sub(self.current_tick);
        if delta < LVL0_SPAN {
            let slot = (entry.target_tick & 0xFF) as usize;
            self.level0[slot].push(entry);
        } else if delta < LVL1_SPAN {
            let slot = ((entry.target_tick >> LVL1_SHIFT) & 0x3F) as usize;
            self.level1[slot].push(entry);
        } else if delta < LVL2_SPAN {
            let slot = ((entry.target_tick >> LVL2_SHIFT) & 0x3F) as usize;
            self.level2[slot].push(entry);
        } else if delta < LVL3_SPAN {
            let slot = ((entry.target_tick >> LVL3_SHIFT) & 0x3F) as usize;
            self.level3[slot].push(entry);
        } else {
            self.overflow.push(entry);
        }
    }

    pub fn advance_tick(&mut self) -> Vec<TimerEntry> {
        self.current_tick = self.current_tick.wrapping_add(1);
        let tick = self.current_tick;

        // Cascade level 1 on 8-bit rollover (every 256 ms)
        if (tick & 0xFF) == 0 {
            let slot1 = ((tick >> LVL1_SHIFT) & 0x3F) as usize;
            let entries = core::mem::take(&mut self.level1[slot1]);
            for entry in entries {
                self.reinsert(entry);
            }
        }

        // Cascade level 2 on 14-bit rollover (every 16.384 s)
        if (tick & 0x3FFF) == 0 {
            let slot2 = ((tick >> LVL2_SHIFT) & 0x3F) as usize;
            let entries = core::mem::take(&mut self.level2[slot2]);
            for entry in entries {
                self.reinsert(entry);
            }
        }

        // Cascade level 3 on 20-bit rollover (every ~17.47 min)
        if (tick & 0xF_FFFF) == 0 {
            let slot3 = ((tick >> LVL3_SHIFT) & 0x3F) as usize;
            let entries = core::mem::take(&mut self.level3[slot3]);
            for entry in entries {
                self.reinsert(entry);
            }

            // Promote from overflow list
            let mut i = 0;
            while i < self.overflow.len() {
                if self.overflow[i].target_tick.saturating_sub(self.current_tick) < LVL3_SPAN {
                    let entry = self.overflow.swap_remove(i);
                    self.reinsert(entry);
                } else {
                    i += 1;
                }
            }
        }

        let slot0 = (tick & 0xFF) as usize;
        let mut expired = Vec::new();
        let mut remaining = Vec::new();
        for entry in core::mem::take(&mut self.level0[slot0]) {
            if entry.target_tick <= self.current_tick {
                expired.push(entry);
            } else {
                remaining.push(entry);
            }
        }
        self.level0[slot0] = remaining;
        expired
    }

    pub fn cancel_sleep_timer(&mut self, target_tid: u64) -> Option<u64> {
        for slot in &mut self.level0 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::Sleep { tid } if tid == target_tid)) {
                return Some(slot.remove(pos).target_tick);
            }
        }
        for slot in &mut self.level1 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::Sleep { tid } if tid == target_tid)) {
                return Some(slot.remove(pos).target_tick);
            }
        }
        for slot in &mut self.level2 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::Sleep { tid } if tid == target_tid)) {
                return Some(slot.remove(pos).target_tick);
            }
        }
        for slot in &mut self.level3 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::Sleep { tid } if tid == target_tid)) {
                return Some(slot.remove(pos).target_tick);
            }
        }
        if let Some(pos) = self.overflow.iter().position(|e| matches!(e.kind, TimerKind::Sleep { tid } if tid == target_tid)) {
            return Some(self.overflow.remove(pos).target_tick);
        }
        None
    }

    pub fn cancel_itimer(&mut self, target_pid: u64) -> Option<(u64, u64)> {
        for slot in &mut self.level0 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                let e = slot.remove(pos);
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &mut self.level1 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                let e = slot.remove(pos);
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &mut self.level2 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                let e = slot.remove(pos);
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &mut self.level3 {
            if let Some(pos) = slot.iter().position(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                let e = slot.remove(pos);
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        if let Some(pos) = self.overflow.iter().position(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
            let e = self.overflow.remove(pos);
            if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
            }
        }
        None
    }

    pub fn get_itimer(&self, target_pid: u64) -> Option<(u64, u64)> {
        for slot in &self.level0 {
            if let Some(e) = slot.iter().find(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &self.level1 {
            if let Some(e) = slot.iter().find(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &self.level2 {
            if let Some(e) = slot.iter().find(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        for slot in &self.level3 {
            if let Some(e) = slot.iter().find(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
                if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                    return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
                }
            }
        }
        if let Some(e) = self.overflow.iter().find(|e| matches!(e.kind, TimerKind::ITimerReal { pid, .. } if pid == target_pid)) {
            if let TimerKind::ITimerReal { interval_ticks, .. } = e.kind {
                return Some((interval_ticks, e.target_tick.saturating_sub(self.current_tick)));
            }
        }
        None
    }

    pub fn rearm_itimer_entry(&mut self, entry_id: u64, target_tick: u64) {
        // Find existing itimer entry and update target_tick
        // If found, reinsert with updated target_tick
        let mut found = None;
        for slot in &mut self.level0 {
            if let Some(pos) = slot.iter().position(|e| e.id == entry_id) {
                found = Some(slot.remove(pos));
                break;
            }
        }
        if found.is_none() {
            for slot in &mut self.level1 {
                if let Some(pos) = slot.iter().position(|e| e.id == entry_id) {
                    found = Some(slot.remove(pos));
                    break;
                }
            }
        }
        if let Some(mut e) = found {
            e.target_tick = target_tick;
            self.reinsert(e);
        }
    }
}

static WHEEL: Mutex<TimerWheel> = Mutex::new(TimerWheel::new());
static GLOBAL_TICKS: AtomicU64 = AtomicU64::new(0);
static REALTIME_BASE_SEC: AtomicI64 = AtomicI64::new(1_789_100_000);
static RTC_INITIALIZED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn read_rtc_seconds() -> Option<u64> {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut addr_port: Port<u8> = Port::new(0x70);
        let mut data_port: Port<u8> = Port::new(0x71);

        let mut attempts = 0;
        loop {
            addr_port.write(0x0A);
            if (data_port.read() & 0x80) == 0 {
                break;
            }
            attempts += 1;
            if attempts > 10000 {
                return None;
            }
        }

        addr_port.write(0x00);
        let sec = data_port.read();
        addr_port.write(0x02);
        let min = data_port.read();
        addr_port.write(0x04);
        let hour = data_port.read();
        addr_port.write(0x07);
        let day = data_port.read();
        addr_port.write(0x08);
        let month = data_port.read();
        addr_port.write(0x09);
        let year = data_port.read();
        addr_port.write(0x32);
        let century = data_port.read();

        addr_port.write(0x0B);
        let reg_b = data_port.read();

        let is_bcd = (reg_b & 0x04) == 0;
        let decode = |v: u8| -> u32 {
            if is_bcd {
                ((v >> 4) * 10 + (v & 0x0F)) as u32
            } else {
                v as u32
            }
        };

        let sec = decode(sec);
        let min = decode(min);
        let mut hour = decode(hour & 0x7F);
        if (reg_b & 0x02) == 0 && (hour & 0x80) != 0 {
            hour = ((hour & 0x7F) + 12) % 24;
        }
        let day = decode(day);
        let month = decode(month);
        let year = decode(year);
        let century = decode(century);

        let full_year = if century >= 19 && century <= 22 {
            century * 100 + year
        } else if year >= 70 {
            1900 + year
        } else {
            2000 + year
        };

        if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || min > 59 || sec > 59 {
            return None;
        }

        let mut days = 0u64;
        for y in 1970..full_year {
            let leap = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            days += leap;
        }
        let leap = if full_year % 4 == 0 && (full_year % 100 != 0 || full_year % 400 == 0) { 1 } else { 0 };
        let days_before = match month {
            1 => 0,
            2 => 31,
            3 => 59 + leap,
            4 => 90 + leap,
            5 => 120 + leap,
            6 => 151 + leap,
            7 => 181 + leap,
            8 => 212 + leap,
            9 => 243 + leap,
            10 => 273 + leap,
            11 => 304 + leap,
            12 => 334 + leap,
            _ => 0,
        };
        days += days_before as u64 + (day - 1) as u64;
        let unix_secs = days * 86400 + (hour as u64) * 3600 + (min as u64) * 60 + (sec as u64);
        Some(unix_secs)
    }
}

fn sync_rtc_if_needed() {
    if !RTC_INITIALIZED.swap(true, Ordering::SeqCst) {
        if let Some(rtc_sec) = read_rtc_seconds() {
            if rtc_sec >= 1_700_000_000 {
                REALTIME_BASE_SEC.store(rtc_sec as i64, Ordering::Release);
                crate::serial_println!("[rtc] system realtime clock synchronized to RTC: {} (UNIX)", rtc_sec);
            }
        }
    }
}

pub fn tick() -> Vec<TimerEntry> {
    GLOBAL_TICKS.fetch_add(1, Ordering::SeqCst);
    let mut wheel = WHEEL.lock();
    wheel.advance_tick()
}

pub fn current_tick() -> u64 {
    GLOBAL_TICKS.load(Ordering::Acquire)
}

pub fn current_monotonic_ms() -> u64 {
    current_tick()
}

pub fn add_sleep_timer(tid: u64, duration_ms: u64) -> u64 {
    let mut wheel = WHEEL.lock();
    let target_tick = wheel.current_tick.saturating_add(duration_ms.max(1));
    wheel.add_timer(target_tick, TimerKind::Sleep { tid });
    target_tick
}

pub fn cancel_sleep_timer(tid: u64) -> Option<u64> {
    let mut wheel = WHEEL.lock();
    wheel.cancel_sleep_timer(tid)
}

pub fn set_itimer(pid: u64, which: u32, interval_ms: u64, value_ms: u64) -> (u64, u64) {
    let mut wheel = WHEEL.lock();
    let old = wheel.cancel_itimer(pid).unwrap_or((0, 0));
    if value_ms > 0 && which == 0 {
        let target_tick = wheel.current_tick.saturating_add(value_ms);
        wheel.add_timer(target_tick, TimerKind::ITimerReal { pid, interval_ticks: interval_ms });
    }
    old
}

pub fn get_itimer(pid: u64, which: u32) -> (u64, u64) {
    if which != 0 {
        return (0, 0);
    }
    let wheel = WHEEL.lock();
    wheel.get_itimer(pid).unwrap_or((0, 0))
}

pub fn rearm_itimer(entry_id: u64, interval_ticks: u64) {
    let mut wheel = WHEEL.lock();
    let target_tick = wheel.current_tick.saturating_add(interval_ticks);
    wheel.rearm_itimer_entry(entry_id, target_tick);
}

pub fn get_clock_time(clock_id: u64) -> (u64, u64) {
    let ms = current_tick();
    let sec = ms / 1000;
    let nsec = (ms % 1000) * 1_000_000;
    match clock_id {
        0 => { // CLOCK_REALTIME
            sync_rtc_if_needed();
            let base = REALTIME_BASE_SEC.load(Ordering::Acquire);
            let real_sec = (base as u64).wrapping_add(sec);
            (real_sec, nsec)
        }
        1 | 4 | 7 => { // CLOCK_MONOTONIC, CLOCK_MONOTONIC_RAW, CLOCK_BOOTTIME
            (sec, nsec)
        }
        2 | 3 => { // CLOCK_PROCESS_CPUTIME_ID, CLOCK_THREAD_CPUTIME_ID
            let cpu_ms = crate::scheduler::current_cpu_ticks();
            let cpu_sec = cpu_ms / 1000;
            let cpu_nsec = (cpu_ms % 1000) * 1_000_000;
            (cpu_sec, cpu_nsec)
        }
        _ => (sec, nsec),
    }
}

pub fn set_clock_time(clock_id: u64, sec: u64, _nsec: u64) -> Result<(), ()> {
    if clock_id != 0 {
        return Err(());
    }
    let ms = current_tick();
    let current_sec = ms / 1000;
    let new_base = (sec as i64) - (current_sec as i64);
    REALTIME_BASE_SEC.store(new_base, Ordering::Release);
    Ok(())
}
