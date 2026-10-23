//! Latest-value ordering of continuous Web controls.
//!
//! A browser may have several requests for one fader in flight (M2.5 throttles
//! to one per control on the client, but connections and retries still
//! reorder). Each page sends a random `client` token and a per-page sequence;
//! a value older than the last one applied for the same client and control is
//! superseded and must not move the fader backwards. The table is bounded:
//! the least recently used entry is evicted for a new key.

use crate::remote::ContinuousControl;

pub const ORDERING_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    client: u32,
    control: ContinuousControl,
    sequence: u32,
    used: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ordering {
    Apply,
    /// A newer value from this client was already applied.
    Superseded,
}

pub struct LatestValueGate {
    entries: [Option<Entry>; ORDERING_SLOTS],
    clock: u32,
}

impl Default for LatestValueGate {
    fn default() -> Self {
        Self::new()
    }
}

impl LatestValueGate {
    pub const fn new() -> Self {
        Self {
            entries: [None; ORDERING_SLOTS],
            clock: 0,
        }
    }

    pub fn admit(&mut self, client: u32, control: ContinuousControl, sequence: u32) -> Ordering {
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.client == client && entry.control == control)
        {
            entry.used = clock;
            if !newer(sequence, entry.sequence) {
                return Ordering::Superseded;
            }
            entry.sequence = sequence;
            return Ordering::Apply;
        }
        let slot = match self.entries.iter().position(Option::is_none) {
            Some(free) => free,
            None => self
                .entries
                .iter()
                .enumerate()
                .max_by_key(|(_, entry)| {
                    entry.map_or(u32::MAX, |entry| clock.wrapping_sub(entry.used))
                })
                .map(|(index, _)| index)
                .unwrap_or(0),
        };
        self.entries[slot] = Some(Entry {
            client,
            control,
            sequence,
            used: clock,
        });
        Ordering::Apply
    }
}

fn newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_controller_core::DeckId;

    const VOLUME: ContinuousControl = ContinuousControl::Volume(DeckId::One);

    #[test]
    fn older_or_repeated_values_are_superseded_per_client_and_control() {
        let mut gate = LatestValueGate::new();
        assert_eq!(gate.admit(7, VOLUME, 10), Ordering::Apply);
        assert_eq!(gate.admit(7, VOLUME, 12), Ordering::Apply);
        // Request 11 arrives late on another connection.
        assert_eq!(gate.admit(7, VOLUME, 11), Ordering::Superseded);
        assert_eq!(gate.admit(7, VOLUME, 12), Ordering::Superseded);
        // Other controls and other pages are independent.
        assert_eq!(
            gate.admit(7, ContinuousControl::Crossfader, 1),
            Ordering::Apply
        );
        assert_eq!(gate.admit(9, VOLUME, 1), Ordering::Apply);
        // Sequences wrap.
        assert_eq!(gate.admit(5, VOLUME, u32::MAX), Ordering::Apply);
        assert_eq!(gate.admit(5, VOLUME, 0), Ordering::Apply);
    }

    #[test]
    fn the_table_is_bounded_and_evicts_the_least_recently_used() {
        let mut gate = LatestValueGate::new();
        for client in 0..ORDERING_SLOTS as u32 {
            gate.admit(client, VOLUME, 100);
        }
        // Keep client 0 hot, then add a new client: client 1 is evicted.
        assert_eq!(gate.admit(0, VOLUME, 101), Ordering::Apply);
        assert_eq!(gate.admit(99, VOLUME, 5), Ordering::Apply);
        assert_eq!(gate.admit(0, VOLUME, 100), Ordering::Superseded);
        // Evicted state is forgotten: an old value from client 1 is accepted
        // (evicting client 2), while client 3 is still ordered.
        assert_eq!(gate.admit(1, VOLUME, 50), Ordering::Apply);
        assert_eq!(gate.admit(3, VOLUME, 99), Ordering::Superseded);
    }
}
