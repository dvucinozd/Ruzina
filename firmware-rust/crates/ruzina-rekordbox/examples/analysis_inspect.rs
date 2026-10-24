//! Host-only, bounded-read validation of DAT and optional EXT files.
use ruzina_core::MediaTrackId;
use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
use ruzina_rekordbox::anlz_analysis::{ANLZ_READ_BYTES, AnalysisFileKind, AnlzImporter};
use ruzina_track_analysis::owned::AnalysisIdentity;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

fn feed(importer: &mut AnlzImporter, path: &str, kind: AnalysisFileKind) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    importer
        .begin_file(
            kind,
            file.metadata().map_err(|error| error.to_string())?.len(),
        )
        .map_err(|error| format!("{kind:?}: {error:?}"))?;
    let mut bytes = [0u8; ANLZ_READ_BYTES];
    while let Some(plan) = importer.read_plan() {
        file.seek(SeekFrom::Start(u64::from(plan.offset)))
            .map_err(|error| error.to_string())?;
        file.read_exact(&mut bytes[..plan.len])
            .map_err(|error| error.to_string())?;
        importer
            .ingest(plan.offset, &bytes[..plan.len])
            .map_err(|error| format!("{kind:?} at {}: {error:?}", plan.offset))?;
    }
    Ok(())
}
fn hash(bytes: impl IntoIterator<Item = u8>, mut digest: u64) -> u64 {
    for byte in bytes {
        digest ^= u64::from(byte);
        digest = digest.wrapping_mul(0x100000001b3);
    }
    digest
}
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 2 {
        return Err("usage: analysis_inspect DAT [EXT]".into());
    }
    let mut importer = AnlzImporter::new();
    feed(&mut importer, &args[0], AnalysisFileKind::Dat)?;
    if let Some(ext) = args.get(1) {
        feed(&mut importer, ext, AnalysisFileKind::Ext)?;
    }
    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap()) else {
        unreachable!()
    };
    let data = importer
        .finish(
            AnalysisIdentity {
                track: MediaTrackId([1; 16]),
                lease,
                catalog_revision: 1,
                load_generation: 1,
            },
            0,
        )
        .map_err(|error| format!("{error:?}"))?;
    let view = data.view();
    let beats = view
        .beat_grid()
        .map(|grid| grid.beats())
        .unwrap_or_default();
    let mut beat_hash = 0xcbf29ce484222325;
    for beat in beats {
        beat_hash = hash(beat.time_ms.to_le_bytes(), beat_hash);
        beat_hash = hash(beat.phase.to_le_bytes(), beat_hash);
        beat_hash = hash(beat.bpm_x100.to_le_bytes(), beat_hash);
    }
    let wave_hash = |columns: &[ruzina_track_analysis::WaveformColumn]| {
        let mut digest = 0xcbf29ce484222325;
        for column in columns {
            digest = hash(column.peak.to_le_bytes(), digest);
            digest = hash(column.color_rgb565.to_le_bytes(), digest);
        }
        digest
    };
    let mut cue_hash = 0xcbf29ce484222325;
    for cue in view.cues() {
        cue_hash = hash([cue.slot], cue_hash);
        cue_hash = hash(cue.start_ms.to_le_bytes(), cue_hash);
        cue_hash = hash(cue.end_ms.unwrap_or(0).to_le_bytes(), cue_hash);
    }
    let mut memory_hash = 0xcbf29ce484222325;
    for cue in view.memory_cues() {
        memory_hash = hash(cue.start_ms.to_le_bytes(), memory_hash);
        memory_hash = hash(cue.end_ms.unwrap_or(0).to_le_bytes(), memory_hash);
    }
    println!(
        "{{\"beats\":{},\"preview\":{},\"detail\":{},\"cues\":{},\"memory_cues\":{},\"color_preview\":{},\"bpm_x100\":{},\"beat_hash\":\"{beat_hash:016x}\",\"cue_hash\":\"{cue_hash:016x}\",\"memory_hash\":\"{memory_hash:016x}\",\"preview_hash\":\"{:016x}\",\"detail_hash\":\"{:016x}\"}}",
        beats.len(),
        view.preview().len(),
        view.detail().len(),
        view.cues().len(),
        view.memory_cues().len(),
        view.color_preview().len(),
        view.bpm_x100(),
        wave_hash(view.preview()),
        wave_hash(view.detail())
    );
    Ok(())
}
