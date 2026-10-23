//! Shared desktop generation counter.
//!
//! The input desktop changes at lock/unlock/UAC. The UDP client uses this to
//! reset its edge accumulator so stale values from the previous desktop do not
//! trigger a handback in the middle of the screen.

use std::sync::atomic::{AtomicU64, Ordering};

static GENERATION: AtomicU64 = AtomicU64::new(1);

pub fn bump() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}
