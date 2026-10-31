//! Heap-backed import stage. Every capacity is reserved once by `try_new`, so
//! a PDB walk never grows the heap; a failed reservation is reported before
//! any page is read.

use alloc::vec::Vec;
use ruzina_media_catalog::playlists::{
    CATALOG_MAX_PLAYLIST_ENTRIES, CATALOG_MAX_PLAYLIST_NODES, LibraryActivation, LibraryBrowser,
    PlaylistBuilder, PlaylistEntry, PlaylistImportStats, PlaylistIndex, PlaylistNode,
    PlaylistStore,
};
use ruzina_media_catalog::{
    CatalogBank, CatalogBankBuilder, CatalogBankPublisher, CatalogBankSnapshot, CatalogTrack,
    MEDIA_CATALOG_MAX_TRACKS,
};
use ruzina_media_identity::VolumeIdentity;
use ruzina_media_session::MediaLease;
use ruzina_rekordbox::{
    PDB_NAME_MAX, PdbNameEntry, PdbNameLookup, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PdbText,
    PdbTrackRow, catalog_track_from_pdb, resolve_pdb_track_names,
};

use super::{
    CatalogError, CatalogStage, PDB_ARTWORK_CAPACITY, PDB_NAME_CAPACITY, PdbNameStage,
    PdbNameStore, PdbProbe, PdbTableProbe, StageAllocationError,
};

pub struct VecNameMap {
    entries: Vec<PdbNameEntry>,
    capacity: usize,
    /// Optional tables (artwork) keep the first row of a repeated id
    /// instead of failing the import, as M2.5 drops such rows.
    optional: bool,
}

impl VecNameMap {
    pub fn try_new(capacity: usize, optional: bool) -> Result<Self, StageAllocationError> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|_| StageAllocationError)?;
        Ok(Self {
            entries,
            capacity,
            optional,
        })
    }
}

impl PdbNameLookup for VecNameMap {
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

impl PdbNameStore for VecNameMap {
    fn clear(&mut self) {
        self.entries.clear();
    }

    fn retain(&mut self, entry: PdbNameEntry) -> Result<(), CatalogError> {
        if self.entries.iter().any(|existing| existing.id == entry.id) {
            if self.optional {
                return Ok(());
            }
            return Err(CatalogError::DuplicateNameId(entry.id));
        }
        if self.entries.len() < self.capacity {
            // Capacity is reserved once up front, so this push cannot grow
            // the heap while a PDB page is being processed.
            self.entries.push(entry);
        }
        Ok(())
    }
}

impl PdbNameStage<VecNameMap> {
    pub fn try_new() -> Result<Self, StageAllocationError> {
        Ok(Self {
            artists: VecNameMap::try_new(PDB_NAME_CAPACITY, false)?,
            albums: VecNameMap::try_new(PDB_NAME_CAPACITY, false)?,
            keys: VecNameMap::try_new(PDB_NAME_CAPACITY, false)?,
            artworks: VecNameMap::try_new(PDB_ARTWORK_CAPACITY, true)?,
        })
    }
}

pub struct VecCatalogBank {
    tracks: Vec<CatalogTrack>,
}

impl VecCatalogBank {
    fn try_new() -> Result<Self, StageAllocationError> {
        let mut tracks = Vec::new();
        tracks
            .try_reserve_exact(MEDIA_CATALOG_MAX_TRACKS)
            .map_err(|_| StageAllocationError)?;
        Ok(Self { tracks })
    }
}

impl CatalogBank for VecCatalogBank {
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

/// Playlist tree and entries reserved once, so a PDB walk never grows the
/// heap (RP-06).
pub struct VecPlaylistStore {
    nodes: Vec<PlaylistNode>,
    entries: Vec<PlaylistEntry>,
}

impl VecPlaylistStore {
    fn try_new() -> Result<Self, StageAllocationError> {
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(CATALOG_MAX_PLAYLIST_NODES)
            .map_err(|_| StageAllocationError)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(CATALOG_MAX_PLAYLIST_ENTRIES)
            .map_err(|_| StageAllocationError)?;
        Ok(Self { nodes, entries })
    }
}

impl PlaylistStore for VecPlaylistStore {
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

/// Two catalog banks and two playlist stores: one published while the next
/// import builds.
pub struct VecCatalogStage {
    publisher: CatalogBankPublisher<VecCatalogBank>,
    available: Option<VecCatalogBank>,
    reserve: Option<VecCatalogBank>,
    builder: Option<CatalogBankBuilder<VecCatalogBank>>,
    next_revision: u32,
    /// Published together with the catalog snapshot it was validated against.
    playlists: Option<PlaylistIndex<VecPlaylistStore>>,
    playlist_builder: Option<PlaylistBuilder<VecPlaylistStore>>,
    playlist_stores: [Option<VecPlaylistStore>; 2],
}

impl VecCatalogStage {
    pub fn try_new() -> Result<Self, StageAllocationError> {
        let first = VecCatalogBank::try_new()?;
        let second = VecCatalogBank::try_new()?;
        let playlist_stores = [
            Some(VecPlaylistStore::try_new()?),
            Some(VecPlaylistStore::try_new()?),
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

    /// The published catalog, if any.
    pub fn current(&self) -> Option<&CatalogBankSnapshot<VecCatalogBank>> {
        self.publisher.current()
    }

    pub fn playlists(&self) -> Option<&PlaylistIndex<VecPlaylistStore>> {
        self.playlists.as_ref()
    }

    fn recycle_bank(&mut self, bank: VecCatalogBank) {
        if self.available.is_none() {
            self.available = Some(bank);
        } else if self.reserve.is_none() {
            self.reserve = Some(bank);
        } else {
            debug_assert!(false, "more than two catalog banks returned to the pool");
        }
    }

    fn recycle_playlist_store(&mut self, store: VecPlaylistStore) {
        match self.playlist_stores.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = Some(store),
            None => debug_assert!(false, "more than two playlist stores returned to the pool"),
        }
    }

    fn retire_playlists(&mut self) {
        if let Some(index) = self.playlists.take() {
            self.recycle_playlist_store(index.into_store());
        }
    }

    /// Drop an unfinished build, and the published catalog if it belongs to
    /// another lease, before a new import under `lease`.
    pub fn prepare_generation(&mut self, lease: MediaLease) {
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

    /// Withdraw everything published or building under `lease`.
    pub fn invalidate(&mut self, lease: MediaLease) {
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

    /// Point `browser` at the published catalog, or clear it.
    pub fn sync_selection(&self, browser: &mut LibraryBrowser) {
        match self.publisher.current() {
            Some(snapshot) => browser.publish(snapshot, self.playlists.as_ref()),
            None => browser.clear(),
        }
    }

    pub fn browse_back(&self, browser: &mut LibraryBrowser) {
        if let Some(snapshot) = self.publisher.current() {
            let _ = browser.back(snapshot, self.playlists.as_ref());
        }
    }

    pub fn activate(&self, browser: &mut LibraryBrowser) -> LibraryActivation {
        match self.publisher.current() {
            Some(snapshot) => browser.activate(snapshot, self.playlists.as_ref()),
            None => LibraryActivation::Stale,
        }
    }
}

impl CatalogStage<VecNameMap> for VecCatalogStage {
    fn validate_names(&self, probe: &PdbProbe) -> Result<(), CatalogError> {
        for table in [probe.artists_layout, probe.albums_layout, probe.keys_layout]
            .into_iter()
            .flatten()
        {
            if table.capacity_exceeded {
                return Err(CatalogError::CatalogNameCapacityExceeded);
            }
            if table.slot_truncated {
                return Err(CatalogError::CatalogTableTruncated);
            }
        }
        Ok(())
    }

    fn validate_tracks(&self, probe: PdbTableProbe) -> Result<(), CatalogError> {
        if probe.truncated {
            return Err(CatalogError::CatalogTableTruncated);
        }
        Ok(())
    }

    fn begin(&mut self, lease: MediaLease) -> Result<(), CatalogError> {
        self.abort();
        let bank = self
            .available
            .take()
            .ok_or(CatalogError::CatalogBankUnavailable)?;
        let Some(store) = self.playlist_stores.iter_mut().find_map(Option::take) else {
            self.recycle_bank(bank);
            return Err(CatalogError::CatalogBankUnavailable);
        };
        self.builder = Some(CatalogBankBuilder::new(lease, bank));
        self.playlist_builder = Some(PlaylistBuilder::new(store));
        Ok(())
    }

    fn retain_track(
        &mut self,
        volume_identity: VolumeIdentity,
        track: &PdbTrackRow,
        names: &PdbNameStage<VecNameMap>,
    ) -> Result<(), CatalogError> {
        let resolved = resolve_pdb_track_names(track, &names.artists, &names.albums, &names.keys)
            .with_artwork(track, &names.artworks);
        let catalog_track = catalog_track_from_pdb(volume_identity, track, resolved)
            .map_err(CatalogError::CatalogDecode)?;
        let builder = self
            .builder
            .as_mut()
            .ok_or(CatalogError::CatalogBankUnavailable)?;
        builder
            .push(catalog_track)
            .map_err(CatalogError::CatalogBuild)?;
        Ok(())
    }

    fn retain_playlist_node(&mut self, row: PdbPlaylistTreeRow, name: &str) -> bool {
        match self.playlist_builder.as_mut() {
            Some(builder) => {
                builder.push_node(row.id, row.parent_id, row.sort_order, row.is_folder, name)
            }
            None => true,
        }
    }

    fn retain_playlist_entry(&mut self, row: PdbPlaylistEntryRow) -> bool {
        match self.playlist_builder.as_mut() {
            Some(builder) => builder.push_entry(row.playlist_id, row.entry_index, row.track_id),
            None => true,
        }
    }

    fn note_playlist_invalid_row(&mut self) {
        if let Some(builder) = self.playlist_builder.as_mut() {
            builder.note_invalid_row();
        }
    }

    fn note_playlist_truncated(&mut self) {
        if let Some(builder) = self.playlist_builder.as_mut() {
            builder.note_truncated();
        }
    }

    /// Publishes the catalog, then validates the staged playlist rows against
    /// that exact snapshot so both share one publication id.
    fn publish(&mut self) -> Result<Option<PlaylistImportStats>, CatalogError> {
        let builder = self
            .builder
            .take()
            .ok_or(CatalogError::CatalogBankUnavailable)?;
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

    fn abort(&mut self) {
        if let Some(builder) = self.builder.take() {
            self.recycle_bank(builder.into_bank());
        }
        if let Some(builder) = self.playlist_builder.take() {
            self.recycle_playlist_store(builder.into_store());
        }
    }
}
