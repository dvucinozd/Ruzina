//! Shared Rekordbox `export.pdb` catalog import (RP-01/06/07/18).
//!
//! The import walks the PDB header, the artist/album/key/artwork name tables,
//! the track table and the playlist tables through [`CatalogFiles`], and
//! stages everything into a [`CatalogStage`] that publishes one catalog and
//! its playlists together. Scanning itself never allocates; storage belongs to
//! the stage. The `alloc` feature provides [`VecCatalogStage`], whose capacity
//! is reserved once before any page is read.
//!
//! The native USB0 broker client and the legacy IDF exFAT owner implement
//! [`CatalogFiles`]; both run this one import.

use ruzina_media_catalog::playlists::PlaylistImportStats;
use ruzina_media_catalog::{CatalogBuildError, MEDIA_CATALOG_MAX_TRACKS};
use ruzina_media_fs::{AsyncFileSystem, BoundedPath, FileHandle, LeaseBound};
use ruzina_media_identity::{IdentityError, VolumeIdentity, derive_track_id};
use ruzina_media_session::MediaLease;
use ruzina_rekordbox::{
    PDB_MAX_NAMES, PDB_NAME_MAX, PDB_PAGE_HEAP_OFFSET, PDB_PATH_MAX, PDB_PLAYLIST_ENTRY_ROW_BYTES,
    PDB_PLAYLIST_TREE_ROW_HEADER_BYTES, PDB_ROW_SLOT_BATCH_ROWS, PDB_TABLE_ALBUMS,
    PDB_TABLE_ARTISTS, PDB_TABLE_ARTWORK, PDB_TABLE_KEYS, PDB_TABLE_PLAYLIST_ENTRIES,
    PDB_TABLE_PLAYLIST_TREE, PDB_TABLE_TRACKS, PDB_TITLE_MAX, PDB_TRACK_ROW_HEADER_BYTES,
    PdbCatalogError, PdbError, PdbHeader, PdbNameEntry, PdbNameLookup, PdbNameRowHeader,
    PdbNameRowLayout, PdbPageSlotWalker, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PdbRowEnds,
    PdbSlotBatchPlan, PdbTableDirectory, PdbTableEntry, PdbText, PdbTrackRow,
    REKORDBOX_EXPORT_PDB_PATH, parse_devicesql_field_header, parse_pdb_header,
    parse_pdb_name_row_header, parse_pdb_page_header, parse_pdb_playlist_entry_row,
    parse_pdb_playlist_tree_row, parse_pdb_table_directory, parse_pdb_track_row_header,
    pdb_track_string_page_offset, recover_pdb_title,
};

#[cfg(feature = "alloc")]
mod vec_stage;
#[cfg(feature = "alloc")]
pub use vec_stage::{VecCatalogStage, VecNameMap};

/// Filesystem operations the import needs, each issued under the import's
/// lease. Errors are the platform's own; the import reports them as
/// [`ImportError::Io`] and never retries.
#[allow(async_fn_in_trait)]
pub trait CatalogFiles {
    type File: FileHandle;
    /// Owned page-read buffer; one read never exceeds its length.
    type Buffer: AsRef<[u8]> + AsMut<[u8]>;
    type Error;

    async fn open_read(
        &mut self,
        lease: MediaLease,
        path: BoundedPath,
    ) -> Result<Self::File, Self::Error>;
    /// Seek to `position` and read into `buffer`. The file and buffer always
    /// return to the caller.
    async fn read_at(
        &mut self,
        lease: MediaLease,
        file: Self::File,
        buffer: Self::Buffer,
        position: u64,
    ) -> (Self::File, Self::Buffer, Result<usize, Self::Error>);
    async fn close_file(&mut self, lease: MediaLease, file: Self::File) -> Result<(), Self::Error>;
}

/// A stage could not reserve its fixed capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageAllocationError;

/// Why a staged catalog could not be built or published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    DuplicateNameId(u32),
    CatalogBankUnavailable,
    CatalogNameCapacityExceeded,
    CatalogTableTruncated,
    CatalogDecode(PdbCatalogError),
    CatalogBuild(CatalogBuildError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportError<E> {
    Io(E),
    PathTooLong,
    Catalog(CatalogError),
    InvalidTrackPathUtf8,
    Identity(IdentityError),
    MissingTracksTable,
    PageOutOfRange(u32),
    PageChainLoop,
    RowCountOverflow,
    Pdb(PdbError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbTableProbe {
    pub pages: u32,
    pub declared_rows: u32,
    pub present_rows: u32,
    pub track_rows: u32,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbNameTableProbe {
    pub pages: u32,
    pub declared_rows: u32,
    pub present_rows: u32,
    pub names: u32,
    pub slot_truncated: bool,
    pub truncated_values: u32,
    /// Rows whose name lies outside the row (or page), skipped as in M2.5.
    pub out_of_row: u32,
    pub capacity_exceeded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdbProbe {
    pub lease: MediaLease,
    pub volume_identity: VolumeIdentity,
    pub file_len: u64,
    pub header: PdbHeader,
    pub tracks_table: PdbTableEntry,
    pub artists_table: Option<PdbTableEntry>,
    pub albums_table: Option<PdbTableEntry>,
    pub keys_table: Option<PdbTableEntry>,
    pub playlist_tree_table: Option<PdbTableEntry>,
    pub playlist_entries_table: Option<PdbTableEntry>,
    pub artwork_table: Option<PdbTableEntry>,
    pub tracks_layout: Option<PdbTableProbe>,
    pub artists_layout: Option<PdbNameTableProbe>,
    pub albums_layout: Option<PdbNameTableProbe>,
    pub keys_layout: Option<PdbNameTableProbe>,
    /// None when absent; a malformed artwork table is dropped, not fatal.
    pub artwork_layout: Option<PdbNameTableProbe>,
    /// Published hierarchy counts; invalid rows never fail the track catalog.
    pub playlists: Option<PlaylistImportStats>,
}

/// One staged name table (artists, albums, keys or artwork paths).
pub trait PdbNameStore: PdbNameLookup {
    fn clear(&mut self);
    fn retain(&mut self, entry: PdbNameEntry) -> Result<(), CatalogError>;
}

/// The name tables a track row resolves against.
pub struct PdbNameStage<M> {
    pub artists: M,
    pub albums: M,
    pub keys: M,
    /// Cover paths; at most one per catalog track (RP-07).
    pub artworks: M,
}

impl<M: PdbNameStore> PdbNameStage<M> {
    pub fn clear(&mut self) {
        self.artists.clear();
        self.albums.clear();
        self.keys.clear();
        self.artworks.clear();
    }
}

/// Name-table capacities: artwork rows beyond one per track are dropped.
pub const PDB_NAME_CAPACITY: usize = PDB_MAX_NAMES;
pub const PDB_ARTWORK_CAPACITY: usize = MEDIA_CATALOG_MAX_TRACKS;

/// Staged catalog and playlists for one import. `begin` starts a build,
/// `publish` makes catalog and playlists current together, and `abort`
/// discards an unfinished build.
pub trait CatalogStage<M> {
    fn validate_names(&self, probe: &PdbProbe) -> Result<(), CatalogError>;
    fn validate_tracks(&self, probe: PdbTableProbe) -> Result<(), CatalogError>;
    fn begin(&mut self, lease: MediaLease) -> Result<(), CatalogError>;
    fn retain_track(
        &mut self,
        volume_identity: VolumeIdentity,
        track: &PdbTrackRow,
        names: &PdbNameStage<M>,
    ) -> Result<(), CatalogError>;
    /// False stops the playlist walk (the builder reached its bound).
    fn retain_playlist_node(&mut self, row: PdbPlaylistTreeRow, name: &str) -> bool;
    fn retain_playlist_entry(&mut self, row: PdbPlaylistEntryRow) -> bool;
    fn note_playlist_invalid_row(&mut self);
    fn note_playlist_truncated(&mut self);
    fn publish(&mut self) -> Result<Option<PlaylistImportStats>, CatalogError>;
    fn abort(&mut self);
}

enum PrefixReadResult<F: CatalogFiles> {
    Complete {
        file: F::File,
        buffer: F::Buffer,
        bytes: usize,
    },
    Failed {
        file: F::File,
        buffer: F::Buffer,
        error: ImportError<F::Error>,
    },
}

async fn read_at<F: CatalogFiles>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    position: u64,
) -> PrefixReadResult<F> {
    match files.read_at(lease, file, buffer, position).await {
        (file, buffer, Ok(bytes)) => PrefixReadResult::Complete {
            bytes: bytes.min(buffer.as_ref().len()),
            file,
            buffer,
        },
        (file, buffer, Err(error)) => PrefixReadResult::Failed {
            file,
            buffer,
            error: ImportError::Io(error),
        },
    }
}

async fn open_export_pdb<F: CatalogFiles>(
    files: &mut F,
    lease: MediaLease,
) -> Result<F::File, ImportError<F::Error>> {
    let path = BoundedPath::new(REKORDBOX_EXPORT_PDB_PATH).map_err(|_| ImportError::PathTooLong)?;
    files.open_read(lease, path).await.map_err(ImportError::Io)
}

async fn close_file<F: CatalogFiles>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
) -> Result<(), ImportError<F::Error>> {
    files.close_file(lease, file).await.map_err(ImportError::Io)
}

/// I/O failure of [`FsCatalogFiles`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsCatalogError<E> {
    /// The lease ended before, during or after the operation.
    Stale,
    Fs(E),
}

/// [`CatalogFiles`] over one mounted [`AsyncFileSystem`] owner. The shared
/// filesystems do not fence their caches by lease, so `is_current` is checked
/// before and after every operation; only a still-live lease yields data.
pub struct FsCatalogFiles<'a, V, C, B> {
    volume: &'a mut V,
    is_current: C,
    _buffer: core::marker::PhantomData<fn(B) -> B>,
}

impl<'a, V: AsyncFileSystem, C: Fn() -> bool, B> FsCatalogFiles<'a, V, C, B> {
    pub fn new(volume: &'a mut V, is_current: C) -> Self {
        Self {
            volume,
            is_current,
            _buffer: core::marker::PhantomData,
        }
    }

    fn live(&self, lease: MediaLease) -> Result<(), FsCatalogError<V::Error>> {
        if self.volume.lease() == lease && (self.is_current)() {
            Ok(())
        } else {
            Err(FsCatalogError::Stale)
        }
    }
}

impl<V, C, B> CatalogFiles for FsCatalogFiles<'_, V, C, B>
where
    V: AsyncFileSystem,
    C: Fn() -> bool,
    B: AsRef<[u8]> + AsMut<[u8]>,
{
    type File = V::File;
    type Buffer = B;
    type Error = FsCatalogError<V::Error>;

    async fn open_read(
        &mut self,
        lease: MediaLease,
        path: BoundedPath,
    ) -> Result<V::File, Self::Error> {
        self.live(lease)?;
        let file = self
            .volume
            .open_read(path.as_str())
            .await
            .map_err(FsCatalogError::Fs)?;
        if file.lease() == lease && self.live(lease).is_ok() {
            Ok(file)
        } else {
            let _ = self.volume.close_file(file).await;
            Err(FsCatalogError::Stale)
        }
    }

    async fn read_at(
        &mut self,
        lease: MediaLease,
        mut file: V::File,
        mut buffer: B,
        position: u64,
    ) -> (V::File, B, Result<usize, Self::Error>) {
        if let Err(error) = self.live(lease) {
            return (file, buffer, Err(error));
        }
        if let Err(error) = self.volume.seek_absolute(&mut file, position).await {
            return (file, buffer, Err(FsCatalogError::Fs(error)));
        }
        let output = buffer.as_mut();
        let mut filled = 0;
        while filled < output.len() {
            if let Err(error) = self.live(lease) {
                return (file, buffer, Err(error));
            }
            match self.volume.read(&mut file, &mut output[filled..]).await {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) => return (file, buffer, Err(FsCatalogError::Fs(error))),
            }
        }
        let result = self.live(lease).map(|()| filled);
        (file, buffer, result)
    }

    async fn close_file(&mut self, lease: MediaLease, file: V::File) -> Result<(), Self::Error> {
        self.volume
            .close_file(file)
            .await
            .map_err(FsCatalogError::Fs)?;
        self.live(lease)
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_track_text<F: CatalogFiles, const N: usize>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    page_start: u64,
    page_size: u32,
    heap_offset: u16,
    relative_offset: u16,
) -> (
    F::File,
    F::Buffer,
    Result<PdbText<N>, ImportError<F::Error>>,
) {
    let page_relative = match pdb_track_string_page_offset(heap_offset, relative_offset, page_size)
    {
        Ok(Some(offset)) => offset,
        Ok(None) => return (file, buffer, Ok(PdbText::new())),
        Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
    };
    let absolute = match page_start.checked_add(u64::from(page_relative)) {
        Some(position) => position,
        None => {
            return (
                file,
                buffer,
                Err(ImportError::Pdb(PdbError::TrackStringOffsetOutOfRange)),
            );
        }
    };

    let read = read_at(files, lease, file, buffer, absolute).await;
    let (file, buffer, bytes) = match read {
        PrefixReadResult::Complete {
            file,
            buffer,
            bytes,
        } => (file, buffer, bytes),
        PrefixReadResult::Failed {
            file,
            buffer,
            error,
        } => return (file, buffer, Err(error)),
    };
    let available = bytes.min(buffer.as_ref().len());

    let field = match parse_devicesql_field_header(&buffer.as_ref()[..available], 0) {
        Ok(field) => field,
        Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
    };
    let field_len = field.total_len_usize();
    let field_end = match page_relative.checked_add(field.total_len as u32) {
        Some(end) => end,
        None => {
            return (
                file,
                buffer,
                Err(ImportError::Pdb(PdbError::DeviceSqlTruncated)),
            );
        }
    };
    if field_end > page_size || field_len > available {
        return (
            file,
            buffer,
            Err(ImportError::Pdb(PdbError::DeviceSqlTruncated)),
        );
    }

    let text =
        PdbText::from_devicesql_field(&buffer.as_ref()[..field_len]).map_err(ImportError::Pdb);
    (file, buffer, text)
}

/// Reads a name that must start and end before the page-relative `limit`
/// (the row end for bounded name tables, else the page size); a name that
/// does not is `None`, as M2.5 082d2c0a drops such rows.
#[allow(clippy::too_many_arguments)]
async fn read_name_text<F: CatalogFiles>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    page_start: u64,
    limit: u32,
    heap_offset: u16,
    header: PdbNameRowHeader,
) -> (
    F::File,
    F::Buffer,
    Result<Option<PdbText<PDB_NAME_MAX>>, ImportError<F::Error>>,
) {
    let page_relative = match (PDB_PAGE_HEAP_OFFSET as u32)
        .checked_add(u32::from(heap_offset))
        .and_then(|offset| offset.checked_add(u32::from(header.text_relative_offset)))
    {
        Some(offset) if offset < limit => offset,
        _ => return (file, buffer, Ok(None)),
    };

    let absolute = match page_start.checked_add(u64::from(page_relative)) {
        Some(position) => position,
        None => {
            return (
                file,
                buffer,
                Err(ImportError::Pdb(PdbError::NameRowOutOfRange)),
            );
        }
    };

    let read = read_at(files, lease, file, buffer, absolute).await;
    let (file, buffer, bytes) = match read {
        PrefixReadResult::Complete {
            file,
            buffer,
            bytes,
        } => (file, buffer, bytes),
        PrefixReadResult::Failed {
            file,
            buffer,
            error,
        } => return (file, buffer, Err(error)),
    };
    let available = bytes.min(buffer.as_ref().len());

    let field = match parse_devicesql_field_header(&buffer.as_ref()[..available], 0) {
        Ok(field) => field,
        Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
    };
    let field_len = field.total_len_usize();
    if page_relative + u32::from(field.total_len) > limit {
        return (file, buffer, Ok(None));
    }
    if field_len > available {
        return (
            file,
            buffer,
            Err(ImportError::Pdb(PdbError::DeviceSqlTruncated)),
        );
    }

    let text = PdbText::from_devicesql_field(&buffer.as_ref()[..field_len])
        .map(Some)
        .map_err(ImportError::Pdb);
    (file, buffer, text)
}

/// Lowers `ends` by every slot batch of the page except `seen`, which the
/// caller has already observed.
#[allow(clippy::too_many_arguments)]
async fn observe_other_batches<F: CatalogFiles>(
    files: &mut F,
    lease: MediaLease,
    mut file: F::File,
    mut buffer: F::Buffer,
    page_start: u64,
    page_size: u32,
    declared_rows: u32,
    seen: PdbSlotBatchPlan,
    ends: &mut PdbRowEnds,
) -> (F::File, F::Buffer, Result<(), ImportError<F::Error>>) {
    let mut scan = match PdbPageSlotWalker::new(page_size, declared_rows) {
        Ok(scan) => scan,
        Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
    };
    while let Some(plan) = scan.next_batch_plan() {
        let absolute = page_start + u64::from(plan.offset);
        let (next_file, next_buffer, bytes) =
            match read_at(files, lease, file, buffer, absolute).await {
                PrefixReadResult::Complete {
                    file,
                    buffer,
                    bytes,
                } => (file, buffer, bytes),
                PrefixReadResult::Failed {
                    file,
                    buffer,
                    error,
                } => return (file, buffer, Err(error)),
            };
        file = next_file;
        buffer = next_buffer;
        let data = &buffer.as_ref()[..bytes];
        let observed = if plan == seen {
            Ok(())
        } else {
            ends.observe(plan, data)
        };
        if let Err(error) = observed.and_then(|()| scan.ingest_batch(plan, data).map(|_| ())) {
            return (file, buffer, Err(ImportError::Pdb(error)));
        }
    }
    (file, buffer, Ok(()))
}

fn table(tables: &PdbTableDirectory, kind: u32) -> Option<PdbTableEntry> {
    tables
        .find(kind)
        .filter(|entry| entry.first_page != u32::MAX)
}

fn parse_first_page<E>(
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    file_len: u64,
    buffer: &[u8],
    bytes: usize,
) -> Result<PdbProbe, ImportError<E>> {
    let available = bytes.min(buffer.as_ref().len());
    let header = parse_pdb_header(&buffer[..available], file_len).map_err(ImportError::Pdb)?;
    let tables =
        parse_pdb_table_directory(&buffer[..available], header).map_err(ImportError::Pdb)?;
    let tracks_table = table(&tables, PDB_TABLE_TRACKS).ok_or(ImportError::MissingTracksTable)?;

    Ok(PdbProbe {
        lease,
        volume_identity,
        file_len,
        header,
        tracks_table,
        artists_table: table(&tables, PDB_TABLE_ARTISTS),
        albums_table: table(&tables, PDB_TABLE_ALBUMS),
        keys_table: table(&tables, PDB_TABLE_KEYS),
        playlist_tree_table: table(&tables, PDB_TABLE_PLAYLIST_TREE),
        playlist_entries_table: table(&tables, PDB_TABLE_PLAYLIST_ENTRIES),
        artwork_table: table(&tables, PDB_TABLE_ARTWORK),
        tracks_layout: None,
        artists_layout: None,
        albums_layout: None,
        keys_layout: None,
        artwork_layout: None,
        playlists: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn scan_table_layout<F: CatalogFiles, M: PdbNameStore, C: CatalogStage<M>>(
    files: &mut F,
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    mut file: F::File,
    mut buffer: F::Buffer,
    header: PdbHeader,
    table: PdbTableEntry,
    names: &PdbNameStage<M>,
    catalog: &mut C,
) -> (
    F::File,
    F::Buffer,
    Result<PdbTableProbe, ImportError<F::Error>>,
) {
    let mut page = table.first_page;
    let mut pages = 0u32;
    let mut declared_rows = 0u32;
    let mut present_rows = 0u32;
    let mut track_rows = 0u32;
    let mut truncated = false;

    loop {
        if page >= header.total_pages {
            return (file, buffer, Err(ImportError::PageOutOfRange(page)));
        }
        if pages >= header.total_pages {
            return (file, buffer, Err(ImportError::PageChainLoop));
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let header_read = read_at(files, lease, file, buffer, page_start).await;
        let (next_file, next_buffer, bytes) = match header_read {
            PrefixReadResult::Complete {
                file,
                buffer,
                bytes,
            } => (file, buffer, bytes),
            PrefixReadResult::Failed {
                file,
                buffer,
                error,
            } => return (file, buffer, Err(error)),
        };
        file = next_file;
        buffer = next_buffer;

        let page_header = match parse_pdb_page_header(&buffer.as_ref()[..bytes], header.page_size) {
            Ok(page_header) => page_header,
            Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
        };
        let mut walker = match PdbPageSlotWalker::new(header.page_size, page_header.declared_rows) {
            Ok(walker) => walker,
            Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let slot_read = read_at(files, lease, file, buffer, absolute).await;
            let (next_file, next_buffer, bytes) = match slot_read {
                PrefixReadResult::Complete {
                    file,
                    buffer,
                    bytes,
                } => (file, buffer, bytes),
                PrefixReadResult::Failed {
                    file,
                    buffer,
                    error,
                } => return (file, buffer, Err(error)),
            };
            file = next_file;
            buffer = next_buffer;

            let row_offsets = match walker.ingest_batch(plan, &buffer.as_ref()[..bytes]) {
                Ok(row_offsets) => row_offsets,
                Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
            };

            for heap_offset in row_offsets.offsets().iter().copied() {
                let page_relative =
                    match (PDB_PAGE_HEAP_OFFSET as u32).checked_add(u32::from(heap_offset)) {
                        Some(offset) => offset,
                        None => {
                            return (
                                file,
                                buffer,
                                Err(ImportError::Pdb(PdbError::TrackRowOutOfRange)),
                            );
                        }
                    };
                let row_end = match page_relative.checked_add(PDB_TRACK_ROW_HEADER_BYTES as u32) {
                    Some(end) => end,
                    None => {
                        return (
                            file,
                            buffer,
                            Err(ImportError::Pdb(PdbError::TrackRowOutOfRange)),
                        );
                    }
                };
                if row_end > header.page_size {
                    return (
                        file,
                        buffer,
                        Err(ImportError::Pdb(PdbError::TrackRowOutOfRange)),
                    );
                }

                let absolute = page_start + u64::from(page_relative);
                let row_read = read_at(files, lease, file, buffer, absolute).await;
                let (next_file, next_buffer, bytes) = match row_read {
                    PrefixReadResult::Complete {
                        file,
                        buffer,
                        bytes,
                    } => (file, buffer, bytes),
                    PrefixReadResult::Failed {
                        file,
                        buffer,
                        error,
                    } => return (file, buffer, Err(error)),
                };
                file = next_file;
                buffer = next_buffer;

                match parse_pdb_track_row_header(&buffer.as_ref()[..bytes]) {
                    Ok(Some(track_header)) => {
                        let (next_file, next_buffer, title) = read_track_text::<F, PDB_TITLE_MAX>(
                            files,
                            lease,
                            file,
                            buffer,
                            page_start,
                            header.page_size,
                            heap_offset,
                            track_header.strings.title,
                        )
                        .await;
                        file = next_file;
                        buffer = next_buffer;
                        // Malformed title encoding falls back to the filename
                        // (M2.5 parity); storage I/O failures still abort.
                        let title = match title {
                            Ok(title) => title,
                            Err(ImportError::Pdb(error)) => recover_pdb_title(Err(error)),
                            Err(error) => return (file, buffer, Err(error)),
                        };

                        let (next_file, next_buffer, filename) =
                            read_track_text::<F, PDB_TITLE_MAX>(
                                files,
                                lease,
                                file,
                                buffer,
                                page_start,
                                header.page_size,
                                heap_offset,
                                track_header.strings.filename,
                            )
                            .await;
                        file = next_file;
                        buffer = next_buffer;
                        let filename = match filename {
                            Ok(filename) => filename,
                            Err(error) => return (file, buffer, Err(error)),
                        };

                        let (next_file, next_buffer, file_path) =
                            read_track_text::<F, PDB_PATH_MAX>(
                                files,
                                lease,
                                file,
                                buffer,
                                page_start,
                                header.page_size,
                                heap_offset,
                                track_header.strings.file_path,
                            )
                            .await;
                        file = next_file;
                        buffer = next_buffer;
                        let file_path = match file_path {
                            Ok(file_path) => file_path,
                            Err(error) => return (file, buffer, Err(error)),
                        };

                        let (next_file, next_buffer, anlz_path) =
                            read_track_text::<F, PDB_PATH_MAX>(
                                files,
                                lease,
                                file,
                                buffer,
                                page_start,
                                header.page_size,
                                heap_offset,
                                track_header.strings.anlz_path,
                            )
                            .await;
                        file = next_file;
                        buffer = next_buffer;
                        let anlz_path = match anlz_path {
                            Ok(anlz_path) => anlz_path,
                            Err(error) => return (file, buffer, Err(error)),
                        };

                        let track = PdbTrackRow::from_header_and_fields(
                            track_header,
                            title,
                            filename,
                            file_path,
                            anlz_path,
                        );
                        if track.file_path.was_truncated() {
                            return (
                                file,
                                buffer,
                                Err(ImportError::Identity(IdentityError::PathTooLong)),
                            );
                        }
                        let audio_path = match track.file_path.as_str() {
                            Some(path) => path,
                            None => {
                                return (file, buffer, Err(ImportError::InvalidTrackPathUtf8));
                            }
                        };
                        if let Err(error) = derive_track_id(volume_identity, audio_path) {
                            return (file, buffer, Err(ImportError::Identity(error)));
                        }
                        if let Err(error) = catalog.retain_track(volume_identity, &track, names) {
                            return (file, buffer, Err(ImportError::Catalog(error)));
                        }

                        track_rows = match track_rows.checked_add(1) {
                            Some(total) => total,
                            None => {
                                return (file, buffer, Err(ImportError::RowCountOverflow));
                            }
                        };
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return (file, buffer, Err(ImportError::Pdb(error)));
                    }
                }
            }
        }

        let summary = walker.summary();
        declared_rows = match declared_rows.checked_add(summary.declared_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(ImportError::RowCountOverflow)),
        };
        present_rows = match present_rows.checked_add(summary.present_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(ImportError::RowCountOverflow)),
        };
        truncated |= summary.truncated;
        pages += 1;

        let Some(next_page) = page_header.next_in_table(page, table) else {
            break;
        };
        if next_page >= header.total_pages {
            return (file, buffer, Err(ImportError::PageOutOfRange(next_page)));
        }
        page = next_page;
    }

    (
        file,
        buffer,
        Ok(PdbTableProbe {
            pages,
            declared_rows,
            present_rows,
            track_rows,
            truncated,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
async fn scan_name_table_layout<F: CatalogFiles, M: PdbNameStore>(
    files: &mut F,
    lease: MediaLease,
    mut file: F::File,
    mut buffer: F::Buffer,
    header: PdbHeader,
    table: PdbTableEntry,
    layout: PdbNameRowLayout,
    stage: &mut M,
) -> (
    F::File,
    F::Buffer,
    Result<PdbNameTableProbe, ImportError<F::Error>>,
) {
    let mut page = table.first_page;
    let mut pages = 0u32;
    let mut declared_rows = 0u32;
    let mut present_rows = 0u32;
    let mut names = 0u32;
    let mut slot_truncated = false;
    let mut truncated_values = 0u32;
    let mut out_of_row = 0u32;

    loop {
        if page >= header.total_pages {
            return (file, buffer, Err(ImportError::PageOutOfRange(page)));
        }
        if pages >= header.total_pages {
            return (file, buffer, Err(ImportError::PageChainLoop));
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let header_read = read_at(files, lease, file, buffer, page_start).await;
        let (next_file, next_buffer, bytes) = match header_read {
            PrefixReadResult::Complete {
                file,
                buffer,
                bytes,
            } => (file, buffer, bytes),
            PrefixReadResult::Failed {
                file,
                buffer,
                error,
            } => return (file, buffer, Err(error)),
        };
        file = next_file;
        buffer = next_buffer;

        let page_header = match parse_pdb_page_header(&buffer.as_ref()[..bytes], header.page_size) {
            Ok(page_header) => page_header,
            Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
        };
        let mut walker = match PdbPageSlotWalker::new(header.page_size, page_header.declared_rows) {
            Ok(walker) => walker,
            Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let slot_read = read_at(files, lease, file, buffer, absolute).await;
            let (next_file, next_buffer, bytes) = match slot_read {
                PrefixReadResult::Complete {
                    file,
                    buffer,
                    bytes,
                } => (file, buffer, bytes),
                PrefixReadResult::Failed {
                    file,
                    buffer,
                    error,
                } => return (file, buffer, Err(error)),
            };
            file = next_file;
            buffer = next_buffer;

            let row_offsets = match walker.ingest_batch(plan, &buffer.as_ref()[..bytes]) {
                Ok(row_offsets) => row_offsets,
                Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
            };
            let mut ends =
                PdbRowEnds::new(&row_offsets, header.page_size, page_header.declared_rows);
            if let Err(error) = ends.observe(plan, &buffer.as_ref()[..bytes]) {
                return (file, buffer, Err(ImportError::Pdb(error)));
            }
            if layout.bounded_to_row()
                && page_header.declared_rows as usize > PDB_ROW_SLOT_BATCH_ROWS
            {
                // Rows of a multi-batch page end at slots of the other batches too.
                let (next_file, next_buffer, observed) = observe_other_batches(
                    files,
                    lease,
                    file,
                    buffer,
                    page_start,
                    header.page_size,
                    page_header.declared_rows,
                    plan,
                    &mut ends,
                )
                .await;
                file = next_file;
                buffer = next_buffer;
                if let Err(error) = observed {
                    return (file, buffer, Err(error));
                }
            }

            for (index, heap_offset) in row_offsets.offsets().iter().copied().enumerate() {
                let page_relative =
                    match (PDB_PAGE_HEAP_OFFSET as u32).checked_add(u32::from(heap_offset)) {
                        Some(offset) => offset,
                        None => {
                            return (
                                file,
                                buffer,
                                Err(ImportError::Pdb(PdbError::NameRowOutOfRange)),
                            );
                        }
                    };
                let limit = if layout.bounded_to_row() {
                    let row_len = ends.row_len(index);
                    if row_len < layout.minimum_bytes() as u32 {
                        out_of_row = out_of_row.saturating_add(1);
                        continue;
                    }
                    page_relative + row_len
                } else {
                    header.page_size
                };
                let row_end = match page_relative.checked_add(layout.minimum_bytes() as u32) {
                    Some(end) => end,
                    None => {
                        return (
                            file,
                            buffer,
                            Err(ImportError::Pdb(PdbError::NameRowOutOfRange)),
                        );
                    }
                };
                if row_end > header.page_size {
                    return (
                        file,
                        buffer,
                        Err(ImportError::Pdb(PdbError::NameRowOutOfRange)),
                    );
                }

                let absolute = page_start + u64::from(page_relative);
                let row_read = read_at(files, lease, file, buffer, absolute).await;
                let (next_file, next_buffer, bytes) = match row_read {
                    PrefixReadResult::Complete {
                        file,
                        buffer,
                        bytes,
                    } => (file, buffer, bytes),
                    PrefixReadResult::Failed {
                        file,
                        buffer,
                        error,
                    } => return (file, buffer, Err(error)),
                };
                file = next_file;
                buffer = next_buffer;

                let row_header = match parse_pdb_name_row_header(&buffer.as_ref()[..bytes], layout)
                {
                    Ok(Some(header)) => header,
                    Ok(None) => continue,
                    Err(error) => return (file, buffer, Err(ImportError::Pdb(error))),
                };

                let (next_file, next_buffer, name) = read_name_text(
                    files,
                    lease,
                    file,
                    buffer,
                    page_start,
                    limit,
                    heap_offset,
                    row_header,
                )
                .await;
                file = next_file;
                buffer = next_buffer;
                let name = match name {
                    Ok(Some(name)) => name,
                    Ok(None) => {
                        out_of_row = out_of_row.saturating_add(1);
                        continue;
                    }
                    Err(error) => return (file, buffer, Err(error)),
                };

                if name.was_truncated() {
                    truncated_values = match truncated_values.checked_add(1) {
                        Some(total) => total,
                        None => {
                            return (file, buffer, Err(ImportError::RowCountOverflow));
                        }
                    };
                }
                if let Some(entry) = PdbNameEntry::from_header_and_field(row_header, name) {
                    names = match names.checked_add(1) {
                        Some(total) => total,
                        None => {
                            return (file, buffer, Err(ImportError::RowCountOverflow));
                        }
                    };
                    if let Err(error) = stage.retain(entry) {
                        return (file, buffer, Err(ImportError::Catalog(error)));
                    }
                }
            }
        }

        let summary = walker.summary();
        declared_rows = match declared_rows.checked_add(summary.declared_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(ImportError::RowCountOverflow)),
        };
        present_rows = match present_rows.checked_add(summary.present_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(ImportError::RowCountOverflow)),
        };
        slot_truncated |= summary.truncated;
        pages += 1;

        let Some(next_page) = page_header.next_in_table(page, table) else {
            break;
        };
        if next_page >= header.total_pages {
            return (file, buffer, Err(ImportError::PageOutOfRange(next_page)));
        }
        page = next_page;
    }

    (
        file,
        buffer,
        Ok(PdbNameTableProbe {
            pages,
            declared_rows,
            present_rows,
            names,
            slot_truncated,
            truncated_values,
            out_of_row,
            capacity_exceeded: names > PDB_MAX_NAMES as u32,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
async fn scan_optional_name_table<F: CatalogFiles, M: PdbNameStore>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    header: PdbHeader,
    table: Option<PdbTableEntry>,
    layout: PdbNameRowLayout,
    stage: &mut M,
) -> (
    F::File,
    F::Buffer,
    Result<Option<PdbNameTableProbe>, ImportError<F::Error>>,
) {
    let Some(table) = table else {
        return (file, buffer, Ok(None));
    };

    let (file, buffer, result) =
        scan_name_table_layout(files, lease, file, buffer, header, table, layout, stage).await;
    (file, buffer, result.map(Some))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlaylistTable {
    Tree,
    Entries,
}

/// Walks a playlist table into the staged builder (RP-06). Storage errors
/// abort the import; malformed rows are counted, and a broken page chain or
/// reached bound stops this table as truncated without failing the tracks.
#[allow(clippy::too_many_arguments)]
async fn scan_playlist_table<F: CatalogFiles, M: PdbNameStore, C: CatalogStage<M>>(
    files: &mut F,
    lease: MediaLease,
    mut file: F::File,
    mut buffer: F::Buffer,
    header: PdbHeader,
    pdb_table: PdbTableEntry,
    table: PlaylistTable,
    catalog: &mut C,
) -> (F::File, F::Buffer, Result<(), ImportError<F::Error>>) {
    let row_bytes = match table {
        PlaylistTable::Tree => PDB_PLAYLIST_TREE_ROW_HEADER_BYTES as u32 + 1,
        PlaylistTable::Entries => PDB_PLAYLIST_ENTRY_ROW_BYTES as u32,
    };
    let mut page = pdb_table.first_page;
    let mut pages = 0u32;

    'pages: loop {
        if page >= header.total_pages || pages >= header.total_pages {
            catalog.note_playlist_truncated();
            break;
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let (next_file, next_buffer, bytes) =
            match read_at(files, lease, file, buffer, page_start).await {
                PrefixReadResult::Complete {
                    file,
                    buffer,
                    bytes,
                } => (file, buffer, bytes),
                PrefixReadResult::Failed {
                    file,
                    buffer,
                    error,
                } => return (file, buffer, Err(error)),
            };
        file = next_file;
        buffer = next_buffer;

        let Ok(page_header) = parse_pdb_page_header(&buffer.as_ref()[..bytes], header.page_size)
        else {
            catalog.note_playlist_truncated();
            break;
        };
        let Ok(mut walker) = PdbPageSlotWalker::new(header.page_size, page_header.declared_rows)
        else {
            catalog.note_playlist_truncated();
            break;
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let (next_file, next_buffer, bytes) =
                match read_at(files, lease, file, buffer, absolute).await {
                    PrefixReadResult::Complete {
                        file,
                        buffer,
                        bytes,
                    } => (file, buffer, bytes),
                    PrefixReadResult::Failed {
                        file,
                        buffer,
                        error,
                    } => return (file, buffer, Err(error)),
                };
            file = next_file;
            buffer = next_buffer;

            let Ok(row_offsets) = walker.ingest_batch(plan, &buffer.as_ref()[..bytes]) else {
                catalog.note_playlist_truncated();
                break 'pages;
            };

            for heap_offset in row_offsets.offsets().iter().copied() {
                let page_relative = PDB_PAGE_HEAP_OFFSET as u32 + u32::from(heap_offset);
                if page_relative.saturating_add(row_bytes) > header.page_size {
                    catalog.note_playlist_invalid_row();
                    continue;
                }

                let absolute = page_start + u64::from(page_relative);
                let (next_file, next_buffer, bytes) =
                    match read_at(files, lease, file, buffer, absolute).await {
                        PrefixReadResult::Complete {
                            file,
                            buffer,
                            bytes,
                        } => (file, buffer, bytes),
                        PrefixReadResult::Failed {
                            file,
                            buffer,
                            error,
                        } => return (file, buffer, Err(error)),
                    };
                file = next_file;
                buffer = next_buffer;

                let keep_reading = match table {
                    PlaylistTable::Entries => {
                        match parse_pdb_playlist_entry_row(&buffer.as_ref()[..bytes]) {
                            Ok(Some(row)) => catalog.retain_playlist_entry(row),
                            Ok(None) | Err(_) => {
                                catalog.note_playlist_invalid_row();
                                true
                            }
                        }
                    }
                    PlaylistTable::Tree => {
                        match parse_pdb_playlist_tree_row(&buffer.as_ref()[..bytes]) {
                            Ok(Some(row)) => {
                                let (next_file, next_buffer, name) = read_name_text(
                                    files,
                                    lease,
                                    file,
                                    buffer,
                                    page_start,
                                    header.page_size,
                                    heap_offset,
                                    row.name,
                                )
                                .await;
                                file = next_file;
                                buffer = next_buffer;
                                match name {
                                    Ok(name) => match name.as_ref().and_then(PdbText::as_str) {
                                        Some(name) => catalog.retain_playlist_node(row, name),
                                        None => {
                                            catalog.note_playlist_invalid_row();
                                            true
                                        }
                                    },
                                    Err(ImportError::Pdb(_)) => {
                                        catalog.note_playlist_invalid_row();
                                        true
                                    }
                                    Err(error) => return (file, buffer, Err(error)),
                                }
                            }
                            Ok(None) | Err(_) => {
                                catalog.note_playlist_invalid_row();
                                true
                            }
                        }
                    }
                };
                if !keep_reading {
                    break 'pages;
                }
            }
        }

        if walker.summary().truncated {
            catalog.note_playlist_truncated();
        }
        pages += 1;
        let Some(next_page) = page_header.next_in_table(page, pdb_table) else {
            break;
        };
        page = next_page;
    }

    (file, buffer, Ok(()))
}

#[allow(clippy::too_many_arguments)]
async fn scan_optional_playlist_table<F: CatalogFiles, M: PdbNameStore, C: CatalogStage<M>>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    header: PdbHeader,
    pdb_table: Option<PdbTableEntry>,
    table: PlaylistTable,
    catalog: &mut C,
) -> (F::File, F::Buffer, Result<(), ImportError<F::Error>>) {
    match pdb_table {
        Some(pdb_table) => {
            scan_playlist_table(
                files, lease, file, buffer, header, pdb_table, table, catalog,
            )
            .await
        }
        None => (file, buffer, Ok(())),
    }
}

async fn scan_probe_tables<F: CatalogFiles, M: PdbNameStore, C: CatalogStage<M>>(
    files: &mut F,
    lease: MediaLease,
    file: F::File,
    buffer: F::Buffer,
    mut probe: PdbProbe,
    staging: &mut PdbNameStage<M>,
    catalog: &mut C,
) -> (F::File, F::Buffer, Result<PdbProbe, ImportError<F::Error>>) {
    let (file, buffer, artists) = scan_optional_name_table(
        files,
        lease,
        file,
        buffer,
        probe.header,
        probe.artists_table,
        PdbNameRowLayout::Artist,
        &mut staging.artists,
    )
    .await;
    probe.artists_layout = match artists {
        Ok(artists) => artists,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, albums) = scan_optional_name_table(
        files,
        lease,
        file,
        buffer,
        probe.header,
        probe.albums_table,
        PdbNameRowLayout::Album,
        &mut staging.albums,
    )
    .await;
    probe.albums_layout = match albums {
        Ok(albums) => albums,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, keys) = scan_optional_name_table(
        files,
        lease,
        file,
        buffer,
        probe.header,
        probe.keys_table,
        PdbNameRowLayout::Key,
        &mut staging.keys,
    )
    .await;
    probe.keys_layout = match keys {
        Ok(keys) => keys,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, artworks) = scan_optional_name_table(
        files,
        lease,
        file,
        buffer,
        probe.header,
        probe.artwork_table,
        PdbNameRowLayout::Artwork,
        &mut staging.artworks,
    )
    .await;
    probe.artwork_layout = match artworks {
        Ok(artworks) => artworks,
        // Covers are optional: a malformed table leaves every track on the
        // fallback; storage errors still abort.
        Err(ImportError::Pdb(_) | ImportError::PageOutOfRange(_))
        | Err(ImportError::PageChainLoop) => {
            staging.artworks.clear();
            None
        }
        Err(error) => return (file, buffer, Err(error)),
    };

    if let Err(error) = catalog.validate_names(&probe) {
        return (file, buffer, Err(ImportError::Catalog(error)));
    }
    if let Err(error) = catalog.begin(lease) {
        return (file, buffer, Err(ImportError::Catalog(error)));
    }

    let (mut file, mut buffer, tracks) = scan_table_layout(
        files,
        lease,
        probe.volume_identity,
        file,
        buffer,
        probe.header,
        probe.tracks_table,
        staging,
        catalog,
    )
    .await;
    let tracks = match tracks {
        Ok(tracks) => tracks,
        Err(error) => return (file, buffer, Err(error)),
    };
    if let Err(error) = catalog.validate_tracks(tracks) {
        return (file, buffer, Err(ImportError::Catalog(error)));
    }
    probe.tracks_layout = Some(tracks);

    // Entries are only meaningful with a tree to resolve them against.
    let entries_table = probe.playlist_tree_table.and(probe.playlist_entries_table);
    for (table, pdb_table) in [
        (PlaylistTable::Tree, probe.playlist_tree_table),
        (PlaylistTable::Entries, entries_table),
    ] {
        let (next_file, next_buffer, result) = scan_optional_playlist_table(
            files,
            lease,
            file,
            buffer,
            probe.header,
            pdb_table,
            table,
            catalog,
        )
        .await;
        file = next_file;
        buffer = next_buffer;
        if let Err(error) = result {
            return (file, buffer, Err(error));
        }
    }

    (file, buffer, Ok(probe))
}

pub async fn probe_export_pdb<F: CatalogFiles, M: PdbNameStore, C: CatalogStage<M>>(
    files: &mut F,
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    buffer: F::Buffer,
    staging: &mut PdbNameStage<M>,
    catalog: &mut C,
) -> (F::Buffer, Result<PdbProbe, ImportError<F::Error>>) {
    let file = match open_export_pdb(files, lease).await {
        Ok(file) => file,
        Err(error) => return (buffer, Err(error)),
    };
    let file_len = file.len();

    let (file, buffer, read_result) = match read_at(files, lease, file, buffer, 0).await {
        PrefixReadResult::Complete {
            file,
            buffer,
            bytes,
        } => (file, buffer, Ok(bytes)),
        PrefixReadResult::Failed {
            file,
            buffer,
            error,
        } => (file, buffer, Err(error)),
    };

    let parse_result = match read_result {
        Ok(bytes) => parse_first_page(lease, volume_identity, file_len, buffer.as_ref(), bytes),
        Err(error) => Err(error),
    };

    let (file, buffer, parse_result) = match parse_result {
        Ok(probe) => scan_probe_tables(files, lease, file, buffer, probe, staging, catalog).await,
        Err(error) => (file, buffer, Err(error)),
    };

    let close_result = close_file(files, lease, file).await;
    let result = match parse_result {
        Err(error) => {
            catalog.abort();
            Err(error)
        }
        Ok(mut probe) => match close_result {
            Ok(()) => match catalog.publish() {
                Ok(playlists) => {
                    probe.playlists = playlists;
                    Ok(probe)
                }
                Err(error) => {
                    catalog.abort();
                    Err(ImportError::Catalog(error))
                }
            },
            Err(error) => {
                catalog.abort();
                Err(error)
            }
        },
    };

    (buffer, result)
}

#[cfg(all(test, feature = "alloc"))]
mod tests;
