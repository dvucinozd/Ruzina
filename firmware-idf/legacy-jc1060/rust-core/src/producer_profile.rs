//! Diagnostic wall intervals, including preemption/wait, never CPU-time claims.
//! Poll includes decode; decode's step/read/push/other are disjoint partitions.
use core::sync::atomic::{AtomicU32, Ordering};

pub(crate) const WORDS: usize = 40;
pub(crate) const POLL: usize = 0;
pub(crate) const DECODE: usize = 1;
pub(crate) const STEP: usize = 2;
pub(crate) const READ: usize = 3;
pub(crate) const PUSH: usize = 4;
const OTHER: usize = 5;

#[derive(Clone, Copy, Default)]
struct Metric {
    calls: u32,
    total: u64,
    maximum: u32,
}

#[derive(Default)]
pub(crate) struct Stats {
    metrics: [Metric; 6],
    busy: u32,
    frames: u32,
    bytes: u32,
    invalid: u32,
    saturated: u32,
    last_poll_end: Option<u64>,
    gap_max: u32,
}

impl Stats {
    pub(crate) fn record(&mut self, stage: usize, start: u64, end: u64) {
        let Some(duration) = end.checked_sub(start) else {
            self.invalid = self.invalid.saturating_add(1);
            return;
        };
        self.add(stage, duration);
    }

    fn add(&mut self, stage: usize, duration: u64) {
        let metric = &mut self.metrics[stage];
        if metric.calls == u32::MAX
            || metric.total.checked_add(duration).is_none()
            || duration > u64::from(u32::MAX)
        {
            self.saturated = 1;
        }
        metric.calls = metric.calls.saturating_add(1);
        metric.total = metric.total.saturating_add(duration);
        metric.maximum = metric.maximum.max(duration.min(u64::from(u32::MAX)) as u32);
    }

    pub(crate) fn children(&self) -> [u64; 3] {
        core::array::from_fn(|i| self.metrics[STEP + i].total)
    }

    pub(crate) fn finish_decode(&mut self, start: u64, end: u64, before: [u64; 3]) {
        let partition = end.checked_sub(start).and_then(|whole| {
            self.children()
                .into_iter()
                .zip(before)
                .try_fold(0u64, |sum, (after, before)| {
                    after
                        .checked_sub(before)
                        .and_then(|delta| sum.checked_add(delta))
                })
                .and_then(|children| whole.checked_sub(children))
        });
        if let Some(other) = partition {
            self.add(OTHER, other);
        } else {
            self.invalid = self.invalid.saturating_add(1);
        }
        self.record(DECODE, start, end);
    }

    pub(crate) fn poll(&mut self, start: u64, end: u64, busy: bool, frames: u32) {
        if let Some(previous) = self.last_poll_end {
            if let Some(gap) = start.checked_sub(previous) {
                if gap > u64::from(u32::MAX) {
                    self.saturated = 1;
                }
                self.gap_max = self.gap_max.max(gap.min(u64::from(u32::MAX)) as u32);
            } else {
                self.invalid = self.invalid.saturating_add(1);
            }
        }
        self.last_poll_end = Some(end);
        self.record(POLL, start, end);
        if self.busy.checked_add(u32::from(busy)).is_none()
            || self.frames.checked_add(frames).is_none()
        {
            self.saturated = 1;
        }
        self.busy = self.busy.saturating_add(u32::from(busy));
        self.frames = self.frames.saturating_add(frames);
    }

    pub(crate) fn read_bytes(&mut self, bytes: usize) {
        if bytes > (u32::MAX - self.bytes) as usize {
            self.saturated = 1;
        }
        self.bytes = self
            .bytes
            .saturating_add(bytes.min(u32::MAX as usize) as u32);
    }

    pub(crate) fn owner_turn(&mut self, start: u64, end: u64, polls: u64) {
        self.record(POLL, start, end);
        if let Some(other) = end
            .checked_sub(start)
            .and_then(|whole| whole.checked_sub(polls))
        {
            self.add(DECODE, polls);
            self.add(OTHER, other);
        } else {
            self.invalid = self.invalid.saturating_add(1);
        }
    }

    /// Metadata slots 2..9 are filled by the single owner at publication.
    pub(crate) fn words(&self) -> [u32; WORDS] {
        let mut words = [0; WORDS];
        words[0] = 1;
        words[10..15].copy_from_slice(&[
            self.busy,
            self.frames,
            self.bytes,
            self.invalid,
            self.saturated,
        ]);
        words[15] = self.gap_max;
        for (i, metric) in self.metrics.iter().enumerate() {
            words[16 + i * 4..20 + i * 4].copy_from_slice(&[
                metric.calls,
                metric.total as u32,
                (metric.total >> 32) as u32,
                metric.maximum,
            ]);
        }
        words
    }
}

struct Slot {
    sequence: AtomicU32,
    words: [AtomicU32; WORDS],
}
impl Slot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            words: [const { AtomicU32::new(0) }; WORDS],
        }
    }
    fn publish(&self, mut words: [u32; WORDS]) {
        // Exactly one SD-client writer. SeqCst makes one bounded read coherent;
        // the USB client never waits for a preempted lower-priority writer.
        let odd = self.sequence.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        words[1] = odd.wrapping_add(1);
        for (destination, value) in self.words.iter().zip(words) {
            destination.store(value, Ordering::SeqCst);
        }
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self) -> Option<[u32; WORDS]> {
        let before = self.sequence.load(Ordering::SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        let words = core::array::from_fn(|i| self.words[i].load(Ordering::SeqCst));
        (words[0] == 1 && before == self.sequence.load(Ordering::SeqCst)).then_some(words)
    }
}
static SLOTS: [Slot; 3] = [Slot::new(), Slot::new(), Slot::new()];

pub(crate) fn publish(deck: usize, words: [u32; WORDS]) {
    SLOTS[deck].publish(words);
}

/// Copies only coherent per-deck snapshots, with availability bits in the return.
/// # Safety
/// `out` must reference an aligned writable array of at least `len` u32 values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_producer_profile_snapshot(out: *mut u32, len: u32) -> u32 {
    if out.is_null() || !(out as usize).is_multiple_of(4) || len != (WORDS * 3) as u32 {
        return 0;
    }
    let mut mask = 0;
    for (deck, slot) in SLOTS.iter().enumerate() {
        if let Some(words) = slot.read() {
            // SAFETY: caller lends disjoint writable output for the synchronous copy.
            unsafe {
                core::ptr::copy_nonoverlapping(words.as_ptr(), out.add(deck * WORDS), WORDS);
            }
            mask |= 1 << deck;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partitions_same_window_and_marks_invalid_or_saturated() {
        let mut stats = Stats::default();
        let before = stats.children();
        stats.record(STEP, 10, 30);
        stats.record(READ, 30, 70);
        stats.record(PUSH, 75, 80);
        stats.finish_decode(0, 100, before);
        assert_eq!(stats.metrics[OTHER].total, 35);
        assert_eq!(stats.metrics[DECODE].total, 100);
        stats.poll(0, 120, true, 512);
        assert_eq!(stats.metrics[POLL].total, 120);
        stats.poll(150, 160, false, 0);
        assert_eq!(stats.words()[15], 30);
        stats.finish_decode(200, 100, stats.children());
        assert!(stats.invalid > 0);
        stats.metrics[STEP].calls = u32::MAX;
        stats.record(STEP, 0, 1);
        assert_eq!(stats.saturated, 1);
    }
    #[test]
    fn unpublished_or_interrupted_slot_is_unavailable_without_spinning() {
        let slot = Slot::new();
        assert!(slot.read().is_none());
        let mut words = Stats::default().words();
        words[4] = 7;
        slot.publish(words);
        let snapshot = slot.read().unwrap();
        assert_eq!((snapshot[1], snapshot[4]), (2, 7));
        slot.sequence.store(3, Ordering::SeqCst);
        assert!(slot.read().is_none());
    }

    #[test]
    fn owner_partitions_and_wide_totals_do_not_subtract_saturated_aggregates() {
        let mut stats = Stats::default();
        stats.owner_turn(10, 110, 75);
        assert_eq!(
            (
                stats.metrics[POLL].total,
                stats.metrics[DECODE].total,
                stats.metrics[OTHER].total
            ),
            (100, 75, 25)
        );
        stats.owner_turn(110, 120, 50);
        assert!(stats.invalid > 0);
        let mut wide = Stats::default();
        wide.metrics[STEP].total = u64::MAX - 10;
        wide.metrics[READ].total = u64::MAX - 10;
        let before = wide.children();
        wide.record(STEP, 0, 3);
        wide.record(READ, 0, 4);
        wide.finish_decode(0, 10, before);
        assert_eq!(wide.metrics[OTHER].total, 3);
        assert_eq!(wide.invalid, 0);
    }

    #[test]
    fn concurrent_publications_are_coherent_or_explicitly_unavailable() {
        let slot = std::sync::Arc::new(Slot::new());
        let writer = slot.clone();
        let thread = std::thread::spawn(move || {
            for value in 1..2000 {
                let mut words = [value; WORDS];
                words[0] = 1;
                writer.publish(words);
            }
        });
        for _ in 0..2000 {
            if let Some(words) = slot.read() {
                assert!(words[2..].iter().all(|v| *v == words[2]));
            }
        }
        thread.join().unwrap();
        assert!(slot.read().is_some());
    }
}
