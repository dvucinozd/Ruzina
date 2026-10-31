use super::*;
use crate::exfat_fixture::mount;
use crate::exfat_fixture::tests::{Port, entries, formatted, ready, setup_with};
use crate::media_bridge::{Binding, ReadClient, ReadPort};
use core::future::Future;
use core::task::{Context, Waker};
use ruzina_core::MediaTrackId;
use ruzina_media_catalog::{CatalogText, CatalogTrack};
use ruzina_media_io::OwnedBlockDevice;
use ruzina_media_session::{MediaHandle, MediaLease, MediaSourceId};
use std::{boxed::Box, vec, vec::Vec};

const DAT: &[u8] = include_bytes!(
    "../../../../../firmware-rust/crates/ruzina-rekordbox/tests/fixtures/track-a.DAT"
);
const EXT: &[u8] = include_bytes!(
    "../../../../../firmware-rust/crates/ruzina-rekordbox/tests/fixtures/track-a.EXT"
);
const AUDIO: &str = "/track.wav";
const ANLZ: &str = "/ANLZ0000.DAT";

struct Card {
    bytes: Vec<u8>,
    dat_lba: u64,
    pdb_tracks_lba: u64,
}

fn put_le16(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_le32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_short_devicesql(data: &mut [u8], offset: usize, text: &[u8]) {
    data[offset] = (((text.len() + 1) as u8) << 1) | 1;
    data[offset + 1..offset + 1 + text.len()].copy_from_slice(text);
}

/// A two-page `export.pdb`: the header and one track table page holding the
/// audio file and analysis path of this card.
fn export_pdb() -> Vec<u8> {
    let mut pdb = vec![0u8; 1024];
    put_le32(&mut pdb, 4, 512);
    put_le32(&mut pdb, 8, 1);
    put_le32(&mut pdb, 0x1c + 8, 1);
    put_le32(&mut pdb, 0x1c + 12, 1);
    let page = &mut pdb[512..];
    put_le32(page, 0x0c, u32::MAX);
    put_le32(page, 0x18, 1);
    put_le16(page, 506, 0);
    put_le16(page, 508, 1);
    let row = 0x28;
    put_le16(page, row, 0x0024);
    put_le32(page, row + 0x38, 12_000);
    put_le32(page, row + 0x48, 7);
    put_le16(page, row + 0x54, 1);
    for (slot, offset, text) in [
        (14usize, 0x90u16, ANLZ.as_bytes()),
        (17, 0xb0, b"Exported".as_slice()),
        (20, 0xc0, AUDIO.as_bytes()),
    ] {
        put_le16(page, row + 0x5e + slot * 2, offset);
        put_short_devicesql(page, row + usize::from(offset), text);
    }
    pdb
}

/// Root directory holding a two-cluster audio file, the DAT/EXT pair and
/// `PIONEER/rekordbox/export.pdb`, each a contiguous (NoFatChain) run.
fn card() -> Card {
    let files: [(&str, Vec<u8>); 3] = [
        ("track.wav", vec![0x52; 600]),
        ("ANLZ0000.DAT", DAT.to_vec()),
        ("ANLZ0000.EXT", EXT.to_vec()),
    ];
    let pdb = export_pdb();
    let data_clusters: u32 = files
        .iter()
        .map(|(_, bytes)| (bytes.len() as u32).div_ceil(512))
        .sum();
    let (mut bytes, heap) = formatted(1 + data_clusters + 2 + 2 + 10);
    let lba = |cluster: u32| (1 + heap + cluster - 2) as usize;
    let put = |bytes: &mut Vec<u8>, dir: u32, entry: &mut usize, set: Vec<[u8; 32]>| {
        let base = lba(dir) * 512;
        for item in set {
            bytes[base + *entry * 32..base + (*entry + 1) * 32].copy_from_slice(&item);
            *entry += 1;
        }
    };
    let mut root_entry = 0;
    let mut cluster = 3;
    let mut dat_lba = 0;
    for (name, data) in &files {
        put(
            &mut bytes,
            2,
            &mut root_entry,
            entries(name, cluster, data.len() as u64, false),
        );
        let start = lba(cluster) * 512;
        bytes[start..start + data.len()].copy_from_slice(data);
        if *name == "ANLZ0000.DAT" {
            dat_lba = lba(cluster) as u64;
        }
        cluster += (data.len() as u32).div_ceil(512);
    }
    let (pioneer, rekordbox, export) = (cluster, cluster + 1, cluster + 2);
    put(
        &mut bytes,
        2,
        &mut root_entry,
        entries("PIONEER", pioneer, 512, true),
    );
    put(
        &mut bytes,
        pioneer,
        &mut 0,
        entries("rekordbox", rekordbox, 512, true),
    );
    put(
        &mut bytes,
        rekordbox,
        &mut 0,
        entries("export.pdb", export, pdb.len() as u64, false),
    );
    let start = lba(export) * 512;
    bytes[start..start + pdb.len()].copy_from_slice(&pdb);
    Card {
        bytes,
        dat_lba,
        pdb_tracks_lba: lba(export + 1) as u64,
    }
}

fn text<const N: usize>(value: &str) -> CatalogText<N> {
    CatalogText::try_from_str(value).unwrap()
}

fn candidate(lease: MediaLease, audio: &str) -> CatalogLoadCandidate {
    CatalogLoadCandidate {
        publication: CatalogPublicationId { lease, revision: 3 },
        index: 0,
        track: CatalogTrack {
            track_id: MediaTrackId([9; 16]),
            source_track_id: 1,
            bpm_x100: 12_000,
            duration_ms: 1_000,
            title: text("Track"),
            artist: text(""),
            album: text(""),
            key: text(""),
            audio_path: text(audio),
            analysis_path: text(ANLZ),
            artwork_path: text(""),
        },
    }
}

fn setup() -> (Port, Binding, ReadClient<Port>, u64) {
    let card = card();
    let (port, binding, client) = setup_with(card.bytes);
    (port, binding, client, card.dat_lba)
}

fn import_stage() -> (PdbNameStage<VecNameMap>, VecCatalogStage) {
    (
        PdbNameStage::try_new().unwrap(),
        VecCatalogStage::try_new().unwrap(),
    )
}

#[test]
fn catalog_import_then_preflight_loads_the_exported_track() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let (mut names, mut stage) = import_stage();
    stage.prepare_generation(binding.lease);
    let identity = VolumeIdentity([0x31; 32]);
    let probe = ready(import_catalog(
        &mut volume,
        identity,
        &current,
        &mut names,
        &mut stage,
    ))
    .unwrap();
    assert_eq!(probe.tracks_layout.unwrap().track_rows, 1);

    let snapshot = stage.current().unwrap();
    let track = *snapshot.get(0).unwrap();
    assert_eq!(track.title.as_str(), "Exported");
    assert_eq!(track.audio_path.as_str(), AUDIO);
    assert_eq!(track.analysis_path.as_str(), ANLZ);
    let candidate = CatalogLoadCandidate {
        publication: CatalogPublicationId {
            lease: snapshot.lease(),
            revision: snapshot.revision(),
        },
        index: 0,
        track,
    };
    let published = |publication: CatalogPublicationId| {
        stage.current().is_some_and(|snapshot| {
            snapshot.lease() == publication.lease && snapshot.revision() == publication.revision
        })
    };
    let prepared = ready(preflight(&mut volume, candidate, 1, &current, &published))
        .ok()
        .unwrap();
    assert!(prepared.source_cues_available);
    assert_eq!(
        prepared.publication.unwrap().identity().track,
        track.track_id
    );
}

#[test]
fn removal_during_catalog_import_publishes_nothing() {
    let card = card();
    let (port, binding, mut client) = setup_with(card.bytes);
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let (mut names, mut stage) = import_stage();
    stage.prepare_generation(binding.lease);
    port.0.borrow_mut().paused = Some(card.pdb_tracks_lba);
    let result = {
        let mut importing = Box::pin(import_catalog(
            &mut volume,
            VolumeIdentity([0x31; 32]),
            &current,
            &mut names,
            &mut stage,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(importing.as_mut().poll(&mut cx).is_pending());
        port.0.borrow_mut().queue.invalidate();
        port.0.borrow_mut().paused = None;
        match importing.as_mut().poll(&mut cx) {
            core::task::Poll::Ready(result) => result,
            core::task::Poll::Pending => panic!("a stale completion must resolve the read"),
        }
    };
    assert!(matches!(result, Err(ImportError::Io(_))));
    assert!(stage.current().is_none());
}

#[test]
fn preflight_imports_analysis_from_a_real_exfat_volume() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let prepared = ready(preflight(
        &mut volume,
        candidate(binding.lease, AUDIO),
        5,
        &current,
        &|_| true,
    ))
    .ok()
    .unwrap();
    let publication = prepared.publication.expect("DAT + EXT analysis");
    assert_eq!(publication.identity().lease, binding.lease);
    assert_eq!(publication.identity().catalog_revision, 3);
    assert_eq!(publication.identity().load_generation, 5);
    assert_eq!(publication.view().bpm_x100(), 12_000);
    assert!(prepared.source_cues_available);
    assert!(!publication.view().cues().is_empty());
}

#[test]
fn missing_or_directory_audio_is_a_source_error() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    for path in ["/missing.wav", "/", ""] {
        assert_eq!(
            ready(preflight(
                &mut volume,
                candidate(binding.lease, path),
                1,
                &current,
                &|_| true,
            ))
            .err(),
            Some(Error::Source)
        );
    }
}

#[test]
fn withdrawn_publication_or_foreign_lease_is_stale() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    assert_eq!(
        ready(preflight(
            &mut volume,
            candidate(binding.lease, AUDIO),
            1,
            &current,
            &|_| false,
        ))
        .err(),
        Some(Error::Stale)
    );
    let mut other = ruzina_media_session::MediaSession::new();
    other.on_connect(MediaSourceId::new(1).unwrap());
    other.on_disconnect(None);
    let ruzina_media_session::ConnectResult::Accepted(foreign) =
        other.on_connect(MediaSourceId::new(1).unwrap())
    else {
        panic!()
    };
    assert_ne!(foreign, binding.lease);
    assert_eq!(
        ready(preflight(
            &mut volume,
            candidate(foreign, AUDIO),
            1,
            &current,
            &|_| true,
        ))
        .err(),
        Some(Error::Stale)
    );
}

#[test]
fn removal_during_an_analysis_read_never_returns_analysis() {
    let (port, binding, mut client, dat_lba) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    port.0.borrow_mut().paused = Some(dat_lba);
    let publication = |_| true;
    let mut loading = Box::pin(preflight(
        &mut volume,
        candidate(binding.lease, AUDIO),
        1,
        &current,
        &publication,
    ));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(loading.as_mut().poll(&mut cx).is_pending());
    port.0.borrow_mut().queue.invalidate();
    port.0.borrow_mut().paused = None;
    match loading.as_mut().poll(&mut cx) {
        core::task::Poll::Ready(result) => assert_eq!(result.err(), Some(Error::Stale)),
        core::task::Poll::Pending => panic!("a stale completion must resolve the read"),
    }
}

#[test]
fn canceled_preflight_drains_then_reloads_under_a_new_lease() {
    let (port, binding, mut client, dat_lba) = setup();
    let current = || port.is_current(binding);
    {
        let mut volume = ready(mount(&mut client, binding.lease, &current))
            .ok()
            .unwrap();
        port.0.borrow_mut().paused = Some(dat_lba);
        let publication = |_| true;
        let mut loading = Box::pin(preflight(
            &mut volume,
            candidate(binding.lease, AUDIO),
            1,
            &current,
            &publication,
        ));
        assert!(
            loading
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        // Cancellation discards the preflight and the volume with its cursors.
    }
    assert!(client.has_pending());
    port.0.borrow_mut().paused = None;
    assert!(ready(client.recover_pending()).unwrap().is_ok());
    let fresh = {
        let mut state = port.0.borrow_mut();
        state.queue.invalidate();
        state
            .queue
            .activate(
                MediaSourceId::new(1).unwrap(),
                MediaHandle::new(1).unwrap(),
                client.geometry(),
            )
            .unwrap()
    };
    assert_ne!(fresh.lease, binding.lease);
    // The old lease can no longer load anything.
    assert!(!port.is_current(binding));
    let mut fresh_client = ReadClient::new(port.clone(), fresh);
    let current = || port.is_current(fresh);
    let mut volume = ready(mount(&mut fresh_client, fresh.lease, &current))
        .ok()
        .unwrap();
    let prepared = ready(preflight(
        &mut volume,
        candidate(fresh.lease, AUDIO),
        2,
        &current,
        &|_| true,
    ))
    .ok()
    .unwrap();
    assert_eq!(prepared.publication.unwrap().identity().lease, fresh.lease);
}

#[test]
fn warm_volume_cache_cannot_serve_a_load_after_removal() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let load = |volume: &mut crate::exfat_fixture::Volume<_>| {
        ready(preflight(
            volume,
            candidate(binding.lease, AUDIO),
            1,
            &current,
            &|_| true,
        ))
    };
    assert!(load(&mut volume).is_ok());
    let reads = port.0.borrow().reads;
    port.0.borrow_mut().queue.invalidate();
    assert_eq!(load(&mut volume).err(), Some(Error::Stale));
    assert_eq!(port.0.borrow().reads, reads, "no IO after removal");
}

#[test]
fn volume_identity_uses_the_native_exfat_derivation() {
    let (port, binding, mut client, _) = setup();
    let current = || port.is_current(binding);
    let volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let boot = volume.boot_info();
    assert_eq!(
        volume_identity(&volume),
        ruzina_media_identity::derive_exfat_volume_identity(
            boot.volume_serial,
            512,
            1,
            boot.volume_length,
            1,
        )
    );
}
