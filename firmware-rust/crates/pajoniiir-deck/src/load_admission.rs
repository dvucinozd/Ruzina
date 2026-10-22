//! Shared track-load admission (RP-04, M2.5 package B6).
//!
//! LOAD LOCK defaults on: a load may not replace a destination deck that plays,
//! has a PLAY request awaiting transport confirmation, or is held by the jog
//! for scratch audio. Every entry point (controller, touch, future Web Remote)
//! asks [`crate::DeckProductState::load_admission`] and then reserves the deck's
//! single-flight [`DeckLoadGate`]. Asynchronous workers must call
//! [`DeckLoadGate::begin_replacement`] immediately before releasing the old
//! audio session; PLAY or scratch taking the deck after admission withdraws
//! the reservation, so the old session survives and the load reports
//! [`LoadRefusal::Superseded`]. Once replacement began, transport claims are
//! refused until the worker finishes, because the old session no longer exists.

use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadAllowed {
    /// LOAD LOCK is switched off: load at any time.
    LockOff,
    /// LOAD LOCK is on and the destination deck is idle.
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadRefusal {
    /// LOAD LOCK is on and the destination deck plays.
    Playing,
    /// LOAD LOCK is on and a PLAY request awaits transport confirmation.
    PlayPending,
    /// LOAD LOCK is on and the jog holds the deck for scratch audio.
    Scratching,
    /// Another accepted load still holds this deck's single-flight permit.
    LoadInFlight,
    /// PLAY or scratch claimed the deck after admission, before replacement.
    Superseded,
    /// Source or local cue banks do not belong to the requested track.
    TrackIdentity,
}

/// What a transport claim (PLAY press, jog touch) found on the deck's gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckClaim {
    /// No load holds the deck; the claim proceeds.
    Free,
    /// A reserved load was withdrawn; the claim proceeds and the old session stays.
    CancelledLoad,
    /// The old session is being replaced; the claim must be refused.
    Replacing,
}

/// Single-flight token for one admitted load on one deck.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadPermit {
    token: u32,
}

impl LoadPermit {
    const fn reserved(self) -> u32 {
        self.token << 1
    }

    const fn replacing(self) -> u32 {
        (self.token << 1) | 1
    }
}

const IDLE: u32 = 0;
const TOKEN_MASK: u32 = 0x7fff_ffff;

/// Lock-free per-deck load gate shared by the control owner and the load
/// worker. State encoding: `0` idle, `token << 1` reserved, `token << 1 | 1`
/// replacing. Tokens are nonzero 31-bit values, so stale permits never match.
pub struct DeckLoadGate {
    state: AtomicU32,
    next_token: AtomicU32,
}

impl Default for DeckLoadGate {
    fn default() -> Self {
        Self::new()
    }
}

impl DeckLoadGate {
    pub const fn new() -> Self {
        Self {
            state: AtomicU32::new(IDLE),
            next_token: AtomicU32::new(0),
        }
    }

    /// Reserve the deck for an admitted load. A second load is refused rather
    /// than queued, so a rejected request never holds the single-flight slot.
    pub fn reserve(&self) -> Result<LoadPermit, LoadRefusal> {
        let permit = LoadPermit {
            token: self.next_token(),
        };
        self.state
            .compare_exchange(IDLE, permit.reserved(), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| permit)
            .map_err(|_| LoadRefusal::LoadInFlight)
    }

    /// PLAY or jog touch takes the deck. A reservation that has not started
    /// replacing is withdrawn; an active replacement refuses the claim.
    pub fn claim_for_transport(&self) -> DeckClaim {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current == IDLE {
                return DeckClaim::Free;
            }
            if current & 1 == 1 {
                return DeckClaim::Replacing;
            }
            match self
                .state
                .compare_exchange(current, IDLE, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return DeckClaim::CancelledLoad,
                Err(observed) => current = observed,
            }
        }
    }

    /// Final admission immediately before the worker releases the old audio
    /// session. Fails if the reservation was withdrawn or superseded.
    pub fn begin_replacement(&self, permit: LoadPermit) -> Result<(), LoadRefusal> {
        self.state
            .compare_exchange(
                permit.reserved(),
                permit.replacing(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| LoadRefusal::Superseded)
    }

    /// Release the permit after commit or failure. Stale permits are ignored.
    pub fn finish(&self, permit: LoadPermit) -> bool {
        [permit.reserved(), permit.replacing()]
            .into_iter()
            .any(|expected| {
                self.state
                    .compare_exchange(expected, IDLE, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            })
    }

    pub fn is_replacing(&self) -> bool {
        self.state.load(Ordering::Acquire) & 1 == 1
    }

    pub fn is_idle(&self) -> bool {
        self.state.load(Ordering::Acquire) == IDLE
    }

    fn next_token(&self) -> u32 {
        loop {
            let token = self
                .next_token
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1)
                & TOKEN_MASK;
            if token != 0 {
                return token;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_is_single_flight_and_released_by_its_own_permit() {
        let gate = DeckLoadGate::new();
        let first = gate.reserve().unwrap();
        assert_eq!(gate.reserve(), Err(LoadRefusal::LoadInFlight));
        assert!(gate.finish(first));
        assert!(gate.is_idle());
        assert!(!gate.finish(first));

        let second = gate.reserve().unwrap();
        assert_ne!(first, second);
        assert!(!gate.finish(first));
        assert!(!gate.is_idle());
        assert!(gate.finish(second));
    }

    #[test]
    fn transport_claim_withdraws_reservation_before_replacement() {
        let gate = DeckLoadGate::new();
        assert_eq!(gate.claim_for_transport(), DeckClaim::Free);

        let permit = gate.reserve().unwrap();
        assert_eq!(gate.claim_for_transport(), DeckClaim::CancelledLoad);
        assert!(gate.is_idle());
        assert_eq!(gate.begin_replacement(permit), Err(LoadRefusal::Superseded));
        assert!(!gate.finish(permit));
    }

    #[test]
    fn replacement_refuses_transport_claims_until_finished() {
        let gate = DeckLoadGate::new();
        let permit = gate.reserve().unwrap();
        gate.begin_replacement(permit).unwrap();
        assert!(gate.is_replacing());
        assert_eq!(gate.claim_for_transport(), DeckClaim::Replacing);
        assert_eq!(gate.reserve(), Err(LoadRefusal::LoadInFlight));
        assert_eq!(gate.begin_replacement(permit), Err(LoadRefusal::Superseded));

        assert!(gate.finish(permit));
        assert_eq!(gate.claim_for_transport(), DeckClaim::Free);
    }

    #[test]
    fn stale_permit_cannot_take_a_newer_reservation() {
        let gate = DeckLoadGate::new();
        let stale = gate.reserve().unwrap();
        assert_eq!(gate.claim_for_transport(), DeckClaim::CancelledLoad);
        let current = gate.reserve().unwrap();

        assert_eq!(gate.begin_replacement(stale), Err(LoadRefusal::Superseded));
        assert!(!gate.finish(stale));
        gate.begin_replacement(current).unwrap();
        assert!(gate.finish(current));
    }

    #[test]
    fn tokens_skip_zero_after_wrap() {
        let gate = DeckLoadGate::new();
        gate.next_token.store(TOKEN_MASK - 1, Ordering::Relaxed);
        let last = gate.reserve().unwrap();
        assert_eq!(last.token, TOKEN_MASK);
        assert!(gate.finish(last));
        let wrapped = gate.reserve().unwrap();
        assert_eq!(wrapped.token, 1);
        assert!(gate.finish(wrapped));
    }
}
