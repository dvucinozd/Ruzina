//! Shared real-product supervisor. One job/pending delivery; bounded recycling.
use crate::product_runtime::{BackgroundAnalysisRefusal, ProductRuntime};
use ruzina_apta_adapter::{
    owner::Output,
    publication::{Outcome, PendingDelivery, Refusal},
    service::{Authority, Command, Retained, Selection, Status},
};
use ruzina_core::DeckId;
use ruzina_media_session::MediaSession;
use ruzina_service::analysis::{Reason, Sample, State};
use ruzina_track_analysis::owned::AnalysisIdentity;

#[derive(Clone, Copy)]
pub(crate) struct Report {
    pub serial: u64,
    pub selection: Option<Selection>,
    pub status: Option<Status>,
    pub reason: Reason,
}
pub(crate) trait Port<'a> {
    fn observe(&mut self, _sample: Sample) {}
    fn ready(&self) -> bool;
    fn media(&self) -> MediaSession;
    fn allow_io(&mut self, allowed: bool);
    fn authority(&self) -> Authority;
    fn report(&self) -> Report;
    #[allow(clippy::result_large_err)]
    fn send(&mut self, command: Command<'a>) -> Result<(), Command<'a>>;
    fn output(&mut self) -> Option<Retained<'a>>;
}
pub(crate) struct Driver<'a> {
    last: Option<(DeckId, u32)>,
    disposition: Option<(State, Reason)>,
    observed: [Option<AnalysisIdentity>; 2],
    active: Option<(Selection, DeckId, AnalysisIdentity)>,
    waiting_cancel: bool,
    pending: Option<PendingDelivery<'a>>,
    recycle: Option<Output<'a>>,
    report_serial: u64,
    recovery_ticks: u16,
    recovery_attempts: u8,
}
impl<'a> Driver<'a> {
    pub(crate) const fn new() -> Self {
        Self {
            last: None,
            disposition: None,
            observed: [None, None],
            active: None,
            waiting_cancel: false,
            pending: None,
            recycle: None,
            report_serial: 0,
            recovery_ticks: 0,
            recovery_attempts: 0,
        }
    }
    fn recycle(&mut self, port: &mut impl Port<'a>) -> bool {
        if let Some(bank) = self.recycle.take() {
            match port.send(Command::Recycle(bank)) {
                Ok(()) => {}
                Err(Command::Recycle(bank)) => {
                    self.recycle = Some(bank);
                    return false;
                }
                Err(_) => unreachable!("port must return the original refused command"),
            }
        }
        true
    }
    pub(crate) fn turn(
        &mut self,
        product: &mut ProductRuntime,
        port: &mut impl Port<'a>,
        transport_idle: bool,
    ) {
        self.drive(product, port, transport_idle);
        let report = port.report();
        let state_reason = if !port.ready() {
            if report.status == Some(Status::Failed) {
                (State::Failed, report.reason)
            } else {
                (State::Waiting, Reason::Startup)
            }
        } else if report.status == Some(Status::Quarantined) {
            (State::Quarantined, Reason::UnexpectedCompletion)
        } else if report.status == Some(Status::RecoveryBlocked) {
            (State::RecoveryBlocked, Reason::Storage)
        } else if !port.media().is_available() {
            (State::Waiting, Reason::NoMedia)
        } else if self.pending.is_some() {
            (State::Deferred, Reason::PlaybackOrTransport)
        } else if self.waiting_cancel
            || !transport_idle
            || product.pending_transport_effects() != 0
            || [DeckId::One, DeckId::Two]
                .iter()
                .any(|&d| !product.deck().deck(d).can_replace_analysis())
        {
            (State::Paused, Reason::PlaybackOrTransport)
        } else if self.recycle.is_some() {
            (State::Waiting, Reason::Recycle)
        } else if let Some((selection, _, _)) = self.active {
            if report.selection == Some(selection) && report.status == Some(Status::NoBank) {
                (State::Waiting, Reason::NoBank)
            } else if report.selection == Some(selection) && report.status == Some(Status::Started)
            {
                (State::Analyzing, Reason::None)
            } else {
                (State::Waiting, Reason::Queued)
            }
        } else {
            self.disposition.unwrap_or((State::Waiting, Reason::NoWork))
        };
        port.observe(Sample {
            state: state_reason.0,
            reason: state_reason.1,
            deck: self.last.map(|v| v.0),
            load_generation: self.last.map(|v| v.1),
        });
    }
    fn drive(
        &mut self,
        product: &mut ProductRuntime,
        port: &mut impl Port<'a>,
        transport_idle: bool,
    ) {
        let media = port.media();
        let quiet = transport_idle
            && product.pending_transport_effects() == 0
            && product.pending_library_effects() == 0
            && [DeckId::One, DeckId::Two]
                .iter()
                .all(|&deck| product.deck().deck(deck).can_replace_analysis());
        let allowed = quiet && media.is_available() && port.ready();
        port.allow_io(allowed);
        if !self.recycle(port) {
            return;
        }
        let report = port.report();
        if report.serial != self.report_serial {
            self.report_serial = report.serial;
            match report.status {
                Some(Status::Cancelled) if self.waiting_cancel => {
                    self.waiting_cancel = false;
                    self.active = None;
                }
                Some(Status::Failed | Status::Rejected)
                    if self.active.is_some_and(|a| Some(a.0) == report.selection) =>
                {
                    if !allowed && let Some((_, deck, _)) = self.active {
                        self.observed[index(deck)] = None;
                    }
                    self.disposition = Some((
                        if report.status == Some(Status::Rejected) {
                            State::Rejected
                        } else {
                            State::Failed
                        },
                        report.reason,
                    ));
                    self.active = None;
                }
                Some(Status::Started | Status::Published | Status::Cancelled) => {
                    self.recovery_attempts = 0;
                    self.recovery_ticks = 0;
                }
                _ => {}
            }
        }
        // Recovery never discards quarantined resources. Bounded retries may
        // unblock transient queue/backend errors; an actual hung backend remains visible.
        if matches!(report.status, Some(Status::RecoveryBlocked)) && self.recovery_attempts < 3 {
            self.recovery_ticks = self.recovery_ticks.saturating_add(1);
            if self.recovery_ticks >= 50 && port.send(Command::RetryRecovery).is_ok() {
                self.recovery_ticks = 0;
                self.recovery_attempts += 1;
            }
        }
        if self.pending.is_none()
            && let Some(output) = port.output()
        {
            if self
                .active
                .is_some_and(|(selection, _, _)| output.matches(selection))
            {
                self.pending = Some(
                    PendingDelivery::new(output)
                        .ok()
                        .expect("service sends unaccepted output"),
                );
            } else {
                self.recycle = Some(
                    output
                        .reject()
                        .ok()
                        .expect("service sends unaccepted output"),
                );
                if !self.recycle(port) {
                    return;
                }
            }
        }
        if let Some(pending) = self.pending.as_mut() {
            let deck = pending.target().expect("pending retains target");
            let expected = product.loaded_track(deck).and_then(|t| t.analysis_identity);
            let outcome =
                pending.try_publish(port.authority(), &media, expected, |target, publication| {
                    if !quiet {
                        return Err(Refusal::Deferred(publication));
                    }
                    product
                        .publish_background_analysis(target, publication, &media, transport_idle)
                        .map_err(|(reason, value)| match reason {
                            BackgroundAnalysisRefusal::Deferred => Refusal::Deferred(value),
                            BackgroundAnalysisRefusal::Rejected => Refusal::Rejected(value),
                        })
                });
            match outcome {
                Outcome::Deferred => return,
                Outcome::Published(bank) => {
                    self.disposition = Some((State::Published, Reason::None));
                    self.recycle = Some(bank);
                }
                Outcome::Rejected(bank) => {
                    self.disposition = Some((State::Rejected, Reason::Rejected));
                    self.recycle = Some(bank);
                }
                Outcome::BuildFailed(_) => {
                    self.disposition = Some((State::Failed, Reason::Capacity));
                    self.recycle = pending.discard();
                }
            }
            self.pending = None;
            self.active = None;
            if !self.recycle(port) {
                return;
            }
        }
        if let Some((_, deck, identity)) = self.active {
            if !allowed
                || product.loaded_track(deck).and_then(|t| t.analysis_identity) != Some(identity)
            {
                if !self.waiting_cancel && port.send(Command::Cancel).is_ok() {
                    self.observed[index(deck)] = None;
                    self.waiting_cancel = true;
                }
                return;
            }
            return;
        }
        if self.waiting_cancel || !allowed {
            return;
        }
        for deck in [DeckId::One, DeckId::Two] {
            let Some(loaded) = product.loaded_track(deck) else {
                continue;
            };
            let Some(identity) = loaded.analysis_identity else {
                continue;
            };
            if self.observed[index(deck)] == Some(identity) || product.analysis_view(deck).is_some()
            {
                continue;
            }
            // Bounded source profile is WAV only. Actual framing is validated by scan.
            let path = loaded.candidate.track.audio_path.as_str();
            if !path
                .rsplit('.')
                .next()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
            {
                self.observed[index(deck)] = Some(identity);
                continue;
            }
            let Ok(selection) =
                Selection::from_candidate(loaded.candidate, identity.load_generation)
            else {
                continue;
            };
            self.last = Some((deck, identity.load_generation));
            if port.send(Command::Analyze(selection, deck)).is_ok() {
                self.disposition = None;
                self.observed[index(deck)] = Some(identity);
                self.active = Some((selection, deck, identity));
            } else {
                self.disposition = Some((State::Waiting, Reason::QueueFull));
            }
            break;
        }
    }
}
const fn index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

/// Same scalar mapping in the concrete port and real-service host harness.
pub(crate) fn failure_reason(value: ruzina_apta_adapter::service::Failure) -> Reason {
    use ruzina_apta_adapter::service::Failure as F;
    match value {
        F::Storage => Reason::Storage,
        F::Format => Reason::Format,
        F::Capacity => Reason::Capacity,
        F::WorkLimit => Reason::WorkLimit,
        F::Contract => Reason::Contract,
        F::Stale => Reason::Stale,
        F::Analysis => Reason::Analysis,
        F::Exhausted => Reason::Exhausted,
    }
}
