//! Lifetime fence for this backend's single, non-remountable SD owner.
//! The full MediaLease remains in typed commands; atomic identity is only
//! a revocation check. No PCM slot or Engine is mutated by the SD owner.
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use ruzina_media_session::MediaLease;
static SOURCE: AtomicU32 = AtomicU32::new(0);
static GENERATION: AtomicU32 = AtomicU32::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PLAYING: AtomicU32 = AtomicU32::new(0);
static PITCH: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];
pub(crate) fn playback(playing: [bool; 2], pitch: [i16; 2]) {
    for i in 0..2 {
        PITCH[i].store(pitch[i] as i32 as u32, Ordering::Relaxed);
    }
    PLAYING.store(
        u32::from(playing[0]) | u32::from(playing[1]) << 1,
        Ordering::Release,
    );
}
pub(crate) fn playing() -> [bool; 2] {
    let mask = PLAYING.load(Ordering::Acquire);
    [mask & 1 != 0, mask & 2 != 0]
}
pub(crate) fn pitch() -> [i16; 2] {
    core::array::from_fn(|i| PITCH[i].load(Ordering::Relaxed) as i16)
}
pub(crate) fn activate(lease: MediaLease) {
    ACTIVE.store(false, Ordering::SeqCst);
    SOURCE.store(lease.source.get(), Ordering::SeqCst);
    GENERATION.store(lease.generation.get(), Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
}
pub(crate) fn revoke() {
    ACTIVE.store(false, Ordering::SeqCst);
    PLAYING.store(0, Ordering::Release);
}
#[cfg(feature = "deck-engine")]
pub(crate) fn current(lease: MediaLease) -> bool {
    ACTIVE.load(Ordering::SeqCst)
        && SOURCE.load(Ordering::SeqCst) == lease.source.get()
        && GENERATION.load(Ordering::SeqCst) == lease.generation.get()
        && ACTIVE.load(Ordering::SeqCst)
}
pub(crate) fn muted() -> bool {
    SOURCE.load(Ordering::SeqCst) != 0 && !ACTIVE.load(Ordering::SeqCst)
}
#[cfg(test)]
pub(crate) fn reset() {
    SOURCE.store(0, Ordering::SeqCst);
    ACTIVE.store(false, Ordering::SeqCst);
    PLAYING.store(0, Ordering::Release);
}
