//! Actual queued/in-flight ownership count; not a playback-state approximation.
use core::sync::atomic::{AtomicU32, Ordering};
pub(crate) struct TransportActivity(AtomicU32);
impl TransportActivity {
    pub(crate) const fn new() -> Self {
        Self(AtomicU32::new(0))
    }
    pub(crate) fn reserve(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
    pub(crate) fn release(&self) {
        let previous = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous != 0);
    }
    pub(crate) fn idle(&self) -> bool {
        self.0.load(Ordering::Acquire) == 0
    }
    pub(crate) fn executing(&self) -> Execution<'_> {
        Execution(self)
    }
}
pub(crate) struct Execution<'a>(&'a TransportActivity);
impl Drop for Execution<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}
