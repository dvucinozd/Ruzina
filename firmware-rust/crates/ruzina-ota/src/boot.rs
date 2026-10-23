//! Candidate readiness and confirmation (M2.5 `firmware_health` and
//! `p4_startup_gate`).
//!
//! A freshly installed image boots as a pending candidate. It becomes the
//! accepted image only after critical startup work is ready, including the
//! networking the saved settings requested; otherwise the deadline rejects
//! it and the bootloader returns to the previous slot. A reset before either
//! outcome also rolls back, because the bootloader never boots a pending
//! image twice. Accepted and factory images are never invalidated here.

/// M2.5 pending-image deadline.
pub const STARTUP_TIMEOUT_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageState {
    /// Factory image without an OTA state record.
    Factory,
    Valid,
    /// Candidate on its first boot, not yet confirmed.
    PendingVerify,
    Invalid,
    Aborted,
}

/// What the running image reports about itself (M1R-P0-085).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunningImage<'a> {
    pub slot: &'a str,
    pub version: &'a str,
    pub state: ImageState,
}

impl RunningImage<'_> {
    pub const fn rollback_pending(&self) -> bool {
        matches!(self.state, ImageState::PendingVerify)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Readiness {
    pub critical_tasks_alive: bool,
    pub critical_allocations_ok: bool,
    /// The requested AP/HTTP service is up.
    pub network_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupVerdict {
    Wait,
    Ready,
    Timeout,
}

/// `network_required` is captured from the saved settings at boot, not from
/// a later toggle, so a candidate cannot pass by switching Wi-Fi off.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupGate {
    boot_started_ms: u64,
    network_required: bool,
}

impl StartupGate {
    pub const fn new(boot_started_ms: u64, network_required: bool) -> Self {
        Self {
            boot_started_ms,
            network_required,
        }
    }

    pub const fn poll(&self, now_ms: u64, ready: Readiness) -> StartupVerdict {
        if now_ms < self.boot_started_ms || now_ms - self.boot_started_ms >= STARTUP_TIMEOUT_MS {
            return StartupVerdict::Timeout;
        }
        if ready.critical_tasks_alive
            && ready.critical_allocations_ok
            && (!self.network_required || ready.network_active)
        {
            StartupVerdict::Ready
        } else {
            StartupVerdict::Wait
        }
    }
}

/// The platform's image-state operations (ESP-IDF otadata).
pub trait BootControl {
    type Error: Copy;
    fn running_state(&mut self) -> Result<ImageState, Self::Error>;
    /// `esp_ota_mark_app_valid_cancel_rollback`.
    fn mark_valid(&mut self) -> Result<(), Self::Error>;
    /// `esp_ota_mark_app_invalid_rollback_and_reboot`; returns only on error.
    fn reject_and_reboot(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confirmation {
    /// Already accepted or factory: nothing to confirm.
    NotPending,
    Confirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootError<E> {
    /// Rejection refused: only a pending candidate can be rolled back.
    NotPending,
    Platform(E),
}

/// Confirms or rejects the running image once per boot.
pub struct BootHealth<B> {
    control: B,
    pending: bool,
}

impl<B: BootControl> BootHealth<B> {
    pub fn new(mut control: B) -> Result<Self, B::Error> {
        let pending = control.running_state()? == ImageState::PendingVerify;
        Ok(Self { control, pending })
    }

    pub const fn pending(&self) -> bool {
        self.pending
    }

    pub fn confirm(&mut self) -> Result<Confirmation, BootError<B::Error>> {
        if !self.pending {
            return Ok(Confirmation::NotPending);
        }
        self.control.mark_valid().map_err(BootError::Platform)?;
        self.pending = false;
        Ok(Confirmation::Confirmed)
    }

    pub fn reject(&mut self) -> Result<(), BootError<B::Error>> {
        if !self.pending {
            return Err(BootError::NotPending);
        }
        self.control
            .reject_and_reboot()
            .map_err(BootError::Platform)
    }

    /// One startup-gate step for a pending candidate: confirm when ready,
    /// reject at the deadline.
    pub fn settle(
        &mut self,
        gate: &StartupGate,
        now_ms: u64,
        ready: Readiness,
    ) -> Result<StartupVerdict, BootError<B::Error>> {
        if !self.pending {
            return Ok(StartupVerdict::Ready);
        }
        let verdict = gate.poll(now_ms, ready);
        match verdict {
            StartupVerdict::Wait => {}
            StartupVerdict::Ready => {
                self.confirm()?;
            }
            StartupVerdict::Timeout => self.reject()?,
        }
        Ok(verdict)
    }

    pub fn into_inner(self) -> B {
        self.control
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Readiness = Readiness {
        critical_tasks_alive: true,
        critical_allocations_ok: true,
        network_active: true,
    };

    #[test]
    fn readiness_requires_requested_network_and_has_a_deadline() {
        let gate = StartupGate::new(1_000, true);
        assert_eq!(gate.poll(1_000, ALL), StartupVerdict::Ready);
        let no_net = Readiness {
            network_active: false,
            ..ALL
        };
        assert_eq!(gate.poll(5_000, no_net), StartupVerdict::Wait);
        assert_eq!(
            StartupGate::new(1_000, false).poll(5_000, no_net),
            StartupVerdict::Ready
        );
        for broken in [
            Readiness {
                critical_tasks_alive: false,
                ..ALL
            },
            Readiness {
                critical_allocations_ok: false,
                ..ALL
            },
        ] {
            assert_eq!(
                StartupGate::new(0, false).poll(10, broken),
                StartupVerdict::Wait
            );
        }
        assert_eq!(
            gate.poll(1_000 + STARTUP_TIMEOUT_MS - 1, no_net),
            StartupVerdict::Wait
        );
        assert_eq!(
            gate.poll(1_000 + STARTUP_TIMEOUT_MS, ALL),
            StartupVerdict::Timeout
        );
        assert_eq!(
            gate.poll(999, ALL),
            StartupVerdict::Timeout,
            "clock went backwards"
        );
    }

    struct Fake {
        state: ImageState,
        marked: u8,
        rejected: u8,
        fail: bool,
    }

    impl BootControl for &mut Fake {
        type Error = ();
        fn running_state(&mut self) -> Result<ImageState, ()> {
            Ok(self.state)
        }
        fn mark_valid(&mut self) -> Result<(), ()> {
            if self.fail {
                return Err(());
            }
            self.marked += 1;
            self.state = ImageState::Valid;
            Ok(())
        }
        fn reject_and_reboot(&mut self) -> Result<(), ()> {
            self.rejected += 1;
            Err(())
        }
    }

    fn fake(state: ImageState) -> Fake {
        Fake {
            state,
            marked: 0,
            rejected: 0,
            fail: false,
        }
    }

    #[test]
    fn accepted_and_factory_images_are_never_rejected() {
        for state in [ImageState::Valid, ImageState::Factory] {
            let mut platform = fake(state);
            let mut health = BootHealth::new(&mut platform).unwrap();
            assert_eq!(health.reject(), Err(BootError::NotPending));
            assert_eq!(health.confirm(), Ok(Confirmation::NotPending));
            let gate = StartupGate::new(0, true);
            assert_eq!(
                health.settle(&gate, STARTUP_TIMEOUT_MS * 2, Readiness::default()),
                Ok(StartupVerdict::Ready)
            );
            assert_eq!((platform.marked, platform.rejected), (0, 0));
        }
    }

    #[test]
    fn a_pending_candidate_is_confirmed_once_or_rejected_at_the_deadline() {
        let gate = StartupGate::new(0, true);
        let mut platform = fake(ImageState::PendingVerify);
        let mut health = BootHealth::new(&mut platform).unwrap();
        assert_eq!(
            health.settle(&gate, 10, Readiness::default()),
            Ok(StartupVerdict::Wait)
        );
        assert_eq!(health.settle(&gate, 20, ALL), Ok(StartupVerdict::Ready));
        assert!(!health.pending());
        assert_eq!(health.confirm(), Ok(Confirmation::NotPending));
        assert_eq!(platform.marked, 1);

        let mut platform = fake(ImageState::PendingVerify);
        let mut health = BootHealth::new(&mut platform).unwrap();
        assert_eq!(
            health.settle(&gate, STARTUP_TIMEOUT_MS, ALL),
            Err(BootError::Platform(())),
            "the rollback call returns only on failure"
        );
        assert_eq!((platform.marked, platform.rejected), (0, 1));

        let mut platform = fake(ImageState::PendingVerify);
        platform.fail = true;
        let mut health = BootHealth::new(&mut platform).unwrap();
        assert_eq!(health.confirm(), Err(BootError::Platform(())));
        assert!(health.pending(), "a failed confirmation stays pending");
    }
}
