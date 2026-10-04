//! Host-only worker example. No audio callback or production provider enablement.
use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{Session, SessionConfig, SourceIdentity},
    wav::Wav,
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::worker::{Pin, Requests, Worker};
use pajoniiir_core::{DeckId, MediaTrackId};
use pajoniiir_deck::DeckProductState;
use pajoniiir_media_session::{MediaHandle, MediaSession, MediaSourceId};
use pajoniiir_track_analysis::Beat;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: analyze_wav INPUT.wav OUTPUT.ppm (new output only)".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&args[1])?
        .take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err("input exceeds 256 MiB host policy".into());
    }
    let wav = Wav::parse(&bytes)?;
    if wav.frame_count() == 0 {
        return Err("empty source".into());
    }
    const FPC: u32 = 256;
    let count = usize::try_from(wav.frame_count().div_ceil(FPC as u64))?;
    let mut columns = vec![WaveformColumn::default(); count];
    let mut queue = [NormalizedSample::default(); 256];
    let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
    let mut flux = vec![0.; BIN_CAPACITY];
    let mut session = Session::new(
        SessionConfig {
            sample_rate: wav.source().sample_rate,
            channel_count: wav.source().channel_count,
            total_frames: wav.frame_count(),
            frames_per_column: FPC,
        },
        &mut queue,
        &mut columns,
    )?;
    println!(
        "storage: session={} onset={} flux={} queue={} overview={} bytes",
        std::mem::size_of_val(&session),
        std::mem::size_of_val(bins.as_slice()),
        std::mem::size_of_val(flux.as_slice()),
        256 * std::mem::size_of::<NormalizedSample>(),
        count * std::mem::size_of::<WaveformColumn>()
    );
    session.enable_tempo(&mut bins, &mut flux)?;
    session.enable_meter()?;
    let mut media = MediaSession::new();
    media.on_connect(MediaSourceId::new(1).unwrap());
    let lease = media.lease().unwrap();
    assert!(media.bind_handle(lease, MediaHandle::new(1).unwrap()));
    assert!(media.commit_mounted(lease));
    let identity = SourceIdentity::new(2, pajoniiir_media_identity::sha256(&bytes).0)?;
    let mut requests = Requests::default();
    let job = requests
        .request(
            &media,
            media.lease().unwrap(),
            MediaTrackId([1; 16]),
            identity,
        )
        .map_err(|e| format!("request: {e:?}"))?;
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
        let mut worker = Worker::new(session, job).map_err(|e| format!("worker: {e:?}"))?;
        let mut scratch = [0.; 512];
        let channels = wav.source().channel_count as usize;
        let mut offset = 0;
        while offset < wav.frame_count() {
            let n = wav.read_frames(offset, &mut scratch[..256 * channels])?;
            if n == 0 {
                return Err("source ended before declared duration".into());
            }
            let accepted = worker
                .push(&requests, &media, &scratch[..n * channels])
                .map_err(|e| format!("PCM: {e:?}"))?;
            offset += accepted as u64;
            worker
                .step(&requests, &media)
                .map_err(|e| format!("step: {e:?}"))?;
        }
        worker
            .finish_input(&requests, &media)
            .map_err(|e| format!("EOF: {e:?}"))?;
        while !worker
            .step(&requests, &media)
            .map_err(|e| format!("drain: {e:?}"))?
        {}
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
