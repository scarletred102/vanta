//! Cryptographically Secure Pseudo-Random Number Generator (CSPRNG) for Vanta OS.
//!
//! Implements the ChaCha20 stream cipher in CSPRNG mode (RFC 8439) to back the Linux
//! `getrandom(2)` syscall, TLS ephemeral key generation, and kernel security tokens.
//!
//! Seeded from hardware entropy sources (x86 RDRAND instruction, RDTSC jitter,
//! and APIC timer entropy). Note: Phase 5 will introduce the full cryptographic
//! subsystem and VirtIO-RNG device support.

#![allow(dead_code)]

use spin::Mutex;

const CHACHA_CONSTANTS: [u32; 4] = [
    0x6170_7865, // "expa"
    0x3320_646e, // "nd 3"
    0x7962_2d32, // "2-by"
    0x6b20_6574, // "te k"
];

pub struct ChaCha20Csprng {
    state: [u32; 16],
    buffer: [u8; 64],
    buffer_idx: usize,
    counter: u64,
    bytes_generated: usize,
}

impl ChaCha20Csprng {
    pub const fn new() -> Self {
        Self {
            state: [0; 16],
            buffer: [0; 64],
            buffer_idx: 64, // Empty initially
            counter: 0,
            bytes_generated: 0,
        }
    }

    /// Initializes or reseeds the generator with hardware entropy.
    pub fn seed_with_entropy(&mut self) {
        let mut key = [0u32; 8];
        for i in 0..8 {
            key[i] = (get_hardware_entropy() ^ (get_hardware_entropy() >> 32)) as u32;
        }

        self.state[0] = CHACHA_CONSTANTS[0];
        self.state[1] = CHACHA_CONSTANTS[1];
        self.state[2] = CHACHA_CONSTANTS[2];
        self.state[3] = CHACHA_CONSTANTS[3];

        for i in 0..8 {
            self.state[4 + i] = key[i];
        }

        self.counter = 1;
        self.state[12] = self.counter as u32;
        self.state[13] = (self.counter >> 32) as u32;
        self.state[14] = (get_hardware_entropy() as u32) ^ 0xfeed_face;
        self.state[15] = ((get_hardware_entropy() >> 32) as u32) ^ 0xdead_beef;

        self.buffer_idx = 64;
        self.bytes_generated = 0;
    }

    /// Generates a single 64-byte block using 20 ChaCha rounds.
    fn generate_block(&mut self) {
        if self.state[0] != CHACHA_CONSTANTS[0] || self.bytes_generated >= 65536 {
            self.seed_with_entropy();
        }

        self.counter = self.counter.wrapping_add(1);
        self.state[12] = self.counter as u32;
        self.state[13] = (self.counter >> 32) as u32;

        let mut x = self.state;

        // 20 rounds (10 double-rounds)
        for _ in 0..10 {
            // Column round
            quarter_round(&mut x, 0, 4, 8, 12);
            quarter_round(&mut x, 1, 5, 9, 13);
            quarter_round(&mut x, 2, 6, 10, 14);
            quarter_round(&mut x, 3, 7, 11, 15);

            // Diagonal round
            quarter_round(&mut x, 0, 5, 10, 15);
            quarter_round(&mut x, 1, 6, 11, 12);
            quarter_round(&mut x, 2, 7, 8, 13);
            quarter_round(&mut x, 3, 4, 9, 14);
        }

        for i in 0..16 {
            let val = x[i].wrapping_add(self.state[i]);
            let bytes = val.to_le_bytes();
            self.buffer[i * 4] = bytes[0];
            self.buffer[i * 4 + 1] = bytes[1];
            self.buffer[i * 4 + 2] = bytes[2];
            self.buffer[i * 4 + 3] = bytes[3];
        }

        self.buffer_idx = 0;
        self.bytes_generated += 64;
    }

    /// Fills the destination buffer with cryptographically secure random bytes.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut offset = 0;
        while offset < dest.len() {
            if self.buffer_idx >= 64 {
                self.generate_block();
            }

            let available = 64 - self.buffer_idx;
            let needed = dest.len() - offset;
            let to_copy = available.min(needed);

            dest[offset..offset + to_copy]
                .copy_from_slice(&self.buffer[self.buffer_idx..self.buffer_idx + to_copy]);

            self.buffer_idx += to_copy;
            offset += to_copy;
        }
    }
}

#[inline(always)]
fn quarter_round(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    x[a] = x[a].wrapping_add(x[b]);
    x[d] ^= x[a];
    x[d] = x[d].rotate_left(16);

    x[c] = x[c].wrapping_add(x[d]);
    x[b] ^= x[c];
    x[b] = x[b].rotate_left(12);

    x[a] = x[a].wrapping_add(x[b]);
    x[d] ^= x[a];
    x[d] = x[d].rotate_left(8);

    x[c] = x[c].wrapping_add(x[d]);
    x[b] ^= x[c];
    x[b] = x[b].rotate_left(7);
}

use crate::virtio_rng::VirtioRng;

static VIRTIO_RNG: Mutex<Option<VirtioRng>> = Mutex::new(None);
static ENTROPY_SOURCE: Mutex<EntropySource> = Mutex::new(EntropySource::RdtscFallback);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropySource {
    VirtioRng,
    RdRand,
    RdtscFallback,
}

pub fn active_entropy_source() -> EntropySource {
    *ENTROPY_SOURCE.lock()
}

pub fn init() {
    match VirtioRng::probe() {
        Ok(rng) => {
            *VIRTIO_RNG.lock() = Some(rng);
            *ENTROPY_SOURCE.lock() = EntropySource::VirtioRng;
            crate::serial_println!("[rng] hardware entropy source: virtio-rng");
        }
        Err(_) => {
            if has_rdrand_support() {
                *ENTROPY_SOURCE.lock() = EntropySource::RdRand;
                crate::serial_println!("[rng] hardware entropy source: rdrand");
            } else {
                *ENTROPY_SOURCE.lock() = EntropySource::RdtscFallback;
                crate::serial_println!("[rng] hardware entropy source: rdtsc-fallback");
            }
        }
    }

    CSPRNG.lock().seed_with_entropy();
}

fn has_rdrand_support() -> bool {
    let ecx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("ecx") ecx,
            out("eax") _,
            out("edx") _,
            options(nomem)
        );
    }
    (ecx & (1 << 30)) != 0
}

fn read_rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Gathers hardware entropy prioritizing: VirtIO-RNG > RDRAND > RDTSC fallback.
fn get_hardware_entropy() -> u64 {
    // 1. VirtIO-RNG (host hypervisor entropy)
    {
        let mut guard = VIRTIO_RNG.lock();
        if let Some(rng) = guard.as_mut() {
            let mut buf = [0u8; 8];
            if let Ok(n) = rng.get_entropy(&mut buf) {
                if n == 8 {
                    return u64::from_le_bytes(buf);
                }
            }
        }
    }

    // 2. x86 RDRAND instruction (CPU hardware entropy)
    if has_rdrand_support() {
        let mut val: u64 = 0;
        let mut ok: u8 = 0;
        unsafe {
            core::arch::asm!(
                "rdrand {0}",
                "jc 2f",
                "mov {1}, 0",
                "jmp 3f",
                "2:",
                "mov {1}, 1",
                "3:",
                out(reg) val,
                out(reg_byte) ok,
                options(nomem, nostack)
            );
        }
        if ok != 0 && val != 0 && val != u64::MAX {
            let tsc = read_rdtsc();
            return val ^ tsc.rotate_left(13);
        }
    }

    // 3. Fallback: splitmix64 on TSC jitter + atomic counter
    let tsc = read_rdtsc();
    static JITTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0xdeadbeef12345678);
    let seq = JITTER.fetch_add(0x9e3779b97f4a7c15, core::sync::atomic::Ordering::Relaxed);
    let mut z = tsc ^ seq;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

static CSPRNG: Mutex<ChaCha20Csprng> = Mutex::new(ChaCha20Csprng::new());

/// Top-level kernel function to fill buffer with cryptographically secure random bytes.
pub fn get_random_bytes(dest: &mut [u8]) -> usize {
    let mut rng = CSPRNG.lock();
    if rng.state[0] != CHACHA_CONSTANTS[0] {
        rng.seed_with_entropy();
    }
    rng.fill_bytes(dest);
    dest.len()
}
