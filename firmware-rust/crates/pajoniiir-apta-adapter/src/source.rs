//! Bounded experimental catalog WAV acquisition over the native filesystem contract.
//! Whole-object caller storage is intentional: the existing portable WAV decoder
//! borrows it. This is not streaming codec or production Embassy integration.
use crate::worker::{Job, Requests, Worker, WorkerError};
use libapta::{
    session::{Session, SourceIdentity},
    wav::Wav,
};
use pajoniiir_media_catalog::{
    CATALOG_PATH_MAX, CatalogBank, CatalogBankSnapshot, CatalogSnapshot, CatalogText, CatalogTrack,
};
use pajoniiir_media_fs::{AsyncFileSystem, FileHandle, LeaseBound};
use pajoniiir_media_identity::Sha256;
use pajoniiir_media_session::{MediaLease, MediaSession};

pub const READ_BYTES: usize = 4096;
pub const PCM_SAMPLES: usize = 512;
/// Object budgets only; source bytes, caller PCM/core/conversion banks, backend
/// handles/futures and nested stack are additional and require separate placement.
pub const REQUEST_OBJECT_BUDGET: usize = 384;
pub const WAV_WORKER_OBJECT_BUDGET: usize = 4096;
const _: () = assert!(core::mem::size_of::<SourceRequest>() <= REQUEST_OBJECT_BUDGET);
const _: () = assert!(core::mem::size_of::<WavWorker<'_, '_>>() <= WAV_WORKER_OBJECT_BUDGET);

#[derive(Debug)]
pub enum SourceError<E> {
    Worker(WorkerError),
    Io(E),
    Capacity,
    Contract,
    Failed,
    Decode(libapta::Error),
}

/// Reserves one lane generation before opening the exact catalog path. The
/// original catalog lease, never a refreshed lease, accompanies that request.
pub struct SourceRequest {
    job: Job,
    path: CatalogText<CATALOG_PATH_MAX>,
}
impl SourceRequest {
    pub fn new<const N: usize>(
        requests: &mut Requests,
        media: &MediaSession,
        catalog: &CatalogSnapshot<N>,
        index: usize,
    ) -> Result<Self, WorkerError> {
        Self::from_track(requests, media, catalog.lease(), catalog.get(index))
    }
    /// Select from the storage-backed catalog used by the foundation Library
    /// owner without copying its bank or retaining a mutable catalog reference.
    pub fn from_bank<B: CatalogBank>(
        requests: &mut Requests,
        media: &MediaSession,
        catalog: &CatalogBankSnapshot<B>,
        index: usize,
    ) -> Result<Self, WorkerError> {
        Self::from_track(requests, media, catalog.lease(), catalog.get(index))
    }
    fn from_track(
        requests: &mut Requests,
        media: &MediaSession,
        lease: MediaLease,
        track: Option<&CatalogTrack>,
    ) -> Result<Self, WorkerError> {
        let track = track.ok_or(WorkerError::Failed)?;
        if track.audio_path.is_empty() {
            return Err(WorkerError::Failed);
        }
        Ok(Self {
            job: requests.begin_source(media, lease, track.track_id)?,
            path: track.audio_path,
        })
    }
    /// Drive open to completion. As with the filesystem trait, cancellation of
    /// open/close itself requires backend-owned handle recovery. Read cancellation
    /// is handled below and cannot authorize analysis or silently resume hashing.
    pub async fn open<'a, F: AsyncFileSystem>(
        self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        storage: &'a mut [u8],
    ) -> Result<SourceRead<'a, F::File>, SourceError<F::Error>> {
        if !requests.accepts(self.job, media) || Some(fs.lease()) != media.lease() {
            return Err(SourceError::Worker(WorkerError::Stale));
        }
        let file = fs
            .open_read(self.path.as_str())
            .await
            .map_err(SourceError::Io)?;
        let valid = file.lease() == fs.lease() && file.position() == 0;
        let len = usize::try_from(file.len())
            .ok()
            .filter(|&n| n != 0 && n <= storage.len());
        if !valid || len.is_none() {
            fs.close_file(file).await.map_err(SourceError::Io)?;
            return Err(if valid {
                SourceError::Capacity
            } else {
                SourceError::Contract
            });
        }
        Ok(SourceRead {
            job: self.job,
            file,
            storage: &mut storage[..len.unwrap()],
            offset: 0,
            hash: Sha256::new(),
            failed: false,
        })
    }
}

/// Owns the open file until finish/abort. Always call one of them, including after
/// an error or cancelled read. No async cleanup is hidden in Drop.
pub struct SourceRead<'a, H> {
    job: Job,
    file: H,
    storage: &'a mut [u8],
    offset: usize,
    hash: Sha256,
    failed: bool,
}
impl<'a, H: FileHandle> SourceRead<'a, H> {
    /// One filesystem read of at most 4096 bytes and hashing only returned bytes.
    /// Backend latency/work is separate from this byte bound. A dropped pending
    /// future poisons the reader before I/O, so uncertain progress cannot be retried.
    pub async fn step<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<bool, SourceError<F::Error>> {
        if self.failed {
            return Err(SourceError::Failed);
        }
        self.failed = true;
        if !requests.accepts(self.job, media) || fs.lease() != self.file.lease() {
            return Err(SourceError::Worker(WorkerError::Stale));
        }
        if self.file.position() != self.offset as u64
            || self.file.len() != self.storage.len() as u64
        {
            return Err(SourceError::Contract);
        }
        if self.offset < self.storage.len() {
            let end = self.offset + READ_BYTES.min(self.storage.len() - self.offset);
            let n = fs
                .read(&mut self.file, &mut self.storage[self.offset..end])
                .await
                .map_err(SourceError::Io)?;
            if n == 0
                || n > end - self.offset
                || self.file.position() != (self.offset + n) as u64
                || self.file.len() != self.storage.len() as u64
                || self.file.lease() != fs.lease()
            {
                return Err(SourceError::Contract);
            }
            self.hash
                .update(&self.storage[self.offset..self.offset + n]);
            self.offset += n;
        }
        self.failed = false;
        Ok(self.offset == self.storage.len())
    }
    pub async fn abort<F: AsyncFileSystem<File = H>>(
        self,
        fs: &mut F,
    ) -> Result<(), SourceError<F::Error>> {
        fs.close_file(self.file).await.map_err(SourceError::Io)
    }
    /// Close success, full byte coverage and WAV validation precede verified job
    /// publication. Hash includes headers, ancillary chunks and trailing bytes.
    pub async fn finish<F: AsyncFileSystem<File = H>>(
        self,
        fs: &mut F,
        requests: &mut Requests,
        media: &MediaSession,
    ) -> Result<VerifiedWav<'a>, SourceError<F::Error>> {
        let lease_matches = fs.lease() == self.file.lease();
        let geometry_matches = self.file.position() == self.offset as u64
            && self.file.len() == self.storage.len() as u64;
        fs.close_file(self.file).await.map_err(SourceError::Io)?;
        if !lease_matches {
            return Err(SourceError::Worker(WorkerError::Stale));
        }
        if !geometry_matches {
            return Err(SourceError::Contract);
        }
        if self.failed || self.offset != self.storage.len() {
            return Err(SourceError::Failed);
        }
        let wav = Wav::parse(self.storage).map_err(SourceError::Decode)?;
        if wav.frame_count() == 0 {
            return Err(SourceError::Contract);
        }
        let identity =
            SourceIdentity::new(2, self.hash.finalize().0).map_err(SourceError::Decode)?;
        let job = requests
            .verify_source(self.job, media, identity)
            .map_err(SourceError::Worker)?;
        Ok(VerifiedWav { wav, job, identity })
    }
}

pub struct VerifiedWav<'a> {
    wav: Wav<'a>,
    job: Job,
    identity: SourceIdentity,
}
impl<'a> VerifiedWav<'a> {
    pub fn source(&self) -> libapta::SourceInfo {
        libapta::SourceInfo {
            fingerprint_kind: self.identity.kind(),
            fingerprint: self.identity.bytes(),
            ..self.wav.source()
        }
    }
    /// Geometry is bound to the decoded object before any PCM is accepted.
    pub fn worker<'s>(self, session: Session<'s>) -> Result<WavWorker<'a, 's>, WorkerError> {
        let c = session.config();
        let s = self.wav.source();
        if c.sample_rate != s.sample_rate
            || c.channel_count != s.channel_count
            || Some(c.total_frames) != s.total_frames
        {
            return Err(WorkerError::Failed);
        }
        Ok(WavWorker {
            worker: Worker::new(session, self.job)?,
            wav: self.wav,
            offset: 0,
            eof: false,
        })
    }
}

/// One decode/push (<=256 frames) and one core step per call. Re-decodes only the
/// unaccepted suffix from immutable bytes, preserving even sub-block backpressure.
pub struct WavWorker<'b, 's> {
    worker: Worker<'s>,
    wav: Wav<'b>,
    offset: u64,
    eof: bool,
}
impl WavWorker<'_, '_> {
    pub fn step(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        scratch: &mut [f32; PCM_SAMPLES],
    ) -> Result<bool, WorkerError> {
        self.worker.check(requests, media)?;
        if self.offset < self.wav.frame_count() {
            let channels = self.wav.source().channel_count as usize;
            let n = match self
                .wav
                .read_frames(self.offset, &mut scratch[..256 * channels])
            {
                Ok(n) => n,
                Err(e) => {
                    self.worker.source_failed();
                    return Err(WorkerError::Core(e));
                }
            };
            self.offset += self
                .worker
                .push(requests, media, &scratch[..n * channels])? as u64;
        } else if !self.eof {
            self.worker.finish_input(requests, media)?;
            self.eof = true;
        }
        self.worker.step(requests, media)
    }
    pub fn complete<'b>(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        bank: &'b mut [pajoniiir_track_analysis::Beat],
    ) -> Result<crate::worker::Completed<'b>, WorkerError> {
        self.worker.complete(requests, media, bank)
    }
    pub fn waveform<'b>(
        &mut self,
        requests: &Requests,
        media: &MediaSession,
        bank: &'b mut [pajoniiir_waveform::WaveformColumn],
        color: u16,
    ) -> Result<pajoniiir_waveform::WaveformView<'b>, WorkerError> {
        self.worker.waveform(requests, media, bank, color)
    }
}
