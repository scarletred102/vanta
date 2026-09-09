#![allow(dead_code, unused_imports)]

use spin::Mutex;
pub use crate::paging::MAP_SWAPPED;

pub const MAX_SWAP_SLOTS: usize = 16384;
const BITMAP_WORDS: usize = MAX_SWAP_SLOTS / 64;

pub struct SwapManager {
    bitmap: [u64; BITMAP_WORDS],
    used_slots: usize,
    clock_hand: u64,
}

impl SwapManager {
    pub const fn new() -> Self {
        Self {
            bitmap: [0; BITMAP_WORDS],
            used_slots: 0,
            clock_hand: 0,
        }
    }

    pub fn alloc_slot(&mut self) -> Option<u32> {
        for (word_idx, word) in self.bitmap.iter_mut().enumerate() {
            if *word != !0u64 {
                let bit_idx = (!*word).trailing_zeros() as usize;
                *word |= 1u64 << bit_idx;
                self.used_slots += 1;
                return Some((word_idx * 64 + bit_idx) as u32);
            }
        }
        None
    }

    pub fn free_slot(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot >= MAX_SWAP_SLOTS {
            return;
        }
        let word_idx = slot / 64;
        let bit_idx = slot % 64;
        if self.bitmap[word_idx] & (1u64 << bit_idx) != 0 {
            self.bitmap[word_idx] &= !(1u64 << bit_idx);
            self.used_slots = self.used_slots.saturating_sub(1);
        }
    }

    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    pub fn free_slots(&self) -> usize {
        MAX_SWAP_SLOTS.saturating_sub(self.used_slots)
    }
}

pub static SWAP_MANAGER: Mutex<SwapManager> = Mutex::new(SwapManager::new());
