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
}

/// Root directory holding a two-cluster audio file and the DAT/EXT pair, each
/// a contiguous (NoFatChain) cluster run.
fn card() -> Card {
    let files: [(&str, Vec<u8>); 3] = [
        ("track.wav", vec![0x52; 600]),
        ("ANLZ0000.DAT", DAT.to_vec()),
        ("ANLZ0000.EXT", EXT.to_vec()),
    ];
    let data_clusters: u32 = files
        .iter()
        .map(|(_, bytes)| (bytes.len() as u32).div_ceil(512))
        .sum();
    let (mut bytes, heap) = formatted(1 + data_clusters + 10);
    let lba = |cluster: u32| (1 + heap + cluster - 2) as usize;
    let root = lba(2) * 512;
    let mut entry = 0;
    let mut cluster = 3;
    let mut dat_lba = 0;
    for (name, data) in &files {
        for set in entries(name, cluster, data.len() as u64, false) {
            bytes[root + entry * 32..root + (entry + 1) * 32].copy_from_slice(&set);
            entry += 1;
        }
        let start = lba(cluster) * 512;
        bytes[start..start + data.len()].copy_from_slice(data);
        if *name == "ANLZ0000.DAT" {
            dat_lba = lba(cluster) as u64;
        }
        cluster += (data.len() as u32).div_ceil(512);
    }
    Card { bytes, dat_lba }
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
