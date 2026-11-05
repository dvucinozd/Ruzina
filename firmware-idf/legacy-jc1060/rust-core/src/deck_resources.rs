//! One-shot SD-owner resource preparation. USB sees only a monotonic scalar.
#[cfg(test)]
use crate::deck_play::RING_FRAMES;
use crate::deck_play::{Producer, RING_STORAGE_FRAMES, Ring, install_ring};
use alloc::boxed::Box;
#[cfg(feature = "deck-engine")]
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::{AtomicU32, Ordering};
use ruzina_audio_dsp::PcmFrame;
#[cfg(feature = "deck-engine")]
use ruzina_audio_runtime::fx::{FX_DECK_STORAGE_SAMPLES, FX_TOTAL_STORAGE_SAMPLES};

pub(crate) const UNINITIALIZED: u32 = 0;
pub(crate) const PREPARING: u32 = 1;
pub(crate) const READY: u32 = 2;
pub(crate) const FAILED: u32 = 3;
pub(crate) struct Gate(AtomicU32);
impl Gate {
    pub(crate) const fn new() -> Self {
        Self(AtomicU32::new(UNINITIALIZED))
    }
    pub(crate) fn begin(&self) -> bool {
        self.0
            .compare_exchange(
                UNINITIALIZED,
                PREPARING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    pub(crate) fn ready(&self) -> bool {
        self.0
            .compare_exchange(PREPARING, READY, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(crate) fn fail(&self) {
        self.0.store(FAILED, Ordering::Release);
    }
    pub(crate) fn state(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }
}
pub(crate) static GATE: Gate = Gate::new();

#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_resources_state() -> u32 {
    GATE.state()
}
#[cfg(test)]
std::thread_local! { static FAIL_AT: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) }; }
fn allowed() -> bool {
    #[cfg(test)]
    return FAIL_AT.with(|n| match n.get() {
        Some(0) => false,
        Some(v) => {
            n.set(Some(v - 1));
            true
        }
        None => true,
    });
    #[cfg(not(test))]
    true
}
/// Caller supplies audited bitwise-copy-safe inline storage, not owning pointers.
pub(crate) unsafe fn try_copy<T>(value: &'static T) -> Option<Box<T>> {
    if !allowed() {
        return None;
    }
    // SAFETY: nonzero audited types; null is handled before any write/reference.
    if core::mem::size_of::<T>() == 0 {
        return None;
    }
    let raw = unsafe { alloc::alloc::alloc(core::alloc::Layout::new::<T>()).cast::<T>() };
    if raw.is_null() {
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(value, raw, 1);
        Some(Box::from_raw(raw))
    }
}
pub(crate) fn try_zeros<T: Default + Copy>(len: usize) -> Option<Box<[T]>> {
    if !allowed() {
        return None;
    }
    let layout = core::alloc::Layout::array::<T>(len).ok()?;
    if layout.size() == 0 {
        return None;
    }
    // SAFETY: one exact allocation, initialized completely before Box exposure.
    // No Vec shrinking/reallocation can turn a fallible path into an OOM abort.
    let raw = unsafe { alloc::alloc::alloc(layout).cast::<T>() };
    if raw.is_null() {
        return None;
    }
    for i in 0..len {
        unsafe {
            raw.add(i).write(T::default());
        }
    }
    Some(unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(raw, len)) })
}
/// The render graph's effect memory, from the SD owner to the USB task, once.
#[cfg(feature = "deck-engine")]
pub(crate) static FX_STORAGE: AtomicPtr<f32> = AtomicPtr::new(core::ptr::null_mut());

/// Hands the published effect memory over, split per deck, once.
#[cfg(feature = "deck-engine")]
pub(crate) fn take_fx_storage() -> Option<[&'static mut [f32]; 2]> {
    let storage = FX_STORAGE.swap(core::ptr::null_mut(), Ordering::AcqRel);
    if storage.is_null() {
        return None;
    }
    // SAFETY: `publish` leaked exactly this many samples, and the swap gives
    // the pointer to one caller only.
    let all = unsafe { core::slice::from_raw_parts_mut(storage, FX_TOTAL_STORAGE_SAMPLES) };
    let (a, b) = all.split_at_mut(FX_DECK_STORAGE_SAMPLES);
    Some([a, b])
}

pub(crate) struct Prepared<File> {
    pub(crate) producers: [Producer<File>; 2],
    rings: [Box<[PcmFrame]>; 2],
    #[cfg(feature = "deck-engine")]
    fx: Box<[f32]>,
}

pub(crate) fn prepare<File>(
    gate: &Gate,
    rings: &[Ring; 2],
    current: impl FnOnce() -> bool,
) -> Option<[Producer<File>; 2]> {
    if !gate.begin() {
        return None;
    }
    let result = (|| {
        let resources = Prepared::try_new()?;
        if !current() {
            return None;
        }
        let producers = resources.publish(rings)?;
        if !gate.ready() {
            return None;
        }
        Some(producers)
    })();
    if result.is_none() {
        gate.fail();
    }
    result
}
impl<File> Prepared<File> {
    pub(crate) fn try_new() -> Option<Self> {
        Some(Self {
            producers: [Producer::try_new()?, Producer::try_new()?],
            rings: [
                try_zeros(RING_STORAGE_FRAMES)?,
                try_zeros(RING_STORAGE_FRAMES)?,
            ],
            #[cfg(feature = "deck-engine")]
            fx: try_zeros(FX_TOTAL_STORAGE_SAMPLES)?,
        })
    }
    /// Single SD owner publishes once, after every fallible allocation succeeds.
    /// Published rings are permanent; even cancellation never frees reader storage.
    pub(crate) fn publish(self, rings: &[Ring; 2]) -> Option<[Producer<File>; 2]> {
        if rings.iter().any(Ring::has_storage) {
            return None;
        }
        let [a, b] = self.rings;
        if !install_ring(&rings[0], a) || !install_ring(&rings[1], b) {
            return None;
        }
        #[cfg(feature = "deck-engine")]
        FX_STORAGE.store(Box::leak(self.fx).as_mut_ptr(), Ordering::Release);
        Some(self.producers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::{GlobalAlloc, Layout};

    std::thread_local! {
        static ALLOCATIONS: core::cell::Cell<Option<(usize, usize)>> = const { core::cell::Cell::new(None) };
        static MINIMUM_SIZE: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
    }
    struct CountedSystem;
    // SAFETY: System owns every allocation. Counting uses allocation-free TLS
    // and is enabled only around the operation under test on its own thread.
    unsafe impl GlobalAlloc for CountedSystem {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { std::alloc::System.alloc(layout) };
            if !ptr.is_null()
                && MINIMUM_SIZE
                    .try_with(|n| layout.size() >= n.get())
                    .unwrap_or(false)
            {
                let _ = ALLOCATIONS.try_with(|n| {
                    if let Some((a, d)) = n.get() {
                        n.set(Some((a + 1, d)));
                    }
                });
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if MINIMUM_SIZE
                .try_with(|n| layout.size() >= n.get())
                .unwrap_or(false)
            {
                let _ = ALLOCATIONS.try_with(|n| {
                    if let Some((a, d)) = n.get() {
                        n.set(Some((a, d + 1)));
                    }
                });
            }
            unsafe { std::alloc::System.dealloc(ptr, layout) };
        }
    }
    #[global_allocator]
    static TEST_ALLOCATOR: CountedSystem = CountedSystem;
    struct Counting;
    impl Counting {
        fn begin() -> Self {
            Self::at_least(0)
        }
        fn at_least(minimum: usize) -> Self {
            MINIMUM_SIZE.with(|n| n.set(minimum));
            ALLOCATIONS.with(|n| {
                assert!(n.get().is_none());
                n.set(Some((0, 0)));
            });
            Self
        }
        fn finish(self) -> (usize, usize) {
            ALLOCATIONS.with(|n| n.get().unwrap())
        }
    }
    impl Drop for Counting {
        fn drop(&mut self) {
            ALLOCATIONS.with(|n| n.set(None));
        }
    }
    #[test]
    fn published_pair_cannot_be_replaced_or_partially_republished() {
        let rings = [Ring::new(), Ring::new()];
        let p = Prepared::<()>::try_new().unwrap().publish(&rings).unwrap();
        assert!(Prepared::<()>::try_new().unwrap().publish(&rings).is_none());
        assert!(rings.iter().all(Ring::has_storage));
        drop(p);
    }
    #[test]
    fn prepared_producers_decode_like_original_storage() {
        use crate::deck_play::{Commands, Reader, tests::Memory};
        const WAV: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.wav"
        );
        const FLAC: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.flac"
        );
        const MP3: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/cbr.mp3"
        );
        const VBR: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/vbr.mp3"
        );
        fn decode(
            mut producer: Producer<&'static [u8]>,
            data: &'static [u8],
        ) -> alloc::vec::Vec<PcmFrame> {
            let ring = Ring::new();
            assert!(install_ring(&ring, try_zeros(RING_STORAGE_FRAMES).unwrap()));
            let mut files = Memory::new(&[data]);
            let mut reader = Reader::new();
            let mut out = alloc::vec::Vec::new();
            let commands = Commands::new();
            producer.begin_load(&mut files, &ring, 0, Some("0"));
            for _ in 0..10000 {
                producer.poll(&mut files, &ring, &commands, |_| Some("0"));
                reader.sync(&ring);
                while let Some(frame) = reader.pop(&ring) {
                    out.push(frame);
                    if out.len() == 4096 {
                        return out;
                    }
                }
                reader.commit(&ring);
                if reader.ended(&ring) {
                    break;
                }
            }
            assert!(!out.is_empty());
            out
        }
        for data in [WAV, FLAC, MP3, VBR] {
            let [a, b] = Prepared::try_new().unwrap().producers;
            let expected = decode(Producer::new(), data);
            assert_eq!(decode(a, data), expected);
            assert_eq!(decode(b, data), expected);
        }
    }
    #[test]
    fn gate_never_reopens_or_overwrites_a_cancellation() {
        let g = Gate::new();
        assert_eq!(g.state(), UNINITIALIZED);
        assert!(g.begin());
        assert!(!g.begin());
        assert_eq!(g.state(), PREPARING);
        g.fail();
        assert!(!g.ready());
        assert!(!g.begin());
        assert_eq!(g.state(), FAILED);
        let g = Gate::new();
        assert!(g.begin());
        assert!(g.ready());
        assert!(!g.begin());
        g.fail();
        assert_eq!(g.state(), FAILED);
    }
    #[test]
    fn every_backing_allocation_can_fail_before_publication() {
        for fail in 0..12 {
            let g = Gate::new();
            let rings = [Ring::new(), Ring::new()];
            FAIL_AT.with(|n| n.set(Some(fail)));
            let counting = Counting::begin();
            let result = prepare::<()>(&g, &rings, || true);
            let allocations = counting.finish();
            assert!(result.is_none(), "allocation {fail}");
            assert_eq!(
                allocations,
                (fail, fail),
                "partial init must free all stores"
            );
            assert_eq!(g.state(), FAILED);
            assert!(rings.iter().all(|r| !r.has_storage()));
        }
        FAIL_AT.with(|n| n.set(None));
        let resources = Prepared::<()>::try_new().unwrap();
        let rings = [Ring::new(), Ring::new()];
        let producers = resources.publish(&rings).unwrap();
        assert_eq!(rings[0].free(), RING_FRAMES);
        assert_eq!(rings[1].free(), RING_FRAMES);
        drop(producers);
    }

    #[test]
    fn both_prepared_decks_reuse_storage_for_load_seek_and_failed_load() {
        use crate::deck_play::{Commands, tests::Memory};
        const WAV: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.wav"
        );
        const FLAC: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/stereo.flac"
        );
        const MP3: &[u8] = include_bytes!(
            "../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/vbr.mp3"
        );
        let gate = Gate::new();
        let rings = [Ring::new(), Ring::new()];
        let mut producers = prepare(&gate, &rings, || true).unwrap();
        let mut files = Memory::new(&[WAV, FLAC, MP3]);
        let commands = Commands::new();
        // SourceDecoder/PcmStreamBlock are still transient per-track objects.
        // Count allocations at least as large as the smallest permanent store.
        let counting = Counting::at_least(core::mem::size_of::<
            ruzina_audio_runtime::decode::Mp3CheckpointStorage,
        >());
        for (producer, ring) in producers.iter_mut().zip(&rings) {
            for _ in 0..2 {
                for path in ["0", "1", "2", "missing"] {
                    producer.begin_load(&mut files, ring, 0, Some(path));
                    for _ in 0..32 {
                        producer.poll(&mut files, ring, &commands, |_| Some(path));
                    }
                    producer.seek(&mut files, ring, 1);
                }
            }
        }
        assert_eq!(counting.finish(), (0, 0));
        assert_eq!(gate.state(), READY);
    }

    #[test]
    fn cancellation_and_repeated_initialization_cannot_publish_ready_again() {
        let g = Gate::new();
        let rings = [Ring::new(), Ring::new()];
        assert!(prepare::<()>(&g, &rings, || false).is_none());
        assert_eq!(g.state(), FAILED);
        assert!(rings.iter().all(|r| !r.has_storage()));
        let g = Gate::new();
        let p = prepare::<()>(&g, &rings, || true).unwrap();
        assert_eq!(g.state(), READY);
        FAIL_AT.with(|n| n.set(Some(0)));
        assert!(prepare::<()>(&g, &rings, || panic!("must not retry initialization")).is_none());
        assert_eq!(g.state(), READY);
        FAIL_AT.with(|n| n.set(None));
        drop(p);
        let g = Gate::new();
        let rings = [Ring::new(), Ring::new()];
        assert!(
            prepare::<()>(&g, &rings, || {
                g.fail();
                true
            })
            .is_none()
        );
        assert_eq!(g.state(), FAILED);
    }
}
