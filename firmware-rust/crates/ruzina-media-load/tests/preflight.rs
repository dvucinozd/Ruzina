use core::cell::Cell;
use core::future::Future;
use core::task::{Context, Poll, Waker};
use ruzina_core::MediaTrackId;
use ruzina_hot_cues::HotCueBank;
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId, CatalogText, CatalogTrack};
use ruzina_media_fs::{BoundedPath, EntryKind, FileHandle, FileStat, LeaseBound};
use ruzina_media_load::{ANLZ_READ_BYTES, LoadFiles, load_source_analysis, validate_audio_source};
use ruzina_media_session::{ConnectResult, MediaLease, MediaSession, MediaSourceId};

const DAT: &[u8] = include_bytes!("../../ruzina-rekordbox/tests/fixtures/track-a.DAT");
const EXT: &[u8] = include_bytes!("../../ruzina-rekordbox/tests/fixtures/track-a.EXT");
const AUDIO: &str = "/Contents/a.wav";
const ANLZ: &str = "/PIONEER/USBANLZ/P000/0001/ANLZ0000.DAT";
const ANLZ_EXT: &str = "/PIONEER/USBANLZ/P000/0001/ANLZ0000.EXT";

fn ready<F: Future>(future: F) -> F::Output {
    match Box::pin(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending"),
    }
}

fn lease() -> MediaLease {
    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap()) else {
        panic!()
    };
    lease
}

fn text<const N: usize>(value: &str) -> CatalogText<N> {
    CatalogText::try_from_str(value).unwrap()
}

fn candidate(lease: MediaLease, audio: &str, analysis: &str) -> CatalogLoadCandidate {
    CatalogLoadCandidate {
        publication: CatalogPublicationId { lease, revision: 7 },
        index: 0,
        track: CatalogTrack {
            track_id: MediaTrackId([4; 16]),
            source_track_id: 1,
            bpm_x100: 12_000,
            duration_ms: 1_000,
            title: text("A"),
            artist: text(""),
            album: text(""),
            key: text(""),
            audio_path: text(audio),
            analysis_path: text(analysis),
            artwork_path: text(""),
        },
    }
}

struct File {
    lease: MediaLease,
    index: usize,
    len: u64,
    position: u64,
}
impl LeaseBound for File {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl FileHandle for File {
    fn len(&self) -> u64 {
        self.len
    }
    fn position(&self) -> u64 {
        self.position
    }
    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

/// In-memory files with a withdrawable publication and a removable lease.
struct Files {
    lease: MediaLease,
    entries: Vec<(&'static str, EntryKind, Vec<u8>)>,
    reads: Cell<usize>,
    withdraw_after: Option<usize>,
    removed: Cell<bool>,
    open: usize,
}
impl Files {
    fn new(lease: MediaLease) -> Self {
        Self {
            lease,
            entries: vec![
                (AUDIO, EntryKind::File, vec![1; 64]),
                (ANLZ, EntryKind::File, DAT.to_vec()),
                (ANLZ_EXT, EntryKind::File, EXT.to_vec()),
                ("/Contents", EntryKind::Directory, Vec::new()),
            ],
            reads: Cell::new(0),
            withdraw_after: None,
            removed: Cell::new(false),
            open: 0,
        }
    }
    fn live(&self, lease: MediaLease) -> bool {
        lease == self.lease && !self.removed.get()
    }
    fn find(&self, path: &BoundedPath) -> Option<usize> {
        self.entries
            .iter()
            .position(|(name, ..)| *name == path.as_str())
    }
}
impl LoadFiles for Files {
    type File = File;
    type Buffer = [u8; ANLZ_READ_BYTES];
    fn publication_current(&self, publication: CatalogPublicationId) -> bool {
        self.live(publication.lease)
            && self
                .withdraw_after
                .is_none_or(|limit| self.reads.get() < limit)
    }
    async fn stat(&mut self, lease: MediaLease, path: BoundedPath) -> Option<FileStat> {
        let (_, kind, bytes) = &self.entries[self.find(&path).filter(|_| self.live(lease))?];
        Some(FileStat {
            kind: *kind,
            len: bytes.len() as u64,
            modified_unix_seconds: None,
        })
    }
    async fn open_read(&mut self, lease: MediaLease, path: BoundedPath) -> Option<File> {
        let index = self.find(&path).filter(|_| self.live(lease))?;
        self.open += 1;
        Some(File {
            lease,
            index,
            len: self.entries[index].2.len() as u64,
            position: 0,
        })
    }
    async fn read_at(
        &mut self,
        lease: MediaLease,
        mut file: File,
        mut buffer: Self::Buffer,
        position: u64,
    ) -> (File, Self::Buffer, usize) {
        if !self.live(lease) {
            return (file, buffer, 0);
        }
        self.reads.set(self.reads.get() + 1);
        let bytes = &self.entries[file.index].2;
        let start = (position as usize).min(bytes.len());
        let count = buffer.len().min(bytes.len() - start);
        buffer[..count].copy_from_slice(&bytes[start..start + count]);
        file.position = (start + count) as u64;
        (file, buffer, count)
    }
    async fn close_file(&mut self, lease: MediaLease, _file: File) -> bool {
        self.open -= 1;
        self.live(lease)
    }
}

#[test]
fn audio_source_must_be_a_nonempty_regular_file_under_the_live_lease() {
    let lease = lease();
    let mut files = Files::new(lease);
    assert!(ready(validate_audio_source(
        &mut files,
        &candidate(lease, AUDIO, ANLZ)
    )));
    for path in ["", "/missing.wav", "/Contents"] {
        assert!(!ready(validate_audio_source(
            &mut files,
            &candidate(lease, path, ANLZ)
        )));
    }
    files.entries[0].2.clear();
    assert!(!ready(validate_audio_source(
        &mut files,
        &candidate(lease, AUDIO, ANLZ)
    )));
    files.entries[0].2.push(1);
    files.removed.set(true);
    assert!(!ready(validate_audio_source(
        &mut files,
        &candidate(lease, AUDIO, ANLZ)
    )));
}

#[test]
fn imports_dat_and_ext_into_source_cues_and_analysis() {
    let lease = lease();
    let mut files = Files::new(lease);
    let loaded = ready(load_source_analysis(
        &mut files,
        candidate(lease, AUDIO, ANLZ),
        [0; ANLZ_READ_BYTES],
        9,
    ));
    let publication = loaded.publication.expect("DAT + EXT analysis");
    let identity = publication.identity();
    assert_eq!(identity.lease, lease);
    assert_eq!(identity.catalog_revision, 7);
    assert_eq!(identity.load_generation, 9);
    assert!(loaded.source_cues_available);
    assert!(!publication.view().cues().is_empty());
    assert_ne!(
        loaded.source,
        HotCueBank::empty(MediaTrackId([4; 16])).as_source()
    );
    assert_eq!(files.open, 0, "every opened analysis file is closed");
}

#[test]
fn missing_dat_yields_no_analysis_and_an_empty_source_bank() {
    let lease = lease();
    let mut files = Files::new(lease);
    let loaded = ready(load_source_analysis(
        &mut files,
        candidate(lease, AUDIO, "/PIONEER/USBANLZ/none/ANLZ0000.DAT"),
        [0; ANLZ_READ_BYTES],
        1,
    ));
    assert!(loaded.publication.is_none());
    assert!(!loaded.source_cues_available);
    assert_eq!(
        loaded.source,
        HotCueBank::empty(MediaTrackId([4; 16])).as_source()
    );
}

#[test]
fn rejected_ext_keeps_the_dat_analysis() {
    let lease = lease();
    let mut files = Files::new(lease);
    files.entries[2].2.truncate(40);
    let loaded = ready(load_source_analysis(
        &mut files,
        candidate(lease, AUDIO, ANLZ),
        [0; ANLZ_READ_BYTES],
        1,
    ));
    assert!(loaded.publication.is_some());
    assert_eq!(files.open, 0);
}

#[test]
fn publication_withdrawn_mid_scan_publishes_nothing() {
    let lease = lease();
    let mut files = Files::new(lease);
    files.withdraw_after = Some(1);
    let loaded = ready(load_source_analysis(
        &mut files,
        candidate(lease, AUDIO, ANLZ),
        [0; ANLZ_READ_BYTES],
        1,
    ));
    assert!(loaded.publication.is_none());
    assert!(!loaded.source_cues_available);
    assert_eq!(files.open, 0);
}

#[test]
fn removed_media_mid_scan_publishes_nothing() {
    let lease = lease();
    let mut files = RemoveAfterFirstRead(Files::new(lease));
    let loaded = ready(load_source_analysis(
        &mut files,
        candidate(lease, AUDIO, ANLZ),
        [0; ANLZ_READ_BYTES],
        1,
    ));
    assert!(loaded.publication.is_none());
    assert_eq!(files.0.open, 0);
}

/// Media disappears after the first successful read.
struct RemoveAfterFirstRead(Files);
impl LoadFiles for RemoveAfterFirstRead {
    type File = File;
    type Buffer = [u8; ANLZ_READ_BYTES];
    fn publication_current(&self, publication: CatalogPublicationId) -> bool {
        self.0.publication_current(publication)
    }
    async fn stat(&mut self, lease: MediaLease, path: BoundedPath) -> Option<FileStat> {
        self.0.stat(lease, path).await
    }
    async fn open_read(&mut self, lease: MediaLease, path: BoundedPath) -> Option<File> {
        self.0.open_read(lease, path).await
    }
    async fn read_at(
        &mut self,
        lease: MediaLease,
        file: File,
        buffer: Self::Buffer,
        position: u64,
    ) -> (File, Self::Buffer, usize) {
        let result = self.0.read_at(lease, file, buffer, position).await;
        self.0.removed.set(true);
        result
    }
    async fn close_file(&mut self, lease: MediaLease, file: File) -> bool {
        self.0.close_file(lease, file).await
    }
}
