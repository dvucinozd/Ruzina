//! Bounded two-pass WAV acquisition. No full source object is retained.
//! Both passes hash every byte; only equal hashes plus successful close release
//! a worker for consumer delivery. Open/close must be driven to completion.
use crate::source::{PCM_SAMPLES, READ_BYTES, SourceError, SourceRequest};
use crate::worker::{Job, Requests, Worker, WorkerError};
use libapta::{
    session::{Session, SourceIdentity},
    wav::{WavLayout, WavScanner},
};
use pajoniiir_media_fs::{AsyncFileSystem, FileHandle, LeaseBound};
use pajoniiir_media_identity::Sha256;
use pajoniiir_media_session::MediaSession;

/// Caller policy for one two-pass job. Bounds are inclusive; zero permits no
/// corresponding work. No value is a firmware default or an elapsed-time limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamLimits {
    pub object_bytes: u64,
    pub duration_ms: u64,
    pub scan_steps: u64,
    pub processing_steps: u64,
}
impl StreamLimits {
    /// Compatibility profile for callers enforcing their own policy externally.
    pub const UNLIMITED: Self = Self {
        object_bytes: u64::MAX,
        duration_ms: u64::MAX,
        scan_steps: u64::MAX,
        processing_steps: u64::MAX,
    };
}

struct Reader<H> {
    file: H,
    len: u64,
    offset: u64,
    hash: Sha256,
    failed: bool,
}
impl<H: FileHandle> Reader<H> {
    async fn read<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        job: Job,
        requests: &Requests,
        media: &MediaSession,
        output: &mut [u8],
    ) -> Result<usize, SourceError<F::Error>> {
        if self.failed {
            return Err(SourceError::Failed);
        }
        self.failed = true; // also poisons a dropped pending read
        if !requests.accepts(job, media)
            || fs.lease() != self.file.lease()
            || Some(fs.lease()) != media.lease()
        {
            return Err(SourceError::Worker(WorkerError::Stale));
        }
        if self.file.position() != self.offset || self.file.len() != self.len || output.is_empty() {
            return Err(SourceError::Contract);
        }
        let size = (self.len - self.offset).min(output.len().min(READ_BYTES) as u64) as usize;
        let n = if size == 0 {
            0
        } else {
            let n = fs
                .read(&mut self.file, &mut output[..size])
                .await
                .map_err(SourceError::Io)?;
            if n == 0 || n > size {
                return Err(SourceError::Contract);
            }
            n
        };
        if self.file.position() != self.offset + n as u64
            || self.file.len() != self.len
            || self.file.lease() != fs.lease()
            || Some(fs.lease()) != media.lease()
        {
            return Err(SourceError::Contract);
        }
        self.hash.update(&output[..n]);
        self.offset += n as u64;
        self.failed = false;
        Ok(n)
    }
    async fn finish<F: AsyncFileSystem<File = H>>(
        self,
        fs: &mut F,
        job: Job,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<SourceIdentity, SourceError<F::Error>> {
        let valid = requests.accepts(job, media)
            && fs.lease() == self.file.lease()
            && Some(fs.lease()) == media.lease();
        let geometry = self.file.position() == self.offset && self.file.len() == self.len;
        fs.close_file(self.file).await.map_err(SourceError::Io)?;
        if !valid || Some(fs.lease()) != media.lease() {
            return Err(SourceError::Worker(WorkerError::Stale));
        }
        if !geometry {
            return Err(SourceError::Contract);
        }
        if self.failed || self.offset != self.len {
            return Err(SourceError::Failed);
        }
        SourceIdentity::new(2, self.hash.finalize().0).map_err(SourceError::Decode)
    }
}
async fn open<F: AsyncFileSystem>(
    request: &SourceRequest,
    fs: &mut F,
    requests: &Requests,
    media: &MediaSession,
) -> Result<Reader<F::File>, SourceError<F::Error>> {
    if !requests.accepts(request.job, media) || Some(fs.lease()) != media.lease() {
        return Err(SourceError::Worker(WorkerError::Stale));
    }
    let file = fs
        .open_read(request.path.as_str())
        .await
        .map_err(SourceError::Io)?;
    let len = file.len();
    if file.lease() != fs.lease()
        || Some(fs.lease()) != media.lease()
        || file.position() != 0
        || len == 0
    {
        fs.close_file(file).await.map_err(SourceError::Io)?;
        return Err(SourceError::Contract);
    }
    Ok(Reader {
        file,
        len,
        offset: 0,
        hash: Sha256::new(),
        failed: false,
    })
}
impl SourceRequest {
    /// First pass. Scratch is borrowed only per step; object length is not a
    /// memory allocation policy. Caller must impose its own duration/work limits.
    pub async fn open_stream<F: AsyncFileSystem>(
        self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<ScanRead<F::File>, SourceError<F::Error>> {
        self.open_stream_with_limits(fs, requests, media, StreamLimits::UNLIMITED)
            .await
    }
    /// Reject oversized objects after open and before reading; close even on
    /// rejection. Duration is checked after complete framing/close, before source
    /// verification or any Session/output-bank construction. Open/close must run
    /// to completion. Step limits count attempts, including cancelled pending I/O.
    pub async fn open_stream_with_limits<F: AsyncFileSystem>(
        self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        limits: StreamLimits,
    ) -> Result<ScanRead<F::File>, SourceError<F::Error>> {
        let reader = open(&self, fs, requests, media).await?;
        if reader.len > limits.object_bytes {
            fs.close_file(reader.file).await.map_err(SourceError::Io)?;
            return Err(SourceError::Limit);
        }
        let scanner = WavScanner::new(reader.len);
        Ok(ScanRead {
            request: self,
            reader,
            scanner,
            limits,
            remaining_steps: limits.scan_steps,
        })
    }
}
pub struct ScanRead<H> {
    request: SourceRequest,
    reader: Reader<H>,
    scanner: WavScanner,
    limits: StreamLimits,
    remaining_steps: u64,
}
impl<H: FileHandle> ScanRead<H> {
    /// Scan to the captured object end, yielding after every incomplete step.
    /// This borrows caller control so cancellation retains the open handle.
    /// Dropped reads or cooperative yields poison the scan. On success call
    /// `finish` to close and verify framing/identity/duration; on error call
    /// `abort` (after any required backend pending-operation recovery).
    /// This method never closes, publishes a source, or resets the step budget.
    pub async fn run<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        scratch: &mut [u8; READ_BYTES],
    ) -> Result<(), SourceError<F::Error>> {
        while !self.step(fs, requests, media, scratch).await? {
            self.reader.failed = true;
            yield_once().await;
            self.reader.failed = false;
        }
        Ok(())
    }

    /// One <=4096-byte read plus bounded framing/hash work; no PCM processing.
    pub async fn step<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        scratch: &mut [u8; READ_BYTES],
    ) -> Result<bool, SourceError<F::Error>> {
        if self.reader.failed {
            return Err(SourceError::Failed);
        }
        if self.reader.offset != self.reader.len {
            if self.remaining_steps == 0 {
                self.reader.failed = true;
                return Err(SourceError::Limit);
            }
            self.remaining_steps -= 1;
        }
        let n = self
            .reader
            .read(fs, self.request.job, requests, media, scratch)
            .await?;
        if let Err(e) = self.scanner.push(&scratch[..n]) {
            self.reader.failed = true;
            return Err(SourceError::Decode(e));
        }
        Ok(self.reader.offset == self.reader.len)
    }
    pub async fn abort<F: AsyncFileSystem<File = H>>(
        self,
        fs: &mut F,
    ) -> Result<(), SourceError<F::Error>> {
        fs.close_file(self.reader.file)
            .await
            .map_err(SourceError::Io)
    }
    pub async fn finish<F: AsyncFileSystem<File = H>>(
        self,
        fs: &mut F,
        requests: &mut Requests,
        media: &MediaSession,
    ) -> Result<VerifiedStream, SourceError<F::Error>> {
        let len = self.reader.len;
        let identity = self
            .reader
            .finish(fs, self.request.job, requests, media)
            .await?;
        let layout = self.scanner.finish().map_err(SourceError::Decode)?;
        if layout.source().total_frames == Some(0) {
            return Err(SourceError::Contract);
        }
        let source = layout.source();
        if u128::from(source.total_frames.unwrap()) * 1000
            > u128::from(self.limits.duration_ms) * u128::from(source.sample_rate)
        {
            return Err(SourceError::Limit);
        }
        let mut request = self.request;
        request.job = requests
            .verify_source(request.job, media, identity)
            .map_err(SourceError::Worker)?;
        Ok(VerifiedStream {
            request,
            layout,
            identity,
            len,
            processing_steps: self.limits.processing_steps,
        })
    }
}
/// Valid framing and first-pass identity; not yet an analyzed deliverable.
pub struct VerifiedStream {
    request: SourceRequest,
    layout: WavLayout,
    identity: SourceIdentity,
    len: u64,
    processing_steps: u64,
}
impl VerifiedStream {
    pub fn source(&self) -> libapta::SourceInfo {
        libapta::SourceInfo {
            fingerprint_kind: self.identity.kind(),
            fingerprint: self.identity.bytes(),
            ..self.layout.source()
        }
    }
    /// Prepare the existing local-tempo/meter profile directly into an EMPTY
    /// caller control slot. No I/O or allocation occurs. An occupied slot fails
    /// unchanged, so its possibly open handle remains recoverable. On any other
    /// failure the slot remains empty; working arrays may have been initialized.
    ///
    /// Keeping construction behind this synchronous boundary avoids returning a
    /// large Session-bearing Result through the async caller's polling frame.
    /// The slot still needs storage; this does not choose placement or a stack size.
    #[inline(never)]
    pub fn prepare_local_tempo<'s, 'b, H: FileHandle>(
        self,
        slot: &mut Option<StreamWorker<'s, 'b, H>>,
        buffers: crate::storage::LocalTempoBuffers<'s>,
        frames_per_column: u32,
        bytes: &'b mut [u8; READ_BYTES],
    ) -> Result<(), WorkerError> {
        if slot.is_some() {
            return Err(WorkerError::Failed);
        }
        let source = self.source();
        let mut session = Session::new(
            libapta::session::SessionConfig {
                sample_rate: source.sample_rate,
                channel_count: source.channel_count,
                total_frames: source.total_frames.ok_or(WorkerError::Failed)?,
                frames_per_column,
            },
            buffers.queue,
            buffers.columns,
        )
        .map_err(WorkerError::Core)?;
        session
            .enable_tempo(buffers.bins, buffers.flux)
            .map_err(WorkerError::Core)?;
        session.enable_meter().map_err(WorkerError::Core)?;
        *slot = Some(self.worker(session, bytes)?);
        Ok(())
    }

    /// Construct in place before asynchronous opening, so a large Session is
    /// never moved into an I/O future. No handle exists on geometry failure.
    pub fn worker<'s, 'b, H: FileHandle>(
        self,
        session: Session<'s>,
        bytes: &'b mut [u8; READ_BYTES],
    ) -> Result<StreamWorker<'s, 'b, H>, WorkerError> {
        let c = session.config();
        let s = self.layout.source();
        if c.sample_rate != s.sample_rate
            || c.channel_count != s.channel_count
            || Some(c.total_frames) != s.total_frames
        {
            return Err(WorkerError::Failed);
        }
        let worker = Worker::new(session, self.request.job)?;
        Ok(StreamWorker {
            worker,
            request: self.request,
            reader: None,
            expected_len: self.len,
            layout: self.layout,
            identity: self.identity,
            bytes,
            filled: 0,
            eof: false,
            done: false,
            opened: false,
            failed: false,
            verified: false,
            remaining_steps: self.processing_steps,
        })
    }
}
/// Private worker/output until `finish` verifies second-pass bytes and close.
/// Always consume the open handle with finish/abort after errors or cancellation. Backend pending
/// operation/DMA recovery remains the filesystem owner's obligation.
pub struct StreamWorker<'s, 'b, H> {
    worker: Worker<'s>,
    request: SourceRequest,
    reader: Option<Reader<H>>,
    expected_len: u64,
    layout: WavLayout,
    identity: SourceIdentity,
    bytes: &'b mut [u8; READ_BYTES],
    filled: usize,
    eof: bool,
    done: bool,
    opened: bool,
    failed: bool,
    verified: bool,
    remaining_steps: u64,
}
impl<H: FileHandle> StreamWorker<'_, '_, H> {
    /// Run the second pass on borrowed caller control, yielding once after each
    /// incomplete bounded step even when the filesystem completes immediately.
    /// Successful return requires the ordinary hash/close acceptance. A step
    /// error closes the handle; a close error takes precedence over that error.
    ///
    /// Cancellation during a read or cooperative yield poisons the worker, but
    /// keeps its handle available for `abort`. Backend recovery must precede
    /// abort when required. Open/close futures must still run to completion.
    /// Keep the control slot outside the driving coroutine when it must survive
    /// cancellation. This method neither allocates nor selects an executor.
    pub async fn run<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        pcm: &mut [f32; PCM_SAMPLES],
    ) -> Result<(), SourceError<F::Error>> {
        self.open(fs, requests, media).await?;
        loop {
            match self.step(fs, requests, media, pcm).await {
                Ok(true) => break,
                Ok(false) => {
                    // A dropped yield must not let a cancelled run resume via
                    // the manual step/finish APIs and authorize delivery.
                    self.failed = true;
                    yield_once().await;
                    self.failed = false;
                }
                Err(error) => {
                    self.abort(fs).await?;
                    return Err(error);
                }
            }
        }
        self.finish(fs, requests, media).await
    }

    /// Reopen the copied path without seeking. Drive this future to completion;
    /// backend recovery remains required if an in-flight open is cancelled.
    pub async fn open<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<(), SourceError<F::Error>> {
        if self.opened || self.failed {
            return Err(SourceError::Failed);
        }
        self.opened = true;
        self.failed = true;
        let reader = open(&self.request, fs, requests, media).await?;
        if reader.len != self.expected_len {
            fs.close_file(reader.file).await.map_err(SourceError::Io)?;
            return Err(SourceError::Contract);
        }
        self.reader = Some(reader);
        self.failed = false;
        Ok(())
    }

    /// At most one read, 256 decoded frames and one core step. Partial-byte reads
    /// and partial PCM acceptance retain the exact unconsumed suffix.
    pub async fn step<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        pcm: &mut [f32; PCM_SAMPLES],
    ) -> Result<bool, SourceError<F::Error>> {
        if self.failed || self.reader.as_ref().is_none_or(|r| r.failed) {
            return Err(SourceError::Failed);
        }
        let result = self.step_inner(fs, requests, media, pcm).await;
        if result.is_err() {
            self.failed = true;
            self.worker.source_failed();
        }
        result
    }
    async fn step_inner<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
        pcm: &mut [f32; PCM_SAMPLES],
    ) -> Result<bool, SourceError<F::Error>> {
        self.worker
            .check(requests, media)
            .map_err(SourceError::Worker)?;
        if self.done {
            return Ok(true);
        }
        if self.remaining_steps == 0 {
            return Err(SourceError::Limit);
        }
        self.remaining_steps -= 1;
        let reader = self.reader.as_mut().ok_or(SourceError::Failed)?;
        let start = self.layout.data_offset();
        let end = start + self.layout.data_bytes();
        let align = self.layout.frame_bytes();
        if reader.offset < start {
            let n = (start - reader.offset).min(READ_BYTES as u64) as usize;
            reader
                .read(fs, self.request.job, requests, media, &mut self.bytes[..n])
                .await?;
        } else if reader.offset < end || self.filled != 0 {
            if self.filled < align && reader.offset < end {
                let n = (end - reader.offset).min((READ_BYTES - self.filled) as u64) as usize;
                let got = reader
                    .read(
                        fs,
                        self.request.job,
                        requests,
                        media,
                        &mut self.bytes[self.filled..self.filled + n],
                    )
                    .await?;
                self.filled += got;
            }
            let frames = (self.filled / align).min(256);
            if frames != 0 {
                let channels = self.layout.source().channel_count as usize;
                self.layout
                    .decode(&self.bytes[..frames * align], &mut pcm[..frames * channels])
                    .map_err(SourceError::Decode)?;
                let accepted = self
                    .worker
                    .push(requests, media, &pcm[..frames * channels])
                    .map_err(SourceError::Worker)?;
                let used = accepted * align;
                self.bytes.copy_within(used..self.filled, 0);
                self.filled -= used;
            }
        } else if reader.offset < reader.len {
            reader
                .read(fs, self.request.job, requests, media, self.bytes)
                .await?;
        } else if !self.eof {
            self.worker
                .finish_input(requests, media)
                .map_err(SourceError::Worker)?;
            self.eof = true;
        }
        let complete = self
            .worker
            .step(requests, media)
            .map_err(SourceError::Worker)?;
        self.done = complete && self.eof;
        Ok(self.done)
    }
    pub async fn abort<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
    ) -> Result<(), SourceError<F::Error>> {
        self.failed = true;
        self.verified = false;
        let reader = self.reader.take().ok_or(SourceError::Failed)?;
        fs.close_file(reader.file).await.map_err(SourceError::Io)
    }
    /// Consumes the handle even on failure. Drive close to completion. Borrows
    /// worker control in place instead of copying Session into the close future.
    pub async fn finish<F: AsyncFileSystem<File = H>>(
        &mut self,
        fs: &mut F,
        requests: &Requests,
        media: &MediaSession,
    ) -> Result<(), SourceError<F::Error>> {
        let failed = self.failed;
        self.failed = true;
        self.verified = false;
        let reader = self.reader.take().ok_or(SourceError::Failed)?;
        let actual = reader.finish(fs, self.request.job, requests, media).await?;
        if failed || !self.done || actual != self.identity {
            return Err(SourceError::Failed);
        }
        self.verified = true;
        self.failed = false;
        Ok(())
    }
}
impl<'s, H> StreamWorker<'s, '_, H> {
    /// Borrow the ordinary worker in place after successful close/hash acceptance.
    /// Avoids moving Session through an async caller's extraction/error temporaries.
    /// Output views still borrow only their independent caller conversion banks.
    pub fn verified_worker(&mut self) -> Result<&mut Worker<'s>, WorkerError> {
        if !self.verified || self.failed {
            return Err(WorkerError::Failed);
        }
        Ok(&mut self.worker)
    }

    /// Synchronous extraction only after successful close/hash acceptance. No
    /// partial, aborted, cancelled or unverified analysis can escape.
    // Preserve a possibly open handle on misuse without allocating. This move
    // is synchronous and does not enlarge an I/O future.
    #[allow(clippy::result_large_err)]
    pub fn into_worker(self) -> Result<Worker<'s>, Self> {
        if !self.verified || self.failed {
            return Err(self);
        }
        Ok(self.worker)
    }
}

/// Control storage excluding backend handle and caller byte/PCM/core/output banks.
/// Actual generic instantiations and future/stack storage need separate budgets.
pub const SCAN_CONTROL_BUDGET: usize = 768;
pub const STREAM_CONTROL_BUDGET: usize = 4864;
const _: () = assert!(core::mem::size_of::<ScanRead<()>>() <= SCAN_CONTROL_BUDGET);
const _: () = assert!(core::mem::size_of::<StreamWorker<'_, '_, ()>>() <= STREAM_CONTROL_BUDGET);

// One self-waking suspension, shared by both bounded pass drivers.
async fn yield_once() {
    let mut yielded = false;
    core::future::poll_fn(|cx| {
        if yielded {
            core::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    })
    .await;
}
