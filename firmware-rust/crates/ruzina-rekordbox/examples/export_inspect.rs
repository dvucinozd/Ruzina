//! Host-only replay of the firmware USB0 library import (`usb0_library.rs`)
//! over a copied `export.pdb`: the same primitives, page walk, 8 KiB reads and
//! error policies, printed as TSV for comparison with the M2.5 donor parser.
use ruzina_media_catalog::playlists::{InlinePlaylistStore, PlaylistBuilder};
use ruzina_media_catalog::{CatalogTrack, CatalogView, MEDIA_CATALOG_MAX_TRACKS};
use ruzina_media_identity::{VolumeIdentity, derive_track_id};
use ruzina_media_session::{ConnectResult, MediaLease, MediaSession, MediaSourceId};
use ruzina_rekordbox::{
    PDB_MAX_NAMES, PDB_NAME_MAX, PDB_PAGE_HEAP_OFFSET, PDB_PATH_MAX, PDB_PLAYLIST_ENTRY_ROW_BYTES,
    PDB_PLAYLIST_TREE_ROW_HEADER_BYTES, PDB_TABLE_ALBUMS, PDB_TABLE_ARTISTS, PDB_TABLE_ARTWORK,
    PDB_TABLE_KEYS, PDB_TABLE_PLAYLIST_ENTRIES, PDB_TABLE_PLAYLIST_TREE, PDB_TABLE_TRACKS,
    PDB_TITLE_MAX, PDB_TRACK_ROW_HEADER_BYTES, PdbHeader, PdbNameEntry, PdbNameLookup,
    PdbNameRowHeader, PdbNameRowLayout, PdbPageSlotWalker, PdbTableDirectory, PdbTableEntry,
    PdbText, PdbTrackRow, catalog_track_from_pdb, parse_devicesql_field_header, parse_pdb_header,
    parse_pdb_name_row_header, parse_pdb_page_header, parse_pdb_playlist_entry_row,
    parse_pdb_playlist_tree_row, parse_pdb_table_directory, parse_pdb_track_row_header,
    pdb_track_string_page_offset, recover_pdb_title, resolve_pdb_track_names,
};

/// `USB0_LIBRARY_PDB_IO_BYTES`.
const IO_BYTES: usize = 8_192;

fn read_at(file: &[u8], position: u64) -> &[u8] {
    let start = (position as usize).min(file.len());
    &file[start..(start + IO_BYTES).min(file.len())]
}

fn read_text<const N: usize>(
    file: &[u8],
    page_start: u64,
    page_size: u32,
    page_relative: u32,
) -> Result<PdbText<N>, String> {
    let buffer = read_at(file, page_start + u64::from(page_relative));
    let field = parse_devicesql_field_header(buffer, 0).map_err(|e| format!("{e:?}"))?;
    if page_relative + u32::from(field.total_len) > page_size
        || field.total_len_usize() > buffer.len()
    {
        return Err("DeviceSqlTruncated".into());
    }
    PdbText::from_devicesql_field(&buffer[..field.total_len_usize()]).map_err(|e| format!("{e:?}"))
}

fn read_track_text<const N: usize>(
    file: &[u8],
    page_start: u64,
    page_size: u32,
    heap_offset: u16,
    relative: u16,
) -> Result<PdbText<N>, String> {
    match pdb_track_string_page_offset(heap_offset, relative, page_size) {
        Ok(Some(offset)) => read_text(file, page_start, page_size, offset),
        Ok(None) => Ok(PdbText::new()),
        Err(error) => Err(format!("{error:?}")),
    }
}

fn read_name_text(
    file: &[u8],
    page_start: u64,
    page_size: u32,
    heap_offset: u16,
    header: PdbNameRowHeader,
) -> Result<PdbText<PDB_NAME_MAX>, String> {
    let offset = PDB_PAGE_HEAP_OFFSET as u32
        + u32::from(heap_offset)
        + u32::from(header.text_relative_offset);
    if offset >= page_size {
        return Err("NameRowOutOfRange".into());
    }
    read_text(file, page_start, page_size, offset)
}

/// Calls `row` for each present row of a table's page chain, as the firmware
/// walkers do.
fn walk_table(
    file: &[u8],
    header: PdbHeader,
    table: PdbTableEntry,
    mut row: impl FnMut(u64, u16) -> Result<(), String>,
) -> Result<(u32, bool), String> {
    let mut page = table.first_page;
    let mut pages = 0u32;
    let mut truncated = false;
    loop {
        if page >= header.total_pages {
            return Err(format!("PageOutOfRange({page}) after {pages} pages"));
        }
        if pages >= header.total_pages {
            return Err("PageChainLoop".into());
        }
        let page_start = u64::from(page) * u64::from(header.page_size);
        let page_header = parse_pdb_page_header(read_at(file, page_start), header.page_size)
            .map_err(|e| format!("page {page}: {e:?}"))?;
        let mut walker = PdbPageSlotWalker::new(header.page_size, page_header.declared_rows)
            .map_err(|e| format!("page {page}: {e:?}"))?;
        while let Some(plan) = walker.next_batch_plan() {
            let offsets = walker
                .ingest_batch(plan, read_at(file, page_start + u64::from(plan.offset)))
                .map_err(|e| format!("page {page}: {e:?}"))?;
            for heap_offset in offsets.offsets().iter().copied() {
                row(page_start, heap_offset).map_err(|e| format!("page {page}: {e}"))?;
            }
        }
        truncated |= walker.summary().truncated;
        pages += 1;
        let Some(next_page) = page_header.next_in_table(page, table) else {
            return Ok((pages, truncated));
        };
        page = next_page;
    }
}

#[derive(Default)]
struct NameMap {
    entries: Vec<PdbNameEntry>,
    capacity: usize,
    optional: bool,
    seen: u32,
}

impl PdbNameLookup for NameMap {
    fn lookup(&self, id: u32) -> Option<&PdbText<PDB_NAME_MAX>> {
        (id != 0)
            .then(|| self.entries.iter().find(|entry| entry.id == id))
            .flatten()
            .map(|entry| &entry.name)
    }
}

fn scan_names(
    file: &[u8],
    header: PdbHeader,
    table: PdbTableEntry,
    layout: PdbNameRowLayout,
    map: &mut NameMap,
) -> Result<(u32, bool), String> {
    walk_table(file, header, table, |page_start, heap_offset| {
        let page_relative = PDB_PAGE_HEAP_OFFSET as u32 + u32::from(heap_offset);
        if page_relative + layout.minimum_bytes() as u32 > header.page_size {
            return Err("NameRowOutOfRange".into());
        }
        let row = read_at(file, page_start + u64::from(page_relative));
        let Some(row_header) =
            parse_pdb_name_row_header(row, layout).map_err(|e| format!("{e:?}"))?
        else {
            return Ok(());
        };
        let name = read_name_text(file, page_start, header.page_size, heap_offset, row_header)?;
        if let Some(entry) = PdbNameEntry::from_header_and_field(row_header, name) {
            map.seen += 1;
            if map.entries.iter().any(|existing| existing.id == entry.id) {
                if map.optional {
                    return Ok(());
                }
                return Err(format!("DuplicateNameId({})", entry.id));
            }
            if map.entries.len() < map.capacity {
                map.entries.push(entry);
            }
        }
        Ok(())
    })
}

struct Catalog {
    lease: MediaLease,
    tracks: Vec<CatalogTrack>,
}

impl CatalogView for Catalog {
    fn lease(&self) -> MediaLease {
        self.lease
    }
    fn revision(&self) -> u32 {
        1
    }
    fn len(&self) -> usize {
        self.tracks.len()
    }
    fn get(&self, index: usize) -> Option<&CatalogTrack> {
        self.tracks.get(index)
    }
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\n'], " ")
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [path] = args.as_slice() else {
        return Err("usage: export_inspect export.pdb".into());
    };
    let file = std::fs::read(path).map_err(|e| e.to_string())?;
    let header =
        parse_pdb_header(read_at(&file, 0), file.len() as u64).map_err(|e| format!("{e:?}"))?;
    let tables: PdbTableDirectory =
        parse_pdb_table_directory(read_at(&file, 0), header).map_err(|e| format!("{e:?}"))?;
    let first = |kind| {
        tables
            .find(kind)
            .filter(|entry| entry.first_page != u32::MAX)
    };
    eprintln!("header {header:?}");
    for entry in tables.entries() {
        eprintln!("table {entry:?}");
    }

    let new_map = |capacity, optional| NameMap {
        capacity,
        optional,
        ..NameMap::default()
    };
    let mut artists = new_map(PDB_MAX_NAMES, false);
    let mut albums = new_map(PDB_MAX_NAMES, false);
    let mut keys = new_map(PDB_MAX_NAMES, false);
    let mut artworks = new_map(MEDIA_CATALOG_MAX_TRACKS, true);
    for (name, kind, layout, map) in [
        (
            "artists",
            PDB_TABLE_ARTISTS,
            PdbNameRowLayout::Artist,
            &mut artists,
        ),
        (
            "albums",
            PDB_TABLE_ALBUMS,
            PdbNameRowLayout::Album,
            &mut albums,
        ),
        ("keys", PDB_TABLE_KEYS, PdbNameRowLayout::Key, &mut keys),
    ] {
        let Some(page) = first(kind) else { continue };
        let (pages, truncated) = scan_names(&file, header, page, layout, map)
            .map_err(|e| format!("IMPORT FAILED in {name}: {e}"))?;
        if truncated || map.seen as usize > PDB_MAX_NAMES {
            return Err(format!(
                "IMPORT FAILED in {name}: truncated or over capacity"
            ));
        }
        eprintln!("{name}: {} entries, {pages} pages", map.entries.len());
    }
    if let Some(page) = first(PDB_TABLE_ARTWORK) {
        // Covers are optional: a malformed table leaves every track on the fallback.
        match scan_names(
            &file,
            header,
            page,
            PdbNameRowLayout::Artwork,
            &mut artworks,
        ) {
            Ok((pages, _)) => eprintln!(
                "artworks: {} entries, {pages} pages",
                artworks.entries.len()
            ),
            Err(error) => {
                eprintln!("artworks dropped: {error}");
                artworks.entries.clear();
            }
        }
    }

    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap()) else {
        unreachable!()
    };
    let volume = VolumeIdentity([0x5a; 32]);
    let mut catalog = Catalog {
        lease,
        tracks: Vec::new(),
    };
    let tracks_page = first(PDB_TABLE_TRACKS).ok_or("IMPORT FAILED: MissingTracksTable")?;
    let (pages, truncated) = walk_table(&file, header, tracks_page, |page_start, heap_offset| {
        let page_relative = PDB_PAGE_HEAP_OFFSET as u32 + u32::from(heap_offset);
        if page_relative + PDB_TRACK_ROW_HEADER_BYTES as u32 > header.page_size {
            return Err("TrackRowOutOfRange".into());
        }
        let row = read_at(&file, page_start + u64::from(page_relative));
        let Some(track_header) = parse_pdb_track_row_header(row).map_err(|e| format!("{e:?}"))?
        else {
            return Ok(());
        };
        let text = |relative| {
            read_track_text::<PDB_TITLE_MAX>(
                &file,
                page_start,
                header.page_size,
                heap_offset,
                relative,
            )
        };
        let path = |relative| {
            read_track_text::<PDB_PATH_MAX>(
                &file,
                page_start,
                header.page_size,
                heap_offset,
                relative,
            )
        };
        let title = text(track_header.strings.title).unwrap_or_else(|_| {
            recover_pdb_title::<PDB_TITLE_MAX>(Err(ruzina_rekordbox::PdbError::DeviceSqlTruncated))
        });
        let track = PdbTrackRow::from_header_and_fields(
            track_header,
            title,
            text(track_header.strings.filename)?,
            path(track_header.strings.file_path)?,
            path(track_header.strings.anlz_path)?,
        );
        if track.file_path.was_truncated() {
            return Err(format!("track {}: PathTooLong", track.track_id));
        }
        let audio_path = track
            .file_path
            .as_str()
            .ok_or(format!("track {}: InvalidTrackPathUtf8", track.track_id))?;
        derive_track_id(volume, audio_path)
            .map_err(|e| format!("track {}: {e:?}", track.track_id))?;
        let resolved = resolve_pdb_track_names(&track, &artists, &albums, &keys)
            .with_artwork(&track, &artworks);
        let entry = catalog_track_from_pdb(volume, &track, resolved)
            .map_err(|e| format!("track {}: {e:?}", track.track_id))?;
        if catalog.tracks.len() >= MEDIA_CATALOG_MAX_TRACKS {
            return Err("CatalogBuild Full".into());
        }
        if catalog.tracks.iter().any(|t| t.track_id == entry.track_id) {
            return Err(format!(
                "DuplicateTrackId for source {}",
                entry.source_track_id
            ));
        }
        if catalog
            .tracks
            .iter()
            .any(|t| t.source_track_id == entry.source_track_id)
        {
            return Err(format!("DuplicateSourceTrackId({})", entry.source_track_id));
        }
        catalog.tracks.push(entry);
        Ok(())
    })
    .map_err(|e| format!("IMPORT FAILED in tracks: {e}"))?;
    if truncated {
        return Err("IMPORT FAILED: CatalogTableTruncated".into());
    }
    eprintln!("tracks: {} rows, {pages} pages", catalog.tracks.len());

    for t in &catalog.tracks {
        println!(
            "T\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            t.source_track_id,
            clean(t.title.as_str()),
            clean(t.artist.as_str()),
            clean(t.album.as_str()),
            clean(t.key.as_str()),
            (t.bpm_x100 + 50) / 100,
            t.duration_ms / 1_000,
            clean(t.audio_path.as_str()),
            clean(t.analysis_path.as_str()),
            clean(t.artwork_path.as_str()),
        );
    }

    // Playlists: tree then entries, each degrading to invalid/truncated stats.
    let mut builder = PlaylistBuilder::new(InlinePlaylistStore::<256, 8_192>::new());
    let tree_page = first(PDB_TABLE_PLAYLIST_TREE);
    let entries_page = tree_page.and(first(PDB_TABLE_PLAYLIST_ENTRIES));
    for (is_tree, page) in [(true, tree_page), (false, entries_page)] {
        let Some(page) = page else { continue };
        let row_bytes = if is_tree {
            PDB_PLAYLIST_TREE_ROW_HEADER_BYTES as u32 + 1
        } else {
            PDB_PLAYLIST_ENTRY_ROW_BYTES as u32
        };
        let result = walk_table(&file, header, page, |page_start, heap_offset| {
            let page_relative = PDB_PAGE_HEAP_OFFSET as u32 + u32::from(heap_offset);
            if page_relative + row_bytes > header.page_size {
                builder.note_invalid_row();
                return Ok(());
            }
            let row = read_at(&file, page_start + u64::from(page_relative));
            let keep = if is_tree {
                match parse_pdb_playlist_tree_row(row) {
                    Ok(Some(node)) => {
                        match read_name_text(
                            &file,
                            page_start,
                            header.page_size,
                            heap_offset,
                            node.name,
                        ) {
                            Ok(name) => match name.as_str() {
                                Some(name) => {
                                    println!(
                                        "P\t{}\t{}\t{}\t{}\t{}",
                                        node.id,
                                        node.parent_id,
                                        node.sort_order,
                                        u8::from(node.is_folder),
                                        clean(name)
                                    );
                                    builder.push_node(
                                        node.id,
                                        node.parent_id,
                                        node.sort_order,
                                        node.is_folder,
                                        name,
                                    )
                                }
                                None => {
                                    builder.note_invalid_row();
                                    true
                                }
                            },
                            Err(_) => {
                                builder.note_invalid_row();
                                true
                            }
                        }
                    }
                    _ => {
                        builder.note_invalid_row();
                        true
                    }
                }
            } else {
                match parse_pdb_playlist_entry_row(row) {
                    Ok(Some(entry)) => {
                        println!(
                            "E\t{}\t{}\t{}",
                            entry.playlist_id, entry.entry_index, entry.track_id
                        );
                        builder.push_entry(entry.playlist_id, entry.entry_index, entry.track_id)
                    }
                    _ => {
                        builder.note_invalid_row();
                        true
                    }
                }
            };
            if keep { Ok(()) } else { Err("bound".into()) }
        });
        match result {
            Ok((_, truncated)) if truncated => builder.note_truncated(),
            Ok(_) => {}
            Err(error) => {
                eprintln!("playlist table stopped: {error}");
                builder.note_truncated();
            }
        }
    }
    let index = builder.finish(&catalog);
    let stats = index.stats();
    let covers = catalog
        .tracks
        .iter()
        .filter(|t| !t.artwork_path.is_empty())
        .count();
    println!(
        "S\ttracks={} artists={} albums={} keys={} artworks={} covered_tracks={covers} playlists nodes={} entries={} invalid={} truncated={}",
        catalog.tracks.len(),
        artists.entries.len(),
        albums.entries.len(),
        keys.entries.len(),
        artworks.entries.len(),
        stats.nodes,
        stats.entries,
        stats.invalid_rows,
        stats.truncated
    );
    Ok(())
}
