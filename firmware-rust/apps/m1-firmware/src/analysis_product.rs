//! Shared real-product supervisor. One job/pending delivery; bounded recycling.
use crate::product_runtime::{BackgroundAnalysisRefusal, ProductRuntime};
use pajoniiir_apta_adapter::{
    owner::Output,
    publication::{Outcome, PendingDelivery, Refusal},
    service::{Authority, Command, Retained, Selection, Status},
};
use pajoniiir_core::DeckId;
use pajoniiir_media_session::MediaSession;
use pajoniiir_track_analysis::owned::AnalysisIdentity;

#[derive(Clone, Copy)]
pub(crate) struct Report {
    pub serial: u64,
    pub selection: Option<Selection>,
    pub status: Option<Status>,
}
pub(crate) trait Port<'a> {
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
                Outcome::Published(bank) | Outcome::Rejected(bank) => self.recycle = Some(bank),
                Outcome::BuildFailed(_) => self.recycle = pending.discard(),
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
            if port.send(Command::Analyze(selection, deck)).is_ok() {
                self.observed[index(deck)] = Some(identity);
                self.active = Some((selection, deck, identity));
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
