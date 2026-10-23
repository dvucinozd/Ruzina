//! Ordered Rekordbox playlist hierarchy and generation-safe Library browsing
//! (RP-06, M2.5 `rekordbox_pdb.c` / `ui_library.c`).
//!
//! Invalid rows are dropped and counted rather than failing the track
//! catalog: a node needs a unique id, a name and a parent chain of folders
//! that reaches the root without a cycle; an entry needs a published playlist
//! (not a folder) and a track present in the same catalog publication. A
//! track may appear in a playlist more than once.

use crate::{CatalogLoadCandidate, CatalogPublicationId, CatalogText, CatalogView};
use ruzina_media_session::MediaLease;

pub const CATALOG_MAX_PLAYLIST_NODES: usize = 256;
pub const CATALOG_MAX_PLAYLIST_ENTRIES: usize = 8_192;
pub const CATALOG_PLAYLIST_NAME_MAX: usize = 96;
/// Folder levels whose selection is restored on BACK; deeper levels still
/// navigate but return to the first row.
pub const LIBRARY_BROWSE_PATH_DEPTH: usize = 16;

/// Parent id of top-level nodes.
pub const PLAYLIST_ROOT: u32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaylistNode {
    pub id: u32,
    pub parent_id: u32,
    pub sort_order: u32,
    pub is_folder: bool,
    pub name: CatalogText<CATALOG_PLAYLIST_NAME_MAX>,
}

impl PlaylistNode {
    pub const EMPTY: Self = Self {
        id: 0,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: CatalogText::EMPTY,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaylistEntry {
    pub playlist_id: u32,
    pub entry_index: u32,
    pub source_track_id: u32,
    /// Catalog row of the track; resolved when the index is finished.
    pub track_index: u16,
    /// Export order, which breaks ties between equal entry indexes.
    ordinal: u16,
}

impl PlaylistEntry {
    pub const EMPTY: Self = Self {
        playlist_id: 0,
        entry_index: 0,
        source_track_id: 0,
        track_index: 0,
        ordinal: 0,
    };
}

/// Backing storage for one playlist index (inline for host/simulator, a
/// reserved PSRAM allocation on target). Pushes happen only below capacity.
pub trait PlaylistStore {
    fn node_capacity(&self) -> usize;
    fn entry_capacity(&self) -> usize;
    fn nodes(&self) -> &[PlaylistNode];
    fn nodes_mut(&mut self) -> &mut [PlaylistNode];
    fn entries(&self) -> &[PlaylistEntry];
    fn entries_mut(&mut self) -> &mut [PlaylistEntry];
    fn clear(&mut self);
    fn push_node_prechecked(&mut self, node: PlaylistNode);
    fn push_entry_prechecked(&mut self, entry: PlaylistEntry);
    fn truncate(&mut self, nodes: usize, entries: usize);
}

#[derive(Clone)]
pub struct InlinePlaylistStore<const N: usize, const E: usize> {
    nodes: [PlaylistNode; N],
    node_len: usize,
    entries: [PlaylistEntry; E],
    entry_len: usize,
}

impl<const N: usize, const E: usize> InlinePlaylistStore<N, E> {
    pub const fn new() -> Self {
        Self {
            nodes: [PlaylistNode::EMPTY; N],
            node_len: 0,
            entries: [PlaylistEntry::EMPTY; E],
            entry_len: 0,
        }
    }
}

impl<const N: usize, const E: usize> Default for InlinePlaylistStore<N, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize, const E: usize> PlaylistStore for InlinePlaylistStore<N, E> {
    fn node_capacity(&self) -> usize {
        N
    }
    fn entry_capacity(&self) -> usize {
        E
    }
    fn nodes(&self) -> &[PlaylistNode] {
        &self.nodes[..self.node_len]
    }
    fn nodes_mut(&mut self) -> &mut [PlaylistNode] {
        &mut self.nodes[..self.node_len]
    }
    fn entries(&self) -> &[PlaylistEntry] {
        &self.entries[..self.entry_len]
    }
    fn entries_mut(&mut self) -> &mut [PlaylistEntry] {
        &mut self.entries[..self.entry_len]
    }
    fn clear(&mut self) {
        self.node_len = 0;
        self.entry_len = 0;
    }
    fn push_node_prechecked(&mut self, node: PlaylistNode) {
        self.nodes[self.node_len] = node;
        self.node_len += 1;
    }
    fn push_entry_prechecked(&mut self, entry: PlaylistEntry) {
        self.entries[self.entry_len] = entry;
        self.entry_len += 1;
    }
    fn truncate(&mut self, nodes: usize, entries: usize) {
        self.node_len = self.node_len.min(nodes);
        self.entry_len = self.entry_len.min(entries);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaylistImportStats {
    pub nodes: u32,
    pub entries: u32,
    /// Rows dropped as malformed, unreachable, cyclic, duplicate or dangling.
    pub invalid_rows: u32,
    /// Node or entry capacity was reached; later rows were not read.
    pub truncated: bool,
}

pub struct PlaylistBuilder<S> {
    store: S,
    stats: PlaylistImportStats,
}

impl<S: PlaylistStore> PlaylistBuilder<S> {
    pub fn new(mut store: S) -> Self {
        store.clear();
        Self {
            store,
            stats: PlaylistImportStats::default(),
        }
    }

    pub const fn stats(&self) -> PlaylistImportStats {
        self.stats
    }

    fn node_limit(&self) -> usize {
        self.store.node_capacity().min(CATALOG_MAX_PLAYLIST_NODES)
    }

    fn entry_limit(&self) -> usize {
        self.store
            .entry_capacity()
            .min(CATALOG_MAX_PLAYLIST_ENTRIES)
            .min(u16::MAX as usize + 1)
    }

    /// A row the caller could not decode (zero id, bad name encoding).
    pub fn note_invalid_row(&mut self) {
        self.stats.invalid_rows = self.stats.invalid_rows.saturating_add(1);
    }

    /// The caller stopped reading a table early (slot or page-chain bound).
    pub fn note_truncated(&mut self) {
        self.stats.truncated = true;
    }

    /// Returns false once the node bound is reached; the caller should stop
    /// reading the tree table.
    pub fn push_node(
        &mut self,
        id: u32,
        parent_id: u32,
        sort_order: u32,
        is_folder: bool,
        name: &str,
    ) -> bool {
        let Ok(name) = CatalogText::try_from_str(name) else {
            self.note_invalid_row();
            return true;
        };
        if id == 0 || name.is_empty() {
            self.note_invalid_row();
            return true;
        }
        if self.store.nodes().len() >= self.node_limit() {
            self.stats.truncated = true;
            return false;
        }
        self.store.push_node_prechecked(PlaylistNode {
            id,
            parent_id,
            sort_order,
            is_folder,
            name,
        });
        true
    }

    /// Returns false once the entry bound is reached.
    pub fn push_entry(&mut self, playlist_id: u32, entry_index: u32, source_track_id: u32) -> bool {
        if playlist_id == 0 || source_track_id == 0 {
            self.note_invalid_row();
            return true;
        }
        let ordinal = self.store.entries().len();
        if ordinal >= self.entry_limit() {
            self.stats.truncated = true;
            return false;
        }
        self.store.push_entry_prechecked(PlaylistEntry {
            playlist_id,
            entry_index,
            source_track_id,
            track_index: 0,
            ordinal: ordinal as u16,
        });
        true
    }

    /// Validates the hierarchy against one catalog publication, orders
    /// siblings by (sort order, name, id) and entries by (entry index, export
    /// order), and binds the index to that publication.
    pub fn finish<V: CatalogView>(mut self, catalog: &V) -> PlaylistIndex<S> {
        let (kept_nodes, dropped_nodes) = retain_reachable_nodes(&mut self.store);
        let nodes = &mut self.store.nodes_mut()[..kept_nodes];
        nodes.sort_unstable_by(|a, b| {
            (a.parent_id, a.sort_order)
                .cmp(&(b.parent_id, b.sort_order))
                .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut kept_entries = 0usize;
        let mut dropped_entries = 0u32;
        for index in 0..self.store.entries().len() {
            let mut entry = self.store.entries()[index];
            let playlist = self.store.nodes()[..kept_nodes]
                .iter()
                .any(|node| node.id == entry.playlist_id && !node.is_folder);
            let track = (0..catalog.len()).find(|&row| {
                catalog
                    .get(row)
                    .is_some_and(|track| track.source_track_id == entry.source_track_id)
            });
            match (playlist, track) {
                (true, Some(row)) if row <= u16::MAX as usize => {
                    entry.track_index = row as u16;
                    self.store.entries_mut()[kept_entries] = entry;
                    kept_entries += 1;
                }
                _ => dropped_entries += 1,
            }
        }
        self.store.truncate(kept_nodes, kept_entries);
        self.store
            .entries_mut()
            .sort_unstable_by_key(|entry| (entry.playlist_id, entry.entry_index, entry.ordinal));

        self.stats.nodes = kept_nodes as u32;
        self.stats.entries = kept_entries as u32;
        self.stats.invalid_rows = self
            .stats
            .invalid_rows
            .saturating_add(dropped_nodes)
            .saturating_add(dropped_entries);
        PlaylistIndex {
            publication: CatalogPublicationId {
                lease: catalog.lease(),
                revision: catalog.revision(),
            },
            store: self.store,
            stats: self.stats,
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

const PARENT_ROOT: u16 = u16::MAX;
const PARENT_MISSING: u16 = u16::MAX - 1;

/// Keeps, in export order, the first node of every id whose parent chain
/// reaches the root through folders only. Returns (kept, dropped).
fn retain_reachable_nodes<S: PlaylistStore>(store: &mut S) -> (usize, u32) {
    let nodes = store.nodes_mut();
    let count = nodes.len().min(CATALOG_MAX_PLAYLIST_NODES);
    let mut parent = [PARENT_MISSING; CATALOG_MAX_PLAYLIST_NODES];
    let mut first = [false; CATALOG_MAX_PLAYLIST_NODES];
    for index in 0..count {
        first[index] = !nodes[..index].iter().any(|node| node.id == nodes[index].id);
        parent[index] = if nodes[index].parent_id == PLAYLIST_ROOT {
            PARENT_ROOT
        } else {
            nodes[..count]
                .iter()
                .position(|node| node.id == nodes[index].parent_id)
                .map_or(PARENT_MISSING, |at| at as u16)
        };
    }

    let mut valid = [false; CATALOG_MAX_PLAYLIST_NODES];
    for index in 0..count {
        if !first[index] {
            continue;
        }
        let mut at = index;
        let mut reached_root = false;
        for _ in 0..=count {
            match parent[at] {
                PARENT_ROOT => {
                    reached_root = true;
                    break;
                }
                PARENT_MISSING => break,
                next => {
                    let next = usize::from(next);
                    if next == index || !nodes[next].is_folder {
                        break;
                    }
                    at = next;
                }
            }
        }
        valid[index] = reached_root;
    }

    let mut kept = 0usize;
    for index in 0..count {
        if valid[index] {
            nodes[kept] = nodes[index];
            kept += 1;
        }
    }
    (kept, (nodes.len() - kept) as u32)
}

pub struct PlaylistIndex<S> {
    publication: CatalogPublicationId,
    store: S,
    stats: PlaylistImportStats,
}

impl<S: PlaylistStore> PlaylistIndex<S> {
    pub const fn publication(&self) -> CatalogPublicationId {
        self.publication
    }

    pub const fn stats(&self) -> PlaylistImportStats {
        self.stats
    }

    pub fn is_empty(&self) -> bool {
        self.store.nodes().is_empty()
    }

    pub fn node(&self, id: u32) -> Option<&PlaylistNode> {
        self.store.nodes().iter().find(|node| node.id == id)
    }

    /// Ordered children of a folder (or of [`PLAYLIST_ROOT`]).
    pub fn children(&self, parent_id: u32) -> &[PlaylistNode] {
        let nodes = self.store.nodes();
        let start = nodes.partition_point(|node| node.parent_id < parent_id);
        let end = nodes.partition_point(|node| node.parent_id <= parent_id);
        &nodes[start..end]
    }

    /// Ordered entries of a playlist; empty for folders and unknown ids.
    pub fn tracks(&self, playlist_id: u32) -> &[PlaylistEntry] {
        let entries = self.store.entries();
        let start = entries.partition_point(|entry| entry.playlist_id < playlist_id);
        let end = entries.partition_point(|entry| entry.playlist_id <= playlist_id);
        &entries[start..end]
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryLevel {
    AllTracks,
    /// Children of a folder; [`PLAYLIST_ROOT`] is the top level.
    Folder(u32),
    Playlist(u32),
}

// no_std without an allocator: the candidate is copied by value here exactly
// as `CatalogLoadCandidate` is everywhere else on the load path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryRow {
    Node(PlaylistNode),
    Track(CatalogLoadCandidate),
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryActivation {
    /// LOAD on a folder/playlist row opened it; nothing is loaded.
    Opened,
    Load(CatalogLoadCandidate),
    Empty,
    /// The catalog or playlist publication changed since this view was built.
    Stale,
}

/// Library cursor over the flat track list and the playlist tree. Every
/// operation is checked against the catalog publication and the playlist
/// index built for it, so a load never resolves a row of an older export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LibraryBrowser {
    publication: Option<CatalogPublicationId>,
    level: LibraryLevel,
    selected: u16,
    count: u16,
    all_selected: u16,
    path: [u16; LIBRARY_BROWSE_PATH_DEPTH],
    depth: u16,
}

impl Default for LibraryBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryBrowser {
    pub const fn new() -> Self {
        Self {
            publication: None,
            level: LibraryLevel::AllTracks,
            selected: 0,
            count: 0,
            all_selected: 0,
            path: [0; LIBRARY_BROWSE_PATH_DEPTH],
            depth: 0,
        }
    }

    pub const fn publication(&self) -> Option<CatalogPublicationId> {
        self.publication
    }

    pub const fn level(&self) -> LibraryLevel {
        self.level
    }

    pub const fn count(&self) -> u16 {
        self.count
    }

    pub const fn selected_index(&self) -> Option<u16> {
        if self.count == 0 {
            None
        } else {
            Some(self.selected)
        }
    }

    /// True while LOAD would open a folder/playlist instead of loading.
    pub const fn on_node_row(&self) -> bool {
        matches!(self.level, LibraryLevel::Folder(_)) && self.count != 0
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn invalidate(&mut self, lease: MediaLease) {
        if self
            .publication
            .is_some_and(|current| current.lease == lease)
        {
            self.clear();
        }
    }

    /// Binds to the current publication. A new publication restarts at the
    /// track list; the same one keeps the level and clamps the cursor.
    pub fn publish<V: CatalogView, S: PlaylistStore>(
        &mut self,
        catalog: &V,
        playlists: Option<&PlaylistIndex<S>>,
    ) {
        let publication = CatalogPublicationId {
            lease: catalog.lease(),
            revision: catalog.revision(),
        };
        if self.publication != Some(publication) {
            *self = Self::new();
            self.publication = Some(publication);
        }
        let playlists = playlists.filter(|index| index.publication == publication);
        if playlists.is_none() && self.level != LibraryLevel::AllTracks {
            self.level = LibraryLevel::AllTracks;
            self.selected = self.all_selected;
            self.depth = 0;
        }
        self.count = level_len(self.level, catalog, playlists);
        self.clamp();
    }

    pub fn browse(&mut self, delta: i16, multiplier: u16) -> bool {
        if self.count == 0 || delta == 0 {
            return false;
        }
        let step = i32::from(delta).saturating_mul(i32::from(multiplier.max(1)));
        let last = i32::from(self.count - 1);
        let next = i32::from(self.selected).saturating_add(step).clamp(0, last) as u16;
        let changed = next != self.selected;
        self.selected = next;
        changed
    }

    /// The donor PLAYLISTS/BACK action: track list -> top level, playlist ->
    /// its folder, folder -> parent, top level -> track list.
    pub fn back<V: CatalogView, S: PlaylistStore>(
        &mut self,
        catalog: &V,
        playlists: Option<&PlaylistIndex<S>>,
    ) -> bool {
        let Some(playlists) = self.current(catalog, playlists) else {
            return false;
        };
        let next = match self.level {
            LibraryLevel::AllTracks => {
                if playlists.children(PLAYLIST_ROOT).is_empty() {
                    return false;
                }
                self.all_selected = self.selected;
                self.enter_level(LibraryLevel::Folder(PLAYLIST_ROOT), catalog, playlists);
                return true;
            }
            LibraryLevel::Folder(PLAYLIST_ROOT) => {
                self.level = LibraryLevel::AllTracks;
                self.count = level_len(self.level, catalog, Some(playlists));
                self.selected = self.all_selected;
                self.depth = 0;
                self.clamp();
                return true;
            }
            LibraryLevel::Folder(id) | LibraryLevel::Playlist(id) => playlists
                .node(id)
                .map_or(PLAYLIST_ROOT, |node| node.parent_id),
        };
        self.level = LibraryLevel::Folder(next);
        self.count = level_len(self.level, catalog, Some(playlists));
        self.depth = self.depth.saturating_sub(1);
        self.selected = self.path.get(usize::from(self.depth)).copied().unwrap_or(0);
        self.clamp();
        true
    }

    /// LOAD on the selected row: open a folder/playlist or resolve a track.
    pub fn activate<V: CatalogView, S: PlaylistStore>(
        &mut self,
        catalog: &V,
        playlists: Option<&PlaylistIndex<S>>,
    ) -> LibraryActivation {
        if self.publication
            != Some(CatalogPublicationId {
                lease: catalog.lease(),
                revision: catalog.revision(),
            })
        {
            return LibraryActivation::Stale;
        }
        if self.count == 0 {
            return LibraryActivation::Empty;
        }
        match self.row(catalog, playlists) {
            Some(LibraryRow::Track(candidate)) => LibraryActivation::Load(candidate),
            Some(LibraryRow::Node(node)) => {
                let Some(playlists) = self.current(catalog, playlists) else {
                    return LibraryActivation::Stale;
                };
                let level = if node.is_folder {
                    LibraryLevel::Folder(node.id)
                } else {
                    LibraryLevel::Playlist(node.id)
                };
                self.enter_level(level, catalog, playlists);
                LibraryActivation::Opened
            }
            None => LibraryActivation::Stale,
        }
    }

    /// The row under the cursor, for presentation and load resolution.
    pub fn row<V: CatalogView, S: PlaylistStore>(
        &self,
        catalog: &V,
        playlists: Option<&PlaylistIndex<S>>,
    ) -> Option<LibraryRow> {
        self.row_at(self.selected, catalog, playlists)
    }

    /// Row `index` of the current level (rendering a visible page).
    pub fn row_at<V: CatalogView, S: PlaylistStore>(
        &self,
        index: u16,
        catalog: &V,
        playlists: Option<&PlaylistIndex<S>>,
    ) -> Option<LibraryRow> {
        let publication = self.publication?;
        if publication.lease != catalog.lease()
            || publication.revision != catalog.revision()
            || index >= self.count
            || level_len(self.level, catalog, self.current(catalog, playlists)) != self.count
        {
            return None;
        }
        let track_row = match self.level {
            LibraryLevel::AllTracks => usize::from(index),
            LibraryLevel::Folder(id) => {
                let node = *self
                    .current(catalog, playlists)?
                    .children(id)
                    .get(usize::from(index))?;
                return Some(LibraryRow::Node(node));
            }
            LibraryLevel::Playlist(id) => usize::from(
                self.current(catalog, playlists)?
                    .tracks(id)
                    .get(usize::from(index))?
                    .track_index,
            ),
        };
        let track = *catalog.get(track_row)?;
        Some(LibraryRow::Track(CatalogLoadCandidate {
            publication,
            index: track_row as u16,
            track,
        }))
    }

    fn current<'a, V: CatalogView, S: PlaylistStore>(
        &self,
        catalog: &V,
        playlists: Option<&'a PlaylistIndex<S>>,
    ) -> Option<&'a PlaylistIndex<S>> {
        let publication = self.publication?;
        (publication.lease == catalog.lease() && publication.revision == catalog.revision())
            .then_some(playlists?)
            .filter(|index| index.publication == publication)
    }

    fn enter_level<V: CatalogView, S: PlaylistStore>(
        &mut self,
        level: LibraryLevel,
        catalog: &V,
        playlists: &PlaylistIndex<S>,
    ) {
        if self.level != LibraryLevel::AllTracks {
            if let Some(slot) = self.path.get_mut(usize::from(self.depth)) {
                *slot = self.selected;
            }
            self.depth = self.depth.saturating_add(1);
        }
        self.level = level;
        self.count = level_len(level, catalog, Some(playlists));
        self.selected = 0;
    }

    fn clamp(&mut self) {
        if self.count == 0 {
            self.selected = 0;
        } else if self.selected >= self.count {
            self.selected = self.count - 1;
        }
    }
}

fn level_len<V: CatalogView, S: PlaylistStore>(
    level: LibraryLevel,
    catalog: &V,
    playlists: Option<&PlaylistIndex<S>>,
) -> u16 {
    let len = match (level, playlists) {
        (LibraryLevel::AllTracks, _) => catalog.len(),
        (LibraryLevel::Folder(id), Some(index)) => index.children(id).len(),
        (LibraryLevel::Playlist(id), Some(index)) => index.tracks(id).len(),
        (_, None) => 0,
    };
    len.min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CatalogBuilder, CatalogSnapshot, CatalogTrack};
    use ruzina_core::MediaTrackId;
    use ruzina_media_session::{MediaGeneration, MediaSourceId};

    type Store = InlinePlaylistStore<16, 32>;

    fn lease(source: u32) -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(source).unwrap(),
        }
    }

    fn track(id: u8, source_track_id: u32) -> CatalogTrack {
        CatalogTrack {
            track_id: MediaTrackId([id; 16]),
            source_track_id,
            title: CatalogText::try_from_str("t").unwrap(),
            ..CatalogTrack::EMPTY
        }
    }

    fn catalog(source: u32, revision: u32) -> CatalogSnapshot<4> {
        let mut builder = CatalogBuilder::<4>::new(lease(source));
        for (id, source_track_id) in [(1, 10), (2, 20), (3, 30)] {
            builder.push(track(id, source_track_id)).unwrap();
        }
        builder.finish(revision)
    }

    /// root: [Warmup (playlist, sort 2), Clubs (folder, sort 1)]
    /// Clubs: [Peak (playlist)]
    fn fixture(catalog: &CatalogSnapshot<4>) -> PlaylistIndex<Store> {
        let mut builder = PlaylistBuilder::new(Store::new());
        assert!(builder.push_node(5, 0, 2, false, "Warmup"));
        assert!(builder.push_node(7, 0, 1, true, "Clubs"));
        assert!(builder.push_node(9, 7, 0, false, "Peak"));
        for (list, index, track) in [(5, 2, 30), (5, 1, 10), (9, 0, 20), (9, 1, 20)] {
            assert!(builder.push_entry(list, index, track));
        }
        builder.finish(catalog)
    }

    fn node_name(row: Option<LibraryRow>) -> &'static str {
        match row {
            Some(LibraryRow::Node(node)) => match node.name.as_str() {
                "Warmup" => "Warmup",
                "Clubs" => "Clubs",
                "Peak" => "Peak",
                _ => "?",
            },
            _ => "-",
        }
    }

    fn track_source(row: Option<LibraryRow>) -> u32 {
        match row {
            Some(LibraryRow::Track(candidate)) => candidate.track.source_track_id,
            _ => 0,
        }
    }

    #[test]
    fn tree_orders_siblings_and_entries_and_keeps_duplicate_tracks() {
        let catalog = catalog(1, 1);
        let index = fixture(&catalog);
        let root: [&str; 2] = [
            index.children(PLAYLIST_ROOT)[0].name.as_str(),
            index.children(PLAYLIST_ROOT)[1].name.as_str(),
        ];
        assert_eq!(root, ["Clubs", "Warmup"]);
        assert_eq!(index.children(7)[0].name.as_str(), "Peak");
        assert!(index.children(5).is_empty());

        let warmup: [u32; 2] = [
            index.tracks(5)[0].source_track_id,
            index.tracks(5)[1].source_track_id,
        ];
        assert_eq!(warmup, [10, 30]);
        assert_eq!(index.tracks(5)[1].track_index, 2);
        assert_eq!(index.tracks(9).len(), 2);
        assert!(index.tracks(7).is_empty());
        assert_eq!(
            index.stats(),
            PlaylistImportStats {
                nodes: 3,
                entries: 4,
                invalid_rows: 0,
                truncated: false,
            }
        );
    }

    #[test]
    fn invalid_cyclic_dangling_and_duplicate_rows_are_dropped_and_counted() {
        let catalog = catalog(1, 1);
        let mut builder = PlaylistBuilder::new(Store::new());
        builder.push_node(1, 0, 0, true, "Root folder");
        builder.push_node(1, 0, 1, true, "Duplicate id");
        builder.push_node(0, 0, 0, false, "Zero id");
        builder.push_node(2, 0, 0, false, "");
        builder.push_node(3, 4, 0, true, "Cycle A");
        builder.push_node(4, 3, 0, true, "Cycle B");
        builder.push_node(6, 6, 0, true, "Self parent");
        builder.push_node(8, 99, 0, false, "Missing parent");
        builder.push_node(10, 11, 0, false, "Under playlist");
        builder.push_node(11, 1, 0, false, "Playlist");
        builder.push_node(12, 3, 0, false, "Under cycle");
        builder.push_entry(11, 0, 10);
        builder.push_entry(11, 1, 77); // track not in this catalog
        builder.push_entry(1, 0, 10); // folder
        builder.push_entry(8, 0, 10); // dropped playlist
        builder.push_entry(0, 0, 10); // zero playlist id
        builder.push_entry(11, 2, 0); // zero track id
        let index = builder.finish(&catalog);

        assert_eq!(index.children(PLAYLIST_ROOT).len(), 1);
        assert_eq!(
            index.children(PLAYLIST_ROOT)[0].name.as_str(),
            "Root folder"
        );
        assert_eq!(index.children(1)[0].id, 11);
        assert_eq!(index.tracks(11).len(), 1);
        assert_eq!(index.stats().nodes, 2);
        assert_eq!(index.stats().entries, 1);
        // 2 malformed + 7 unreachable/duplicate nodes, 2 malformed + 3 dangling entries.
        assert_eq!(index.stats().invalid_rows, 14);
        assert!(!index.stats().truncated);
    }

    #[test]
    fn node_and_entry_bounds_set_truncation() {
        let mut builder = PlaylistBuilder::new(InlinePlaylistStore::<2, 1>::new());
        assert!(builder.push_node(1, 0, 0, false, "A"));
        assert!(builder.push_node(2, 0, 0, false, "B"));
        assert!(!builder.push_node(3, 0, 0, false, "C"));
        assert!(builder.push_entry(1, 0, 10));
        assert!(!builder.push_entry(1, 1, 20));
        assert!(builder.stats().truncated);
        let index = builder.finish(&catalog(1, 1));
        assert_eq!(index.stats().nodes, 2);
        assert!(index.stats().truncated);
    }

    #[test]
    fn browser_navigates_the_tree_and_loads_playlist_tracks() {
        let catalog = catalog(1, 1);
        let index = fixture(&catalog);
        let playlists = Some(&index);
        let mut browser = LibraryBrowser::new();
        browser.publish(&catalog, playlists);
        assert_eq!(browser.count(), 3);
        browser.browse(1, 1);

        assert!(browser.back(&catalog, playlists));
        assert_eq!(browser.level(), LibraryLevel::Folder(PLAYLIST_ROOT));
        assert!(browser.on_node_row());
        assert_eq!(node_name(browser.row(&catalog, playlists)), "Clubs");

        assert_eq!(
            browser.activate(&catalog, playlists),
            LibraryActivation::Opened
        );
        assert_eq!(browser.level(), LibraryLevel::Folder(7));
        assert_eq!(node_name(browser.row(&catalog, playlists)), "Peak");
        assert_eq!(
            browser.activate(&catalog, playlists),
            LibraryActivation::Opened
        );
        assert_eq!(browser.level(), LibraryLevel::Playlist(9));
        assert!(!browser.on_node_row());
        assert_eq!(browser.count(), 2);
        browser.browse(1, 1);
        let LibraryActivation::Load(candidate) = browser.activate(&catalog, playlists) else {
            panic!("track row must resolve a load");
        };
        assert_eq!(candidate.track.source_track_id, 20);
        assert_eq!(candidate.index, 1);
        assert_eq!(candidate.publication.revision, 1);

        // BACK restores each enclosing selection, then the track-list cursor.
        assert!(browser.back(&catalog, playlists));
        assert_eq!(browser.level(), LibraryLevel::Folder(7));
        assert!(browser.back(&catalog, playlists));
        assert_eq!(browser.level(), LibraryLevel::Folder(PLAYLIST_ROOT));
        assert_eq!(node_name(browser.row(&catalog, playlists)), "Clubs");
        browser.browse(1, 1);
        assert_eq!(
            browser.activate(&catalog, playlists),
            LibraryActivation::Opened
        );
        assert_eq!(track_source(browser.row(&catalog, playlists)), 10);
        assert!(browser.back(&catalog, playlists));
        assert_eq!(node_name(browser.row(&catalog, playlists)), "Warmup");
        assert!(browser.back(&catalog, playlists));
        assert_eq!(browser.level(), LibraryLevel::AllTracks);
        assert_eq!(browser.selected_index(), Some(1));
        assert_eq!(track_source(browser.row(&catalog, playlists)), 20);
    }

    #[test]
    fn browser_rejects_rows_of_an_older_publication() {
        let old = catalog(1, 1);
        let index = fixture(&old);
        let mut browser = LibraryBrowser::new();
        browser.publish(&old, Some(&index));
        assert!(browser.back(&old, Some(&index)));
        assert_eq!(
            browser.activate(&old, Some(&index)),
            LibraryActivation::Opened
        );

        // A re-export of the same medium: the old cursor resolves nothing.
        let next = catalog(1, 2);
        assert_eq!(browser.row(&next, Some(&index)), None);
        assert_eq!(
            browser.activate(&next, Some(&index)),
            LibraryActivation::Stale
        );
        assert!(!browser.back(&next, Some(&index)));

        // Republishing restarts at the track list; a playlist index built
        // for the old revision is not used for the new one.
        browser.publish(&next, Some(&index));
        assert_eq!(browser.level(), LibraryLevel::AllTracks);
        assert!(!browser.back(&next, Some(&index)));
        let fresh = fixture(&next);
        assert!(browser.back(&next, Some(&fresh)));

        browser.invalidate(lease(2));
        assert!(browser.publication().is_some());
        browser.invalidate(lease(1));
        assert_eq!(browser.publication(), None);
        assert_eq!(browser.selected_index(), None);
    }

    #[test]
    fn browser_without_playlists_stays_on_the_track_list() {
        let catalog = catalog(1, 1);
        let mut browser = LibraryBrowser::new();
        browser.publish::<_, Store>(&catalog, None);
        assert!(!browser.back::<_, Store>(&catalog, None));
        assert_eq!(browser.level(), LibraryLevel::AllTracks);
        let LibraryActivation::Load(candidate) = browser.activate::<_, Store>(&catalog, None)
        else {
            panic!("track list must resolve a load");
        };
        assert_eq!(candidate.track.source_track_id, 10);
    }
}
