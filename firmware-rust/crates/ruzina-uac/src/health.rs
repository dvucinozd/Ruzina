//! Active-versus-idle UAC fault accounting (M2.5 `audio_uac_health`).
//!
//! The isochronous consumer runs continuously and zero-fills while nothing
//! plays, so idle underflow is not a fault. Counters are cumulative; the
//! monitor reports deltas per sample, latches data loss for the playback
//! session, and forgives exactly one producer-prime underflow after PLAY or
//! a new stream, and only once the ring has recovered. Sampling is a few
//! integer comparisons: no locks, formatting or heap walks.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Flags(u8);

impl Flags {
    pub const PRESSURE_LOW: Self = Self(1 << 0);
    pub const PRESSURE_HIGH: Self = Self(1 << 1);
    pub const DROPPED: Self = Self(1 << 2);
    pub const OVERFLOW: Self = Self(1 << 3);
    pub const UNDERFLOW: Self = Self(1 << 4);
    pub const PACKET_LOSS: Self = Self(1 << 5);
    const DATA_LOSS: u8 =
        Self::DROPPED.0 | Self::OVERFLOW.0 | Self::UNDERFLOW.0 | Self::PACKET_LOSS.0;

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    fn set(&mut self, flag: Self, on: bool) {
        if on {
            self.0 |= flag.0;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RingState {
    Unavailable,
    Idle,
    Low,
    Nominal,
    High,
}

/// Below half full: the regulator keeps the ring above 5/8.
pub const fn low_alarm_frames(capacity: u32) -> u32 {
    capacity / 2
}

/// Above 15/16: the regulator's upper edge is 7/8.
pub const fn high_alarm_frames(capacity: u32) -> u32 {
    (capacity as u64 * 15 / 16) as u32
}

pub const fn ring_state(
    playing: bool,
    submitted_blocks: u32,
    queued: u32,
    capacity: u32,
) -> RingState {
    if capacity == 0 || submitted_blocks == 0 {
        RingState::Unavailable
    } else if !playing {
        RingState::Idle
    } else if queued < low_alarm_frames(capacity) {
        RingState::Low
    } else if queued > high_alarm_frames(capacity) {
        RingState::High
    } else {
        RingState::Nominal
    }
}

/// One sample of cumulative counters from the stream and its producer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counters {
    pub playing: bool,
    /// Advances whenever all-idle becomes active.
    pub playback_session: u32,
    /// The UAC stream epoch; zero while no stream exists.
    pub stream_epoch: u32,
    pub submitted_blocks: u32,
    pub queued_frames: u32,
    pub capacity_frames: u32,
    pub dropped_blocks: u32,
    pub overflow_frames: u32,
    pub underflow_frames: u32,
    pub lost_frames: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sample {
    pub flags: Flags,
    pub dropped_blocks: u32,
    pub overflow_frames: u32,
    pub underflow_frames: u32,
    pub lost_frames: u32,
    /// Data loss latched since this playback session started.
    pub session_loss: Flags,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HealthMonitor {
    last: Option<Counters>,
    grace_pending: bool,
    session_loss: Flags,
}

impl HealthMonitor {
    pub const fn new() -> Self {
        Self {
            last: None,
            grace_pending: false,
            session_loss: Flags(0),
        }
    }

    pub fn sample(&mut self, now: Counters) -> Sample {
        let mut out = Sample::default();
        let previous = self.last.replace(now);
        let started = now.playing
            && previous
                .is_none_or(|last| !last.playing || last.playback_session != now.playback_session);
        // Epoch zero (disconnect) cannot clear a latched fault.
        let new_stream = now.playing
            && now.stream_epoch != 0
            && previous.is_some_and(|last| last.stream_epoch != now.stream_epoch);
        if let Some(last) = previous {
            out.dropped_blocks = now.dropped_blocks.saturating_sub(last.dropped_blocks);
            out.overflow_frames = now.overflow_frames.saturating_sub(last.overflow_frames);
            out.underflow_frames = now.underflow_frames.saturating_sub(last.underflow_frames);
            out.lost_frames = now.lost_frames.saturating_sub(last.lost_frames);
        }
        if !now.playing {
            self.session_loss = Flags::default();
            self.grace_pending = false;
            return Sample::default();
        }
        if started {
            self.session_loss = Flags::default();
            self.grace_pending = true;
            out = Sample::default();
        } else if new_stream {
            // Only the new stream's prime underflow is eligible for grace.
            self.session_loss = Flags::default();
            self.grace_pending = true;
            out.underflow_frames = 0;
        }
        let state = ring_state(
            true,
            now.submitted_blocks,
            now.queued_frames,
            now.capacity_frames,
        );
        if !started && !new_stream && self.grace_pending {
            if matches!(state, RingState::Nominal | RingState::High) {
                out.underflow_frames = 0;
            }
            self.grace_pending = false;
        }
        out.flags.set(Flags::PRESSURE_LOW, state == RingState::Low);
        out.flags
            .set(Flags::PRESSURE_HIGH, state == RingState::High);
        out.flags.set(Flags::DROPPED, out.dropped_blocks > 0);
        out.flags.set(Flags::OVERFLOW, out.overflow_frames > 0);
        out.flags.set(Flags::UNDERFLOW, out.underflow_frames > 0);
        out.flags.set(Flags::PACKET_LOSS, out.lost_frames > 0);
        self.session_loss.0 |= out.flags.0 & Flags::DATA_LOSS;
        out.session_loss = self.session_loss;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOMINAL: Counters = Counters {
        playing: true,
        playback_session: 1,
        stream_epoch: 4,
        submitted_blocks: 10,
        queued_frames: 1_500,
        capacity_frames: 2_048,
        dropped_blocks: 0,
        overflow_frames: 0,
        underflow_frames: 0,
        lost_frames: 0,
    };

    #[test]
    fn idle_underflow_is_not_a_fault() {
        let mut monitor = HealthMonitor::new();
        let idle = Counters {
            playing: false,
            ..NOMINAL
        };
        monitor.sample(idle);
        let sample = monitor.sample(Counters {
            underflow_frames: 5_000,
            ..idle
        });
        assert!(sample.flags.is_empty());
        assert_eq!(sample.underflow_frames, 0);
        assert_eq!(ring_state(false, 10, 0, 2_048), RingState::Idle);
    }

    #[test]
    fn one_prime_underflow_is_forgiven_only_after_recovery() {
        let mut monitor = HealthMonitor::new();
        monitor.sample(Counters {
            playing: false,
            ..NOMINAL
        });
        assert!(monitor.sample(NOMINAL).flags.is_empty(), "PLAY sample");
        let recovered = monitor.sample(Counters {
            underflow_frames: 40,
            ..NOMINAL
        });
        assert!(
            recovered.flags.is_empty(),
            "prime underflow with a recovered ring"
        );
        let later = monitor.sample(Counters {
            underflow_frames: 41,
            ..NOMINAL
        });
        assert!(later.flags.contains(Flags::UNDERFLOW));
        assert!(later.session_loss.contains(Flags::UNDERFLOW));
        assert!(
            monitor
                .sample(Counters {
                    underflow_frames: 41,
                    ..NOMINAL
                })
                .session_loss
                .contains(Flags::UNDERFLOW),
            "latched for the session"
        );

        let mut monitor = HealthMonitor::new();
        monitor.sample(NOMINAL);
        let still_low = monitor.sample(Counters {
            underflow_frames: 40,
            queued_frames: 100,
            ..NOMINAL
        });
        assert!(
            still_low.flags.contains(Flags::UNDERFLOW),
            "no grace when the ring stays low"
        );
        assert!(still_low.flags.contains(Flags::PRESSURE_LOW));
    }

    #[test]
    fn reconnect_forgives_prime_underflow_but_not_loss_drops_or_overflow() {
        let mut monitor = HealthMonitor::new();
        monitor.sample(NOMINAL);
        monitor.sample(NOMINAL);
        let reconnect = monitor.sample(Counters {
            stream_epoch: 5,
            underflow_frames: 30,
            lost_frames: 3,
            dropped_blocks: 1,
            overflow_frames: 2,
            ..NOMINAL
        });
        assert!(!reconnect.flags.contains(Flags::UNDERFLOW));
        assert!(reconnect.flags.contains(Flags::PACKET_LOSS));
        assert!(reconnect.flags.contains(Flags::DROPPED));
        assert!(reconnect.flags.contains(Flags::OVERFLOW));
        let gone = monitor.sample(Counters {
            stream_epoch: 0,
            underflow_frames: 30,
            lost_frames: 3,
            dropped_blocks: 1,
            overflow_frames: 2,
            ..NOMINAL
        });
        assert!(
            gone.session_loss.contains(Flags::PACKET_LOSS),
            "disconnect keeps the latch"
        );
    }

    #[test]
    fn a_stop_play_between_samples_starts_a_new_session() {
        let mut monitor = HealthMonitor::new();
        monitor.sample(NOMINAL);
        let lossy = monitor.sample(Counters {
            lost_frames: 1,
            ..NOMINAL
        });
        assert!(lossy.session_loss.contains(Flags::PACKET_LOSS));
        let next = monitor.sample(Counters {
            lost_frames: 1,
            playback_session: 2,
            ..NOMINAL
        });
        assert!(next.session_loss.is_empty());
        assert_eq!(high_alarm_frames(2_048), 1_920);
        assert_eq!(ring_state(true, 10, 1_921, 2_048), RingState::High);
        assert_eq!(ring_state(true, 0, 1_000, 2_048), RingState::Unavailable);
    }
}
