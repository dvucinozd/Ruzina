//! Persistent background analysis ownership. No provider selection or audio work.
// SPDX-License-Identifier: Apache-2.0
use crate::{
    source::{PCM_SAMPLES, READ_BYTES, SourceError, SourceRequest},
    storage::LocalTempoBuffers,
    streaming::StreamLimits,
    worker::{Completed, Requests, WorkerError},
};
use pajoniiir_media_catalog::{CatalogBank, CatalogBankSnapshot, CatalogSnapshot};
use pajoniiir_media_fs::analysis::{Client, Completion, Error, Transport};
use pajoniiir_media_session::MediaSession;
use pajoniiir_track_analysis::Beat;
use pajoniiir_waveform::{WaveformColumn, WaveformView};

/// Arrays are allocated once by the platform owner, independently of job futures.
/// Reborrowing them per job permits reuse without resetting request generations.
pub struct Workspace<'a> {
    pub local: LocalTempoBuffers<'a>,
    pub bytes: &'a mut [u8; READ_BYTES],
    pub pcm: &'a mut [f32; PCM_SAMPLES],
}

/// A free retained bank is borrowed separately from working storage. A selected
/// Pin/Delivery keeps this bank immutable while the owner runs into another bank.
pub struct Output<'a> {
    pub beats: &'a mut [Beat],
    pub waveform: &'a mut [WaveformColumn],
}
pub struct Delivery<'a> {
    pub completed: Completed<'a>,
    pub waveform: WaveformView<'a>,
}

/// Keep this owner alive across cancelled job futures. The broker owns actual
/// pending handles/buffers; dropping temporary scanner/worker logical tokens is
/// safe ONLY because the same Client is recovered before the next job.
pub struct Owner<'a, T: Transport> {
    client: Client<T>,
    requests: Requests,
    selected: Option<SourceRequest>,
    workspace: Workspace<'a>,
}
impl<'a, T: Transport> Owner<'a, T> {
    /// Move the lane's existing Requests into the owner; initialize it only once
    /// for a new endpoint, never while an earlier generation/pin remains live.
    pub fn new(client: Client<T>, requests: Requests, workspace: Workspace<'a>) -> Self {
        Self {
            client,
            requests,
            selected: None,
            workspace,
        }
    }
    /// Reserve a generation only in the receiving owner, using the copied original lease.
    pub fn select_owned(
        &mut self,
        media: &MediaSession,
        selection: crate::service::Selection,
    ) -> Result<(), WorkerError> {
        self.selected = Some(SourceRequest {
            job: self
                .requests
                .begin_source(media, selection.lease, selection.track)?,
            path: selection.path,
        });
        Ok(())
    }
    pub fn requests(&self) -> &Requests {
        &self.requests
    }
    pub fn is_idle(&self) -> bool {
        self.client.is_idle()
    }
    pub fn select<const N: usize>(
        &mut self,
        media: &MediaSession,
        catalog: &CatalogSnapshot<N>,
        index: usize,
    ) -> Result<(), WorkerError> {
        self.selected = Some(SourceRequest::new(
            &mut self.requests,
            media,
            catalog,
            index,
        )?);
        Ok(())
    }
    pub fn select_bank<B: CatalogBank>(
        &mut self,
        media: &MediaSession,
        catalog: &CatalogBankSnapshot<B>,
        index: usize,
    ) -> Result<(), WorkerError> {
        self.selected = Some(SourceRequest::from_bank(
            &mut self.requests,
            media,
            catalog,
            index,
        )?);
        Ok(())
    }
    /// Call after dropping the borrowed run future. Selected Deck pins survive.
    pub fn cancel(&mut self) {
        self.selected = None;
        self.requests.cancel();
    }
    /// Cancellation/error cleanup can itself be cancelled and retried. Unexpected
    /// resources stay quarantined until explicitly transferred to the broker owner.
    pub async fn recover(&mut self) -> Result<(), Error<T::Error>> {
        self.client.recover().await
    }
    pub fn take_unexpected(&mut self) -> Option<Completion<T::Handle, T::Error, T::Unexpected>> {
        self.client.take_unexpected()
    }
    /// Idle shutdown only; never reissues the endpoint or resets generations.
    pub fn retire_buffer(&mut self) -> Option<&'static mut [u8]> {
        self.client.retire_buffer()
    }

    /// Runs the latest catalog selection. Recovery precedes consuming it, so a
    /// cancelled/failed recovery preserves a queued replacement. Once started,
    /// cancellation consumes that selection; explicitly select again to retry.
    /// All output is withheld until both hashes and both closes succeed.
    pub async fn run<'b>(
        &mut self,
        media: &MediaSession,
        limits: StreamLimits,
        frames_per_column: u32,
        output: Output<'b>,
        color: u16,
    ) -> Result<Delivery<'b>, SourceError<Error<T::Error>>> {
        self.client.recover().await.map_err(SourceError::Io)?;
        let request = self.selected.take().ok_or(SourceError::Failed)?;
        self.client
            .bind(request.job.lease())
            .map_err(SourceError::Io)?;
        let result = async {
            let verified = request
                .scan(
                    &mut None,
                    &mut self.client,
                    &mut self.requests,
                    media,
                    limits,
                    self.workspace.bytes,
                )
                .await?;
            let mut stream = None;
            verified
                .prepare_local_tempo(
                    &mut stream,
                    LocalTempoBuffers {
                        queue: self.workspace.local.queue,
                        columns: self.workspace.local.columns,
                        bins: self.workspace.local.bins,
                        flux: self.workspace.local.flux,
                    },
                    frames_per_column,
                    self.workspace.bytes,
                )
                .map_err(SourceError::Worker)?;
            let stream = stream.as_mut().unwrap();
            stream
                .run(&mut self.client, &self.requests, media, self.workspace.pcm)
                .await?;
            let worker = stream.verified_worker().map_err(SourceError::Worker)?;
            let waveform = worker
                .waveform(&self.requests, media, output.waveform, color)
                .map_err(SourceError::Worker)?;
            let completed = worker
                .complete(&self.requests, media, output.beats)
                .map_err(SourceError::Worker)?;
            Ok(Delivery {
                completed,
                waveform,
            })
        }
        .await;
        if result.is_err() {
            self.requests.cancel();
            // Cleanup failure takes precedence and remains explicitly recoverable.
            self.client.recover().await.map_err(SourceError::Io)?;
        }
        result
    }
}
