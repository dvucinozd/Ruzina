//! Pull-OTA operation state (M2.5 `p4_ota_pull`).
//!
//! One operation at a time: a check or an install claims the coordinator,
//! and the HTTP adapter must also hold the RP-12 Wi-Fi transition lease for
//! the AP -> STA -> AP round trip. A check publishes an offer only for a
//! newer comparable release; an install starts only from a fresh offer that
//! the page names back, so a stale page cannot install something it never
//! saw. Status details are fixed operator texts, never URLs or credentials.

use crate::channel::{self, ChannelError, Offer};
use crate::install::{Failure, RebootPermit, Stage};
use crate::version::{ReleaseOrder, compare};

/// M2.5 transport bounds the adapter applies.
pub const HTTP_TIMEOUT_MS: u32 = 15_000;
pub const STA_TIMEOUT_MS: u32 = 20_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    JoinNetwork,
    FetchChannel,
    ParseChannel,
    CompareRelease,
    Offer,
    Install(Stage),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate {
        detail: &'static str,
    },
    /// See [`PullCoordinator::offer`].
    Available,
    Downloading,
    ReadyToReboot,
    Failed {
        phase: Phase,
        detail: &'static str,
    },
}

/// How the adapter's network part of a check or install ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    JoinTimeout,
    JoinFailed,
    NotFound,
    TooLarge,
    Unreachable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    Busy,
    NoOffer,
    Expired,
    /// The page named another release than the one offered.
    ReleaseMismatch,
}

pub struct PullCoordinator {
    state: State,
    /// Present exactly while `state` is `Available`, with its check time.
    offer: Option<(Offer, u32)>,
    busy: bool,
}

impl Default for PullCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl PullCoordinator {
    pub const fn new() -> Self {
        Self {
            state: State::Idle,
            offer: None,
            busy: false,
        }
    }

    pub const fn state(&self) -> &State {
        &self.state
    }

    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref().map(|(offer, _)| offer)
    }

    fn set(&mut self, state: State) {
        self.state = state;
        self.offer = None;
    }

    pub fn begin_check(&mut self) -> Result<(), Refusal> {
        if self.busy {
            return Err(Refusal::Busy);
        }
        self.busy = true;
        self.set(State::Checking);
        Ok(())
    }

    fn fail(&mut self, phase: Phase, detail: &'static str) {
        self.set(State::Failed { phase, detail });
        self.busy = false;
    }

    fn network_failure(&mut self, error: NetworkError) {
        match error {
            NetworkError::JoinTimeout => self.fail(Phase::JoinNetwork, "no address from network"),
            NetworkError::JoinFailed => self.fail(Phase::JoinNetwork, "could not join network"),
            NetworkError::NotFound => self.fail(Phase::FetchChannel, "no update published"),
            NetworkError::TooLarge => self.fail(Phase::FetchChannel, ChannelError::TooLarge.name()),
            NetworkError::Unreachable => {
                self.fail(Phase::FetchChannel, "could not reach update server")
            }
        }
    }

    /// Ends a check with the fetched channel document.
    pub fn finish_check(
        &mut self,
        fetched: Result<&[u8], NetworkError>,
        target: &str,
        running_version: &str,
        now_ms: u32,
    ) {
        if self.state != State::Checking {
            return;
        }
        let doc = match fetched {
            Ok(doc) => doc,
            Err(error) => return self.network_failure(error),
        };
        let offer = match channel::parse(doc, target) {
            Ok(offer) => offer,
            Err(error) => return self.fail(Phase::ParseChannel, error.name()),
        };
        self.busy = false;
        self.set(match compare(offer.release.as_str(), running_version) {
            ReleaseOrder::Newer => {
                self.state = State::Available;
                self.offer = Some((offer, now_ms));
                return;
            }
            ReleaseOrder::Same => State::UpToDate {
                detail: "already running this build",
            },
            ReleaseOrder::Older => State::UpToDate {
                detail: "older release ignored; use signed local upload to roll back",
            },
            ReleaseOrder::Unordered => State::Failed {
                phase: Phase::CompareRelease,
                detail: "release version is not comparable",
            },
        });
    }

    /// Claims the offer for an install. The returned offer is what the
    /// installer binds the bundle to.
    pub fn begin_install(&mut self, expected_release: &str, now_ms: u32) -> Result<Offer, Refusal> {
        if self.busy {
            return Err(Refusal::Busy);
        }
        let Some((offer, offered_at_ms)) = self.offer else {
            return Err(Refusal::NoOffer);
        };
        if !channel::offer_fresh(now_ms, offered_at_ms, channel::OFFER_TTL_MS) {
            self.set(State::Failed {
                phase: Phase::Offer,
                detail: "update offer expired; check again",
            });
            return Err(Refusal::Expired);
        }
        if expected_release.is_empty() || expected_release != offer.release.as_str() {
            return Err(Refusal::ReleaseMismatch);
        }
        self.busy = true;
        self.set(State::Downloading);
        Ok(offer)
    }

    /// Ends an install. On success the coordinator stays claimed until the
    /// reboot, so no second radio transition can start in between.
    pub fn finish_install(&mut self, result: Result<&RebootPermit, Result<Failure, NetworkError>>) {
        if self.state != State::Downloading {
            return;
        }
        match result {
            Ok(_) => self.set(State::ReadyToReboot),
            Err(Ok(failure)) => self.fail(Phase::Install(failure.stage), failure.fault.name()),
            Err(Err(error)) => self.network_failure(error),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::install::Fault;
    use std::format;
    use std::string::String;

    const RUNNING: &str = "M2.5-0-g20f1c3f";
    const HASH: &str = "83ba98aa00112233445566778899aabbccddeeff00112233445566778899ae22";

    fn doc(release: &str) -> String {
        format!(
            r#"{{"schema_version":1,"release":"{release}","p4":{{"url":"{release}/m1.ddjota","size":4407,"sha256":"{HASH}"}}}}"#
        )
    }

    fn checked(release: &str, now: u32) -> PullCoordinator {
        let mut pull = PullCoordinator::new();
        pull.begin_check().unwrap();
        pull.finish_check(Ok(doc(release).as_bytes()), "p4", RUNNING, now);
        pull
    }

    #[test]
    fn checks_offer_only_newer_comparable_releases() {
        let available = checked("M2.5-4-g1a2b3c4d", 0);
        assert_eq!(available.state(), &State::Available);
        assert_eq!(available.offer().unwrap().size, 4407);
        assert!(checked(RUNNING, 0).offer().is_none());
        assert_eq!(
            checked(RUNNING, 0).state(),
            &State::UpToDate {
                detail: "already running this build"
            }
        );
        assert!(matches!(
            checked("M2.4-9-g1234567", 0).state(),
            State::UpToDate { detail } if detail.contains("signed local upload")
        ));
        assert!(matches!(
            checked("M2.5-0-gdeadbee", 0).state(),
            State::Failed {
                phase: Phase::CompareRelease,
                ..
            }
        ));
        let mut pull = PullCoordinator::new();
        pull.begin_check().unwrap();
        assert_eq!(pull.begin_check(), Err(Refusal::Busy));
        assert_eq!(
            pull.begin_install("M2.5-4-g1a2b3c4d", 0),
            Err(Refusal::Busy)
        );
        pull.finish_check(Ok(b"{\"schema_version\":1"), "p4", RUNNING, 0);
        assert_eq!(
            pull.state(),
            &State::Failed {
                phase: Phase::ParseChannel,
                detail: "malformed"
            }
        );
        pull.begin_check().unwrap();
        pull.finish_check(Err(NetworkError::JoinTimeout), "p4", RUNNING, 0);
        assert_eq!(
            pull.state(),
            &State::Failed {
                phase: Phase::JoinNetwork,
                detail: "no address from network"
            }
        );
    }

    #[test]
    fn installs_start_only_from_a_fresh_offer_named_back() {
        let release = "M2.5-4-g1a2b3c4d";
        let mut pull = checked(release, 100);
        assert_eq!(pull.begin_install("", 100), Err(Refusal::ReleaseMismatch));
        assert_eq!(
            pull.begin_install("M2.5-5-g1a2b3c4d", 100),
            Err(Refusal::ReleaseMismatch)
        );
        let offer = pull
            .begin_install(release, 100 + channel::OFFER_TTL_MS)
            .unwrap();
        assert_eq!(offer.release.as_str(), release);
        assert_eq!(pull.state(), &State::Downloading);
        assert!(pull.offer().is_none(), "an offer is claimed once");
        assert_eq!(pull.begin_check(), Err(Refusal::Busy));
        pull.finish_install(Err(Ok(Failure {
            stage: Stage::VerifySignature,
            fault: Fault::Manifest(crate::bundle::ManifestError::Signature),
        })));
        assert_eq!(
            pull.state(),
            &State::Failed {
                phase: Phase::Install(Stage::VerifySignature),
                detail: "invalid manifest signature"
            }
        );
        assert_eq!(pull.begin_install(release, 100), Err(Refusal::NoOffer));

        let mut pull = checked(release, 100);
        assert_eq!(
            pull.begin_install(release, 101 + channel::OFFER_TTL_MS),
            Err(Refusal::Expired)
        );
        assert!(matches!(
            pull.state(),
            State::Failed {
                phase: Phase::Offer,
                ..
            }
        ));
        assert_eq!(pull.begin_install(release, 100), Err(Refusal::NoOffer));
        pull.begin_check().unwrap();
    }
}
