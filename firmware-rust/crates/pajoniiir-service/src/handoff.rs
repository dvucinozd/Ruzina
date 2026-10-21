//! Serialized diagnostic-only handoff. A full queue drops an observation, never
//! control work. Producer tokens belong to one owner lifetime, not a peripheral
//! connection identifier. External task synchronization is the caller's job.
use crate::publication::{Activity, Freshness, Observation, Publication, Slot, Stamp};

pub const HANDOFF_CAPACITY: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerState {
    Idle,
    Active,
    Backpressured,
    Disconnected,
    Failed,
}
pub trait OwnerSample: Copy {
    fn state(self) -> OwnerState;
    /// Cumulative fault total in this owner lifetime; never a current condition.
    fn errors(self) -> u64;
}

#[derive(Debug)]
pub struct Producer {
    generation: u64,
    sequence: u64,
}
impl Producer {
    pub fn sample<T: OwnerSample>(&mut self, sampled_ms: u64, value: T) -> Option<Publication<T>> {
        self.sequence = self.sequence.checked_add(1)?;
        Some(Publication {
            stamp: Stamp {
                generation: self.generation,
                sequence: self.sequence,
                sampled_ms,
                activity: if matches!(
                    value.state(),
                    OwnerState::Active | OwnerState::Backpressured
                ) {
                    Activity::Active
                } else {
                    Activity::Idle
                },
            },
            value,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HandoffStats {
    pub accepted: u64,
    pub queue_full: u64,
    pub rejected: u64,
    pub discarded_on_restart: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    UnboundOrSuperseded,
    Full,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Condition {
    Unbound,
    Awaiting,
    Stale,
    ClockReversed,
    Idle,
    Active,
    Disconnected,
    Backpressured,
    Failed,
    NewErrors,
    CounterReset,
}
#[derive(Clone, Copy, Debug)]
pub struct OwnerView<T> {
    pub observation: Observation,
    pub condition: Condition,
    /// Last sample, including when stale. condition gates its current meaning.
    pub value: Option<T>,
    /// Deltas in the last nonempty accepted drain batch; not a fault latch.
    pub new_errors: u64,
    pub handoff: HandoffStats,
}

pub struct OwnerChannel<T: OwnerSample> {
    slot: Slot<T>,
    queue: [Option<Publication<T>>; HANDOFF_CAPACITY],
    head: usize,
    len: usize,
    generation: u64,
    bound: bool,
    last_errors: Option<u64>,
    new_errors: u64,
    counter_reset: bool,
    stats: HandoffStats,
}
impl<T: OwnerSample> OwnerChannel<T> {
    pub const fn new(max_age_ms: u64) -> Self {
        Self {
            slot: Slot::new(max_age_ms),
            queue: [None; HANDOFF_CAPACITY],
            head: 0,
            len: 0,
            generation: 0,
            bound: false,
            last_errors: None,
            new_errors: 0,
            counter_reset: false,
            stats: HandoffStats {
                accepted: 0,
                queue_full: 0,
                rejected: 0,
                discarded_on_restart: 0,
            },
        }
    }
    pub fn stop(&mut self) {
        self.slot.unbind();
        self.bound = false;
        self.stats.discarded_on_restart = self
            .stats
            .discarded_on_restart
            .saturating_add(self.len as u64);
        self.queue = [None; HANDOFF_CAPACITY];
        self.head = 0;
        self.len = 0;
        self.last_errors = None;
        self.new_errors = 0;
        self.counter_reset = false;
    }
    /// Only the lifecycle coordinator calls restart. Producers cannot self-bind.
    pub fn restart(&mut self) -> Option<Producer> {
        self.stop();
        self.generation = self.generation.checked_add(1)?;
        self.bound = self.slot.bind(self.generation);
        self.bound.then_some(Producer {
            generation: self.generation,
            sequence: 0,
        })
    }
    pub fn try_send(&mut self, sample: Publication<T>) -> Result<(), SendError> {
        if !self.bound || sample.stamp.generation != self.generation {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return Err(SendError::UnboundOrSuperseded);
        }
        if self.len == HANDOFF_CAPACITY {
            self.stats.queue_full = self.stats.queue_full.saturating_add(1);
            return Err(SendError::Full);
        }
        self.queue[(self.head + self.len) % HANDOFF_CAPACITY] = Some(sample);
        self.len += 1;
        Ok(())
    }
    /// At most HANDOFF_CAPACITY samples. A delayed drain preserves sample time.
    pub fn drain(&mut self, now_ms: u64) {
        let mut accepted = false;
        let mut delta = 0u64;
        let mut reset = false;
        for _ in 0..HANDOFF_CAPACITY {
            if self.len == 0 {
                break;
            }
            let sample = self.queue[self.head].take().expect("occupied handoff");
            self.head = (self.head + 1) % HANDOFF_CAPACITY;
            self.len -= 1;
            if self.slot.accept(sample, now_ms).is_err() {
                self.stats.rejected = self.stats.rejected.saturating_add(1);
                continue;
            }
            let errors = sample.value.errors();
            if let Some(old) = self.last_errors {
                reset |= errors < old;
                delta = delta.saturating_add(errors.saturating_sub(old));
            }
            self.last_errors = Some(errors);
            self.stats.accepted = self.stats.accepted.saturating_add(1);
            accepted = true;
        }
        if accepted {
            self.new_errors = delta;
            self.counter_reset = reset;
        }
    }
    pub fn view(&self, now_ms: u64) -> OwnerView<T> {
        let observation = self.slot.observation(now_ms);
        let value = self.slot.latest().map(|p| p.value);
        let condition = match observation.freshness {
            Freshness::Unbound => Condition::Unbound,
            Freshness::Awaiting => Condition::Awaiting,
            Freshness::Stale => Condition::Stale,
            Freshness::ClockReversed => Condition::ClockReversed,
            Freshness::Fresh => match value.expect("fresh publication").state() {
                OwnerState::Failed => Condition::Failed,
                OwnerState::Backpressured => Condition::Backpressured,
                _ if self.counter_reset => Condition::CounterReset,
                _ if self.new_errors != 0 => Condition::NewErrors,
                OwnerState::Idle => Condition::Idle,
                OwnerState::Active => Condition::Active,
                OwnerState::Disconnected => Condition::Disconnected,
            },
        };
        OwnerView {
            observation,
            condition,
            value,
            new_errors: self.new_errors,
            handoff: self.stats,
        }
    }
}

/// Shared with the actual product owner; counters remain owned by that runtime.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProductCounters {
    pub accepted_connections: u32,
    pub accepted_deck_controls: u32,
    pub accepted_mixer_controls: u32,
    pub accepted_library_controls: u32,
    pub library_load_locked: u32,
    pub library_failures: u32,
    pub effect_backpressure: u32,
    pub deferred_controls: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductObservation {
    pub counters: ProductCounters,
    pub queues: [u16; 4],
    pub playing: [bool; 2],
    pub backpressured: bool,
}
impl OwnerSample for ProductObservation {
    fn state(self) -> OwnerState {
        if self.backpressured {
            OwnerState::Backpressured
        } else if self.playing.iter().any(|v| *v) || self.queues.iter().any(|v| *v != 0) {
            OwnerState::Active
        } else {
            OwnerState::Idle
        }
    }
    fn errors(self) -> u64 {
        u64::from(self.counters.library_failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn idle() -> ProductObservation {
        ProductObservation {
            counters: ProductCounters::default(),
            queues: [0; 4],
            playing: [false; 2],
            backpressured: false,
        }
    }
    #[test]
    fn invalid_packets_cannot_renew_data_and_drains_are_bounded() {
        let mut channel = OwnerChannel::new(10);
        let mut tx = channel.restart().unwrap();
        let original = tx.sample(1, idle()).unwrap();
        channel.try_send(original).unwrap();
        channel.drain(1);
        let future = tx.sample(20, idle()).unwrap();
        channel.try_send(future).unwrap();
        channel.try_send(original).unwrap();
        channel.drain(11);
        assert_eq!(channel.view(11).handoff.rejected, 2);
        assert_eq!(channel.view(11).observation.stamp, Some(original.stamp));
        channel.drain(12);
        assert_eq!(channel.view(12).condition, Condition::Stale);
        let resumed = tx.sample(12, idle()).unwrap();
        channel.try_send(resumed).unwrap();
        channel.drain(12);
        assert_eq!(channel.view(12).condition, Condition::Idle);
        assert_eq!(channel.view(0).condition, Condition::ClockReversed);
    }
    #[test]
    fn restart_discards_pending_packets_and_exhaustion_does_not_rebind() {
        let mut channel = OwnerChannel::new(10);
        let mut tx = channel.restart().unwrap();
        let old = tx.sample(0, idle()).unwrap();
        channel.try_send(old).unwrap();
        let mut new_tx = channel.restart().unwrap();
        assert_eq!(channel.view(0).condition, Condition::Awaiting);
        assert_eq!(channel.view(0).handoff.discarded_on_restart, 1);
        assert_eq!(channel.try_send(old), Err(SendError::UnboundOrSuperseded));
        new_tx.sequence = u64::MAX;
        assert!(new_tx.sample(0, idle()).is_none());
        channel.generation = u64::MAX;
        assert!(channel.restart().is_none());
        assert_eq!(channel.view(0).condition, Condition::Unbound);
        assert_eq!(channel.try_send(old), Err(SendError::UnboundOrSuperseded));
    }
}
