//! Control-plane routing. Drivers advertise readiness only after stream setup.
//! No bus probe, format conversion or device initialization runs in render.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelPair {
    pub left: u8,
    pub right: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyStream {
    pub connection_epoch: u32,
    pub stream_epoch: u32,
    pub interface: u8,
    pub channels: u8,
    pub pair: ChannelPair,
    pub sample_rate_hz: u32,
    pub sample_bits: u8,
}

impl ReadyStream {
    fn valid(self) -> bool {
        // The render rate, or 44.1 kHz through the adapter's exact-phase
        // converter (`ruzina-uac::resample`). Nothing else has a path.
        self.connection_epoch != 0
            && self.stream_epoch != 0
            && matches!(
                self.sample_rate_hz,
                crate::render::RENDER_SAMPLE_RATE_HZ | 44_100
            )
            && matches!(self.sample_bits, 16 | 24 | 32)
            && self.pair.left != self.pair.right
            && self.pair.left < self.channels
            && self.pair.right < self.channels
    }

    fn overlaps(self, other: Self) -> bool {
        self.connection_epoch == other.connection_epoch
            && self.interface == other.interface
            && (self.pair.left == other.pair.left
                || self.pair.left == other.pair.right
                || self.pair.right == other.pair.left
                || self.pair.right == other.pair.right)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputAvailability {
    pub external_dac: Option<ReadyStream>,
    pub controller_main: Option<ReadyStream>,
    pub controller_headphones: Option<ReadyStream>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MainRoute {
    #[default]
    Muted,
    ExternalDac(ReadyStream),
    Controller(ReadyStream),
}

/// The type deliberately has no external-DAC or onboard-codec headphone case.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HeadphoneRoute {
    #[default]
    Unavailable,
    Controller(ReadyStream),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputRoutes {
    pub main: MainRoute,
    pub headphones: HeadphoneRoute,
}

pub fn select_routes(available: OutputAvailability) -> OutputRoutes {
    let phones = available
        .controller_headphones
        .filter(|stream| stream.valid());
    // A MAIN map must never target the controller's headphone channels. Keep
    // the independently valid headphone route when MAIN metadata is invalid.
    let controller_main = available.controller_main.filter(|stream| {
        stream.valid()
            && phones.is_none_or(|phones| {
                stream.connection_epoch == phones.connection_epoch
                    && (stream.interface != phones.interface
                        || stream.stream_epoch == phones.stream_epoch)
                    && !stream.overlaps(phones)
            })
    });
    let main = if let Some(dac) = available.external_dac.filter(|stream| stream.valid()) {
        MainRoute::ExternalDac(dac)
    } else if let Some(controller) = controller_main {
        MainRoute::Controller(controller)
    } else {
        MainRoute::Muted
    };
    OutputRoutes {
        main,
        headphones: phones.map(HeadphoneRoute::Controller).unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteTransition {
    pub revision: u64,
    pub from: OutputRoutes,
    pub to: OutputRoutes,
}

#[derive(Default)]
pub struct OutputRouter {
    active: OutputRoutes,
    desired: OutputRoutes,
    pending: Option<RouteTransition>,
    revision: u64,
    main_revision: u64,
    headphone_revision: u64,
}

impl OutputRouter {
    pub const fn active(&self) -> OutputRoutes {
        self.active
    }
    pub const fn pending(&self) -> Option<RouteTransition> {
        self.pending
    }
    pub const fn main_revision(&self) -> u64 {
        self.main_revision
    }
    pub const fn headphone_revision(&self) -> u64 {
        self.headphone_revision
    }

    /// Invalidate changed bindings before the caller awaits any device work.
    /// A MAIN-only transition leaves the controller headphone binding active.
    pub fn update(&mut self, availability: OutputAvailability) -> Option<RouteTransition> {
        let desired = select_routes(availability);
        if desired == self.desired {
            return None;
        }
        if desired.main != self.desired.main {
            self.main_revision = self.main_revision.wrapping_add(1).max(1);
        }
        if desired.headphones != self.desired.headphones {
            self.headphone_revision = self.headphone_revision.wrapping_add(1).max(1);
        }
        self.desired = desired;
        self.revision = self.revision.wrapping_add(1).max(1);
        let transition = RouteTransition {
            revision: self.revision,
            from: self.active,
            to: desired,
        };
        if self.active.main != desired.main {
            self.active.main = MainRoute::Muted;
        }
        if self.active.headphones != desired.headphones {
            self.active.headphones = HeadphoneRoute::Unavailable;
        }
        self.pending = Some(transition);
        Some(transition)
    }

    /// Driver acknowledgement after old-route fade/mute and buffer drain,
    /// new-stream preparation/priming, and a fresh readiness update. A delayed
    /// acknowledgement cannot activate a superseded/disconnected stream.
    pub fn commit_prepared(&mut self, transition: RouteTransition) -> bool {
        if self.pending != Some(transition) {
            return false;
        }
        self.active = transition.to;
        self.pending = None;
        true
    }

    pub fn accepts_main(&self, route: MainRoute) -> bool {
        route != MainRoute::Muted && self.active.main == route
    }

    pub fn accepts_headphones(&self, route: HeadphoneRoute) -> bool {
        route != HeadphoneRoute::Unavailable && self.active.headphones == route
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(epoch: u32, left: u8, right: u8) -> ReadyStream {
        ReadyStream {
            connection_epoch: epoch,
            stream_epoch: 1,
            interface: 1,
            channels: 4,
            pair: ChannelPair { left, right },
            sample_rate_hz: 48_000,
            sample_bits: 16,
        }
    }
    fn controller() -> OutputAvailability {
        OutputAvailability {
            controller_main: Some(stream(1, 0, 1)),
            controller_headphones: Some(stream(1, 2, 3)),
            ..OutputAvailability::default()
        }
    }

    #[test]
    fn dac_priority_changes_only_main_and_requires_prepared_acknowledgement() {
        let mut router = OutputRouter::default();
        let fallback = router.update(controller()).unwrap();
        assert_eq!(router.active(), OutputRoutes::default());
        assert!(router.commit_prepared(fallback));
        let phone = router.active().headphones;
        let mut available = controller();
        available.external_dac = Some(stream(10, 0, 1));
        let upgrade = router.update(available).unwrap();
        assert_eq!(router.active().main, MainRoute::Muted);
        assert_eq!(router.active().headphones, phone);
        assert!(router.accepts_headphones(phone));
        assert!(router.commit_prepared(upgrade));
        assert!(matches!(router.active().main, MainRoute::ExternalDac(_)));
        assert_eq!(router.active().headphones, phone);
    }

    #[test]
    fn dac_loss_falls_back_to_controller_and_does_not_interrupt_headphones() {
        let mut router = OutputRouter::default();
        let mut available = controller();
        available.external_dac = Some(stream(10, 0, 1));
        let start = router.update(available).unwrap();
        router.commit_prepared(start);
        let old = router.active().main;
        let phone = router.active().headphones;
        let fallback = router.update(controller()).unwrap();
        assert!(!router.accepts_main(old));
        assert!(router.accepts_headphones(phone));
        assert!(router.commit_prepared(fallback));
        assert!(matches!(router.active().main, MainRoute::Controller(_)));
    }

    #[test]
    fn controller_loss_with_external_main_makes_headphones_unavailable() {
        let mut router = OutputRouter::default();
        let dac = stream(10, 0, 1);
        let mut available = controller();
        available.external_dac = Some(dac);
        let start = router.update(available).unwrap();
        router.commit_prepared(start);
        let old_phone = router.active().headphones;
        let detach = router
            .update(OutputAvailability {
                external_dac: Some(dac),
                ..OutputAvailability::default()
            })
            .unwrap();
        assert!(router.accepts_main(MainRoute::ExternalDac(dac)));
        assert!(!router.accepts_headphones(old_phone));
        router.commit_prepared(detach);
        assert_eq!(router.active().headphones, HeadphoneRoute::Unavailable);
    }

    #[test]
    fn reconnect_and_fault_recovery_reject_superseded_preparation_and_old_packets() {
        let mut router = OutputRouter::default();
        let old = router.update(controller()).unwrap();
        let none = router.update(OutputAvailability::default()).unwrap();
        assert!(!router.commit_prepared(old));
        assert!(router.commit_prepared(none));
        let mut reconnected = controller();
        reconnected
            .controller_main
            .as_mut()
            .unwrap()
            .connection_epoch = 2;
        reconnected
            .controller_headphones
            .as_mut()
            .unwrap()
            .connection_epoch = 2;
        let new = router.update(reconnected).unwrap();
        assert!(!router.accepts_main(old.to.main));
        assert!(router.commit_prepared(new));
        assert!(!router.accepts_headphones(old.to.headphones));
        assert!(router.accepts_headphones(new.to.headphones));
        assert!(!router.commit_prepared(old));
    }

    #[test]
    fn overlapping_or_invalid_main_map_cannot_send_main_audio_to_headphones() {
        for pair in [
            ChannelPair { left: 2, right: 3 },
            ChannelPair { left: 1, right: 2 },
            ChannelPair { left: 0, right: 4 },
        ] {
            let mut available = controller();
            available.controller_main.as_mut().unwrap().pair = pair;
            let routes = select_routes(available);
            assert_eq!(routes.main, MainRoute::Muted);
            assert!(matches!(routes.headphones, HeadphoneRoute::Controller(_)));
        }
    }

    #[test]
    fn separate_controller_interfaces_can_use_their_own_stereo_pairs() {
        let mut available = controller();
        let headphones = available.controller_headphones.as_mut().unwrap();
        headphones.interface = 2;
        headphones.channels = 2;
        headphones.pair = ChannelPair { left: 0, right: 1 };
        let routes = select_routes(available);
        assert!(matches!(routes.main, MainRoute::Controller(_)));
        assert!(matches!(routes.headphones, HeadphoneRoute::Controller(_)));
    }

    #[test]
    fn unsupported_format_or_uninitialized_epoch_is_not_a_ready_dac() {
        for bad in [
            ReadyStream {
                sample_rate_hz: 96_000,
                ..stream(10, 0, 1)
            },
            ReadyStream {
                sample_rate_hz: 32_000,
                ..stream(10, 0, 1)
            },
            ReadyStream {
                sample_bits: 8,
                ..stream(10, 0, 1)
            },
            ReadyStream {
                stream_epoch: 0,
                ..stream(10, 0, 1)
            },
            stream(0, 0, 1),
        ] {
            let mut available = controller();
            available.external_dac = Some(bad);
            assert!(matches!(
                select_routes(available).main,
                MainRoute::Controller(_)
            ));
        }
    }

    #[test]
    fn a_44_1_khz_controller_stream_is_routable_through_the_converter() {
        let mut available = controller();
        for stream in [
            available.controller_main.as_mut().unwrap(),
            available.controller_headphones.as_mut().unwrap(),
        ] {
            stream.sample_rate_hz = 44_100;
        }
        let routes = select_routes(available);
        assert!(matches!(routes.main, MainRoute::Controller(_)));
        assert!(matches!(routes.headphones, HeadphoneRoute::Controller(_)));
    }

    #[test]
    fn midi_only_controller_does_not_create_headphones_and_no_outputs_means_mute() {
        assert_eq!(
            select_routes(OutputAvailability::default()),
            OutputRoutes::default()
        );
        let routes = select_routes(OutputAvailability {
            external_dac: Some(stream(10, 0, 1)),
            ..OutputAvailability::default()
        });
        assert!(matches!(routes.main, MainRoute::ExternalDac(_)));
        assert_eq!(routes.headphones, HeadphoneRoute::Unavailable);
    }

    #[test]
    fn repeated_availability_does_not_restart_a_transition() {
        let mut router = OutputRouter::default();
        let transition = router.update(controller()).unwrap();
        assert!(router.update(controller()).is_none());
        assert_eq!(router.pending(), Some(transition));
        assert!(router.commit_prepared(transition));
        assert!(router.update(controller()).is_none());
    }

    #[test]
    fn incoherent_controller_epochs_cannot_mix_old_main_with_new_headphones() {
        for stream_change in [false, true] {
            let mut available = controller();
            let main = available.controller_main.as_mut().unwrap();
            if stream_change {
                main.stream_epoch = 2;
            } else {
                main.connection_epoch = 2;
            }
            let routes = select_routes(available);
            assert_eq!(routes.main, MainRoute::Muted);
            assert!(matches!(routes.headphones, HeadphoneRoute::Controller(_)));
        }
    }
}
