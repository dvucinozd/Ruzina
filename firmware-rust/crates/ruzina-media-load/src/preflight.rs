//! Library load preflight: audio stat, Rekordbox ANLZ DAT/EXT import and
//! catalog-publication fencing. Analysis publication allocates, so the caller
//! must provide a global allocator.

use ruzina_hot_cues::HotCueBank;
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId};
use ruzina_media_fs::{AsyncFileSystem, BoundedPath, EntryKind, FileHandle, FileStat, LeaseBound};
use ruzina_media_session::MediaLease;
use ruzina_rekordbox::anlz_analysis::{AnalysisFileKind, AnlzImporter, extended_path};
use ruzina_track_analysis::owned::{AnalysisIdentity, AnalysisPublication};

pub use ruzina_rekordbox::anlz_analysis::ANLZ_READ_BYTES;

/// Filesystem operations the load preflight needs. Every operation names the
/// lease it was issued for; an implementation fails it once that lease is no
/// longer live, including for bytes it could serve from a cache.
#[allow(async_fn_in_trait)]
pub trait LoadFiles {
    type File: FileHandle;
    /// Owned read buffer of at least [`ANLZ_READ_BYTES`].
    type Buffer: AsRef<[u8]> + AsMut<[u8]>;

    /// Whether the catalog publication that produced the candidate is current.
    fn publication_current(&self, publication: CatalogPublicationId) -> bool;
    async fn stat(&mut self, lease: MediaLease, path: BoundedPath) -> Option<FileStat>;
    async fn open_read(&mut self, lease: MediaLease, path: BoundedPath) -> Option<Self::File>;
    /// Seek to `position` and read into `buffer`. Any failure reports 0 bytes;
    /// the file and buffer always return to the caller.
    async fn read_at(
        &mut self,
        lease: MediaLease,
        file: Self::File,
        buffer: Self::Buffer,
        position: u64,
    ) -> (Self::File, Self::Buffer, usize);
    async fn close_file(&mut self, lease: MediaLease, file: Self::File) -> bool;
}

/// A loadable source is a non-empty regular file at the catalog's audio path.
pub async fn validate_audio_source<F: LoadFiles>(
    files: &mut F,
    candidate: &CatalogLoadCandidate,
) -> bool {
    if candidate.track.audio_path.is_empty() {
        return false;
    }
    let Ok(path) = BoundedPath::new(candidate.track.audio_path.as_str()) else {
        return false;
    };
    files
        .stat(candidate.publication.lease, path)
        .await
        .is_some_and(|stat| stat.kind == EntryKind::File && stat.len != 0)
}

/// Source cues and analysis read for one candidate.
pub struct SourceAnalysis<B> {
    pub buffer: B,
    pub source: HotCueBank,
    pub source_cues_available: bool,
    pub publication: Option<AnalysisPublication>,
}

/// Import the candidate's DAT and optional EXT analysis. A missing or invalid
/// DAT yields no analysis and an empty source bank; a rejected EXT keeps the
/// DAT analysis, as in M2.5. A publication withdrawn mid-scan yields nothing.
pub async fn load_source_analysis<F: LoadFiles>(
    files: &mut F,
    candidate: CatalogLoadCandidate,
    buffer: F::Buffer,
    generation: u32,
) -> SourceAnalysis<F::Buffer> {
    let mut source = HotCueBank::empty(candidate.track.track_id).as_source();
    let mut buffer = buffer;
    let mut importer = AnlzImporter::new();
    let mut publication = None;
    if let Ok(path) = BoundedPath::new(candidate.track.analysis_path.as_str()) {
        let (next_buffer, dat) = scan_analysis_file(
            files,
            candidate,
            path,
            buffer,
            &mut importer,
            AnalysisFileKind::Dat,
        )
        .await;
        buffer = next_buffer;
        if dat == AnalysisScanResult::Complete {
            let mut ext_storage = [0u8; 256];
            if let Some(path) =
                extended_path(candidate.track.analysis_path.as_str(), &mut ext_storage)
                    .and_then(|path| BoundedPath::new(path).ok())
            {
                let (next_buffer, ext) = scan_analysis_file(
                    files,
                    candidate,
                    path,
                    buffer,
                    &mut importer,
                    AnalysisFileKind::Ext,
                )
                .await;
                buffer = next_buffer;
                // A rejected EXT keeps the DAT analysis, as in M2.5.
                if ext == AnalysisScanResult::Invalid {
                    importer.discard_ext();
                }
            }
            if files.publication_current(candidate.publication) {
                let identity = AnalysisIdentity {
                    track: candidate.track.track_id,
                    lease: candidate.publication.lease,
                    catalog_revision: candidate.publication.revision,
                    load_generation: generation,
                };
                publication = importer.finish(identity, candidate.track.bpm_x100).ok();
            }
        }
    }
    if let Some(data) = publication.as_ref() {
        for cue in data.view().cues() {
            match cue.end_ms {
                Some(end) => {
                    source.set_loop(cue.slot, cue.start_ms, end);
                }
                None => {
                    source.set_single(cue.slot, cue.start_ms);
                }
            }
        }
    }
    SourceAnalysis {
        buffer,
        source: source.as_source(),
        source_cues_available: publication.is_some(),
        publication,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnalysisScanResult {
    Missing,
    Complete,
    Invalid,
}

async fn scan_analysis_file<F: LoadFiles>(
    files: &mut F,
    candidate: CatalogLoadCandidate,
    path: BoundedPath,
    buffer: F::Buffer,
    importer: &mut AnlzImporter,
    kind: AnalysisFileKind,
) -> (F::Buffer, AnalysisScanResult) {
    let lease = candidate.publication.lease;
    let Some(mut file) = files.open_read(lease, path).await else {
        return (buffer, AnalysisScanResult::Missing);
    };
    let mut valid = importer.begin_file(kind, file.len()).is_ok();
    let mut buffer = buffer;
    while valid {
        let Some(plan) = importer.read_plan() else {
            break;
        };
        if !files.publication_current(candidate.publication) {
            valid = false;
            break;
        }
        let (next_file, next_buffer, bytes) = files
            .read_at(lease, file, buffer, u64::from(plan.offset))
            .await;
        file = next_file;
        buffer = next_buffer;
        valid = bytes >= plan.len
            && buffer
                .as_ref()
                .get(..plan.len)
                .is_some_and(|bytes| importer.ingest(plan.offset, bytes).is_ok());
    }
    valid &= importer.file_complete();
    valid &= files.close_file(lease, file).await;
    (
        buffer,
        if valid {
            AnalysisScanResult::Complete
        } else {
            AnalysisScanResult::Invalid
        },
    )
}

/// [`LoadFiles`] over one mounted [`AsyncFileSystem`] owner. The shared
/// filesystems do not fence their caches by lease, so `is_current` is checked
/// before and after every operation; only a still-live lease yields data.
pub struct FsLoadFiles<'a, V, C, P, B> {
    volume: &'a mut V,
    is_current: C,
    publication_current: P,
    _buffer: core::marker::PhantomData<fn(B) -> B>,
}

impl<'a, V, C, P, B> FsLoadFiles<'a, V, C, P, B>
where
    V: AsyncFileSystem,
    C: Fn() -> bool,
    P: Fn(CatalogPublicationId) -> bool,
{
    pub fn new(volume: &'a mut V, is_current: C, publication_current: P) -> Self {
        Self {
            volume,
            is_current,
            publication_current,
            _buffer: core::marker::PhantomData,
        }
    }

    fn live(&self, lease: MediaLease) -> bool {
        self.volume.lease() == lease && (self.is_current)()
    }
}

impl<V, C, P, B> LoadFiles for FsLoadFiles<'_, V, C, P, B>
where
    V: AsyncFileSystem,
    C: Fn() -> bool,
    P: Fn(CatalogPublicationId) -> bool,
    B: AsRef<[u8]> + AsMut<[u8]>,
{
    type File = V::File;
    type Buffer = B;

    fn publication_current(&self, publication: CatalogPublicationId) -> bool {
        self.live(publication.lease) && (self.publication_current)(publication)
    }

    async fn stat(&mut self, lease: MediaLease, path: BoundedPath) -> Option<FileStat> {
        if !self.live(lease) {
            return None;
        }
        let stat = self.volume.stat(path.as_str()).await.ok();
        stat.filter(|_| self.live(lease))
    }

    async fn open_read(&mut self, lease: MediaLease, path: BoundedPath) -> Option<V::File> {
        if !self.live(lease) {
            return None;
        }
        let file = self.volume.open_read(path.as_str()).await.ok()?;
        if file.lease() == lease && self.live(lease) {
            Some(file)
        } else {
            let _ = self.volume.close_file(file).await;
            None
        }
    }

    async fn read_at(
        &mut self,
        lease: MediaLease,
        mut file: V::File,
        mut buffer: B,
        position: u64,
    ) -> (V::File, B, usize) {
        if !self.live(lease)
            || self
                .volume
                .seek_absolute(&mut file, position)
                .await
                .is_err()
        {
            return (file, buffer, 0);
        }
        let output = buffer.as_mut();
        let mut filled = 0;
        while filled < output.len() {
            if !self.live(lease) {
                return (file, buffer, 0);
            }
            match self.volume.read(&mut file, &mut output[filled..]).await {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(_) => return (file, buffer, 0),
            }
        }
        let filled = if self.live(lease) { filled } else { 0 };
        (file, buffer, filled)
    }

    async fn close_file(&mut self, lease: MediaLease, file: V::File) -> bool {
        let closed = self.volume.close_file(file).await.is_ok();
        closed && self.live(lease)
    }
}
