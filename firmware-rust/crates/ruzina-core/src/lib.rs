#![no_std]
#![forbid(unsafe_code)]

/// Logical deck identity used across controller, audio and UI boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DeckId {
    One,
    Two,
}

/// Stable product-side track identity.
///
/// The concrete derivation is intentionally outside this core type. M1 will
/// derive it from stable media identity plus normalized relative path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MediaTrackId(pub [u8; 16]);

/// Evidence state used by bring-up and qualification tooling.
///
/// The ordering is descriptive, not an automatic promotion mechanism.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationState {
    HostVerified,
    SimulatorVerified,
    CompileVerified,
    HardwarePending,
    HardwareVerified,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_ids_are_distinct() {
        assert_ne!(DeckId::One, DeckId::Two);
    }

    #[test]
    fn media_track_id_is_value_semantic() {
        let a = MediaTrackId([0x5a; 16]);
        let b = MediaTrackId([0x5a; 16]);
        assert_eq!(a, b);
    }
}
