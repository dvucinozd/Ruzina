//! External DAC readiness and bounded MAIN output changes (RP-10).
//!
//! A DAC is a MAIN candidate only once it is configured and streaming: an
//! I2C acknowledge proves that something answers, not that a stereo DAC is
//! ready. A MAIN change then runs in bounded steps under PLAY, LOAD or
//! scratch: a planned switch fades the old stream and drains its in-flight
//! buffers up to a deadline; a removal skips straight to rebinding. Either
//! way the router revokes the old binding, so its late completions earn no
//! progress credit, and the controller headphone lane is never touched.

use crate::routing::{OutputAvailability, OutputRouter, ReadyStream, RouteTransition};
use crate::sink::{SinkError, SinkLane};

/// 10 ms fade at the render rate.
pub const MAIN_FADE_FRAMES: u32 = 480;
/// In-flight buffers get 100 ms to complete before they are revoked.
pub const MAIN_DRAIN_FRAMES: u64 = 4_800;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DacState {
    Absent,
    /// Answers on its bus; not yet a usable output.
    Responding,
    /// Registers written and read back; no stream yet.
    Configured,
    Ready(ReadyStream),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalDac {
    state: DacState,
    connection_epoch: u32,
}

impl Default for ExternalDac {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalDac {
    pub const fn new() -> Self {
        Self {
            state: DacState::Absent,
            connection_epoch: 0,
        }
    }

    pub const fn state(&self) -> DacState {
        self.state
    }

    pub fn acknowledged(&mut self) {
        if self.state == DacState::Absent {
            self.connection_epoch = self.connection_epoch.wrapping_add(1).max(1);
            self.state = DacState::Responding;
        }
    }

    /// `verified`: the identity and configuration read back as written.
    pub fn configured(&mut self, verified: bool) {
        if self.state == DacState::Responding {
            self.state = if verified {
                DacState::Configured
            } else {
                DacState::Absent
            };
        }
    }

    /// The I2S/USB stream is running with this shape.
    pub fn streaming(&mut self, stream_epoch: u32, stream: ReadyStream) {
        if self.state == DacState::Configured || matches!(self.state, DacState::Ready(_)) {
            self.state = DacState::Ready(ReadyStream {
                connection_epoch: self.connection_epoch,
                stream_epoch,
                ..stream
            });
        }
    }

    pub fn lost(&mut self) {
        self.state = DacState::Absent;
    }

    /// What routing may use as `OutputAvailability::external_dac`.
    pub const fn ready(&self) -> Option<ReadyStream> {
        match self.state {
            DacState::Ready(stream) => Some(stream),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    /// The old MAIN still works (a DAC arrives, a better route appears).
    Planned,
    /// The old MAIN is gone: no fade or drain can reach it.
    Removal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Fading { until: u64 },
    Draining { deadline: u64 },
    Rebinding(RouteTransition),
    Done,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeStatus {
    InProgress,
    /// The new route is committed; its lane starts muted and fades in.
    Committed,
    /// No MAIN route remains; MAIN stays muted.
    Muted,
    /// A newer availability superseded this change before it committed.
    Superseded,
}

/// One MAIN change, advanced from the output owner at block boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MainChange {
    target: OutputAvailability,
    phase: Phase,
    /// In-flight buffers revoked at the drain deadline.
    pub revoked: bool,
}

impl MainChange {
    pub fn begin(
        kind: ChangeKind,
        target: OutputAvailability,
        main: &mut SinkLane<'_>,
        now: u64,
    ) -> Result<Self, SinkError> {
        let phase = match (kind, main.token()) {
            (ChangeKind::Planned, Some(token)) => {
                main.set_mute(token, true, MAIN_FADE_FRAMES)?;
                Phase::Fading {
                    until: now + u64::from(MAIN_FADE_FRAMES),
                }
            }
            _ => Phase::Draining { deadline: now },
        };
        Ok(Self {
            target,
            phase,
            revoked: false,
        })
    }

    /// `prepared`: the driver primed the new stream and reported readiness
    /// again after the router invalidated the old binding.
    pub fn tick(
        &mut self,
        router: &mut OutputRouter,
        main: &mut SinkLane<'_>,
        now: u64,
        prepared: bool,
    ) -> Result<ChangeStatus, SinkError> {
        match self.phase {
            Phase::Fading { until } => {
                if now >= until {
                    self.phase = Phase::Draining {
                        deadline: now + MAIN_DRAIN_FRAMES,
                    };
                }
                Ok(ChangeStatus::InProgress)
            }
            Phase::Draining { deadline } => {
                if main.in_flight() != 0 && now < deadline {
                    return Ok(ChangeStatus::InProgress);
                }
                self.revoked = main.in_flight() != 0;
                let transition = router.update(self.target);
                main.sync_route(router, now)?;
                match transition {
                    Some(transition) => {
                        self.phase = Phase::Rebinding(transition);
                        Ok(ChangeStatus::InProgress)
                    }
                    None => {
                        self.phase = Phase::Done;
                        Ok(ChangeStatus::Committed)
                    }
                }
            }
            Phase::Rebinding(transition) => {
                if router.pending() != Some(transition) {
                    self.phase = Phase::Done;
                    return Ok(ChangeStatus::Superseded);
                }
                if !prepared {
                    return Ok(ChangeStatus::InProgress);
                }
                router.commit_prepared(transition);
                main.sync_route(router, now)?;
                self.phase = Phase::Done;
                match main.token() {
                    Some(token) => {
                        main.set_mute(token, false, MAIN_FADE_FRAMES)?;
                        Ok(ChangeStatus::Committed)
                    }
                    None => Ok(ChangeStatus::Muted),
                }
            }
            Phase::Done => Ok(ChangeStatus::Committed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RENDER_BLOCK_FRAMES;
    use crate::routing::{ChannelPair, MainRoute};
    use crate::sink::{SinkBlock, SinkLane, SinkRole};
    use ruzina_audio_dsp::PcmFrame;

    fn stream(connection: u32, pair: (u8, u8)) -> ReadyStream {
        ReadyStream {
            connection_epoch: connection,
            stream_epoch: 1,
            interface: 1,
            channels: 4,
            pair: ChannelPair {
                left: pair.0,
                right: pair.1,
            },
            sample_rate_hz: 48_000,
            sample_bits: 16,
        }
    }

    fn controller() -> OutputAvailability {
        OutputAvailability {
            controller_main: Some(stream(1, (0, 1))),
            controller_headphones: Some(stream(1, (2, 3))),
            external_dac: None,
        }
    }

    #[test]
    fn an_acknowledge_alone_is_never_a_ready_dac() {
        let mut dac = ExternalDac::new();
        dac.acknowledged();
        assert_eq!(dac.ready(), None);
        dac.streaming(1, stream(9, (0, 1)));
        assert_eq!(dac.ready(), None, "no stream before configuration");
        dac.configured(false);
        assert_eq!(dac.state(), DacState::Absent, "failed readback");
        dac.acknowledged();
        dac.configured(true);
        assert_eq!(dac.ready(), None);
        dac.streaming(3, stream(9, (0, 1)));
        let ready = dac.ready().unwrap();
        assert_eq!((ready.connection_epoch, ready.stream_epoch), (2, 3));
        dac.lost();
        dac.acknowledged();
        dac.configured(true);
        dac.streaming(1, stream(9, (0, 1)));
        assert_eq!(
            dac.ready().unwrap().connection_epoch,
            3,
            "a reconnect is a new DAC"
        );
    }

    fn lanes<'a>(
        main: &'a mut [SinkBlock],
        phones: &'a mut [SinkBlock],
        router: &mut OutputRouter,
    ) -> (SinkLane<'a>, SinkLane<'a>) {
        let transition = router.update(controller()).unwrap();
        router.commit_prepared(transition);
        let mut main = SinkLane::new(SinkRole::Main, main).unwrap();
        let mut phones = SinkLane::new(SinkRole::Headphones, phones).unwrap();
        main.sync_route(router, 0).unwrap();
        phones.sync_route(router, 0).unwrap();
        for lane in [&mut main, &mut phones] {
            let token = lane.token().unwrap();
            lane.set_mute(token, false, 0).unwrap();
            lane.expect_active(token, true).unwrap();
        }
        (main, phones)
    }

    #[test]
    fn a_planned_dac_switch_fades_drains_rebinds_and_spares_headphones() {
        let mut router = OutputRouter::default();
        let (mut main_storage, mut phone_storage) = ([SinkBlock::EMPTY; 4], [SinkBlock::EMPTY; 4]);
        let (mut main, mut phones) = lanes(&mut main_storage, &mut phone_storage, &mut router);
        let phones_token = phones.token();
        let old = main.token().unwrap();
        let mut out = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let in_flight = main.issue(old, 0, 64, &mut out).unwrap();

        let mut target = controller();
        let mut dac = ExternalDac::new();
        dac.acknowledged();
        dac.configured(true);
        dac.streaming(1, stream(0, (0, 1)));
        target.external_dac = dac.ready();
        let mut change = MainChange::begin(ChangeKind::Planned, target, &mut main, 64).unwrap();
        assert!(!main.is_muted(), "fading, not cut");
        assert_eq!(
            change.tick(&mut router, &mut main, 64 + 479, false),
            Ok(ChangeStatus::InProgress)
        );
        assert_eq!(
            change.tick(&mut router, &mut main, 64 + 480, false),
            Ok(ChangeStatus::InProgress)
        );
        // The old buffer completes inside the drain window and is credited.
        main.complete(in_flight).unwrap();
        assert_eq!(
            change.tick(&mut router, &mut main, 600, false),
            Ok(ChangeStatus::InProgress)
        );
        assert!(!change.revoked);
        assert_eq!(
            main.token(),
            None,
            "old binding invalidated before device work"
        );
        assert_eq!(
            change.tick(&mut router, &mut main, 700, false),
            Ok(ChangeStatus::InProgress)
        );
        assert_eq!(
            change.tick(&mut router, &mut main, 800, true),
            Ok(ChangeStatus::Committed)
        );
        assert!(matches!(router.active().main, MainRoute::ExternalDac(_)));
        assert_eq!(
            phones.token(),
            phones_token,
            "headphones keep their binding"
        );
        phones.sync_route(&router, 800).unwrap();
        assert_eq!(phones.token(), phones_token);
        assert_eq!(phones.stats().discarded_frames, 0);
    }

    #[test]
    fn removal_under_play_is_bounded_and_late_completions_earn_nothing() {
        let mut router = OutputRouter::default();
        let (mut main_storage, mut phone_storage) = ([SinkBlock::EMPTY; 4], [SinkBlock::EMPTY; 4]);
        let (mut main, mut phones) = lanes(&mut main_storage, &mut phone_storage, &mut router);
        let phones_token = phones.token();
        let old = main.token().unwrap();
        let mut out = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let stuck = main.issue(old, 0, 128, &mut out).unwrap();

        // The controller's MAIN interface disappears; its headphones remain.
        let mut target = controller();
        target.controller_main = None;
        let mut change = MainChange::begin(ChangeKind::Removal, target, &mut main, 128).unwrap();
        assert_eq!(
            change.tick(&mut router, &mut main, 128, false),
            Ok(ChangeStatus::InProgress)
        );
        assert!(change.revoked, "no wait for a device that is gone");
        assert_eq!(main.complete(stuck), Err(SinkError::Stale));
        assert_eq!(main.stats().completed_frames, 0);
        assert_eq!(
            change.tick(&mut router, &mut main, 129, true),
            Ok(ChangeStatus::Muted)
        );
        assert_eq!(router.active().main, MainRoute::Muted);
        phones.sync_route(&router, 129).unwrap();
        assert_eq!(
            phones.token(),
            phones_token,
            "no headphone fallback or restart"
        );
    }

    #[test]
    fn a_planned_switch_whose_old_buffers_never_return_is_revoked_at_the_deadline() {
        let mut router = OutputRouter::default();
        let (mut main_storage, mut phone_storage) = ([SinkBlock::EMPTY; 4], [SinkBlock::EMPTY; 4]);
        let (mut main, _phones) = lanes(&mut main_storage, &mut phone_storage, &mut router);
        let old = main.token().unwrap();
        let mut out = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        main.issue(old, 0, 128, &mut out).unwrap();
        let mut target = controller();
        target.external_dac = Some(stream(5, (0, 1)));
        let mut change = MainChange::begin(ChangeKind::Planned, target, &mut main, 0).unwrap();
        change.tick(&mut router, &mut main, 480, false).unwrap();
        assert_eq!(
            change.tick(&mut router, &mut main, 480 + MAIN_DRAIN_FRAMES - 1, false),
            Ok(ChangeStatus::InProgress)
        );
        assert_eq!(main.token(), Some(old), "still draining");
        change
            .tick(&mut router, &mut main, 480 + MAIN_DRAIN_FRAMES, false)
            .unwrap();
        assert!(change.revoked);
        assert!(main.stats().revoked_in_flight_frames >= 128);

        // A newer availability arrives before the driver prepared.
        router.update(controller());
        assert_eq!(
            change.tick(&mut router, &mut main, 6_000, true),
            Ok(ChangeStatus::Superseded)
        );
    }
}
