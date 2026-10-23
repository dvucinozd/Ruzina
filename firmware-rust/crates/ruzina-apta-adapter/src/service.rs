//! Bounded long-lived owner scheduling and movable retained banks.
// SPDX-License-Identifier: Apache-2.0
use crate::{
    Analysis,
    owner::{Delivery, Output, Owner},
    streaming::StreamLimits,
    worker::{Completed, Job, Requests, WorkerError, check_authority},
};
use embassy_futures::select::{Either, select};
use ruzina_core::{DeckId, MediaTrackId};
use ruzina_deck::DeckProductState;
use ruzina_media_catalog::{
    CATALOG_PATH_MAX, CatalogBank, CatalogBankSnapshot, CatalogSnapshot, CatalogText, CatalogTrack,
};
use ruzina_media_fs::analysis::Transport;
use ruzina_media_session::{MediaLease, MediaSession};
use ruzina_track_analysis::{BeatGrid, TrackAnalysis};
use ruzina_waveform::WaveformView;

/// Copy at the catalog owner. No Requests access or catalog borrow crosses I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub(crate) lease: MediaLease,
    revision: u32,
    catalog_revision: u32,
    pub(crate) track: MediaTrackId,
    pub(crate) path: CatalogText<CATALOG_PATH_MAX>,
}
impl Selection {
    /// Product intent serial, independent of the service's Requests generations.
    /// Prevents a queued completion attaching after an A -> B -> A selection.
    pub fn with_revision(mut self, revision: u32) -> Self {
        self.revision = revision;
        self
    }
    pub fn lease(self) -> MediaLease {
        self.lease
    }
    pub fn track_id(self) -> MediaTrackId {
        self.track
    }
    pub fn new<const N: usize>(
        catalog: &CatalogSnapshot<N>,
        index: usize,
    ) -> Result<Self, WorkerError> {
        Self::track(catalog.lease(), catalog.revision(), catalog.get(index))
    }
    pub fn from_bank<B: CatalogBank>(
        catalog: &CatalogBankSnapshot<B>,
        index: usize,
    ) -> Result<Self, WorkerError> {
        Self::track(catalog.lease(), catalog.revision(), catalog.get(index))
    }
    /// Copy an already committed Library candidate; bind it to that exact load.
    pub fn from_candidate(
        candidate: ruzina_media_catalog::CatalogLoadCandidate,
        load_generation: u32,
    ) -> Result<Self, WorkerError> {
        if load_generation == 0 {
            return Err(WorkerError::Failed);
        }
        Self::track(
            candidate.publication.lease,
            candidate.publication.revision,
            Some(&candidate.track),
        )
        .map(|value| value.with_revision(load_generation))
    }
    fn track(
        lease: MediaLease,
        catalog_revision: u32,
        track: Option<&CatalogTrack>,
    ) -> Result<Self, WorkerError> {
        let track = track.ok_or(WorkerError::Failed)?;
        if track.audio_path.is_empty() {
            return Err(WorkerError::Failed);
        }
        Ok(Self {
            lease,
            revision: 0,
            catalog_revision,
            track: track.track_id,
            path: track.audio_path,
        })
    }
}

/// Read-only delivery authority. Cannot issue/reset request generations.
#[derive(Clone, Copy, Default)]
pub struct Authority(Option<Job>);
impl Authority {
    pub const EMPTY: Self = Self(None);
    fn new(requests: &Requests) -> Self {
        Self(requests.current)
    }
}

/// Owns the bank and scalar metadata, never references its own fields. Short
/// borrowed neutral views prevent moving/returning the bank while a view is used.
/// ```compile_fail
/// use ruzina_apta_adapter::service::Retained;
/// fn recycle(bank: Retained<'_>, media: &ruzina_media_session::MediaSession,
///            decks: &ruzina_deck::DeckProductState) {
///     let view = bank.track(media);
///     let _free = bank.retire(decks);
///     let _still_used = view.unwrap().generation();
/// }
/// ```
pub struct Retained<'a> {
    output: Output<'a>,
    completed: Completed<'static>,
    beats: Option<usize>,
    columns: usize,
    target: DeckId,
    selection: (MediaLease, MediaTrackId, u32, u32),
    accepted: bool,
}
impl<'a> Retained<'a> {
    /// A stopped product may bind `revision` to its nonzero load generation.
    #[cfg(feature = "owned-handoff")]
    pub fn publication_identity(&self) -> Option<ruzina_track_analysis::owned::AnalysisIdentity> {
        (self.selection.2 != 0).then_some(ruzina_track_analysis::owned::AnalysisIdentity {
            lease: self.selection.0,
            track: self.selection.1,
            load_generation: self.selection.2,
            catalog_revision: self.selection.3,
        })
    }

    #[cfg(feature = "owned-handoff")]
    pub(crate) fn candidate(
        &self,
        authority: Authority,
        media: &MediaSession,
    ) -> Result<(TrackAnalysis<'_>, WaveformView<'_>), WorkerError> {
        if self.accepted
            || authority.0 != Some(self.completed.job)
            || !media.is_available()
            || !media.validate(self.completed.job.lease())
        {
            return Err(WorkerError::Stale);
        }
        Ok((
            self.completed().analysis.track(),
            WaveformView {
                columns: &self.output.waveform[..self.columns],
            },
        ))
    }

    #[cfg(feature = "owned-handoff")]
    pub(crate) fn is_accepted(&self) -> bool {
        self.accepted
    }

    fn completed(&self) -> Completed<'_> {
        let mut value = self.completed;
        let t = value.analysis.track;
        value.analysis.track = TrackAnalysis::new(
            t.provider(),
            t.generation(),
            t.bpm_x100(),
            self.beats.map(|n| BeatGrid::new(&self.output.beats[..n])),
        );
        value
    }
    /// Call at the actual product boundary with fresh media and actual Deck state.
    /// Queued delivery is invalidated by any later analyze/cancel command.
    pub fn accept(
        &mut self,
        authority: Authority,
        media: &MediaSession,
        decks: &DeckProductState,
    ) -> Result<(), WorkerError> {
        if authority.0 != Some(self.completed.job) {
            return Err(WorkerError::Stale);
        }
        check_authority(self.target, decks, self.completed(), authority.0, media)?;
        self.accepted = true;
        Ok(())
    }
    pub fn matches(&self, selection: Selection) -> bool {
        self.selection
            == (
                selection.lease,
                selection.track,
                selection.revision,
                selection.catalog_revision,
            )
    }
    pub fn track_id(&self) -> MediaTrackId {
        self.selection.1
    }
    pub fn target(&self) -> DeckId {
        self.target
    }
    pub fn track(&self, media: &MediaSession) -> Option<TrackAnalysis<'_>> {
        (self.accepted && media.is_available() && media.validate(self.completed.job.lease()))
            .then(|| self.completed().analysis.track())
    }
    pub fn waveform(&self, media: &MediaSession) -> Option<WaveformView<'_>> {
        self.track(media).map(|_| WaveformView {
            columns: &self.output.waveform[..self.columns],
        })
    }
    /// Return an unaccepted delivery after queue refusal. An accepted Deck bank
    /// can only be returned by retire with actual Deck state.
    #[allow(clippy::result_large_err)]
    pub fn reject(self) -> Result<Output<'a>, Self> {
        if self.accepted {
            Err(self)
        } else {
            Ok(self.output)
        }
    }
    /// Explicit retirement only. Pending transport and playing both prevent it,
    /// even on stale media. Rejected/unselected deliveries can return immediately.
    #[allow(clippy::result_large_err)]
    pub fn retire(self, decks: &DeckProductState) -> Result<Output<'a>, Self> {
        if self.accepted && !decks.deck(self.target).can_replace_analysis() {
            return Err(self);
        }
        Ok(self.output)
    }
}

#[allow(clippy::large_enum_variant)]
pub enum Command<'a> {
    /// Latest *received* Analyze replaces active/queued work. Full enqueue returns
    /// ownership to the sender; it must not silently displace another command.
    Analyze(Selection, DeckId),
    Cancel,
    Recycle(Output<'a>),
    RetryRecovery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Cancelled,
    Started,
    Published,
    NoBank,
    Rejected,
    Failed,
    RecoveryBlocked,
    /// Unexpected completion remains owned by the Client; explicit investigation required.
    Quarantined,
}

/// Scalar diagnostic category; never transports backend-owned resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Storage,
    Format,
    Capacity,
    WorkLimit,
    Contract,
    Stale,
    Analysis,
    Exhausted,
}
impl Failure {
    fn source<E>(error: &crate::source::SourceError<ruzina_media_fs::analysis::Error<E>>) -> Self {
        use crate::source::SourceError as S;
        use ruzina_media_fs::analysis::Error as E;
        match error {
            S::Capacity => Self::Capacity,
            S::Worker(WorkerError::Analysis(crate::Error::Capacity))
            | S::Worker(WorkerError::Core(
                libapta::Error::BufferTooSmall | libapta::Error::LimitExceeded,
            )) => Self::Capacity,
            S::Worker(WorkerError::Analysis(crate::Error::Generation)) => Self::Exhausted,
            S::Limit => Self::WorkLimit,
            S::Contract => Self::Contract,
            S::Decode(_) => Self::Format,
            S::Io(E::Stale) | S::Worker(WorkerError::Stale) => Self::Stale,
            S::Io(E::Exhausted) => Self::Exhausted,
            S::Io(E::Contract | E::Unexpected) => Self::Contract,
            S::Io(_) => Self::Storage,
            _ => Self::Analysis,
        }
    }
}

/// Platform uses its existing bounded channel/short-lock snapshot mechanisms.
/// report is a bounded status snapshot, not an unbounded event log. publish must
/// return ownership on refusal; authority updates and consumer acceptance must
/// be serialized by the same product-owner/short-lock boundary.
#[allow(async_fn_in_trait)]
pub trait Port<'a> {
    async fn receive(&mut self) -> Command<'a>;
    fn media(&self) -> MediaSession;
    fn authority(&mut self, authority: Authority);
    #[allow(clippy::result_large_err)]
    fn publish(&mut self, output: Retained<'a>) -> Result<(), Output<'a>>;
    fn report(&mut self, status: Status);
    fn failure(&mut self, _reason: Failure) {
        self.report(Status::Failed);
    }
}

/// Client, generations, scratch borrows and free banks outlive every job future.
/// Cancelling run itself also preserves these resources; invoke it again to
/// recover. The endpoint/quarantine remains accessible through owner_mut.
pub struct Service<'a, T: Transport> {
    owner: Owner<'a, T>,
    free: [Option<Output<'a>>; 2],
    selected: Option<(Selection, DeckId)>,
}
impl<'a, T: Transport> Service<'a, T> {
    pub fn new(owner: Owner<'a, T>, banks: [Output<'a>; 2]) -> Self {
        let [a, b] = banks;
        Self {
            owner,
            free: [Some(a), Some(b)],
            selected: None,
        }
    }
    pub fn owner_mut(&mut self) -> &mut Owner<'a, T> {
        &mut self.owner
    }
    fn command(&mut self, command: Command<'a>, port: &mut impl Port<'a>) {
        match command {
            Command::Analyze(selection, deck) => {
                self.owner.cancel();
                self.selected = Some((selection, deck));
            }
            Command::Cancel => {
                self.owner.cancel();
                self.selected = None;
                port.report(Status::Cancelled);
            }
            Command::Recycle(bank) => {
                // Only transferred banks return through this lane. A full free
                // pool is a caller contract error; never overwrite a live bank.
                if let Some(slot) = self.free.iter_mut().find(|b| b.is_none()) {
                    *slot = Some(bank);
                } else {
                    port.report(Status::Rejected);
                }
            }
            Command::RetryRecovery => {}
        }
        port.authority(Authority::new(self.owner.requests()));
    }
    pub async fn run(
        &mut self,
        port: &mut impl Port<'a>,
        limits: StreamLimits,
        frames_per_column: u32,
        color: u16,
    ) -> ! {
        self.owner.cancel();
        port.authority(Authority::new(self.owner.requests()));
        loop {
            // Always drain/close before new work; commands may interrupt recovery
            // but cannot replace the persistent Client or reclaim its buffer.
            match select(port.receive(), self.owner.recover()).await {
                Either::First(command) => {
                    self.command(command, port);
                    continue;
                }
                Either::Second(Err(error)) => {
                    port.report(
                        if matches!(error, ruzina_media_fs::analysis::Error::Unexpected) {
                            Status::Quarantined
                        } else {
                            Status::RecoveryBlocked
                        },
                    );
                    let command = port.receive().await;
                    self.command(command, port);
                    continue;
                }
                Either::Second(Ok(())) => {}
            }
            let Some((selection, deck)) = self.selected.take() else {
                let command = port.receive().await;
                self.command(command, port);
                continue;
            };
            let Some(index) = self.free.iter().position(Option::is_some) else {
                self.selected = Some((selection, deck));
                port.report(Status::NoBank);
                // Keep the latest selection, then sleep on the command lane.
                // A Recycle automatically retries; Cancel/replacement supersede it.
                let command = port.receive().await;
                self.command(command, port);
                continue;
            };
            let media = port.media();
            if self.owner.select_owned(&media, selection).is_err() {
                port.report(Status::Rejected);
                continue;
            }
            port.authority(Authority::new(self.owner.requests()));
            port.report(Status::Started);
            let bank = self.free[index].as_mut().unwrap();
            let result = select(
                port.receive(),
                self.owner.run(
                    &media,
                    limits,
                    frames_per_column,
                    Output {
                        beats: bank.beats,
                        waveform: bank.waveform,
                    },
                    color,
                ),
            )
            .await;
            match result {
                Either::First(command) => {
                    // A recycle command must not cancel useful work. It does
                    // interrupt this job, so explicitly queue the same selection
                    // for retry under a fresh generation, after recovery.
                    self.owner.cancel();
                    self.selected = Some((selection, deck));
                    self.command(command, port);
                }
                Either::Second(Err(error)) => {
                    self.owner.cancel();
                    port.authority(Authority::new(self.owner.requests()));
                    port.failure(Failure::source(&error));
                }
                Either::Second(Ok(Delivery {
                    completed,
                    waveform,
                })) => {
                    let track = completed.analysis.track();
                    let beats = track.beat_grid().map(|g| g.beats().len());
                    let metadata = Completed {
                        job: completed.job,
                        analysis: Analysis {
                            source: completed.analysis.source,
                            lineage: completed.analysis.lineage,
                            track: TrackAnalysis::new(
                                track.provider(),
                                track.generation(),
                                track.bpm_x100(),
                                None,
                            ),
                        },
                    };
                    let columns = waveform.columns.len();
                    // Verify source at handoff; consumer rechecks fresh state at
                    // acceptance, including a newer command's authority.
                    if !port.media().validate(completed.job.lease()) || !port.media().is_available()
                    {
                        self.owner.cancel();
                        port.authority(Authority::new(self.owner.requests()));
                        port.report(Status::Rejected);
                        continue;
                    }
                    port.authority(Authority::new(self.owner.requests()));
                    let retained = Retained {
                        output: self.free[index].take().unwrap(),
                        completed: metadata,
                        beats,
                        columns,
                        target: deck,
                        selection: (
                            selection.lease,
                            selection.track,
                            selection.revision,
                            selection.catalog_revision,
                        ),
                        accepted: false,
                    };
                    match port.publish(retained) {
                        Ok(()) => port.report(Status::Published),
                        Err(bank) => {
                            self.free[index] = Some(bank);
                            port.report(Status::Rejected);
                        }
                    }
                }
            }
        }
    }
}
