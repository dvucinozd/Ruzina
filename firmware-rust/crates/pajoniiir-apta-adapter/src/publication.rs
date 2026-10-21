//! Bounded pending delivery to the current owned publication model.
//! One caller-owned slot per deck; never recycles a temporarily refused result.
use crate::{
    owner::Output,
    service::{Authority, Retained},
};
use pajoniiir_core::DeckId;
use pajoniiir_media_session::MediaSession;
use pajoniiir_track_analysis::{
    WaveformColumn,
    owned::{
        AnalysisBuilder, AnalysisDataError, AnalysisIdentity, AnalysisPublication,
        MAX_PREVIEW_COLUMNS,
    },
};

pub enum Refusal {
    Deferred(AnalysisPublication),
    Rejected(AnalysisPublication),
}
pub enum Outcome<'a> {
    Published(Output<'a>),
    Deferred,
    Rejected(Output<'a>),
    /// The slot and bank remain owned; retry allocation or explicitly discard.
    BuildFailed(AnalysisDataError),
}
pub struct PendingDelivery<'a> {
    retained: Option<Retained<'a>>,
    publication: Option<AnalysisPublication>,
}
impl<'a> PendingDelivery<'a> {
    /// Accepted borrowed pins cannot be recycled by the owned-publication path.
    #[allow(clippy::result_large_err)]
    pub fn new(retained: Retained<'a>) -> Result<Self, Retained<'a>> {
        if retained.is_accepted() {
            return Err(retained);
        }
        Ok(Self {
            retained: Some(retained),
            publication: None,
        })
    }
    pub fn identity(&self) -> Option<AnalysisIdentity> {
        self.retained.as_ref()?.publication_identity()
    }
    pub fn is_pending(&self) -> bool {
        self.retained.is_some()
    }
    /// Returns the exact unaccepted bank once; dropping a built copy cannot
    /// invalidate a publication already moved into the product owner.
    pub fn discard(&mut self) -> Option<Output<'a>> {
        self.publication = None;
        self.retained.take().map(|bank| match bank.reject() {
            Ok(output) => output,
            Err(_) => unreachable!("PendingDelivery only accepts unaccepted banks"),
        })
    }
    /// Call outside short locks/audio. The product callback must check its fresh
    /// loaded identity, ANLZ priority and actual queued/in-flight transport.
    /// All authority/identity checks repeat even when an allocated copy is cached.
    pub fn try_publish(
        &mut self,
        authority: Authority,
        media: &MediaSession,
        expected: Option<AnalysisIdentity>,
        mut publish: impl FnMut(DeckId, AnalysisPublication) -> Result<(), Refusal>,
    ) -> Outcome<'a> {
        let Some(retained) = self.retained.as_ref() else {
            return Outcome::Deferred;
        };
        let Some(identity) = retained
            .publication_identity()
            .filter(|i| Some(*i) == expected)
        else {
            return Outcome::Rejected(self.discard().unwrap());
        };
        let Ok((track, waveform)) = retained.candidate(authority, media) else {
            return Outcome::Rejected(self.discard().unwrap());
        };
        if self.publication.is_none() {
            let build = || -> Result<AnalysisPublication, AnalysisDataError> {
                let mut builder = AnalysisBuilder::new();
                if let Some(grid) = track.beat_grid() {
                    builder.reserve_beats(grid.beats().len())?;
                    for &beat in grid.beats() {
                        builder.push_beat(beat)?;
                    }
                }
                let columns = waveform.columns;
                let count = columns.len().min(MAX_PREVIEW_COLUMNS);
                builder.reserve_waveform(false, count)?;
                // Whole-source peak buckets preserve coverage and fit current
                // preview capacity. No fractional detail-Hz rate is invented.
                for i in 0..count {
                    let first = (i as u128 * columns.len() as u128 / count as u128) as usize;
                    let end = ((i + 1) as u128 * columns.len() as u128 / count as u128) as usize;
                    let mut column = WaveformColumn {
                        peak: 0,
                        color_rgb565: columns[first].color_rgb565,
                    };
                    for value in &columns[first..end] {
                        column.peak = column.peak.max(value.peak);
                    }
                    builder.push_waveform(false, column)?;
                }
                builder.finish(identity, track.provider(), track.bpm_x100())
            };
            match build() {
                Ok(value) => self.publication = Some(value),
                Err(error) => return Outcome::BuildFailed(error),
            }
        }
        let target = self.retained.as_ref().unwrap().target();
        match publish(target, self.publication.take().unwrap()) {
            Ok(()) => Outcome::Published(self.discard().unwrap()),
            Err(Refusal::Deferred(value)) => {
                self.publication = Some(value);
                Outcome::Deferred
            }
            Err(Refusal::Rejected(_)) => Outcome::Rejected(self.discard().unwrap()),
        }
    }
}
