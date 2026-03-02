use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::{sync::Arc, vec::Vec};

use crate::{
    drivers::timer::uptime,
    memory::uaccess::copy_to_user_slice,
    per_cpu_private,
    sync::{CondVar, OnceLock, SpinLock},
};
use blake2::{Blake2s256, Digest};
use chacha20::ChaCha20Rng;
use libkernel::memory::address::TUA;
use libkernel::{error::Result, sync::condvar::WakeupType};
use rand::{Rng, SeedableRng};

/// A hardware or software source of entropy that the pool can query.
pub trait EntropySource: Send + Sync {
    /// Pull up to `buf.len()` bytes of entropy from this source.
    /// Returns `(bytes_written, approx_entropy_bits)`.
    fn get_entropy(&self, buf: &mut [u8]) -> (usize, usize);
}

/// Number of bytes generated before a per-CPU RNG reseeds from the entropy pool.
const RESEED_BYTES: usize = 1024 * 1024;

pub struct EntropyPool {
    state: SpinLock<Blake2s256>,
    pool_waiters: CondVar<bool>,
    pool_bits: AtomicUsize,
    sources: SpinLock<Vec<Arc<dyn EntropySource>>>,
}

impl EntropyPool {
    fn new() -> Self {
        Self {
            state: SpinLock::new(Blake2s256::default()),
            pool_waiters: CondVar::new(false),
            pool_bits: AtomicUsize::new(0),
            sources: SpinLock::new(Vec::new()),
        }
    }

    pub fn add_entropy(&self, entropy: &[u8], approx_bits: usize) {
        self.state.lock_save_irq().update(entropy);

        let old_val = self.pool_bits.fetch_add(approx_bits, Ordering::Relaxed);

        if old_val + approx_bits >= 256 && old_val < 256 {
            // Pool was initialised. Wake up any waiters.
            self.pool_waiters.update(|s| {
                *s = true;
                WakeupType::All
            });
        }
    }

    /// Add entropy into this pool based upon the uptime of the system.
    ///
    /// WARNING: This method should *not* be called for predictable or periodic
    /// events.
    pub fn add_temporal_entropy(&self) {
        let uptime = uptime();

        self.add_entropy(
            &uptime.subsec_micros().to_le_bytes(),
            1_000_000_usize.ilog2() as usize,
        );
    }

    /// Poll all registered entropy sources and feed their output into the pool.
    fn poll_sources(&self) {
        let sources = self.sources.lock_save_irq();
        let mut buf = [0u8; 64];

        for source in sources.iter() {
            let (written, bits) = source.get_entropy(&mut buf);
            if written > 0 && bits > 0 {
                self.add_entropy(&buf[..written], bits);
            }
        }
    }

    /// Block until the pool has accumulated at least 256 bits of entropy, then
    /// return a 32-byte seed derived from the pool state.
    pub async fn extract_seed(&self) -> [u8; 32] {
        self.poll_sources();

        self.pool_waiters
            .wait_until(|s| if *s { Some(()) } else { None })
            .await;

        self.poll_sources();
        self.extract_seed_inner()
    }

    /// Non-blocking seed extraction.  Returns `None` if the pool has not yet
    /// accumulated 256 bits of entropy.
    pub fn try_extract_seed(&self) -> Option<[u8; 32]> {
        self.poll_sources();

        if self.pool_bits.load(Ordering::Relaxed) < 256 {
            return None;
        }

        Some(self.extract_seed_inner())
    }

    fn extract_seed_inner(&self) -> [u8; 32] {
        let mut state = self.state.lock_save_irq();

        let hash = (*state).clone().finalize();

        let mut seed = [0u8; 32];
        seed.copy_from_slice(&hash);

        // Feed the extracted hash back so the pool state diverges from the
        // output (forward secrecy).
        state.update(seed);

        seed
    }
}

pub fn entropy_pool() -> &'static EntropyPool {
    ENTROPY_POOL.get_or_init(EntropyPool::new)
}

/// Register a new entropy source that the pool can pull from.
pub fn register_entropy_source(source: Arc<dyn EntropySource>) {
    entropy_pool().sources.lock_save_irq().push(source);
}

static ENTROPY_POOL: OnceLock<EntropyPool> = OnceLock::new();

struct CpuRng {
    rng: ChaCha20Rng,
    seeded: bool,
    bytes_since_reseed: usize,
}

unsafe impl Send for CpuRng {}

impl CpuRng {
    fn new() -> Self {
        Self {
            rng: ChaCha20Rng::from_seed([0u8; 32]),
            seeded: false,
            bytes_since_reseed: 0,
        }
    }

    fn apply_seed(&mut self, seed: [u8; 32]) {
        self.rng = ChaCha20Rng::from_seed(seed);
        self.seeded = true;
        self.bytes_since_reseed = 0;
    }

    /// Reseed by XOR-ing a fresh BLAKE2 seed with 32 bytes of our own output,
    /// then constructing a new ChaCha20 instance from the combined material.
    fn reseed_with_blake(&mut self, blake_seed: [u8; 32]) {
        let mut self_bytes = [0u8; 32];
        self.rng.fill_bytes(&mut self_bytes);

        let mut new_seed = [0u8; 32];

        for i in 0..32 {
            new_seed[i] = blake_seed[i] ^ self_bytes[i];
        }

        self.rng = ChaCha20Rng::from_seed(new_seed);
        self.bytes_since_reseed = 0;
    }

    fn fill(&mut self, buf: &mut [u8]) {
        self.rng.fill_bytes(buf);
        self.bytes_since_reseed += buf.len();
    }
}

per_cpu_private! {
    static CPU_RNG: CpuRng = CpuRng::new;
}

/// Fill `buf` with cryptographically-strong random bytes.
///
/// On the first invocation per CPU the call blocks until the global entropy
/// pool has been seeded (>= 256 bits).  Subsequent calls are non-blocking;
/// periodic reseeding from the pool happens inline.
pub async fn fill_random_bytes(buf: &mut [u8]) {
    // Ensure the per-CPU RNG has been seeded at least once.
    let seeded = CPU_RNG.borrow().seeded;

    if !seeded {
        let seed = entropy_pool().extract_seed().await;
        CPU_RNG.borrow_mut().apply_seed(seed);
    }

    // Reseed from the entropy pool if we have generated enough bytes.
    let needs_reseed = CPU_RNG.borrow().bytes_since_reseed >= RESEED_BYTES;
    if needs_reseed && let Some(blake_seed) = entropy_pool().try_extract_seed() {
        CPU_RNG.borrow_mut().reseed_with_blake(blake_seed);
    }

    CPU_RNG.borrow_mut().fill(buf);
}

const GETRANDOM_CHUNK: usize = 256;

pub async fn sys_getrandom(ubuf: TUA<u8>, size: isize, _flags: u32) -> Result<usize> {
    let total = size as usize;
    let mut buf = [0u8; GETRANDOM_CHUNK];
    let mut offset = 0;

    while offset < total {
        let n = (total - offset).min(GETRANDOM_CHUNK);
        let chunk = &mut buf[..n];

        fill_random_bytes(chunk).await;

        copy_to_user_slice(chunk, ubuf.to_untyped().add_bytes(offset)).await?;

        offset += n;
    }

    Ok(total)
}
