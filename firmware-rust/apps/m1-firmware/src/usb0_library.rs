use core::cell::RefCell;

#[cfg(feature = "p4-psram-storage")]
use alloc::vec::Vec;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    signal::Signal,
};
#[cfg(feature = "p4-psram-storage")]
use pajoniiir_media_catalog::playlists::{
    CATALOG_MAX_PLAYLIST_ENTRIES, CATALOG_MAX_PLAYLIST_NODES, PlaylistBuilder, PlaylistEntry,
    PlaylistIndex, PlaylistNode, PlaylistStore,
};
use pajoniiir_media_catalog::playlists::{LibraryActivation, LibraryBrowser, PlaylistImportStats};
use pajoniiir_media_catalog::{
    CatalogBank, CatalogBankBuilder, CatalogBankPublisher, CatalogBuildError, CatalogLoadCandidate,
    CatalogPublicationId, CatalogTrack, MEDIA_CATALOG_MAX_TRACKS,
};
use pajoniiir_media_fs::{BoundedPath, FileHandle};
use pajoniiir_media_identity::{IdentityError, VolumeIdentity, derive_track_id};
use pajoniiir_media_session::MediaLease;
use pajoniiir_rekordbox::{
    PDB_MAX_NAMES, PDB_NAME_MAX, PDB_PAGE_HEAP_OFFSET, PDB_PATH_MAX, PDB_PLAYLIST_ENTRY_ROW_BYTES,
    PDB_PLAYLIST_TREE_ROW_HEADER_BYTES, PDB_TABLE_ALBUMS, PDB_TABLE_ARTISTS, PDB_TABLE_ARTWORK,
    PDB_TABLE_KEYS, PDB_TABLE_PLAYLIST_ENTRIES, PDB_TABLE_PLAYLIST_TREE, PDB_TABLE_TRACKS,
    PDB_TITLE_MAX, PDB_TRACK_ROW_HEADER_BYTES, PdbCatalogError, PdbError, PdbHeader, PdbNameEntry,
    PdbNameLookup, PdbNameRowHeader, PdbNameRowLayout, PdbPageSlotWalker, PdbPlaylistEntryRow,
    PdbPlaylistTreeRow, PdbTableDirectory, PdbText, PdbTrackRow, REKORDBOX_EXPORT_PDB_PATH,
    catalog_track_from_pdb, parse_devicesql_field_header, parse_pdb_header,
    parse_pdb_name_row_header, parse_pdb_page_header, parse_pdb_playlist_entry_row,
    parse_pdb_playlist_tree_row, parse_pdb_table_directory, parse_pdb_track_row_header,
    pdb_track_string_page_offset, recover_pdb_title, resolve_pdb_track_names,
};
use static_cell::StaticCell;

use crate::product_runtime::{LibraryEffect, set_library_cursor_on_node};
use crate::usb0_msc::{
    USB0_LIBRARY_EVENTS, Usb0FilesystemClient, Usb0FsCompletion, Usb0FsFileHandle,
    Usb0FsOperationError, Usb0FsRequest, Usb0FsSubmitResult, Usb0LibraryEvent, Usb0MscBinding,
};

const USB0_LIBRARY_PDB_IO_BYTES: usize = 8_192;

static USB0_LIBRARY_PDB_IO: StaticCell<[u8; USB0_LIBRARY_PDB_IO_BYTES]> = StaticCell::new();

static USB0_LIBRARY_STATE: CriticalSectionMutex<RefCell<Usb0LibraryState>> =
    CriticalSectionMutex::new(RefCell::new(Usb0LibraryState::Idle));

const USB0_LIBRARY_COMMAND_DEPTH: usize = 1;
const USB0_LIBRARY_COMPLETION_DEPTH: usize = 1;

pub(crate) static USB0_LIBRARY_COMMANDS: Channel<
    CriticalSectionRawMutex,
    Usb0LibraryCommand,
    USB0_LIBRARY_COMMAND_DEPTH,
> = Channel::new();

pub(crate) static USB0_LIBRARY_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    Usb0LibraryCompletion,
    USB0_LIBRARY_COMPLETION_DEPTH,
> = Channel::new();

static USB0_LIBRARY_PUBLICATION: CriticalSectionMutex<RefCell<Option<CatalogPublicationId>>> =
    CriticalSectionMutex::new(RefCell::new(None));
static USB0_LIBRARY_MEDIA_LEASE: CriticalSectionMutex<RefCell<Option<MediaLease>>> =
    CriticalSectionMutex::new(RefCell::new(None));
static USB0_LIBRARY_MEDIA_LEASE_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryCommand {
    pub(crate) request_id: u32,
    pub(crate) effect: LibraryEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryCommandError {
    CatalogUnavailable,
    EmptyCatalog,
    StaleSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryCompletion {
    Applied {
        request_id: u32,
    },
    LoadResolved {
        request_id: u32,
        deck: pajoniiir_controller_session::DeckId,
        candidate: CatalogLoadCandidate,
    },
    Failed {
        request_id: u32,
        error: Usb0LibraryCommandError,
    },
}

pub(crate) fn catalog_publication_matches(publication: CatalogPublicationId) -> bool {
    USB0_LIBRARY_PUBLICATION.lock(|cell| *cell.borrow() == Some(publication))
}

pub(crate) fn current_media_lease() -> Option<MediaLease> {
    USB0_LIBRARY_MEDIA_LEASE.lock(|cell| *cell.borrow())
}

pub(crate) async fn wait_for_media_lease_change() {
    USB0_LIBRARY_MEDIA_LEASE_CHANGED.wait().await;
}

fn set_media_lease(lease: MediaLease) {
    let changed = USB0_LIBRARY_MEDIA_LEASE.lock(|cell| {
        let mut current = cell.borrow_mut();
        if *current == Some(lease) {
            false
        } else {
            *current = Some(lease);
            true
        }
    });
    if changed {
        USB0_LIBRARY_MEDIA_LEASE_CHANGED.signal(());
    }
}

fn clear_media_lease(lease: MediaLease) {
    let changed = USB0_LIBRARY_MEDIA_LEASE.lock(|cell| {
        let mut current = cell.borrow_mut();
        if *current == Some(lease) {
            *current = None;
            true
        } else {
            false
        }
    });
    if changed {
        USB0_LIBRARY_MEDIA_LEASE_CHANGED.signal(());
    }
}

fn set_catalog_publication(publication: Option<CatalogPublicationId>) {
    USB0_LIBRARY_PUBLICATION.lock(|cell| {
        *cell.borrow_mut() = publication;
    });
}

/// Mirrors the cursor for load admission and stale-candidate checks.
fn publish_browser(browser: &LibraryBrowser) {
    set_catalog_publication(browser.publication());
    set_library_cursor_on_node(browser.on_node_row());
}

#[cfg(feature = "p4-psram-storage")]
struct Usb0LibraryNameMap {
    entries: Vec<PdbNameEntry>,
    capacity: usize,
    /// Optional tables (artwork) keep the first row of a repeated id
    /// instead of failing the import, as M2.5 drops such rows.
    optional: bool,
}

#[cfg(not(feature = "p4-psram-storage"))]
struct Usb0LibraryNameMap;

#[cfg(feature = "p4-psram-storage")]
impl PdbNameLookup for Usb0LibraryNameMap {
    fn lookup(&self, id: u32) -> Option<&PdbText<PDB_NAME_MAX>> {
        if id == 0 {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.name)
    }
}

impl Usb0LibraryNameMap {
    fn try_new(capacity: usize, optional: bool) -> Result<Self, ()> {
        #[cfg(feature = "p4-psram-storage")]
        {
            let mut entries = Vec::new();
            entries.try_reserve_exact(capacity).map_err(|_| ())?;
            Ok(Self {
                entries,
                capacity,
                optional,
            })
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        {
            let _ = (capacity, optional);
            Ok(Self)
        }
    }

    fn clear(&mut self) {
        #[cfg(feature = "p4-psram-storage")]
        self.entries.clear();
    }

    fn retain(&mut self, entry: PdbNameEntry) -> Result<(), Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        {
            if self.entries.iter().any(|existing| existing.id == entry.id) {
                if self.optional {
                    return Ok(());
                }
                return Err(Usb0LibraryError::DuplicateNameId(entry.id));
            }
            if self.entries.len() < self.capacity {
                // Capacity is reserved once at worker startup, so this push cannot
                // grow the heap while a PDB page is being processed.
                self.entries.push(entry);
            }
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = entry;

        Ok(())
    }
}

struct Usb0LibraryNameStage {
    artists: Usb0LibraryNameMap,
    albums: Usb0LibraryNameMap,
    keys: Usb0LibraryNameMap,
    /// Cover paths; at most one per catalog track (RP-07).
    artworks: Usb0LibraryNameMap,
}

impl Usb0LibraryNameStage {
    fn try_new() -> Result<Self, ()> {
        Ok(Self {
            artists: Usb0LibraryNameMap::try_new(PDB_MAX_NAMES, false)?,
            albums: Usb0LibraryNameMap::try_new(PDB_MAX_NAMES, false)?,
            keys: Usb0LibraryNameMap::try_new(PDB_MAX_NAMES, false)?,
            artworks: Usb0LibraryNameMap::try_new(MEDIA_CATALOG_MAX_TRACKS, true)?,
        })
    }

    fn clear(&mut self) {
        self.artists.clear();
        self.albums.clear();
        self.keys.clear();
        self.artworks.clear();
    }
}

#[cfg(feature = "p4-psram-storage")]
struct Usb0CatalogBank {
    tracks: Vec<CatalogTrack>,
}

#[cfg(feature = "p4-psram-storage")]
impl Usb0CatalogBank {
    fn try_new() -> Result<Self, ()> {
        let mut tracks = Vec::new();
        tracks
            .try_reserve_exact(MEDIA_CATALOG_MAX_TRACKS)
            .map_err(|_| ())?;
        Ok(Self { tracks })
    }
}

#[cfg(feature = "p4-psram-storage")]
impl CatalogBank for Usb0CatalogBank {
    fn capacity(&self) -> usize {
        MEDIA_CATALOG_MAX_TRACKS
    }

    fn tracks(&self) -> &[CatalogTrack] {
        &self.tracks
    }

    fn clear(&mut self) {
        self.tracks.clear();
    }

    fn push_prechecked(&mut self, track: CatalogTrack) -> usize {
        debug_assert!(self.tracks.len() < MEDIA_CATALOG_MAX_TRACKS);
        let index = self.tracks.len();
        self.tracks.push(track);
        index
    }
}

/// Playlist tree and entries reserved once in PSRAM, so a PDB walk never
/// grows the heap (RP-06).
#[cfg(feature = "p4-psram-storage")]
struct Usb0PlaylistStore {
    nodes: Vec<PlaylistNode>,
    entries: Vec<PlaylistEntry>,
}

#[cfg(feature = "p4-psram-storage")]
impl Usb0PlaylistStore {
    fn try_new() -> Result<Self, ()> {
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(CATALOG_MAX_PLAYLIST_NODES)
            .map_err(|_| ())?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(CATALOG_MAX_PLAYLIST_ENTRIES)
            .map_err(|_| ())?;
        Ok(Self { nodes, entries })
    }
}

#[cfg(feature = "p4-psram-storage")]
impl PlaylistStore for Usb0PlaylistStore {
    fn node_capacity(&self) -> usize {
        CATALOG_MAX_PLAYLIST_NODES
    }
    fn entry_capacity(&self) -> usize {
        CATALOG_MAX_PLAYLIST_ENTRIES
    }
    fn nodes(&self) -> &[PlaylistNode] {
        &self.nodes
    }
    fn nodes_mut(&mut self) -> &mut [PlaylistNode] {
        &mut self.nodes
    }
    fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }
    fn entries_mut(&mut self) -> &mut [PlaylistEntry] {
        &mut self.entries
    }
    fn clear(&mut self) {
        self.nodes.clear();
        self.entries.clear();
    }
    fn push_node_prechecked(&mut self, node: PlaylistNode) {
        debug_assert!(self.nodes.len() < CATALOG_MAX_PLAYLIST_NODES);
        self.nodes.push(node);
    }
    fn push_entry_prechecked(&mut self, entry: PlaylistEntry) {
        debug_assert!(self.entries.len() < CATALOG_MAX_PLAYLIST_ENTRIES);
        self.entries.push(entry);
    }
    fn truncate(&mut self, nodes: usize, entries: usize) {
        self.nodes.truncate(nodes);
        self.entries.truncate(entries);
    }
}

struct Usb0LibraryCatalogStage {
    #[cfg(feature = "p4-psram-storage")]
    publisher: CatalogBankPublisher<Usb0CatalogBank>,
    #[cfg(feature = "p4-psram-storage")]
    available: Option<Usb0CatalogBank>,
    #[cfg(feature = "p4-psram-storage")]
    reserve: Option<Usb0CatalogBank>,
    #[cfg(feature = "p4-psram-storage")]
    builder: Option<CatalogBankBuilder<Usb0CatalogBank>>,
    #[cfg(feature = "p4-psram-storage")]
    next_revision: u32,
    /// Published together with the catalog snapshot it was validated against.
    #[cfg(feature = "p4-psram-storage")]
    playlists: Option<PlaylistIndex<Usb0PlaylistStore>>,
    #[cfg(feature = "p4-psram-storage")]
    playlist_builder: Option<PlaylistBuilder<Usb0PlaylistStore>>,
    #[cfg(feature = "p4-psram-storage")]
    playlist_stores: [Option<Usb0PlaylistStore>; 2],
}

impl Usb0LibraryCatalogStage {
    fn try_new() -> Result<Self, ()> {
        #[cfg(feature = "p4-psram-storage")]
        {
            let first = Usb0CatalogBank::try_new()?;
            let second = Usb0CatalogBank::try_new()?;
            // Two stores: one published while the next import builds.
            let playlist_stores = [
                Some(Usb0PlaylistStore::try_new()?),
                Some(Usb0PlaylistStore::try_new()?),
            ];
            Ok(Self {
                publisher: CatalogBankPublisher::new(),
                available: Some(first),
                reserve: Some(second),
                builder: None,
                next_revision: 1,
                playlists: None,
                playlist_builder: None,
                playlist_stores,
            })
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        {
            Ok(Self {})
        }
    }

    #[cfg(feature = "p4-psram-storage")]
    fn recycle_bank(&mut self, bank: Usb0CatalogBank) {
        if self.available.is_none() {
            self.available = Some(bank);
        } else if self.reserve.is_none() {
            self.reserve = Some(bank);
        } else {
            debug_assert!(false, "more than two catalog banks returned to the pool");
        }
    }

    #[cfg(feature = "p4-psram-storage")]
    fn recycle_playlist_store(&mut self, store: Usb0PlaylistStore) {
        match self.playlist_stores.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = Some(store),
            None => debug_assert!(false, "more than two playlist stores returned to the pool"),
        }
    }

    #[cfg(feature = "p4-psram-storage")]
    fn retire_playlists(&mut self) {
        if let Some(index) = self.playlists.take() {
            self.recycle_playlist_store(index.into_store());
        }
    }

    fn prepare_generation(&mut self, lease: MediaLease) {
        #[cfg(feature = "p4-psram-storage")]
        {
            self.abort();
            let different_lease = self
                .publisher
                .current()
                .is_some_and(|snapshot| snapshot.lease() != lease);
            if different_lease {
                if let Some(snapshot) = self.publisher.take_current() {
                    self.recycle_bank(snapshot.into_bank());
                }
                self.retire_playlists();
            }
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = lease;
    }

    fn begin(&mut self, lease: MediaLease) -> Result<(), Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        {
            self.abort();
            let bank = self
                .available
                .take()
                .ok_or(Usb0LibraryError::CatalogBankUnavailable)?;
            let Some(store) = self.playlist_stores.iter_mut().find_map(Option::take) else {
                self.recycle_bank(bank);
                return Err(Usb0LibraryError::CatalogBankUnavailable);
            };
            self.builder = Some(CatalogBankBuilder::new(lease, bank));
            self.playlist_builder = Some(PlaylistBuilder::new(store));
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = lease;

        Ok(())
    }

    fn retain_playlist_node(&mut self, row: PdbPlaylistTreeRow, name: &str) -> bool {
        #[cfg(feature = "p4-psram-storage")]
        if let Some(builder) = self.playlist_builder.as_mut() {
            return builder.push_node(row.id, row.parent_id, row.sort_order, row.is_folder, name);
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = (row, name);

        true
    }

    fn retain_playlist_entry(&mut self, row: PdbPlaylistEntryRow) -> bool {
        #[cfg(feature = "p4-psram-storage")]
        if let Some(builder) = self.playlist_builder.as_mut() {
            return builder.push_entry(row.playlist_id, row.entry_index, row.track_id);
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = row;

        true
    }

    fn note_playlist_invalid_row(&mut self) {
        #[cfg(feature = "p4-psram-storage")]
        if let Some(builder) = self.playlist_builder.as_mut() {
            builder.note_invalid_row();
        }
    }

    fn note_playlist_truncated(&mut self) {
        #[cfg(feature = "p4-psram-storage")]
        if let Some(builder) = self.playlist_builder.as_mut() {
            builder.note_truncated();
        }
    }

    fn validate_names(&self, probe: &Usb0LibraryPdbProbe) -> Result<(), Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        {
            for table in [probe.artists_layout, probe.albums_layout, probe.keys_layout]
                .into_iter()
                .flatten()
            {
                if table.capacity_exceeded {
                    return Err(Usb0LibraryError::CatalogNameCapacityExceeded);
                }
                if table.slot_truncated {
                    return Err(Usb0LibraryError::CatalogTableTruncated);
                }
            }
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = probe;

        Ok(())
    }

    fn validate_tracks(&self, probe: Usb0LibraryTableProbe) -> Result<(), Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        if probe.truncated {
            return Err(Usb0LibraryError::CatalogTableTruncated);
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = probe;

        Ok(())
    }

    fn retain_track(
        &mut self,
        volume_identity: VolumeIdentity,
        track: &PdbTrackRow,
        names: &Usb0LibraryNameStage,
    ) -> Result<(), Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        {
            let resolved =
                resolve_pdb_track_names(track, &names.artists, &names.albums, &names.keys)
                    .with_artwork(track, &names.artworks);
            let catalog_track = catalog_track_from_pdb(volume_identity, track, resolved)
                .map_err(Usb0LibraryError::CatalogDecode)?;
            let builder = self
                .builder
                .as_mut()
                .ok_or(Usb0LibraryError::CatalogBankUnavailable)?;
            builder
                .push(catalog_track)
                .map_err(Usb0LibraryError::CatalogBuild)?;
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = (volume_identity, track, names);

        Ok(())
    }

    /// Publishes the catalog, then validates the staged playlist rows against
    /// that exact snapshot so both share one publication id.
    fn publish(&mut self) -> Result<Option<PlaylistImportStats>, Usb0LibraryError> {
        #[cfg(feature = "p4-psram-storage")]
        {
            let builder = self
                .builder
                .take()
                .ok_or(Usb0LibraryError::CatalogBankUnavailable)?;
            let revision = self.next_revision;
            self.next_revision = self.next_revision.wrapping_add(1).max(1);
            let retired = self.publisher.publish(builder.finish(revision));
            if let Some(snapshot) = retired {
                self.recycle_bank(snapshot.into_bank());
            } else if let Some(bank) = self.reserve.take() {
                self.recycle_bank(bank);
            }

            self.retire_playlists();
            let (Some(playlists), Some(snapshot)) =
                (self.playlist_builder.take(), self.publisher.current())
            else {
                return Ok(None);
            };
            let index = playlists.finish(snapshot);
            let stats = index.stats();
            self.playlists = Some(index);
            Ok(Some(stats))
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        {
            Ok(None)
        }
    }

    fn sync_selection(&self, browser: &mut LibraryBrowser) {
        #[cfg(feature = "p4-psram-storage")]
        match self.publisher.current() {
            Some(snapshot) => browser.publish(snapshot, self.playlists.as_ref()),
            None => browser.clear(),
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        browser.clear();

        publish_browser(browser);
    }

    fn browse_back(&self, browser: &mut LibraryBrowser) {
        #[cfg(feature = "p4-psram-storage")]
        if let Some(snapshot) = self.publisher.current() {
            let _ = browser.back(snapshot, self.playlists.as_ref());
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = browser;
    }

    fn activate(&self, browser: &mut LibraryBrowser) -> LibraryActivation {
        #[cfg(feature = "p4-psram-storage")]
        {
            match self.publisher.current() {
                Some(snapshot) => browser.activate(snapshot, self.playlists.as_ref()),
                None => LibraryActivation::Stale,
            }
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        {
            let _ = browser;
            LibraryActivation::Stale
        }
    }

    fn abort(&mut self) {
        #[cfg(feature = "p4-psram-storage")]
        {
            if let Some(builder) = self.builder.take() {
                self.recycle_bank(builder.into_bank());
            }
            if let Some(builder) = self.playlist_builder.take() {
                self.recycle_playlist_store(builder.into_store());
            }
        }
    }

    fn invalidate(&mut self, lease: MediaLease) {
        #[cfg(feature = "p4-psram-storage")]
        {
            self.abort();
            if let Some(snapshot) = self.publisher.take_if_lease(lease) {
                self.recycle_bank(snapshot.into_bank());
            }
            if self
                .playlists
                .as_ref()
                .is_some_and(|index| index.publication().lease == lease)
            {
                self.retire_playlists();
            }
        }

        #[cfg(not(feature = "p4-psram-storage"))]
        let _ = lease;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryError {
    FilesystemUnavailable,
    PathTooLong,
    BrokerBusy,
    BrokerPending,
    BrokerWrongClient,
    #[cfg(feature = "apta-usb0-bringup")]
    FilesystemAnalysisPaused,
    FilesystemNoMedia,
    FilesystemStaleLease,
    FilesystemHandleKindMismatch,
    FilesystemFat32,
    FilesystemExFat,
    MissingVolumeIdentity,
    StagingAllocation,
    DuplicateNameId(u32),
    CatalogBankUnavailable,
    CatalogNameCapacityExceeded,
    CatalogTableTruncated,
    CatalogDecode(PdbCatalogError),
    CatalogBuild(CatalogBuildError),
    InvalidTrackPathUtf8,
    Identity(IdentityError),
    MissingTracksTable,
    PageOutOfRange(u32),
    PageChainLoop,
    RowCountOverflow,
    Pdb(PdbError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryTableProbe {
    pub(crate) pages: u32,
    pub(crate) declared_rows: u32,
    pub(crate) present_rows: u32,
    pub(crate) track_rows: u32,
    pub(crate) truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryNameTableProbe {
    pub(crate) pages: u32,
    pub(crate) declared_rows: u32,
    pub(crate) present_rows: u32,
    pub(crate) names: u32,
    pub(crate) slot_truncated: bool,
    pub(crate) truncated_values: u32,
    pub(crate) capacity_exceeded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryPdbProbe {
    pub(crate) lease: MediaLease,
    pub(crate) volume_identity: VolumeIdentity,
    pub(crate) file_len: u64,
    pub(crate) header: PdbHeader,
    pub(crate) tracks_first_page: u32,
    pub(crate) artists_first_page: Option<u32>,
    pub(crate) albums_first_page: Option<u32>,
    pub(crate) keys_first_page: Option<u32>,
    pub(crate) playlist_tree_first_page: Option<u32>,
    pub(crate) playlist_entries_first_page: Option<u32>,
    pub(crate) artwork_first_page: Option<u32>,
    pub(crate) tracks_layout: Option<Usb0LibraryTableProbe>,
    pub(crate) artists_layout: Option<Usb0LibraryNameTableProbe>,
    pub(crate) albums_layout: Option<Usb0LibraryNameTableProbe>,
    pub(crate) keys_layout: Option<Usb0LibraryNameTableProbe>,
    /// None when absent; a malformed artwork table is dropped, not fatal.
    pub(crate) artwork_layout: Option<Usb0LibraryNameTableProbe>,
    /// Published hierarchy counts; invalid rows never fail the track catalog.
    pub(crate) playlists: Option<PlaylistImportStats>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryState {
    Idle,
    Probing {
        binding: Usb0MscBinding,
    },
    Ready {
        binding: Usb0MscBinding,
        pdb: Usb0LibraryPdbProbe,
    },
    Failed {
        binding: Usb0MscBinding,
        error: Usb0LibraryError,
    },
    Detached {
        binding: Usb0MscBinding,
    },
}

fn set_library_state(state: Usb0LibraryState) {
    USB0_LIBRARY_STATE.lock(|cell| {
        *cell.borrow_mut() = state;
    });
}

pub(crate) fn library_state() -> Usb0LibraryState {
    USB0_LIBRARY_STATE.lock(|cell| *cell.borrow())
}

fn map_filesystem_error(error: Usb0FsOperationError) -> Usb0LibraryError {
    match error {
        #[cfg(feature = "apta-usb0-bringup")]
        Usb0FsOperationError::AnalysisPaused => Usb0LibraryError::FilesystemAnalysisPaused,
        Usb0FsOperationError::NoMedia => Usb0LibraryError::FilesystemNoMedia,
        Usb0FsOperationError::StaleLease => Usb0LibraryError::FilesystemStaleLease,
        Usb0FsOperationError::HandleKindMismatch => Usb0LibraryError::FilesystemHandleKindMismatch,
        Usb0FsOperationError::Fat32(_) => Usb0LibraryError::FilesystemFat32,
        Usb0FsOperationError::ExFat(_) => Usb0LibraryError::FilesystemExFat,
    }
}

fn broker_invariant_reset() -> ! {
    esp_hal::system::software_reset()
}

async fn open_export_pdb(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
) -> Result<Usb0FsFileHandle, Usb0LibraryError> {
    let path =
        BoundedPath::new(REKORDBOX_EXPORT_PDB_PATH).map_err(|_| Usb0LibraryError::PathTooLong)?;
    let ticket = client.issue_ticket(lease);

    match client
        .submit(Usb0FsRequest::OpenRead { ticket, path })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::OpenRead { result, .. }) => {
            result.map_err(map_filesystem_error)
        }
        Usb0FsSubmitResult::QueueFull(_) => Err(Usb0LibraryError::BrokerBusy),
        Usb0FsSubmitResult::PendingCompletion(_) => Err(Usb0LibraryError::BrokerPending),
        Usb0FsSubmitResult::WrongClient(_) => Err(Usb0LibraryError::BrokerWrongClient),
        Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
            broker_invariant_reset()
        }
    }
}

enum PrefixReadResult {
    Complete {
        file: Usb0FsFileHandle,
        buffer: &'static mut [u8],
        bytes: usize,
    },
    Failed {
        file: Usb0FsFileHandle,
        buffer: &'static mut [u8],
        error: Usb0LibraryError,
    },
}

enum SeekResult {
    Complete {
        file: Usb0FsFileHandle,
    },
    Failed {
        file: Usb0FsFileHandle,
        error: Usb0LibraryError,
    },
}

async fn seek_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    position: u64,
) -> SeekResult {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::Seek {
            ticket,
            file,
            position,
        })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::Seek { file, result, .. }) => {
            match result {
                Ok(()) => SeekResult::Complete { file },
                Err(error) => SeekResult::Failed {
                    file,
                    error: map_filesystem_error(error),
                },
            }
        }
        Usb0FsSubmitResult::QueueFull(Usb0FsRequest::Seek { file, .. }) => SeekResult::Failed {
            file,
            error: Usb0LibraryError::BrokerBusy,
        },
        Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Seek { file, .. }) => {
            SeekResult::Failed {
                file,
                error: Usb0LibraryError::BrokerPending,
            }
        }
        Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Seek { file, .. }) => SeekResult::Failed {
            file,
            error: Usb0LibraryError::BrokerWrongClient,
        },
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::QueueFull(_)
        | Usb0FsSubmitResult::PendingCompletion(_)
        | Usb0FsSubmitResult::WrongClient(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

async fn read_prefix(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
) -> PrefixReadResult {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::Read {
            ticket,
            file,
            buffer,
        })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::Read {
            file,
            buffer,
            result,
            ..
        }) => match result {
            Ok(bytes) => PrefixReadResult::Complete {
                file,
                buffer,
                bytes,
            },
            Err(error) => PrefixReadResult::Failed {
                file,
                buffer,
                error: map_filesystem_error(error),
            },
        },
        Usb0FsSubmitResult::QueueFull(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerBusy,
            }
        }
        Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerPending,
            }
        }
        Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerWrongClient,
            }
        }
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::QueueFull(_)
        | Usb0FsSubmitResult::PendingCompletion(_)
        | Usb0FsSubmitResult::WrongClient(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

async fn read_at(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    position: u64,
) -> PrefixReadResult {
    let file = match seek_file(client, lease, file, position).await {
        SeekResult::Complete { file } => file,
        SeekResult::Failed { file, error } => {
            return PrefixReadResult::Failed {
                file,
                buffer,
                error,
            };
        }
    };
    read_prefix(client, lease, file, buffer).await
}

async fn read_track_text<const N: usize>(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    page_start: u64,
    page_size: u32,
    heap_offset: u16,
    relative_offset: u16,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<PdbText<N>, Usb0LibraryError>,
) {
    let page_relative = match pdb_track_string_page_offset(heap_offset, relative_offset, page_size)
    {
        Ok(Some(offset)) => offset,
        Ok(None) => return (file, buffer, Ok(PdbText::new())),
        Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
    };
    let absolute = match page_start.checked_add(u64::from(page_relative)) {
        Some(position) => position,
        None => {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::Pdb(PdbError::TrackStringOffsetOutOfRange)),
            );
        }
    };

    let read = read_at(client, lease, file, buffer, absolute).await;
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
    let available = bytes.min(buffer.len());

    let field = match parse_devicesql_field_header(&buffer[..available], 0) {
        Ok(field) => field,
        Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
    };
    let field_len = field.total_len_usize();
    let field_end = match page_relative.checked_add(field.total_len as u32) {
        Some(end) => end,
        None => {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::Pdb(PdbError::DeviceSqlTruncated)),
            );
        }
    };
    if field_end > page_size || field_len > available {
        return (
            file,
            buffer,
            Err(Usb0LibraryError::Pdb(PdbError::DeviceSqlTruncated)),
        );
    }

    let text = PdbText::from_devicesql_field(&buffer[..field_len]).map_err(Usb0LibraryError::Pdb);
    (file, buffer, text)
}

async fn read_name_text(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    page_start: u64,
    page_size: u32,
    heap_offset: u16,
    header: PdbNameRowHeader,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<PdbText<PDB_NAME_MAX>, Usb0LibraryError>,
) {
    let page_relative = match (PDB_PAGE_HEAP_OFFSET as u32)
        .checked_add(u32::from(heap_offset))
        .and_then(|offset| offset.checked_add(u32::from(header.text_relative_offset)))
    {
        Some(offset) if offset < page_size => offset,
        _ => {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::Pdb(PdbError::NameRowOutOfRange)),
            );
        }
    };

    let absolute = match page_start.checked_add(u64::from(page_relative)) {
        Some(position) => position,
        None => {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::Pdb(PdbError::NameRowOutOfRange)),
            );
        }
    };

    let read = read_at(client, lease, file, buffer, absolute).await;
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
    let available = bytes.min(buffer.len());

    let field = match parse_devicesql_field_header(&buffer[..available], 0) {
        Ok(field) => field,
        Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
    };
    let field_len = field.total_len_usize();
    let field_end = match page_relative.checked_add(u32::from(field.total_len)) {
        Some(end) => end,
        None => {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::Pdb(PdbError::DeviceSqlTruncated)),
            );
        }
    };
    if field_end > page_size || field_len > available {
        return (
            file,
            buffer,
            Err(Usb0LibraryError::Pdb(PdbError::DeviceSqlTruncated)),
        );
    }

    let text = PdbText::from_devicesql_field(&buffer[..field_len]).map_err(Usb0LibraryError::Pdb);
    (file, buffer, text)
}

async fn close_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
) -> Result<(), Usb0LibraryError> {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::CloseFile { ticket, file })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::CloseFile { result, .. }) => {
            result.map_err(map_filesystem_error)
        }
        Usb0FsSubmitResult::QueueFull(_) => Err(Usb0LibraryError::BrokerBusy),
        Usb0FsSubmitResult::PendingCompletion(_) => Err(Usb0LibraryError::BrokerPending),
        Usb0FsSubmitResult::WrongClient(_) => Err(Usb0LibraryError::BrokerWrongClient),
        Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
            broker_invariant_reset()
        }
    }
}

fn first_page(tables: &PdbTableDirectory, kind: u32) -> Option<u32> {
    tables
        .find(kind)
        .and_then(|entry| (entry.first_page != u32::MAX).then_some(entry.first_page))
}

fn parse_first_page(
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    file_len: u64,
    buffer: &[u8],
    bytes: usize,
) -> Result<Usb0LibraryPdbProbe, Usb0LibraryError> {
    let available = bytes.min(buffer.len());
    let header = parse_pdb_header(&buffer[..available], file_len).map_err(Usb0LibraryError::Pdb)?;
    let tables =
        parse_pdb_table_directory(&buffer[..available], header).map_err(Usb0LibraryError::Pdb)?;
    let tracks_first_page =
        first_page(&tables, PDB_TABLE_TRACKS).ok_or(Usb0LibraryError::MissingTracksTable)?;

    Ok(Usb0LibraryPdbProbe {
        lease,
        volume_identity,
        file_len,
        header,
        tracks_first_page,
        artists_first_page: first_page(&tables, PDB_TABLE_ARTISTS),
        albums_first_page: first_page(&tables, PDB_TABLE_ALBUMS),
        keys_first_page: first_page(&tables, PDB_TABLE_KEYS),
        playlist_tree_first_page: first_page(&tables, PDB_TABLE_PLAYLIST_TREE),
        playlist_entries_first_page: first_page(&tables, PDB_TABLE_PLAYLIST_ENTRIES),
        artwork_first_page: first_page(&tables, PDB_TABLE_ARTWORK),
        tracks_layout: None,
        artists_layout: None,
        albums_layout: None,
        keys_layout: None,
        artwork_layout: None,
        playlists: None,
    })
}

async fn scan_table_layout(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    mut file: Usb0FsFileHandle,
    mut buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: u32,
    names: &Usb0LibraryNameStage,
    catalog: &mut Usb0LibraryCatalogStage,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<Usb0LibraryTableProbe, Usb0LibraryError>,
) {
    let mut page = first_page;
    let mut pages = 0u32;
    let mut declared_rows = 0u32;
    let mut present_rows = 0u32;
    let mut track_rows = 0u32;
    let mut truncated = false;

    loop {
        if page >= header.total_pages {
            return (file, buffer, Err(Usb0LibraryError::PageOutOfRange(page)));
        }
        if pages >= header.total_pages {
            return (file, buffer, Err(Usb0LibraryError::PageChainLoop));
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let header_read = read_at(client, lease, file, buffer, page_start).await;
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

        let page_header = match parse_pdb_page_header(&buffer[..bytes], header.page_size) {
            Ok(page_header) => page_header,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };
        let mut walker = match PdbPageSlotWalker::new(header.page_size, page_header.declared_rows) {
            Ok(walker) => walker,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let slot_read = read_at(client, lease, file, buffer, absolute).await;
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

            let row_offsets = match walker.ingest_batch(plan, &buffer[..bytes]) {
                Ok(row_offsets) => row_offsets,
                Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
            };

            for heap_offset in row_offsets.offsets().iter().copied() {
                let page_relative =
                    match (PDB_PAGE_HEAP_OFFSET as u32).checked_add(u32::from(heap_offset)) {
                        Some(offset) => offset,
                        None => {
                            return (
                                file,
                                buffer,
                                Err(Usb0LibraryError::Pdb(PdbError::TrackRowOutOfRange)),
                            );
                        }
                    };
                let row_end = match page_relative.checked_add(PDB_TRACK_ROW_HEADER_BYTES as u32) {
                    Some(end) => end,
                    None => {
                        return (
                            file,
                            buffer,
                            Err(Usb0LibraryError::Pdb(PdbError::TrackRowOutOfRange)),
                        );
                    }
                };
                if row_end > header.page_size {
                    return (
                        file,
                        buffer,
                        Err(Usb0LibraryError::Pdb(PdbError::TrackRowOutOfRange)),
                    );
                }

                let absolute = page_start + u64::from(page_relative);
                let row_read = read_at(client, lease, file, buffer, absolute).await;
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

                match parse_pdb_track_row_header(&buffer[..bytes]) {
                    Ok(Some(track_header)) => {
                        let (next_file, next_buffer, title) = read_track_text::<PDB_TITLE_MAX>(
                            client,
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
                            Err(Usb0LibraryError::Pdb(error)) => recover_pdb_title(Err(error)),
                            Err(error) => return (file, buffer, Err(error)),
                        };

                        let (next_file, next_buffer, filename) = read_track_text::<PDB_TITLE_MAX>(
                            client,
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

                        let (next_file, next_buffer, file_path) = read_track_text::<PDB_PATH_MAX>(
                            client,
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

                        let (next_file, next_buffer, anlz_path) = read_track_text::<PDB_PATH_MAX>(
                            client,
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
                                Err(Usb0LibraryError::Identity(IdentityError::PathTooLong)),
                            );
                        }
                        let audio_path = match track.file_path.as_str() {
                            Some(path) => path,
                            None => {
                                return (file, buffer, Err(Usb0LibraryError::InvalidTrackPathUtf8));
                            }
                        };
                        if let Err(error) = derive_track_id(volume_identity, audio_path) {
                            return (file, buffer, Err(Usb0LibraryError::Identity(error)));
                        }
                        if let Err(error) = catalog.retain_track(volume_identity, &track, names) {
                            return (file, buffer, Err(error));
                        }

                        track_rows = match track_rows.checked_add(1) {
                            Some(total) => total,
                            None => {
                                return (file, buffer, Err(Usb0LibraryError::RowCountOverflow));
                            }
                        };
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return (file, buffer, Err(Usb0LibraryError::Pdb(error)));
                    }
                }
            }
        }

        let summary = walker.summary();
        declared_rows = match declared_rows.checked_add(summary.declared_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        present_rows = match present_rows.checked_add(summary.present_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        truncated |= summary.truncated;
        pages += 1;

        if page_header.next_page == u32::MAX {
            break;
        }
        if page_header.next_page >= header.total_pages {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::PageOutOfRange(page_header.next_page)),
            );
        }
        page = page_header.next_page;
    }

    (
        file,
        buffer,
        Ok(Usb0LibraryTableProbe {
            pages,
            declared_rows,
            present_rows,
            track_rows,
            truncated,
        }),
    )
}

async fn scan_name_table_layout(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    mut file: Usb0FsFileHandle,
    mut buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: u32,
    layout: PdbNameRowLayout,
    stage: &mut Usb0LibraryNameMap,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<Usb0LibraryNameTableProbe, Usb0LibraryError>,
) {
    let mut page = first_page;
    let mut pages = 0u32;
    let mut declared_rows = 0u32;
    let mut present_rows = 0u32;
    let mut names = 0u32;
    let mut slot_truncated = false;
    let mut truncated_values = 0u32;

    loop {
        if page >= header.total_pages {
            return (file, buffer, Err(Usb0LibraryError::PageOutOfRange(page)));
        }
        if pages >= header.total_pages {
            return (file, buffer, Err(Usb0LibraryError::PageChainLoop));
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let header_read = read_at(client, lease, file, buffer, page_start).await;
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

        let page_header = match parse_pdb_page_header(&buffer[..bytes], header.page_size) {
            Ok(page_header) => page_header,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };
        let mut walker = match PdbPageSlotWalker::new(header.page_size, page_header.declared_rows) {
            Ok(walker) => walker,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let slot_read = read_at(client, lease, file, buffer, absolute).await;
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

            let row_offsets = match walker.ingest_batch(plan, &buffer[..bytes]) {
                Ok(row_offsets) => row_offsets,
                Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
            };

            for heap_offset in row_offsets.offsets().iter().copied() {
                let page_relative =
                    match (PDB_PAGE_HEAP_OFFSET as u32).checked_add(u32::from(heap_offset)) {
                        Some(offset) => offset,
                        None => {
                            return (
                                file,
                                buffer,
                                Err(Usb0LibraryError::Pdb(PdbError::NameRowOutOfRange)),
                            );
                        }
                    };
                let row_end = match page_relative.checked_add(layout.minimum_bytes() as u32) {
                    Some(end) => end,
                    None => {
                        return (
                            file,
                            buffer,
                            Err(Usb0LibraryError::Pdb(PdbError::NameRowOutOfRange)),
                        );
                    }
                };
                if row_end > header.page_size {
                    return (
                        file,
                        buffer,
                        Err(Usb0LibraryError::Pdb(PdbError::NameRowOutOfRange)),
                    );
                }

                let absolute = page_start + u64::from(page_relative);
                let row_read = read_at(client, lease, file, buffer, absolute).await;
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

                let row_header = match parse_pdb_name_row_header(&buffer[..bytes], layout) {
                    Ok(Some(header)) => header,
                    Ok(None) => continue,
                    Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
                };

                let (next_file, next_buffer, name) = read_name_text(
                    client,
                    lease,
                    file,
                    buffer,
                    page_start,
                    header.page_size,
                    heap_offset,
                    row_header,
                )
                .await;
                file = next_file;
                buffer = next_buffer;
                let name = match name {
                    Ok(name) => name,
                    Err(error) => return (file, buffer, Err(error)),
                };

                if name.was_truncated() {
                    truncated_values = match truncated_values.checked_add(1) {
                        Some(total) => total,
                        None => {
                            return (file, buffer, Err(Usb0LibraryError::RowCountOverflow));
                        }
                    };
                }
                if let Some(entry) = PdbNameEntry::from_header_and_field(row_header, name) {
                    names = match names.checked_add(1) {
                        Some(total) => total,
                        None => {
                            return (file, buffer, Err(Usb0LibraryError::RowCountOverflow));
                        }
                    };
                    if let Err(error) = stage.retain(entry) {
                        return (file, buffer, Err(error));
                    }
                }
            }
        }

        let summary = walker.summary();
        declared_rows = match declared_rows.checked_add(summary.declared_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        present_rows = match present_rows.checked_add(summary.present_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        slot_truncated |= summary.truncated;
        pages += 1;

        if page_header.next_page == u32::MAX {
            break;
        }
        if page_header.next_page >= header.total_pages {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::PageOutOfRange(page_header.next_page)),
            );
        }
        page = page_header.next_page;
    }

    (
        file,
        buffer,
        Ok(Usb0LibraryNameTableProbe {
            pages,
            declared_rows,
            present_rows,
            names,
            slot_truncated,
            truncated_values,
            capacity_exceeded: names > PDB_MAX_NAMES as u32,
        }),
    )
}

async fn scan_optional_name_table(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: Option<u32>,
    layout: PdbNameRowLayout,
    stage: &mut Usb0LibraryNameMap,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<Option<Usb0LibraryNameTableProbe>, Usb0LibraryError>,
) {
    let Some(first_page) = first_page else {
        return (file, buffer, Ok(None));
    };

    let (file, buffer, result) = scan_name_table_layout(
        client, lease, file, buffer, header, first_page, layout, stage,
    )
    .await;
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
async fn scan_playlist_table(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    mut file: Usb0FsFileHandle,
    mut buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: u32,
    table: PlaylistTable,
    catalog: &mut Usb0LibraryCatalogStage,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<(), Usb0LibraryError>,
) {
    let row_bytes = match table {
        PlaylistTable::Tree => PDB_PLAYLIST_TREE_ROW_HEADER_BYTES as u32 + 1,
        PlaylistTable::Entries => PDB_PLAYLIST_ENTRY_ROW_BYTES as u32,
    };
    let mut page = first_page;
    let mut pages = 0u32;

    'pages: loop {
        if page >= header.total_pages || pages >= header.total_pages {
            catalog.note_playlist_truncated();
            break;
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let (next_file, next_buffer, bytes) =
            match read_at(client, lease, file, buffer, page_start).await {
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

        let Ok(page_header) = parse_pdb_page_header(&buffer[..bytes], header.page_size) else {
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
                match read_at(client, lease, file, buffer, absolute).await {
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

            let Ok(row_offsets) = walker.ingest_batch(plan, &buffer[..bytes]) else {
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
                    match read_at(client, lease, file, buffer, absolute).await {
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
                        match parse_pdb_playlist_entry_row(&buffer[..bytes]) {
                            Ok(Some(row)) => catalog.retain_playlist_entry(row),
                            Ok(None) | Err(_) => {
                                catalog.note_playlist_invalid_row();
                                true
                            }
                        }
                    }
                    PlaylistTable::Tree => match parse_pdb_playlist_tree_row(&buffer[..bytes]) {
                        Ok(Some(row)) => {
                            let (next_file, next_buffer, name) = read_name_text(
                                client,
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
                                Ok(name) => match name.as_str() {
                                    Some(name) => catalog.retain_playlist_node(row, name),
                                    None => {
                                        catalog.note_playlist_invalid_row();
                                        true
                                    }
                                },
                                Err(Usb0LibraryError::Pdb(_)) => {
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
                    },
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
        if page_header.next_page == u32::MAX {
            break;
        }
        page = page_header.next_page;
    }

    (file, buffer, Ok(()))
}

async fn scan_optional_playlist_table(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: Option<u32>,
    table: PlaylistTable,
    catalog: &mut Usb0LibraryCatalogStage,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<(), Usb0LibraryError>,
) {
    match first_page {
        Some(first_page) => {
            scan_playlist_table(
                client, lease, file, buffer, header, first_page, table, catalog,
            )
            .await
        }
        None => (file, buffer, Ok(())),
    }
}

async fn scan_probe_tables(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    mut probe: Usb0LibraryPdbProbe,
    staging: &mut Usb0LibraryNameStage,
    catalog: &mut Usb0LibraryCatalogStage,
) -> (
    Usb0FsFileHandle,
    &'static mut [u8],
    Result<Usb0LibraryPdbProbe, Usb0LibraryError>,
) {
    let (file, buffer, artists) = scan_optional_name_table(
        client,
        lease,
        file,
        buffer,
        probe.header,
        probe.artists_first_page,
        PdbNameRowLayout::Artist,
        &mut staging.artists,
    )
    .await;
    probe.artists_layout = match artists {
        Ok(artists) => artists,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, albums) = scan_optional_name_table(
        client,
        lease,
        file,
        buffer,
        probe.header,
        probe.albums_first_page,
        PdbNameRowLayout::Album,
        &mut staging.albums,
    )
    .await;
    probe.albums_layout = match albums {
        Ok(albums) => albums,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, keys) = scan_optional_name_table(
        client,
        lease,
        file,
        buffer,
        probe.header,
        probe.keys_first_page,
        PdbNameRowLayout::Key,
        &mut staging.keys,
    )
    .await;
    probe.keys_layout = match keys {
        Ok(keys) => keys,
        Err(error) => return (file, buffer, Err(error)),
    };

    let (file, buffer, artworks) = scan_optional_name_table(
        client,
        lease,
        file,
        buffer,
        probe.header,
        probe.artwork_first_page,
        PdbNameRowLayout::Artwork,
        &mut staging.artworks,
    )
    .await;
    probe.artwork_layout = match artworks {
        Ok(artworks) => artworks,
        // Covers are optional: a malformed table leaves every track on the
        // fallback; storage errors still abort.
        Err(Usb0LibraryError::Pdb(_) | Usb0LibraryError::PageOutOfRange(_))
        | Err(Usb0LibraryError::PageChainLoop) => {
            staging.artworks.clear();
            None
        }
        Err(error) => return (file, buffer, Err(error)),
    };

    if let Err(error) = catalog.validate_names(&probe) {
        return (file, buffer, Err(error));
    }
    if let Err(error) = catalog.begin(lease) {
        return (file, buffer, Err(error));
    }

    let (mut file, mut buffer, tracks) = scan_table_layout(
        client,
        lease,
        probe.volume_identity,
        file,
        buffer,
        probe.header,
        probe.tracks_first_page,
        staging,
        catalog,
    )
    .await;
    let tracks = match tracks {
        Ok(tracks) => tracks,
        Err(error) => return (file, buffer, Err(error)),
    };
    if let Err(error) = catalog.validate_tracks(tracks) {
        return (file, buffer, Err(error));
    }
    probe.tracks_layout = Some(tracks);

    // Entries are only meaningful with a tree to resolve them against.
    let entries_first_page = probe
        .playlist_tree_first_page
        .and(probe.playlist_entries_first_page);
    for (table, first_page) in [
        (PlaylistTable::Tree, probe.playlist_tree_first_page),
        (PlaylistTable::Entries, entries_first_page),
    ] {
        let (next_file, next_buffer, result) = scan_optional_playlist_table(
            client,
            lease,
            file,
            buffer,
            probe.header,
            first_page,
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

async fn probe_export_pdb(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    volume_identity: VolumeIdentity,
    buffer: &'static mut [u8],
    staging: &mut Usb0LibraryNameStage,
    catalog: &mut Usb0LibraryCatalogStage,
) -> (
    &'static mut [u8],
    Result<Usb0LibraryPdbProbe, Usb0LibraryError>,
) {
    let file = match open_export_pdb(client, lease).await {
        Ok(file) => file,
        Err(error) => return (buffer, Err(error)),
    };
    let file_len = file.len();

    let (file, buffer, read_result) = match read_prefix(client, lease, file, buffer).await {
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
        Ok(bytes) => parse_first_page(lease, volume_identity, file_len, buffer, bytes),
        Err(error) => Err(error),
    };

    let (file, buffer, parse_result) = match parse_result {
        Ok(probe) => scan_probe_tables(client, lease, file, buffer, probe, staging, catalog).await,
        Err(error) => (file, buffer, Err(error)),
    };

    let close_result = close_file(client, lease, file).await;
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
                    Err(error)
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

fn handle_library_command(
    catalog: Option<&Usb0LibraryCatalogStage>,
    browser: &mut LibraryBrowser,
    command: Usb0LibraryCommand,
) -> Usb0LibraryCompletion {
    let request_id = command.request_id;
    let completion = match command.effect {
        LibraryEffect::Browse { delta, multiplier } => {
            let _ = browser.browse(delta, multiplier);
            Usb0LibraryCompletion::Applied { request_id }
        }
        LibraryEffect::OpenLibrary => Usb0LibraryCompletion::Applied { request_id },
        LibraryEffect::Back => {
            if let Some(catalog) = catalog {
                catalog.browse_back(browser);
            }
            Usb0LibraryCompletion::Applied { request_id }
        }
        // No permit: a stale mirror that now rests on a track loads nothing.
        LibraryEffect::Open => {
            if let Some(catalog) = catalog
                && browser.on_node_row()
            {
                let _ = catalog.activate(browser);
            }
            Usb0LibraryCompletion::Applied { request_id }
        }
        LibraryEffect::Load { deck, .. } => {
            let Some(catalog) = catalog else {
                return Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::CatalogUnavailable,
                };
            };
            match catalog.activate(browser) {
                // The permit is finished by the Library deck owner on Applied.
                LibraryActivation::Opened => Usb0LibraryCompletion::Applied { request_id },
                LibraryActivation::Load(candidate) => Usb0LibraryCompletion::LoadResolved {
                    request_id,
                    deck,
                    candidate,
                },
                LibraryActivation::Empty => Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::EmptyCatalog,
                },
                LibraryActivation::Stale => Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::StaleSelection,
                },
            }
        }
    };
    publish_browser(browser);
    completion
}

pub(crate) async fn run_usb0_library_worker() -> ! {
    let mut client = Usb0FilesystemClient::library();
    let mut page_buffer = &mut USB0_LIBRARY_PDB_IO.init([0; USB0_LIBRARY_PDB_IO_BYTES])[..];
    let mut name_stage: Option<Usb0LibraryNameStage> = None;
    let mut catalog_stage: Option<Usb0LibraryCatalogStage> = None;
    let mut selection = LibraryBrowser::new();

    loop {
        match select(USB0_LIBRARY_EVENTS.wait(), USB0_LIBRARY_COMMANDS.receive()).await {
            Either::First(Usb0LibraryEvent::Ready {
                binding,
                kind: _,
                volume_identity,
            }) => {
                set_media_lease(binding.lease);
                let Some(volume_identity) = volume_identity else {
                    if let Some(catalog) = catalog_stage.as_mut() {
                        catalog.invalidate(binding.lease);
                        catalog.sync_selection(&mut selection);
                    } else {
                        selection.clear();
                        publish_browser(&selection);
                    }
                    if let Some(staging) = name_stage.as_mut() {
                        staging.clear();
                    }
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::MissingVolumeIdentity,
                    });
                    continue;
                };

                if name_stage.is_none() {
                    name_stage = Usb0LibraryNameStage::try_new().ok();
                }
                let Some(staging) = name_stage.as_mut() else {
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::StagingAllocation,
                    });
                    continue;
                };
                if catalog_stage.is_none() {
                    catalog_stage = Usb0LibraryCatalogStage::try_new().ok();
                }
                let Some(catalog) = catalog_stage.as_mut() else {
                    staging.clear();
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::StagingAllocation,
                    });
                    continue;
                };
                catalog.prepare_generation(binding.lease);
                catalog.sync_selection(&mut selection);
                staging.clear();

                set_library_state(Usb0LibraryState::Probing { binding });
                let (restored, result) = probe_export_pdb(
                    &mut client,
                    binding.lease,
                    volume_identity,
                    page_buffer,
                    staging,
                    catalog,
                )
                .await;
                page_buffer = restored;

                match result {
                    Ok(pdb) => {
                        catalog.sync_selection(&mut selection);
                        set_library_state(Usb0LibraryState::Ready { binding, pdb });
                    }
                    Err(error) => {
                        catalog.abort();
                        staging.clear();
                        set_library_state(Usb0LibraryState::Failed { binding, error });
                    }
                }
            }
            Either::First(Usb0LibraryEvent::FilesystemFailed { binding }) => {
                clear_media_lease(binding.lease);
                if let Some(staging) = name_stage.as_mut() {
                    staging.clear();
                }
                if let Some(catalog) = catalog_stage.as_mut() {
                    catalog.invalidate(binding.lease);
                    catalog.sync_selection(&mut selection);
                } else {
                    selection.clear();
                    publish_browser(&selection);
                }
                set_library_state(Usb0LibraryState::Failed {
                    binding,
                    error: Usb0LibraryError::FilesystemUnavailable,
                });
            }
            Either::First(Usb0LibraryEvent::Detached { binding }) => {
                clear_media_lease(binding.lease);
                if let Some(staging) = name_stage.as_mut() {
                    staging.clear();
                }
                if let Some(catalog) = catalog_stage.as_mut() {
                    catalog.invalidate(binding.lease);
                    catalog.sync_selection(&mut selection);
                } else {
                    selection.clear();
                    publish_browser(&selection);
                }
                set_library_state(Usb0LibraryState::Detached { binding });
            }
            Either::Second(command) => {
                let completion =
                    handle_library_command(catalog_stage.as_ref(), &mut selection, command);
                USB0_LIBRARY_COMPLETIONS.send(completion).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease() -> MediaLease {
        use pajoniiir_media_session::{MediaGeneration, MediaSourceId};

        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        }
    }

    fn volume_identity() -> VolumeIdentity {
        VolumeIdentity([0x42; 32])
    }

    fn put_le32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
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
            parse_first_page(lease(), volume_identity(), 512 * 6, &page, page.len()).unwrap();
        assert_eq!(probe.header.page_size, 512);
        assert_eq!(probe.tracks_first_page, 1);
        assert_eq!(probe.artists_first_page, Some(2));
        assert_eq!(probe.albums_first_page, Some(3));
        assert_eq!(probe.keys_first_page, Some(4));
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

        let probe = parse_first_page(
            lease(),
            volume_identity(),
            65_536 * 2,
            &prefix,
            prefix.len(),
        )
        .unwrap();

        assert_eq!(probe.header.page_size, 65_536);
        assert_eq!(probe.tracks_first_page, 1);
    }
}
