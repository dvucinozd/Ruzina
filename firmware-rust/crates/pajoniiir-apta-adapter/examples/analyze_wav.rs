//! Host-only worker example. No audio callback or production provider enablement.
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{Session, SessionConfig},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{
    source::SourceRequest,
    worker::{Pin, Requests},
};
use pajoniiir_media_catalog::{CatalogBuilder, CatalogText, CatalogTrack};
mod support;
use pajoniiir_core::{DeckId, MediaTrackId};
use pajoniiir_deck::DeckProductState;
use pajoniiir_media_session::{MediaHandle, MediaSession, MediaSourceId};
use pajoniiir_track_analysis::Beat;
use std::io::Write;
use support::{HostFs, ready};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: analyze_wav INPUT.wav OUTPUT.ppm (new output only)".into());
    }
    let len = std::fs::metadata(&args[1])?.len();
    if len > 256 * 1024 * 1024 {
        return Err("input exceeds 256 MiB host policy".into());
    }
    let mut bytes = [0; 4096];
    let mut media = MediaSession::new();
    media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = media.lease().unwrap();
    assert!(media.bind_handle(lease, MediaHandle::new(1).unwrap()));
    assert!(media.commit_mounted(lease));
    let mut catalog = CatalogBuilder::<1>::new(lease);
    catalog
        .push(CatalogTrack {
            track_id: MediaTrackId([1; 16]),
            source_track_id: 1,
            bpm_x100: 0,
            duration_ms: 0,
            title: CatalogText::EMPTY,
            artist: CatalogText::EMPTY,
            album: CatalogText::EMPTY,
            key: CatalogText::EMPTY,
            analysis_path: CatalogText::EMPTY,
            audio_path: CatalogText::try_from_str("/TRACK.WAV").unwrap(),
        })
        .map_err(|e| format!("catalog: {e:?}"))?;
    let catalog = catalog.finish(1);
    let mut requests = Requests::default();
    let request = SourceRequest::new(&mut requests, &media, &catalog, 0)
        .map_err(|e| format!("request: {e:?}"))?;
    let mut fs = HostFs {
        path: args[1].clone().into(),
        lease,
    };
    let mut read = ready(request.open_stream(&mut fs, &requests, &media))
        .map_err(|e| format!("open: {e:?}"))?;
    loop {
        match ready(read.step(&mut fs, &requests, &media, &mut bytes)) {
            Ok(true) => break,
            Ok(false) => (),
            Err(e) => {
                let _ = ready(read.abort(&mut fs));
                return Err(format!("read: {e:?}").into());
            }
        }
    }
    let wav =
        ready(read.finish(&mut fs, &mut requests, &media)).map_err(|e| format!("source: {e:?}"))?;
    let total = wav.source().total_frames.unwrap();
    const FPC: u32 = 256;
    let count = usize::try_from(total.div_ceil(FPC as u64))?;
    let mut columns = vec![WaveformColumn::default(); count];
    let mut queue = [NormalizedSample::default(); 256];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.; BIN_CAPACITY];
    let mut session = Session::new(
        SessionConfig {
            sample_rate: wav.source().sample_rate,
            channel_count: wav.source().channel_count,
            total_frames: total,
            frames_per_column: FPC,
        },
        &mut queue,
        &mut columns,
    )?;
    println!(
        "storage: source_scratch=4096 pcm_scratch=2048 session={} onset={} flux={} queue={} overview={} bytes",
        std::mem::size_of_val(&session),
        std::mem::size_of_val(bins.as_slice()),
        std::mem::size_of_val(flux.as_slice()),
        256 * std::mem::size_of::<NormalizedSample>(),
        count * std::mem::size_of::<WaveformColumn>()
    );
    session.enable_tempo(&mut bins, &mut flux)?;
    session.enable_meter()?;
    // Explicit host cap. Capacity failure is returned rather than dropping beats.
    let mut beats = vec![
        Beat {
            time_ms: 0,
            phase: 0,
            bpm_x100: 0
        };
        65536
    ];
    let mut output = vec![
        pajoniiir_waveform::WaveformColumn {
            peak: 0,
            color_rgb565: 0
        };
        count
    ];
    let view = {
        let mut worker = wav
            .worker(session, &mut bytes)
            .map_err(|e| format!("worker: {e:?}"))?;
        ready(worker.open(&mut fs, &requests, &media)).map_err(|e| format!("reopen: {e:?}"))?;
        let mut scratch = [0.; 512];
        loop {
            match ready(worker.step(&mut fs, &requests, &media, &mut scratch)) {
                Ok(true) => break,
                Ok(false) => (),
                Err(e) => {
                    let _ = ready(worker.abort(&mut fs));
                    return Err(format!("step: {e:?}").into());
                }
            }
        }
        ready(worker.finish(&mut fs, &requests, &media)).map_err(|e| format!("verify: {e:?}"))?;
        let mut worker = worker.into_worker().map_err(|_| "unverified worker")?;
        let completed = worker
            .complete(&requests, &media, &mut beats)
            .map_err(|e| format!("analysis: {e:?}"))?;
        let decks = DeckProductState::new();
        let pin = Pin::load(DeckId::One, &decks, completed, &requests, &media)
            .map_err(|e| format!("pin: {e:?}"))?;
        let track = pin.track(&media).ok_or("stale media")?;
        println!(
            "provider={:?} generation={} bpm_x100={} beats={}",
            track.provider(),
            track.generation(),
            track.bpm_x100(),
            track.beat_grid().map_or(0, |g| g.beats().len())
        );
        worker
            .waveform(&requests, &media, &mut output, 0x07e0)
            .map_err(|e| format!("waveform: {e:?}"))?
    };
    let mut pixels = vec![0; 800 * 128];
    pajoniiir_waveform::Rgb565Surface::new(&mut pixels, 800, 128)
        .unwrap()
        .draw_centered_waveform(0, 0, 800, 128, view);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    file.write_all(b"P6\n800 128\n255\n")?;
    for p in pixels {
        file.write_all(&[
            ((p >> 11) * 255 / 31) as u8,
            (((p >> 5) & 63) * 255 / 63) as u8,
            ((p & 31) * 255 / 31) as u8,
        ])?;
    }
    Ok(())
}
