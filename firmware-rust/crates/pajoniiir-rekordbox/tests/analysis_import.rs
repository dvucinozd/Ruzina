#![cfg(feature = "analysis")]
use pajoniiir_rekordbox::anlz_analysis::{
    AnalysisFileKind, AnlzImporter, ImportError, extended_path,
};
use pajoniiir_track_analysis::{
    AnalysisProvider,
    owned::{AnalysisDataError, AnalysisIdentity, AnalysisPins},
};

const DAT: &[u8] = include_bytes!("fixtures/track-a.DAT");
const EXT: &[u8] = include_bytes!("fixtures/track-a.EXT");

fn identity(generation: u32) -> AnalysisIdentity {
    use pajoniiir_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap()) else {
        panic!()
    };
    AnalysisIdentity {
        track: pajoniiir_core::MediaTrackId([1; 16]),
        lease,
        catalog_revision: 2,
        load_generation: generation,
    }
}
fn feed(
    importer: &mut AnlzImporter,
    kind: AnalysisFileKind,
    bytes: &[u8],
) -> Result<(), ImportError> {
    importer.begin_file(kind, bytes.len() as u64)?;
    while let Some(plan) = importer.read_plan() {
        assert!(plan.len <= 512 && plan.len != 0);
        let data = bytes
            .get(plan.offset as usize..plan.offset as usize + plan.len)
            .ok_or(ImportError::ShortRead)?;
        importer.ingest(plan.offset, data)?;
    }
    assert!(importer.file_complete());
    Ok(())
}
fn section(bytes: &[u8], tag: &[u8; 4]) -> usize {
    bytes.windows(4).position(|bytes| bytes == tag).unwrap()
}
fn write32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn imports_grid_preview_detail_and_hot_cues_as_neutral_immutable_data() {
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    feed(&mut importer, AnalysisFileKind::Ext, EXT).unwrap();
    let data = importer.finish(identity(3), 9999).unwrap();
    let view = data.view();
    assert_eq!(view.provider(), AnalysisProvider::RekordboxImport);
    assert_eq!(view.generation(), 3);
    assert_eq!(view.bpm_x100(), 12000);
    let beats = view.beat_grid().unwrap().beats();
    assert_eq!(beats.len(), 8);
    for (index, beat) in beats.iter().enumerate() {
        assert_eq!(beat.phase, (index % 4) as u16);
        assert_eq!(beat.time_ms, 1000 + index as u32 * 500);
    }
    assert_eq!(view.preview().len(), 400);
    assert_eq!(view.preview()[31].peak, u16::MAX);
    assert_eq!(view.detail().len(), 1025);
    assert_eq!(view.detail_rate_hz(), 150);
    assert_eq!(view.detail_window(1000, 20), &view.detail()[140..160]);
    assert_eq!(view.detail_window(u32::MAX, usize::MAX), view.detail());
    assert_eq!(view.cues().len(), 2);
    assert_eq!(view.cues()[0].start_ms, 1000);
    assert_eq!(view.cues()[1].end_ms, Some(4000));
}
#[test]
fn missing_optional_ext_preserves_valid_preview_and_grid() {
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    let data = importer.finish(identity(1), 0).unwrap();
    assert_eq!(data.view().preview().len(), 400);
    assert!(data.view().detail().is_empty());
}
#[test]
fn malformed_ext_or_short_reads_cannot_publish_partial_dat() {
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    let mut ext = EXT.to_vec();
    write32(&mut ext, 44, 2000);
    assert!(feed(&mut importer, AnalysisFileKind::Ext, &ext).is_err());
    assert!(importer.finish(identity(1), 0).is_err());
    let mut importer = AnlzImporter::new();
    importer
        .begin_file(AnalysisFileKind::Dat, DAT.len() as u64)
        .unwrap();
    assert_eq!(importer.ingest(0, &DAT[..11]), Err(ImportError::ShortRead));
    assert!(importer.finish(identity(1), 0).is_err());
}
#[test]
fn container_lengths_nested_roots_and_trailing_fragments_fail_closed() {
    for (at, value) in [(0, 0), (4, 11), (8, DAT.len() as u32 - 1), (32, u32::MAX)] {
        let mut bytes = DAT.to_vec();
        write32(&mut bytes, at, value);
        assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
    }
    let mut bytes = DAT.to_vec();
    bytes[28..32].copy_from_slice(b"PMAI");
    assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
    let mut bytes = DAT.to_vec();
    bytes.push(0);
    let len = bytes.len() as u32;
    write32(&mut bytes, 8, len);
    assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
}
#[test]
fn beat_phase_zero_tempo_unsorted_times_and_count_mismatch_are_rejected() {
    let p = section(DAT, b"PQTZ");
    for (at, value) in [(p + 20, 9), (p + 24, 0), (p + 32 + 4, 999)] {
        let mut bytes = DAT.to_vec();
        write32(&mut bytes, at, value);
        assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
    }
}
#[test]
fn duplicate_grid_sections_and_duplicate_hot_cue_slots_are_rejected() {
    let p = section(DAT, b"PQTZ");
    let len = u32::from_be_bytes(DAT[p + 8..p + 12].try_into().unwrap()) as usize;
    let mut bytes = DAT.to_vec();
    bytes.extend_from_slice(&DAT[p..p + len]);
    let len = bytes.len() as u32;
    write32(&mut bytes, 8, len);
    assert_eq!(
        feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes),
        Err(ImportError::Duplicate)
    );
    let mut bytes = DAT.to_vec();
    let hot = bytes
        .windows(4)
        .rposition(|bytes| bytes == b"PCPT")
        .unwrap();
    write32(&mut bytes, hot + 12, 1);
    assert_eq!(
        feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes),
        Err(ImportError::Data(AnalysisDataError::DuplicateCue))
    );
}
#[test]
fn pins_reject_old_load_generations_tracks_catalogs_and_media_leases() {
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    let data = importer.finish(identity(1), 0).unwrap();
    let mut pins = AnalysisPins::new();
    assert!(pins.publish(0, identity(1), data));
    assert!(pins.view(0, identity(1)).is_some());
    assert!(pins.view(0, identity(2)).is_none());
    assert!(pins.view(1, identity(1)).is_none());
    let mut wrong = identity(1);
    wrong.track = pajoniiir_core::MediaTrackId([2; 16]);
    assert!(pins.view(0, wrong).is_none());
    wrong = identity(1);
    wrong.catalog_revision += 1;
    assert!(pins.view(0, wrong).is_none());
    use pajoniiir_media_session::{ConnectResult, MediaSession, MediaSourceId};
    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(2).unwrap()) else {
        panic!()
    };
    wrong = identity(1);
    wrong.lease = lease;
    assert!(pins.view(0, wrong).is_none());
    pins.clear(0);
    assert!(pins.view(0, identity(1)).is_none());
}
#[test]
fn same_track_reload_still_requires_the_new_publication_token() {
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    let data = importer.finish(identity(1), 0).unwrap();
    let mut pins = AnalysisPins::new();
    assert!(!pins.publish(0, identity(2), data));
    assert!(pins.view(0, identity(2)).is_none());
}
#[test]
fn ext_path_is_bounded_case_insensitive_and_preserves_utf8_directories() {
    let mut storage = [0; 256];
    assert_eq!(
        extended_path("/Glazba/čć/ANLZ0000.dat", &mut storage),
        Some("/Glazba/čć/ANLZ0000.EXT")
    );
    assert!(extended_path("/test.DAT", &mut [0; 8]).is_none());
    assert!(extended_path("/test.EXT", &mut storage).is_none());
}

#[test]
fn file_section_record_and_allocation_limits_are_enforced_before_publication() {
    use pajoniiir_rekordbox::anlz_analysis::ANLZ_MAX_FILE_BYTES;
    use pajoniiir_track_analysis::owned::{
        AnalysisBuilder, MAX_ANALYSIS_BEATS, MAX_DETAIL_COLUMNS,
    };
    let mut importer = AnlzImporter::new();
    assert!(
        importer
            .begin_file(AnalysisFileKind::Dat, u64::from(ANLZ_MAX_FILE_BYTES) + 1)
            .is_err()
    );
    let mut builder = AnalysisBuilder::new();
    assert_eq!(
        builder.reserve_beats(MAX_ANALYSIS_BEATS + 1),
        Err(AnalysisDataError::Capacity)
    );
    assert_eq!(
        builder.reserve_waveform(true, MAX_DETAIL_COLUMNS + 1),
        Err(AnalysisDataError::Capacity)
    );
    assert_eq!(
        builder.reserve_waveform(false, 401),
        Err(AnalysisDataError::Capacity)
    );
    let mut bytes = b"PMAI".to_vec();
    bytes.extend_from_slice(&12u32.to_be_bytes());
    bytes.extend_from_slice(&(12u32 + 257 * 12).to_be_bytes());
    for _ in 0..257 {
        bytes.extend_from_slice(b"UNKN");
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(&12u32.to_be_bytes());
    }
    assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
}

#[test]
fn wrong_offsets_incomplete_streams_and_ext_before_dat_remain_unpublishable() {
    let mut importer = AnlzImporter::new();
    assert!(
        importer
            .begin_file(AnalysisFileKind::Ext, EXT.len() as u64)
            .is_err()
    );
    let mut importer = AnlzImporter::new();
    importer
        .begin_file(AnalysisFileKind::Dat, DAT.len() as u64)
        .unwrap();
    assert_eq!(
        importer.ingest(1, &DAT[..12]),
        Err(ImportError::UnexpectedRead)
    );
    assert!(importer.finish(identity(1), 0).is_err());
    let mut importer = AnlzImporter::new();
    importer
        .begin_file(AnalysisFileKind::Dat, DAT.len() as u64)
        .unwrap();
    importer.ingest(0, &DAT[..12]).unwrap();
    assert!(importer.finish(identity(1), 0).is_err());
}

#[test]
fn malformed_preview_detail_stride_and_cue_loop_are_rejected() {
    let mut oversized_cue = DAT.to_vec();
    let cue = section(DAT, b"PCPT");
    write32(&mut oversized_cue, cue + 4, 1000);
    assert!(
        feed(
            &mut AnlzImporter::new(),
            AnalysisFileKind::Dat,
            &oversized_cue
        )
        .is_err()
    );
    let mut preview = DAT.to_vec();
    let p = section(DAT, b"PWAV");
    write32(&mut preview, p + 12, 399);
    assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &preview).is_err());
    let mut detail = EXT.to_vec();
    let p = section(EXT, b"PWV3");
    write32(&mut detail, p + 12, 2);
    let mut importer = AnlzImporter::new();
    feed(&mut importer, AnalysisFileKind::Dat, DAT).unwrap();
    assert!(feed(&mut importer, AnalysisFileKind::Ext, &detail).is_err());
    let mut cues = DAT.to_vec();
    let p = cues.windows(4).rposition(|bytes| bytes == b"PCPT").unwrap();
    write32(&mut cues, p + 36, 1999);
    assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &cues).is_err());
}

#[test]
fn zero_bpm_and_invalid_bar_positions_do_not_become_a_usable_grid() {
    let p = section(DAT, b"PQTZ") + 24;
    for (at, value) in [(p, 0u16), (p, 5u16), (p + 2, 0u16)] {
        let mut bytes = DAT.to_vec();
        bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
        assert!(feed(&mut AnlzImporter::new(), AnalysisFileKind::Dat, &bytes).is_err());
    }
}
