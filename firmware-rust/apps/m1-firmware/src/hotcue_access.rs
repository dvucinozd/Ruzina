//! A failed durable operation latches the store unavailable for this boot.
//! Keep servicing requests so a Library load queued before the fault terminates.
use pajoniiir_core::MediaTrackId;
use pajoniiir_hot_cues::{HotCueBank, HotCueStore};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotCueLoadResult {
    Ready(Option<HotCueBank>),
    Failed,
}

pub(crate) struct HotCueAccess<S> {
    store: Option<S>,
    faulted: bool,
}

impl<S: HotCueStore> HotCueAccess<S> {
    pub(crate) fn new(store: Option<S>) -> Self {
        let faulted = store.is_none();
        Self { store, faulted }
    }

    pub(crate) const fn faulted(&self) -> bool {
        self.faulted
    }

    pub(crate) fn latch_fault(&mut self) {
        self.faulted = true;
    }

    pub(crate) fn load(&mut self, track: MediaTrackId) -> HotCueLoadResult {
        if self.faulted {
            return HotCueLoadResult::Failed;
        }
        match self.store.as_mut().expect("available store").load(track) {
            Ok(bank) => HotCueLoadResult::Ready(bank),
            Err(_) => {
                self.latch_fault();
                HotCueLoadResult::Failed
            }
        }
    }

    pub(crate) fn save(&mut self, bank: &HotCueBank) -> bool {
        if self.faulted {
            return false;
        }
        if self
            .store
            .as_mut()
            .expect("available store")
            .save(bank)
            .is_ok()
        {
            true
        } else {
            self.latch_fault();
            false
        }
    }
}
