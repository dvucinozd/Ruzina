//! Bounded ownership transfer between the USB control and SD producer tasks.
//! No allocation, spinning, or borrowed Rust layout crosses the C ABI.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, Ordering};
use ruzina_deck::{DeckLoadGate, LoadPermit};
use ruzina_media_session::MediaLease;

/// A single item. CAS temporarily owns the value; a busy peer is retried on
/// the next task turn. The sender must retain an item if `put` returns it.
pub(crate) struct Slot<T: Copy> {
    state: AtomicU8,
    value: UnsafeCell<Option<T>>,
}
// SAFETY: the CAS serializes all accesses to value; no references escape.
unsafe impl<T: Copy + Send> Sync for Slot<T> {}
impl<T: Copy> Slot<T> {
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
            value: UnsafeCell::new(None),
        }
    }
    pub(crate) fn put(&self, value: T) -> Result<(), T> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(value);
        }
        // SAFETY: exclusively owns the slot until release publication.
        unsafe { *self.value.get() = Some(value) };
        self.state.store(2, Ordering::Release);
        Ok(())
    }
    pub(crate) fn take(&self) -> Option<T> {
        if self
            .state
            .compare_exchange(2, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        // SAFETY: exclusively owns the slot, including during reset.
        let value = unsafe { (*self.value.get()).take() };
        self.state.store(0, Ordering::Release);
        value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoadRequest {
    pub(crate) lease: MediaLease,
    pub(crate) permit: LoadPermit,
    pub(crate) index: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoadResult {
    pub(crate) request: LoadRequest,
    pub(crate) ready: bool,
    pub(crate) track_id: [u8; 16],
    pub(crate) bpm_x100: u32,
}
pub(crate) static GATES: [DeckLoadGate; 2] = [DeckLoadGate::new(), DeckLoadGate::new()];
pub(crate) static LOAD_REQUESTS: [Slot<LoadRequest>; 2] = [Slot::new(), Slot::new()];
pub(crate) static LOAD_RESULTS: [Slot<LoadResult>; 2] = [Slot::new(), Slot::new()];
pub(crate) static MEDIA: Slot<MediaLease> = Slot::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SeekRequest {
    pub(crate) source: LoadRequest,
    pub(crate) id: u32,
    pub(crate) target_ms: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SeekOutcome {
    Ready,
    Eof,
    Failed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SeekResult {
    pub(crate) request: SeekRequest,
    pub(crate) outcome: SeekOutcome,
    pub(crate) epoch: u32,
    pub(crate) position_ms: u32,
}
pub(crate) static SEEK_REQUESTS: [Slot<SeekRequest>; 2] = [Slot::new(), Slot::new()];
pub(crate) static SEEK_RESULTS: [Slot<SeekResult>; 2] = [Slot::new(), Slot::new()];

#[cfg(test)]
pub(crate) fn execute_seek<F: crate::deck_play::SourceFiles>(
    deck: usize,
    request: SeekRequest,
    source: Option<LoadRequest>,
    producer: &mut crate::deck_play::Producer<F::File>,
    files: &mut F,
) -> SeekResult {
    let ring = &crate::deck_play::RINGS[deck];
    if source != Some(request.source) {
        return SeekResult {
            request,
            outcome: SeekOutcome::Cancelled,
            epoch: ring.epoch(),
            position_ms: 0,
        };
    }
    let ok = matches!(
        producer.seek(files, ring, request.target_ms),
        Some(crate::deck_play::Event::Seeked { ok: true, .. })
    );
    let (epoch, position_ms, eof) = ring.seek_position();
    SeekResult {
        request,
        outcome: if !ok {
            SeekOutcome::Failed
        } else if eof {
            SeekOutcome::Eof
        } else {
            SeekOutcome::Ready
        },
        epoch,
        position_ms,
    }
}

#[cfg(test)]
pub(crate) fn guarded_load<F: crate::deck_play::SourceFiles>(
    deck: usize,
    request: LoadRequest,
    lease: MediaLease,
    producer: &mut crate::deck_play::Producer<F::File>,
    files: &mut F,
    path: Option<&str>,
) -> Option<crate::deck_play::Event> {
    match begin_load(deck, request, lease, producer, files, path) {
        Err(()) => None,
        Ok(Some(event)) => Some(event),
        Ok(None) => {
            for _ in 0..8192 {
                if let Some(event) = producer.poll_prime(files, &crate::deck_play::RINGS[deck]) {
                    return Some(event);
                }
            }
            panic!("test load never primed")
        }
    }
}

/// Production entry: permission is checked once, immediately before source replacement.
pub(crate) fn begin_load<F: crate::deck_play::SourceFiles>(
    deck: usize,
    request: LoadRequest,
    lease: MediaLease,
    producer: &mut crate::deck_play::Producer<F::File>,
    files: &mut F,
    path: Option<&str>,
) -> Result<Option<crate::deck_play::Event>, ()> {
    if request.lease != lease
        || !crate::media_signal::current(lease)
        || GATES[deck].begin_replacement(request.permit).is_err()
    {
        GATES[deck].finish(request.permit);
        return Err(());
    }
    Ok(producer.begin_load(files, &crate::deck_play::RINGS[deck], request.index, path))
}

pub(crate) fn seek_result(deck: usize, request: SeekRequest, ok: bool) -> SeekResult {
    let (epoch, position_ms, eof) = crate::deck_play::RINGS[deck].seek_position();
    SeekResult {
        request,
        outcome: if !ok {
            SeekOutcome::Failed
        } else if eof {
            SeekOutcome::Eof
        } else {
            SeekOutcome::Ready
        },
        epoch,
        position_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slot_never_overwrites_an_unconsumed_item() {
        let slot = Slot::new();
        assert_eq!(slot.put(123), Ok(()));
        assert_eq!(slot.put(456), Err(456));
        assert_eq!(slot.take(), Some(123));
        assert_eq!(slot.take(), None);
        assert_eq!(slot.put(456), Ok(()));
        assert_eq!(slot.take(), Some(456));
    }
    #[test]
    fn slot_transfers_whole_values_between_threads() {
        let slot = std::sync::Arc::new(Slot::new());
        let writer = slot.clone();
        let thread = std::thread::spawn(move || {
            for n in 0..10_000u32 {
                let mut item = (n, !n);
                while let Err(returned) = writer.put(item) {
                    item = returned;
                    std::thread::yield_now();
                }
            }
        });
        for expected in 0..10_000u32 {
            loop {
                if let Some(pair) = slot.take() {
                    assert_eq!(pair, (expected, !expected));
                    break;
                }
                std::thread::yield_now();
            }
        }
        thread.join().unwrap();
    }
}
