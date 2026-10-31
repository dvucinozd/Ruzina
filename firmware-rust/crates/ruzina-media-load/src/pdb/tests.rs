extern crate std;

use super::*;
use core::future::Future;
use core::task::{Context, Poll, Waker};
use ruzina_media_fs::LeaseBound;
use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
use std::{boxed::Box, vec, vec::Vec};

const PAGE: usize = 512;

fn ready<T>(future: impl Future<Output = T>) -> T {
    match Box::pin(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending"),
    }
}

/// The lease of the `generation`th connection of one media source.
fn lease(generation: u32) -> MediaLease {
    let mut media = MediaSession::new();
    let source = MediaSourceId::new(1).unwrap();
    for _ in 1..generation {
        media.on_connect(source);
        media.on_disconnect(None);
    }
    let ConnectResult::Accepted(lease) = media.on_connect(source) else {
        panic!()
    };
    lease
}

fn volume_identity() -> VolumeIdentity {
    VolumeIdentity([0x42; 32])
}

fn put_le16(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_le32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_short_devicesql(data: &mut [u8], offset: usize, text: &[u8]) {
    let total = text.len() + 1;
    assert!(total <= 0x7f);
    data[offset] = ((total as u8) << 1) | 1;
    data[offset + 1..offset + 1 + text.len()].copy_from_slice(text);
}

/// Rows at the given heap offsets, all present, in one slot group.
fn page_rows(page: &mut [u8], rows: &[u16]) {
    put_le32(page, 0x0c, u32::MAX);
    put_le32(page, 0x18, rows.len() as u32);
    let mut present = 0u16;
    for (index, &offset) in rows.iter().enumerate() {
        put_le16(page, PAGE - 6 - index * 2, offset);
        present |= 1 << index;
    }
    put_le16(page, PAGE - 4, present);
}

struct Track {
    id: u32,
    artist: u32,
    title: &'static [u8],
    path: &'static [u8],
}

const TRACKS: [Track; 2] = [
    Track {
        id: 1,
        artist: 5,
        title: b"First",
        path: b"/Contents/first.wav",
    },
    Track {
        id: 2,
        artist: 5,
        title: b"Second",
        path: b"/Contents/second.mp3",
    },
];

/// Header page, tracks (1), artists (2), playlist tree (3), entries (4).
fn export() -> Vec<u8> {
    let mut bytes = vec![0u8; PAGE * 5];
    put_le32(&mut bytes, 4, PAGE as u32);
    put_le32(&mut bytes, 8, 4);
    for (index, (kind, page)) in [
        (PDB_TABLE_TRACKS, 1u32),
        (PDB_TABLE_ARTISTS, 2),
        (PDB_TABLE_PLAYLIST_TREE, 3),
        (PDB_TABLE_PLAYLIST_ENTRIES, 4),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = 0x1c + index * 16;
        put_le32(&mut bytes, entry, kind);
        put_le32(&mut bytes, entry + 8, page);
        put_le32(&mut bytes, entry + 12, page);
    }

    let tracks = &mut bytes[PAGE..PAGE * 2];
    let rows = [0u16, 0xc0];
    page_rows(tracks, &rows);
    for (track, &row) in TRACKS.iter().zip(&rows) {
        let row = PDB_PAGE_HEAP_OFFSET + usize::from(row);
        put_le16(tracks, row, 0x0024);
        put_le32(tracks, row + 0x38, 12_000);
        put_le32(tracks, row + 0x44, track.artist);
        put_le32(tracks, row + 0x48, track.id);
        put_le16(tracks, row + 0x54, 180);
        // Title (17) and file path (20) string offsets, relative to the row.
        put_le16(tracks, row + 0x5e + 17 * 2, 0x90);
        put_le16(tracks, row + 0x5e + 20 * 2, 0xa0);
        put_short_devicesql(tracks, row + 0x90, track.title);
        put_short_devicesql(tracks, row + 0xa0, track.path);
    }

    let artists = &mut bytes[PAGE * 2..PAGE * 3];
    page_rows(artists, &[0]);
    let row = PDB_PAGE_HEAP_OFFSET;
    put_le16(artists, row, 0x60);
    put_le32(artists, row + 4, 5);
    artists[row + 9] = 10;
    put_short_devicesql(artists, row + 10, b"Artist");

    let tree = &mut bytes[PAGE * 3..PAGE * 4];
    page_rows(tree, &[0]);
    let row = PDB_PAGE_HEAP_OFFSET;
    put_le32(tree, row + 12, 9);
    put_short_devicesql(tree, row + 20, b"Warmup");

    let entries = &mut bytes[PAGE * 4..PAGE * 5];
    page_rows(entries, &[0, 12]);
    for (index, track) in [2u32, 1].into_iter().enumerate() {
        let row = PDB_PAGE_HEAP_OFFSET + index * 12;
        put_le32(entries, row, index as u32);
        put_le32(entries, row + 4, track);
        put_le32(entries, row + 8, 9);
    }
    bytes
}

struct File {
    lease: MediaLease,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Io {
    Missing,
    Removed,
}

/// One `export.pdb` whose media can be removed after a number of reads.
struct Files {
    lease: MediaLease,
    bytes: Vec<u8>,
    reads: usize,
    remove_after: Option<usize>,
    open: usize,
}
impl Files {
    fn new(lease: MediaLease, bytes: Vec<u8>) -> Self {
        Self {
            lease,
            bytes,
            reads: 0,
            remove_after: None,
            open: 0,
        }
    }
    fn removed(&self) -> bool {
        self.remove_after.is_some_and(|limit| self.reads >= limit)
    }
}
impl CatalogFiles for Files {
    type File = File;
    type Buffer = [u8; 4096];
    type Error = Io;
    async fn open_read(&mut self, lease: MediaLease, path: BoundedPath) -> Result<File, Io> {
        if lease != self.lease || self.removed() {
            return Err(Io::Removed);
        }
        if path.as_str() != REKORDBOX_EXPORT_PDB_PATH {
            return Err(Io::Missing);
        }
        self.open += 1;
        Ok(File {
            lease,
            len: self.bytes.len() as u64,
            position: 0,
        })
    }
    async fn read_at(
        &mut self,
        lease: MediaLease,
        mut file: File,
        mut buffer: [u8; 4096],
        position: u64,
    ) -> (File, [u8; 4096], Result<usize, Io>) {
        if lease != self.lease || self.removed() {
            return (file, buffer, Err(Io::Removed));
        }
        self.reads += 1;
        let start = (position as usize).min(self.bytes.len());
        let count = buffer.len().min(self.bytes.len() - start);
        buffer[..count].copy_from_slice(&self.bytes[start..start + count]);
        file.position = (start + count) as u64;
        (file, buffer, Ok(count))
    }
    async fn close_file(&mut self, lease: MediaLease, file: File) -> Result<(), Io> {
        assert_eq!(file.lease, lease);
        self.open -= 1;
        if self.removed() {
            Err(Io::Removed)
        } else {
            Ok(())
        }
    }
}

struct Import {
    names: PdbNameStage<VecNameMap>,
    stage: VecCatalogStage,
}
impl Import {
    fn new() -> Self {
        Self {
            names: PdbNameStage::try_new().unwrap(),
            stage: VecCatalogStage::try_new().unwrap(),
        }
    }
    fn run(&mut self, files: &mut Files) -> Result<PdbProbe, ImportError<Io>> {
        let lease = files.lease;
        self.stage.prepare_generation(lease);
        self.names.clear();
        let (_, result) = ready(probe_export_pdb(
            files,
            lease,
            volume_identity(),
            [0; 4096],
            &mut self.names,
            &mut self.stage,
        ));
        result
    }
}

#[test]
fn imports_tracks_names_and_playlists_into_one_publication() {
    let mut files = Files::new(lease(1), export());
    let mut import = Import::new();
    let probe = import.run(&mut files).unwrap();

    assert_eq!(files.open, 0, "export.pdb is closed");
    assert_eq!(probe.tracks_layout.unwrap().track_rows, 2);
    assert_eq!(probe.artists_layout.unwrap().names, 1);
    let snapshot = import.stage.current().unwrap();
    assert_eq!(snapshot.lease(), lease(1));
    assert_eq!(snapshot.len(), 2);
    let first = snapshot.get(0).unwrap();
    assert_eq!(first.title.as_str(), "First");
    assert_eq!(first.artist.as_str(), "Artist");
    assert_eq!(first.audio_path.as_str(), "/Contents/first.wav");
    assert_eq!(snapshot.get(1).unwrap().title.as_str(), "Second");

    let stats = probe.playlists.unwrap();
    let playlists = import.stage.playlists().unwrap();
    assert_eq!(playlists.publication().lease, lease(1));
    assert_eq!(playlists.publication().revision, snapshot.revision());
    assert_eq!(stats, playlists.stats());
}

#[test]
fn removal_mid_import_publishes_nothing_and_a_reinsert_imports_again() {
    let mut import = Import::new();
    for limit in 1..8 {
        let mut files = Files::new(lease(1), export());
        files.remove_after = Some(limit);
        assert_eq!(
            import.run(&mut files).err(),
            Some(ImportError::Io(Io::Removed))
        );
        assert!(
            import.stage.current().is_none(),
            "nothing after {limit} reads"
        );
        assert_eq!(files.open, 0);
        import.stage.invalidate(lease(1));
    }
    // Every aborted build returned its banks: the next generation imports.
    let mut files = Files::new(lease(2), export());
    import.run(&mut files).unwrap();
    assert_eq!(import.stage.current().unwrap().lease(), lease(2));
}

#[test]
fn a_new_generation_retires_the_previous_lease_and_invalidate_withdraws() {
    let mut import = Import::new();
    import.run(&mut Files::new(lease(1), export())).unwrap();
    let first = import.stage.current().unwrap().revision();
    import.run(&mut Files::new(lease(2), export())).unwrap();
    let snapshot = import.stage.current().unwrap();
    assert_eq!(snapshot.lease(), lease(2));
    assert_ne!(snapshot.revision(), first);

    import.stage.invalidate(lease(1));
    assert!(
        import.stage.current().is_some(),
        "another lease is untouched"
    );
    import.stage.invalidate(lease(2));
    assert!(import.stage.current().is_none());
    assert!(import.stage.playlists().is_none());
}

#[test]
fn missing_export_or_tracks_table_fails_without_publishing() {
    let mut import = Import::new();
    let mut bytes = export();
    put_le32(&mut bytes, 0x1c, PDB_TABLE_KEYS);
    assert_eq!(
        import.run(&mut Files::new(lease(1), bytes)).err(),
        Some(ImportError::MissingTracksTable)
    );
    assert!(import.stage.current().is_none());

    let mut files = Files::new(lease(1), export());
    files.lease = lease(3);
    assert!(matches!(
        ready(probe_export_pdb(
            &mut files,
            lease(1),
            volume_identity(),
            [0; 4096],
            &mut import.names,
            &mut import.stage,
        ))
        .1,
        Err(ImportError::Io(Io::Removed))
    ));
}

#[test]
fn duplicate_artist_ids_fail_the_import() {
    let mut bytes = export();
    let artists = &mut bytes[PAGE * 2..PAGE * 3];
    page_rows(artists, &[0, 32]);
    let row = PDB_PAGE_HEAP_OFFSET + 32;
    put_le16(artists, row, 0x60);
    put_le32(artists, row + 4, 5);
    artists[row + 9] = 10;
    put_short_devicesql(artists, row + 10, b"Again");
    let mut import = Import::new();
    assert_eq!(
        import.run(&mut Files::new(lease(1), bytes)).err(),
        Some(ImportError::Catalog(CatalogError::DuplicateNameId(5)))
    );
    assert!(import.stage.current().is_none());
}

#[test]
fn first_page_probe_extracts_rekordbox_table_roots() {
    let mut page = [0u8; 512];
    put_le32(&mut page, 4, 512);
    put_le32(&mut page, 8, 4);

    for (index, (kind, first_page)) in [
        (PDB_TABLE_TRACKS, 1u32),
        (PDB_TABLE_ARTISTS, 2),
        (PDB_TABLE_ALBUMS, 3),
        (PDB_TABLE_KEYS, 4),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 0x1c + index * 16;
        put_le32(&mut page, offset, kind);
        put_le32(&mut page, offset + 8, first_page);
        put_le32(&mut page, offset + 12, first_page);
    }

    let probe =
        parse_first_page::<()>(lease(1), volume_identity(), 512 * 6, &page, page.len()).unwrap();
    assert_eq!(probe.header.page_size, 512);
    assert_eq!(probe.tracks_table.first_page, 1);
    assert_eq!(probe.artists_table.map(|table| table.first_page), Some(2));
    assert_eq!(probe.albums_table.map(|table| table.first_page), Some(3));
    assert_eq!(probe.keys_table.map(|table| table.last_page), Some(4));
    assert_eq!(probe.tracks_layout, None);
}

#[test]
fn first_page_probe_accepts_large_pdb_page_from_bounded_prefix() {
    let mut prefix = [0u8; 64];
    put_le32(&mut prefix, 4, 65_536);
    put_le32(&mut prefix, 8, 1);
    put_le32(&mut prefix, 0x1c, PDB_TABLE_TRACKS);
    put_le32(&mut prefix, 0x24, 1);
    put_le32(&mut prefix, 0x28, 1);

    let probe = parse_first_page::<()>(
        lease(1),
        volume_identity(),
        65_536 * 2,
        &prefix,
        prefix.len(),
    )
    .unwrap();

    assert_eq!(probe.header.page_size, 65_536);
    assert_eq!(probe.tracks_table.first_page, 1);
}
