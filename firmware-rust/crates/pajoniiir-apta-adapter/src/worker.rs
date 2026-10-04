//! Experimental media worker, called outside the real-time audio callback.
//! Caller owns PCM acquisition, storage placement and two independent beat banks.
use crate::{Analysis, Error, convert};
use libapta::session::{CancellationToken, Session, SessionState, SourceIdentity, WorkBudget};
use pajoniiir_core::{DeckId, MediaTrackId};
use pajoniiir_deck::DeckProductState;
use pajoniiir_media_session::{MediaLease, MediaSession};
use pajoniiir_track_analysis::{Beat, TrackAnalysis};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Job {
    lease: MediaLease,
    track: MediaTrackId,
    generation: u32,
    identity: SourceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Stale,
    Incomplete,
    Failed,
    Analysis(Error),
    Core(libapta::Error),
}

/// One analysis lane. Each request supersedes prior in-flight work, even for the
/// same track. Cancellation invalidates delivery. Generation exhaustion is fatal
/// to new requests rather than wrapping into a retained generation. Keep this
/// single lane owner alive while any jobs/completions/pins exist; do not reset it
/// or mix jobs from independently created request owners.
#[derive(Default)]
pub struct Requests {
    next: u32,
    current: Option<Job>,
}
impl Requests {
    /// Use the lease carried by the originating catalog/media request, not a
    /// freshly read replacement lease for an old catalog entry.
    pub fn request(
        &mut self,
        media: &MediaSession,
        lease: MediaLease,
        track: MediaTrackId,
        identity: SourceIdentity,
    ) -> Result<Job, WorkerError> {
        if identity.kind() == 0 {
            return Err(WorkerError::Stale);
        }
        self.issue(media, lease, track, identity)
    }
    fn issue(
        &mut self,
        media: &MediaSession,
        lease: MediaLease,
        track: MediaTrackId,
        identity: SourceIdentity,
    ) -> Result<Job, WorkerError> {
        if !media.is_available() || !media.validate(lease) {
            return Err(WorkerError::Stale);
        }
        let generation = self
            .next
            .checked_add(1)
            .ok_or(WorkerError::Analysis(Error::Generation))?;
        let job = Job {
            lease,
            track,
            generation,
            identity,
        };
        self.next = generation;
        self.current = Some(job);
        Ok(job)
    }
    pub(crate) fn begin_source(
        &mut self,
        media: &MediaSession,
        lease: MediaLease,
        track: MediaTrackId,
    ) -> Result<Job, WorkerError> {
        self.issue(media, lease, track, SourceIdentity::default())
    }
    pub(crate) fn verify_source(
        &mut self,
        job: Job,
        media: &MediaSession,
        identity: SourceIdentity,
    ) -> Result<Job, WorkerError> {
        if !self.accepts(job, media) {
            return Err(WorkerError::Stale);
        }
        let job = Job { identity, ..job };
        self.current = Some(job);
        Ok(job)
    }
    pub fn cancel(&mut self) {
        self.current = None;
    }
    pub(crate) fn accepts(&self, job: Job, media: &MediaSession) -> bool {
        self.current == Some(job) && media.is_available() && media.validate(job.lease)
    }
}

/// Owns an already-configured portable session borrowing caller storage. Only
/// a Created session is accepted (setting identity enforces this). No allocation,
/// desktop runtime, filesystem or audio callback is introduced.
pub struct Worker<'a> {
    session: Session<'a>,
    job: Job,
    failed: bool,
}
impl<'a> Worker<'a> {
    pub fn new(mut session: Session<'a>, job: Job) -> Result<Self, WorkerError> {
        if job.identity.kind() == 0 {
            return Err(WorkerError::Failed);
        }
        session
            .set_source_identity(job.identity)
            .map_err(WorkerError::Core)?;
        Ok(Self {
            session,
            job,
            failed: false,
        })
    }
    pub(crate) fn check(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<(), WorkerError> {
        if self.failed {
            return Err(WorkerError::Failed);
        }
        if !requests.accepts(self.job, media) {
            self.failed = true;
            return Err(WorkerError::Stale);
        }
        Ok(())
    }
    /// Report failed reads/decoding; partial PCM can never become a completion.
    pub fn source_failed(&mut self) {
        self.failed = true;
    }
    /// Return accepted frames; caller must retain/retry the unaccepted suffix.
    pub fn push(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        pcm: &[f32],
    ) -> Result<usize, WorkerError> {
        self.check(requests, media)?;
        self.session
            .push_interleaved(pcm)
            .map_err(WorkerError::Core)
    }
    /// One cooperative core step, at most 256 input frames. Not a timing bound.
    pub fn step(&mut self, requests: &Requests, media: &MediaSession) -> Result<bool, WorkerError> {
        self.check(requests, media)?;
        if let Err(e) = self.session.process(
            WorkBudget {
                maximum_input_frames: 256,
                maximum_steps: 1,
            },
            &CancellationToken::new(),
        ) {
            self.failed = true;
            return Err(WorkerError::Core(e));
        }
        Ok(self.session.state() == SessionState::Complete)
    }
    pub fn finish_input(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<(), WorkerError> {
        self.check(requests, media)?;
        self.session.finish_input().map_err(WorkerError::Core)
    }
    /// Convert a final overview into an independent caller waveform bank.
    pub fn waveform<'b>(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        bank: &'b mut [pajoniiir_waveform::WaveformColumn],
        color: u16,
    ) -> Result<pajoniiir_waveform::WaveformView<'b>, WorkerError> {
        self.check(requests, media)?;
        if self.session.state() != SessionState::Complete {
            return Err(WorkerError::Incomplete);
        }
        let config = self.session.config();
        let spans = [libapta::WaveformSpan {
            first_frame: 0,
            end_frame: config.total_frames,
            first_column_index: 0,
            column_count: u32::try_from(self.session.columns().len())
                .map_err(|_| WorkerError::Analysis(Error::Capacity))?,
            data_column_offset: 0,
        }];
        crate::waveform(
            libapta::NativeOverview {
                frames_per_column: config.frames_per_column,
                origin_frame: 0,
                state: libapta::FeatureState::Final,
                confidence: 255,
                spans: &spans,
                columns: self.session.columns(),
            },
            bank,
            color,
        )
        .map_err(WorkerError::Analysis)
    }

    /// Completed data borrows only the conversion bank. Capacity errors can be
    /// retried with a larger bank; no partial result is published.
    pub fn complete<'b>(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        bank: &'b mut [Beat],
    ) -> Result<Completed<'b>, WorkerError> {
        self.check(requests, media)?;
        if self.session.state() != SessionState::Complete {
            return Err(WorkerError::Incomplete);
        }
        // Borrow only the data this consumer uses. The full core snapshot also
        // contains inline detail scratch and is deliberately unnecessary here.
        let local = self.session.local_grid();
        let local_coverage = local.map(|g| [g.coverage]);
        let local_segments = local.map(|g| [g.segment]);
        let local_grid = local.map(|g| libapta::NativeGrid {
            state: g.state,
            confidence: g.confidence,
            flags: g.flags,
            representation: libapta::GridRepresentation::Segments,
            requested_range: g.requested_range,
            evidence_range: g.evidence_range,
            applicability_range: g.applicability_range,
            coverage_ranges: local_coverage.as_ref().unwrap(),
            segments: local_segments.as_ref().unwrap(),
            beats: &[],
        });
        let global = self.session.global_grid();
        let global_coverage = global.map(|g| [g.coverage_range]);
        let global_grid = global.map(|g| libapta::NativeGrid {
            state: g.state,
            confidence: g.confidence,
            flags: g.flags,
            representation: g.representation,
            requested_range: g.requested_range,
            evidence_range: g.evidence_range,
            applicability_range: g.applicability_range,
            coverage_ranges: global_coverage.as_ref().unwrap(),
            segments: g.segments,
            beats: g.beats,
        });
        let config = self.session.config();
        let input = libapta::NativeResultInput {
            source: libapta::SourceInfo {
                sample_rate: config.sample_rate,
                channel_count: config.channel_count,
                channel_layout: config.channel_count,
                total_frames: Some(config.total_frames),
                fingerprint_kind: self.job.identity.kind(),
                fingerprint: self.job.identity.bytes(),
            },
            info: libapta::NativeResultInfo {
                generation: self.job.generation as u64,
                ..Default::default()
            },
            provenance: libapta::Provenance {
                origin: libapta::ProvenanceOrigin::NativeAnalysis,
                source_name: "libapta",
                source_version: "",
            },
            tempo: self.session.tempo(),
            local_grid,
            global_grid,
            revision: self.session.grid_revision(),
            meter: self.session.meter(),
            overview: None,
            detail: None,
            metadata: None,
            key: None,
            quality: &[],
        };
        let analysis = convert(&input, bank).map_err(WorkerError::Analysis)?;
        Ok(Completed {
            job: self.job,
            analysis,
        })
    }
}

/// Delivery is still checked against the current request at the deck boundary.
#[derive(Clone, Copy, Debug)]
pub struct Completed<'a> {
    job: Job,
    analysis: Analysis<'a>,
}

/// Retains one selected bank. Selection reads actual Deck state rather than a
/// caller-supplied playing boolean. New track loads use a new pin at a stopped
/// boundary; an existing pin can only upgrade the same media/track/source.
pub struct Pin<'a> {
    deck: DeckId,
    selected: Completed<'a>,
}
impl<'a> Pin<'a> {
    pub fn load(
        deck: DeckId,
        decks: &DeckProductState,
        next: Completed<'a>,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<Self, WorkerError> {
        check_delivery(deck, decks, next, requests, media)?;
        Ok(Self {
            deck,
            selected: next,
        })
    }
    pub fn upgrade(
        &mut self,
        decks: &DeckProductState,
        next: Completed<'a>,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<(), WorkerError> {
        check_delivery(self.deck, decks, next, requests, media)?;
        if self.selected.job.lease != next.job.lease || self.selected.job.track != next.job.track {
            return Err(WorkerError::Analysis(Error::Identity));
        }
        self.selected
            .analysis
            .upgrade(next.analysis, decks.deck(self.deck).playing)
            .map_err(WorkerError::Analysis)?;
        self.selected.job = next.job;
        Ok(())
    }
    /// Disconnect/unmount immediately hides retained analysis, including after
    /// reinsertion. A replacement request does not disturb a playing deck's pin.
    pub fn track(&self, media: &MediaSession) -> Option<TrackAnalysis<'a>> {
        (media.is_available() && media.validate(self.selected.job.lease))
            .then(|| self.selected.analysis.track())
    }
    pub fn track_id(&self) -> MediaTrackId {
        self.selected.job.track
    }
}
fn check_delivery(
    deck: DeckId,
    decks: &DeckProductState,
    next: Completed<'_>,
    requests: &Requests,
    media: &MediaSession,
) -> Result<(), WorkerError> {
    if !requests.accepts(next.job, media) {
        return Err(WorkerError::Stale);
    }
    if !decks.deck(deck).can_replace_analysis() {
        return Err(WorkerError::Analysis(Error::Playing));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_media_session::{MediaHandle, MediaSourceId};
    #[test]
    fn generation_exhaustion_preserves_last_request_without_wrapping() {
        let mut media = MediaSession::new();
        media.on_connect(MediaSourceId::new(1).unwrap());
        let lease = media.lease().unwrap();
        media.bind_handle(lease, MediaHandle::new(1).unwrap());
        media.commit_mounted(lease);
        let mut requests = Requests {
            next: u32::MAX - 1,
            current: None,
        };
        let job = requests
            .begin_source(&media, lease, MediaTrackId([1; 16]))
            .unwrap();
        assert_eq!(job.generation, u32::MAX);
        assert_eq!(
            requests.begin_source(&media, lease, MediaTrackId([2; 16])),
            Err(WorkerError::Analysis(Error::Generation))
        );
        assert!(requests.accepts(job, &media));
        requests.cancel();
        assert!(
            requests
                .begin_source(&media, lease, MediaTrackId([1; 16]))
                .is_err()
        );
    }
}
