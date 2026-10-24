use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::decode::Mp3CheckpointStorage;
use ruzina_audio_runtime::decode::StreamChange;
use ruzina_audio_runtime::{
    decode::{
        AudioCodec, DecodeAction, DecoderStorage, ENCODED_BUFFER_BYTES, SourceDecoder,
        SourceDescriptor,
    },
    pcm_stream::PcmStreamBlock,
    wav::{WavPcm16Format, WavPcm16SourceDescriptor},
};

const WAV: &[u8] = include_bytes!("fixtures/codecs/stereo.wav");
const CBR: &[u8] = include_bytes!("fixtures/codecs/cbr.mp3");
const VBR: &[u8] = include_bytes!("fixtures/codecs/vbr.mp3");
const MONO: &[u8] = include_bytes!("fixtures/codecs/mono.mp3");
const FLAC: &[u8] = include_bytes!("fixtures/codecs/stereo.flac");
const FLAC_MONO: &[u8] = include_bytes!("fixtures/codecs/mono.flac");
const DEEP: &[u8] = include_bytes!("fixtures/codecs/deep.flac");

struct Driver<'a> {
    file: &'a [u8],
    storage: Box<DecoderStorage>,
    checkpoints: [Box<Mp3CheckpointStorage>; 3],
    encoded: Vec<u8>,
    stream: SourceDecoder,
    reads: usize,
    chunk: usize,
}
impl<'a> Driver<'a> {
    fn new(file: &'a [u8], codec: AudioCodec) -> Self {
        let mut storage = Box::new(DecoderStorage::new());
        let stream = if codec == AudioCodec::WavPcm16 {
            SourceDecoder::wav(
                WavPcm16SourceDescriptor::new(
                    WavPcm16Format {
                        channels: 2,
                        sample_rate_hz: 48000,
                        byte_rate: 192000,
                        block_align: 4,
                    },
                    44,
                    (file.len() - 44) as u64,
                )
                .unwrap(),
                file.len() as u64,
                1,
            )
            .unwrap()
        } else {
            SourceDecoder::compressed(codec, file.len() as u64, 1, &mut storage).unwrap()
        };
        Self {
            file,
            storage,
            checkpoints: core::array::from_fn(|_| Box::new(Mp3CheckpointStorage::new())),
            encoded: vec![0; ENCODED_BUFFER_BYTES],
            stream,
            reads: 0,
            chunk: 512,
        }
    }
    fn block(&mut self) -> PcmStreamBlock {
        for _ in 0..100000 {
            match self.stream.step(
                &mut self.storage,
                &mut views(&mut self.checkpoints),
                &mut self.encoded,
            ) {
                DecodeAction::Read { offset, maximum } => {
                    let offset = offset as usize;
                    let end = self.file.len().min(offset + maximum.min(self.chunk));
                    assert!(
                        self.stream
                            .provide(&mut self.encoded, &self.file[offset..end])
                    );
                    self.reads += 1;
                }
                DecodeAction::Pending => {}
                DecodeAction::Block(block) => return block,
            }
        }
        panic!("decoder failed to make bounded progress");
    }
    fn all(&mut self) -> Vec<PcmFrame> {
        let mut out = Vec::new();
        for _ in 0..100000 {
            let b = self.block();
            assert!(!b.faulted, "fault at {}", b.first_frame);
            out.extend_from_slice(b.frames());
            if b.eof {
                return out;
            }
        }
        panic!("missing EOF");
    }
    /// Runs the background MP3 frame walk for at most `limit` reads.
    fn scan(&mut self, limit: usize) -> usize {
        let mut reads = 0;
        while reads < limit {
            let Some((offset, len)) = self.stream.index_read(&mut self.storage, 512) else {
                break;
            };
            let at = offset as usize;
            assert!(
                self.stream
                    .provide_index(&mut self.storage, offset, &self.file[at..at + len])
            );
            reads += 1;
        }
        reads
    }
    /// File reads until the first block after a change.
    fn first_block_reads(&mut self) -> (usize, PcmStreamBlock) {
        let before = self.reads;
        let block = self.block();
        (self.reads - before, block)
    }
    fn change(&mut self, generation: u32, change: StreamChange) {
        assert!(self.stream.change(
            &mut self.storage,
            &mut views(&mut self.checkpoints),
            &mut self.encoded,
            generation,
            change
        ));
    }
}
fn pcm(bytes: &[u8]) -> Vec<PcmFrame> {
    bytes
        .chunks_exact(4)
        .map(|v| PcmFrame {
            left: i16::from_le_bytes([v[0], v[1]]),
            right: i16::from_le_bytes([v[2], v[3]]),
        })
        .collect()
}
fn compressed() -> [(&'static [u8], AudioCodec); 6] {
    [
        (CBR, AudioCodec::Mp3),
        (VBR, AudioCodec::Mp3),
        (MONO, AudioCodec::Mp3),
        (FLAC, AudioCodec::Flac),
        (FLAC_MONO, AudioCodec::Flac),
        (DEEP, AudioCodec::Flac),
    ]
}

#[test]
fn shared_wav_contract_matches_existing_pcm_sample_order_and_eof() {
    let mut d = Driver::new(WAV, AudioCodec::WavPcm16);
    assert_eq!(d.all(), pcm(&WAV[44..]));
    assert_eq!(d.stream.descriptor().unwrap().frames(), Some(12000));
}
#[test]
fn lossless_stereo_and_mono_match_original_pcm_and_duplicate_mono() {
    assert_eq!(Driver::new(FLAC, AudioCodec::Flac).all(), pcm(&WAV[44..]));
    let mono = include_bytes!("fixtures/codecs/mono.wav");
    let expected: Vec<_> = mono[44..]
        .chunks_exact(2)
        .map(|v| {
            let x = i16::from_le_bytes([v[0], v[1]]);
            PcmFrame { left: x, right: x }
        })
        .collect();
    assert_eq!(Driver::new(FLAC_MONO, AudioCodec::Flac).all(), expected);
}
#[test]
fn mp3_cbr_vbr_and_mono_match_independent_ffmpeg_length_and_signal() {
    for (file, reference) in [
        (
            CBR,
            include_bytes!("fixtures/codecs/cbr-reference.pcm").as_slice(),
        ),
        (
            VBR,
            include_bytes!("fixtures/codecs/vbr-reference.pcm").as_slice(),
        ),
        (
            MONO,
            include_bytes!("fixtures/codecs/mono-reference.pcm").as_slice(),
        ),
    ] {
        let mut d = Driver::new(file, AudioCodec::Mp3);
        let actual = d.all();
        let expected = pcm(reference);
        assert_eq!(actual.len(), expected.len());
        assert_eq!(
            d.stream.descriptor().unwrap().frames(),
            Some(expected.len() as u64)
        );
        let mut signal = 0.0f64;
        let mut error = 0.0f64;
        let mut maximum = 0i32;
        for (a, e) in actual.iter().zip(&expected) {
            for (a, e) in [(a.left, e.left), (a.right, e.right)] {
                let delta = i32::from(a) - i32::from(e);
                maximum = maximum.max(delta.abs());
                signal += f64::from(e).powi(2);
                error += f64::from(delta).powi(2);
            }
        }
        assert!(maximum <= 16, "maximum sample error={maximum}");
        assert!(error == 0.0 || 10.0 * (signal / error).log10() > 70.0);
    }
}
#[test]
fn deep_flac_96khz_has_correct_frame_count_and_rounded_pcm16_conversion() {
    let actual = Driver::new(DEEP, AudioCodec::Flac).all();
    let expected = pcm(include_bytes!("fixtures/codecs/deep-reference.pcm"));
    assert_eq!(actual.len(), 24000);
    assert_eq!(actual.len(), expected.len());
    assert!(
        actual
            .iter()
            .zip(&expected)
            .all(|(a, e)| (i32::from(a.left) - i32::from(e.left)).abs() <= 1
                && (i32::from(a.right) - i32::from(e.right)).abs() <= 1)
    );
}

#[test]
fn full_scale_32bit_flac_preserves_polarity_and_saturates_pcm16_boundaries() {
    let file = include_bytes!("fixtures/codecs/wide32.flac");
    let actual = Driver::new(file, AudioCodec::Flac).all();
    let expected = pcm(include_bytes!("fixtures/codecs/wide32-reference.pcm"));
    assert_eq!(actual.len(), 256);
    assert!(
        actual
            .iter()
            .zip(&expected)
            .all(|(a, e)| (i32::from(a.left) - i32::from(e.left)).abs() <= 1
                && (i32::from(a.right) - i32::from(e.right)).abs() <= 1)
    );
    assert_eq!(actual[1].left, i16::MAX);
    assert_eq!(actual[2].left, i16::MIN);
}

#[test]
fn replacing_mp3_flac_and_wav_reuses_dirty_context_without_old_audio_or_metadata() {
    let mut storage = Box::new(DecoderStorage::new());
    let mut encoded = vec![0xff; ENCODED_BUFFER_BYTES];
    let mut checkpoints = core::array::from_fn(|_| Box::new(Mp3CheckpointStorage::new()));
    for (i, (file, codec)) in [
        (CBR, AudioCodec::Mp3),
        (FLAC_MONO, AudioCodec::Flac),
        (WAV, AudioCodec::WavPcm16),
        (VBR, AudioCodec::Mp3),
    ]
    .into_iter()
    .enumerate()
    {
        let mut fresh = Driver::new(file, codec);
        let baseline = fresh.all();
        let stream = if codec == AudioCodec::WavPcm16 {
            SourceDecoder::wav(
                match fresh.stream.descriptor().unwrap() {
                    SourceDescriptor::Wav(w) => w,
                    _ => unreachable!(),
                },
                file.len() as u64,
                i as u32 + 10,
            )
            .unwrap()
        } else {
            SourceDecoder::compressed(codec, file.len() as u64, i as u32 + 10, &mut storage)
                .unwrap()
        };
        let mut d = Driver {
            file,
            storage,
            checkpoints,
            encoded,
            stream,
            reads: 0,
            chunk: 512,
        };
        assert_eq!(d.all(), baseline);
        assert_eq!(d.stream.generation(), i as u32 + 10);
        storage = d.storage;
        encoded = d.encoded;
        checkpoints = d.checkpoints;
    }
}
#[test]
fn every_codec_is_invariant_under_single_byte_and_odd_short_reads() {
    for (file, codec) in compressed()
        .into_iter()
        .chain([(WAV, AudioCodec::WavPcm16)])
    {
        let baseline = Driver::new(file, codec).all();
        for chunk in [1, 3, 127, 511] {
            let mut d = Driver::new(file, codec);
            d.chunk = chunk;
            assert_eq!(d.all(), baseline, "codec={codec:?} chunk={chunk}");
        }
    }
}
#[test]
fn seek_is_sample_exact_for_vbr_gapless_and_flac_and_rejects_old_pcm() {
    use ruzina_audio_runtime::source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES};
    for (file, codec) in compressed()
        .into_iter()
        .chain([(WAV, AudioCodec::WavPcm16)])
    {
        let mut d = Driver::new(file, codec);
        let first = d.block();
        let baseline = Driver::new(file, codec).all();
        let rate = d.stream.descriptor().unwrap().sample_rate_hz();
        for (i, ms) in [71, 5, 149, 0].into_iter().enumerate() {
            d.change(i as u32 + 2, StreamChange::SeekMilliseconds(ms));
            let offset = (u64::from(ms) * u64::from(rate) / 1000) as usize;
            let decoded = d.all();
            assert_eq!(decoded, baseline[offset..], "codec={codec:?} ms={ms}");
        }
        let mut history = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
        let mut source = CanonicalSource::new(&mut history).unwrap();
        source.reset(5);
        assert_eq!(source.ingest(&first), IngestResult::Stale);
    }
}
#[test]
fn compressed_loop_prefix_wrap_resize_exit_and_eof_keep_exact_media_coordinates() {
    for (file, codec) in compressed() {
        let baseline = Driver::new(file, codec).all();
        let mut d = Driver::new(file, codec);
        d.block();
        let rate = d.stream.descriptor().unwrap().sample_rate_hz();
        d.change(
            2,
            StreamChange::Loop {
                position_frame: 0,
                region_ms: Some((10, 80)),
            },
        );
        for _ in 0..45 {
            let b = d.block();
            assert_eq!(b.generation, 2);
            assert!(!b.eof && !b.faulted);
            for (i, sample) in b.frames().iter().enumerate() {
                let frame = b.timeline.media_frame(b.first_frame + i as u64).unwrap();
                assert_eq!(*sample, baseline[frame as usize], "codec={codec:?}");
            }
        }
        let position = d.stream.media_frame();
        d.change(
            3,
            StreamChange::Loop {
                position_frame: position,
                region_ms: Some((20, 60)),
            },
        );
        let b = d.block();
        assert!(!b.faulted);
        assert_eq!(b.frames()[0], baseline[b.timeline.origin() as usize]);
        let position = d.stream.media_frame();
        d.change(
            4,
            StreamChange::Loop {
                position_frame: position,
                region_ms: None,
            },
        );
        assert_eq!(d.all(), baseline[position as usize..]);
        assert_eq!(rate, d.stream.descriptor().unwrap().sample_rate_hz());
    }
}
#[test]
fn short_compressed_loop_is_cached_without_repeated_file_reads() {
    for (file, codec) in compressed() {
        let baseline = Driver::new(file, codec).all();
        let mut d = Driver::new(file, codec);
        d.block();
        let rate = u64::from(d.stream.descriptor().unwrap().sample_rate_hz());
        let start = 30 * rate / 1000;
        let end = 31 * rate / 1000;
        d.change(
            2,
            StreamChange::Loop {
                position_frame: start,
                region_ms: Some((30, 31)),
            },
        );
        let first = d.block();
        assert!(!first.faulted);
        assert_eq!(first.len as u64, end - start);
        let reads = d.reads;
        for _ in 0..20 {
            let b = d.block();
            assert_eq!(b.len, 256);
            assert!(!b.eof && !b.faulted);
            for (i, sample) in b.frames().iter().enumerate() {
                let at = b.timeline.media_frame(b.first_frame + i as u64).unwrap();
                assert_eq!(*sample, baseline[at as usize]);
            }
        }
        assert_eq!(d.reads, reads);
    }
}
#[test]
fn malformed_flac_crc_truncated_mp3_and_impossible_bounds_fail_closed() {
    let mut crc = FLAC.to_vec();
    let index = crc.len() - 5;
    crc[index] ^= 0x80;
    for (file, codec) in [
        (&crc[..], AudioCodec::Flac),
        (&CBR[..CBR.len() * 3 / 4], AudioCodec::Mp3),
    ] {
        let mut d = Driver::new(file, codec);
        let mut failed = false;
        for _ in 0..500 {
            let b = d.block();
            if b.faulted {
                failed = true;
                break;
            }
            assert!(!b.eof, "malformed stream reported EOF");
        }
        assert!(failed);
    }
    let mut large = FLAC.to_vec();
    large[10..12].copy_from_slice(&16384u16.to_be_bytes());
    assert!(Driver::new(&large, AudioCodec::Flac).block().faulted);
    let mut d = Driver::new(&[0; 1000], AudioCodec::Mp3);
    assert!(d.block().faulted);
    let mut d = Driver::new(CBR, AudioCodec::Mp3);
    d.block();
    assert!(!d.stream.change(
        &mut d.storage,
        &mut views(&mut d.checkpoints),
        &mut d.encoded,
        0,
        StreamChange::SeekMilliseconds(0)
    ));
    assert!(!d.stream.change(
        &mut d.storage,
        &mut views(&mut d.checkpoints),
        &mut d.encoded,
        2,
        StreamChange::Loop {
            position_frame: 0,
            region_ms: Some((80, 10))
        }
    ));
}
#[test]
fn io_failure_is_generation_bound_and_waiting_for_input_never_advances_pcm() {
    let mut d = Driver::new(CBR, AudioCodec::Mp3);
    for _ in 0..5 {
        assert!(matches!(
            d.stream.step(
                &mut d.storage,
                &mut views(&mut d.checkpoints),
                &mut d.encoded
            ),
            DecodeAction::Read { offset: 0, .. }
        ));
        assert_eq!(d.stream.media_frame(), 0);
    }
    assert!(!d.stream.provide(&mut d.encoded, &[]));
    let DecodeAction::Block(b) = d.stream.step(
        &mut d.storage,
        &mut views(&mut d.checkpoints),
        &mut d.encoded,
    ) else {
        panic!()
    };
    assert!(b.faulted);
    assert_eq!(b.generation, 1);
}
#[test]
fn exact_end_seek_reports_eof_and_unknown_metadata_does_not_fake_a_length() {
    for (file, codec) in compressed() {
        let mut d = Driver::new(file, codec);
        assert!(d.stream.descriptor().is_none());
        d.block();
        let count = d.stream.descriptor().unwrap().frames().unwrap();
        d.change(2, StreamChange::SeekMilliseconds(u32::MAX));
        let b = d.block();
        assert!(b.eof && !b.faulted);
        assert_eq!(b.first_frame, count);
        assert_eq!(b.len, 0);
    }
}
#[test]
fn decoder_storage_and_input_are_fixed_and_reusable_across_formats() {
    assert!(core::mem::size_of::<DecoderStorage>() < 110 * 1024);
    assert!(core::mem::size_of::<Mp3CheckpointStorage>() < 40 * 1024);
    assert!(core::mem::size_of::<SourceDecoder>() < 4096);
    assert_eq!(ENCODED_BUFFER_BYTES, 98304);
    let mut storage = DecoderStorage::new();
    assert!(SourceDecoder::compressed(AudioCodec::WavPcm16, 100, 1, &mut storage).is_none());
    assert!(SourceDecoder::compressed(AudioCodec::Mp3, 0, 1, &mut storage).is_none());
    for codec in [AudioCodec::Mp3, AudioCodec::Flac, AudioCodec::Mp3] {
        let mut d = SourceDecoder::compressed(codec, 100, 1, &mut storage).unwrap();
        let mut short = [0; 512];
        let mut checkpoints = core::array::from_fn(|_| Box::new(Mp3CheckpointStorage::new()));
        let DecodeAction::Block(b) = d.step(&mut storage, &mut views(&mut checkpoints), &mut short)
        else {
            panic!()
        };
        assert!(b.faulted);
    }
    assert!(matches!(
        SourceDescriptor::from(
            WavPcm16SourceDescriptor::new(
                WavPcm16Format {
                    channels: 2,
                    sample_rate_hz: 48000,
                    byte_rate: 192000,
                    block_align: 4
                },
                44,
                48000
            )
            .unwrap()
        ),
        SourceDescriptor::Wav(_)
    ));
}

#[test]
fn id3_cover_art_is_skipped_by_length_even_when_it_contains_valid_mp3_audio() {
    let baseline = Driver::new(CBR, AudioCodec::Mp3).all();
    let body = CBR.repeat(60);
    let size = body.len();
    let mut tagged = b"ID3\x04\x00\x00".to_vec();
    tagged.extend_from_slice(&[
        ((size >> 21) & 127) as u8,
        ((size >> 14) & 127) as u8,
        ((size >> 7) & 127) as u8,
        (size & 127) as u8,
    ]);
    tagged.extend_from_slice(&body);
    tagged.extend_from_slice(CBR);
    let mut d = Driver::new(&tagged, AudioCodec::Mp3);
    assert_eq!(d.all(), baseline);
    assert!(d.reads < 25, "metadata body was read into the decoder");
    tagged[6] = 0x80;
    assert!(Driver::new(&tagged, AudioCodec::Mp3).block().faulted);
}

#[test]
fn mp3_without_xing_has_no_invented_length_and_seek_is_still_exact() {
    let file = include_bytes!("fixtures/codecs/untagged.mp3");
    let mut d = Driver::new(file, AudioCodec::Mp3);
    let baseline = d.all();
    assert!(baseline.len() >= 12000);
    assert_eq!(d.stream.descriptor().unwrap().frames(), None);
    d.change(2, StreamChange::SeekMilliseconds(25));
    assert_eq!(d.all(), baseline[1200..]);
    d.change(3, StreamChange::SeekMilliseconds(u32::MAX));
    assert!(d.block().faulted);
}

fn composed_two_decks(fail_first: bool) {
    use ruzina_audio_runtime::{
        fx::FX_DECK_STORAGE_SAMPLES,
        playback::PlaybackOutput,
        render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
        sink::{BlockProgress, SinkBlock, SinkRole},
        source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES},
    };
    use ruzina_mixer::HeadphoneGains;
    let mut drivers = [
        Driver::new(CBR, AudioCodec::Mp3),
        Driver::new(FLAC_MONO, AudioCodec::Flac),
    ];
    let mut pending = [Some(drivers[0].block()), Some(drivers[1].block())];
    let mut a = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut b = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut a).unwrap(),
        CanonicalSource::new(&mut b).unwrap(),
    ];
    for source in &mut sources {
        source.reset(1);
    }
    let mut control = RenderControl {
        decks: core::array::from_fn(|i| RenderDeckControl {
            source_epoch: 1,
            stream_generation: 1,
            source_sample_rate_hz: drivers[i].stream.descriptor().unwrap().sample_rate_hz(),
            pfl_enabled: true,
            playing: true,
            ..RenderDeckControl::idle()
        }),
        headphones: HeadphoneGains {
            cue: 1.0,
            master: 0.0,
            level: 1.0,
        },
        ..RenderControl::idle()
    };
    let mut fa = vec![0.0; FX_DECK_STORAGE_SAMPLES];
    let mut fb = vec![0.0; FX_DECK_STORAGE_SAMPLES];
    let mut graph = FixedRateRenderGraph::with_fx([&mut fa, &mut fb]).unwrap();
    let mut output = RenderOutputBlock::silence();
    let mut main = [SinkBlock::EMPTY; 1];
    let mut phones = [SinkBlock::EMPTY; 1];
    let mut playback = PlaybackOutput::new_reference(&mut main, &mut phones).unwrap();
    playback.sync_sources([1, 1], 0).unwrap();
    let token = playback.lane(SinkRole::Main).token().unwrap();
    playback
        .lane_mut(SinkRole::Main)
        .set_mute(token, false, 0)
        .unwrap();
    let mut ends = [0u64; 2];
    let mut terminal = [false; 2];
    let mut pressure = 0;
    let mut last_b = 0;
    for n in 0..140u64 {
        if fail_first && n == 20 {
            let mut fault = drivers[0].stream.fault_block();
            fault.first_frame = ends[0];
            assert_eq!(sources[0].ingest(&fault), IngestResult::Fault);
            terminal[0] = true;
            pending[0] = None;
        }
        if n == 40 {
            control.decks[1].master_tempo = true;
            control.decks[1].pitch_centipercent = 500;
        }
        if n == 80 {
            control.decks[1].master_tempo = false;
            control.decks[1].pitch_centipercent = -300;
        }
        for i in 0..2 {
            if terminal[i] {
                continue;
            }
            for _ in 0..16 {
                let block = pending[i].take().unwrap_or_else(|| drivers[i].block());
                match sources[i].ingest(&block) {
                    IngestResult::Accepted => {
                        ends[i] = block.end_frame();
                        if block.eof {
                            terminal[i] = true;
                            break;
                        }
                    }
                    IngestResult::Backpressure => {
                        pressure += 1;
                        pending[i] = Some(block);
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
        let report = graph.render_canonical_block(control, &mut output, &mut sources);
        assert_eq!(
            report.underflow_frames[1], 0,
            "second deck underrun at block {n}"
        );
        if fail_first && n >= 20 {
            assert!(output.pfl[0].iter().all(|f| *f == PcmFrame::default()));
            assert_eq!(output.headphones, output.pfl[1]);
        }
        let progress = BlockProgress {
            decks: core::array::from_fn(|i| {
                sources[i].take_progress(control.decks[i].source_control())
            }),
            source_frames: report.source_frames_consumed.map(|v| v as u16),
        };
        let first = n * 128;
        assert!(
            playback.submit_render(playback.render_permit(), first, &output, progress)[0].is_ok()
        );
        let receipt = playback
            .lane_mut(SinkRole::Main)
            .issue(token, first, 128, &mut output.main)
            .unwrap();
        assert_eq!(
            playback.confirmed_progress(1).map_or(0, |p| p.end_frame),
            last_b
        );
        playback.complete(receipt).unwrap();
        if let Some(p) = playback.confirmed_progress(1) {
            assert!(p.end_frame >= last_b);
            last_b = p.end_frame;
        }
    }
    assert!(pressure > 0);
    assert!(last_b > 7000);
    assert!(!playback.confirmed_progress(1).unwrap().eof);
    if !fail_first {
        assert!(playback.confirmed_progress(0).unwrap().eof);
    }
}

#[test]
fn mixed_rate_mp3_flac_run_together_through_master_tempo_pfl_backpressure_and_receipts() {
    composed_two_decks(false);
}

#[test]
fn compressed_source_failure_clears_only_its_wet_history_while_other_deck_keeps_running() {
    composed_two_decks(true);
}

#[test]
fn late_mp3_loop_restores_exact_state_without_redecoding_the_track_prefix() {
    let file = include_bytes!("fixtures/codecs/long.mp3");
    let baseline = Driver::new(file, AudioCodec::Mp3).all();
    let mut d = Driver::new(file, AudioCodec::Mp3);
    d.block();
    d.change(
        2,
        StreamChange::Loop {
            position_frame: 6 * 48000,
            region_ms: Some((6000, 6250)),
        },
    );
    let first = d.block();
    assert!(!first.faulted);
    let reads = d.reads;
    let mut wraps = 0;
    let mut previous = first.timeline.media_frame(first.first_frame).unwrap();
    for _ in 0..240 {
        let block = d.block();
        assert!(!block.faulted && !block.eof);
        for (i, sample) in block.frames().iter().enumerate() {
            let at = block
                .timeline
                .media_frame(block.first_frame + i as u64)
                .unwrap();
            if at < previous {
                wraps += 1;
            }
            previous = at;
            assert_eq!(*sample, baseline[at as usize]);
        }
    }
    assert!(wraps >= 4);
    assert!(
        d.reads - reads < 100,
        "loop re-read the six-second prefix: {} reads",
        d.reads - reads
    );
}

#[test]
fn recent_mp3_seek_restores_checkpoint_samples_and_bounds_file_reads() {
    let file = include_bytes!("fixtures/codecs/long.mp3");
    let mut d = Driver::new(file, AudioCodec::Mp3);
    let baseline = d.all();
    let reads = d.reads;
    d.change(2, StreamChange::SeekMilliseconds(7500));
    assert_eq!(d.all(), baseline[360000..]);
    assert!(d.reads - reads < 50, "warm seek replayed the entire track");
}

// RP-08: cold seeks resume at walked frame offsets through the codec's own
// reservoir preroll and stay sample-identical to a linear decode.
const LONG_VBR: &[u8] = include_bytes!("fixtures/codecs/long-vbr.mp3");
const LONG_VBR_UNTAGGED: &[u8] = include_bytes!("fixtures/codecs/long-vbr-untagged.mp3");
const LONG_CBR: &[u8] = include_bytes!("fixtures/codecs/long.mp3");

fn cold_seek(file: &[u8], scan: bool, ms: u32, baseline: &[PcmFrame]) -> usize {
    let mut d = Driver::new(file, AudioCodec::Mp3);
    d.block();
    if scan {
        assert!(d.scan(usize::MAX) > 0);
        assert!(d.storage.mp3_index().complete());
    }
    d.change(2, StreamChange::SeekMilliseconds(ms));
    let rate = d.stream.descriptor().unwrap().sample_rate_hz();
    let offset = (u64::from(ms) * u64::from(rate) / 1000) as usize;
    let (reads, first) = d.first_block_reads();
    assert!(!first.faulted);
    let mut decoded = first.frames().to_vec();
    decoded.extend(d.all());
    assert_eq!(decoded, baseline[offset..], "ms={ms} scan={scan}");
    reads
}

#[test]
fn cold_mp3_seeks_use_the_frame_index_and_stay_sample_exact() {
    use ruzina_audio_runtime::mp3_index::Mp3TagKind;
    for (file, kind) in [
        (LONG_VBR, Mp3TagKind::Variable),
        (LONG_CBR, Mp3TagKind::Constant),
    ] {
        let mut probe = Driver::new(file, AudioCodec::Mp3);
        let baseline = probe.all();
        probe.scan(usize::MAX);
        let index = probe.storage.mp3_index();
        assert_eq!(index.kind(), kind);
        assert_eq!(index.variable_bitrate(), kind == Mp3TagKind::Variable);
        for ms in [7900, 6000, 4999, 3100, 777, 30] {
            let indexed = cold_seek(file, true, ms, &baseline);
            assert!(
                indexed < 24,
                "{kind:?} ms={ms}: {indexed} reads before audio"
            );
            if ms >= 3100 {
                let linear = cold_seek(file, false, ms, &baseline);
                assert!(
                    linear > 3 * indexed,
                    "{kind:?} ms={ms}: {linear} vs {indexed}"
                );
            }
        }
    }
}

#[test]
fn untagged_vbr_keeps_the_exact_fallback_and_reports_scanned_duration() {
    use ruzina_audio_runtime::{decode::SourceDuration, mp3_index::Mp3TagKind};
    let mut d = Driver::new(LONG_VBR_UNTAGGED, AudioCodec::Mp3);
    let baseline = d.all();
    assert_eq!(d.stream.duration(&d.storage), SourceDuration::Unknown);
    assert_eq!(d.stream.duration(&d.storage).millis(44_100), None);
    d.scan(usize::MAX);
    let index = d.storage.mp3_index();
    assert_eq!(index.kind(), Mp3TagKind::Plain);
    assert!(index.variable_bitrate() && index.complete());
    assert_eq!(
        d.stream.duration(&d.storage),
        SourceDuration::Scanned(baseline.len() as u64)
    );
    // The deck learns the walked length in whole milliseconds (RP-08).
    let rate = d.stream.descriptor().unwrap().sample_rate_hz();
    assert_eq!(
        d.stream.duration(&d.storage).millis(rate),
        Some((baseline.len() as u64 * 1_000 / u64::from(rate)) as u32)
    );
    assert_eq!(SourceDuration::Scanned(1).millis(0), None);
    // The codec's byte arithmetic would misplace an untagged VBR seek, so it
    // is never used; the result is the linear-exact fallback.
    cold_seek(LONG_VBR_UNTAGGED, true, 6500, &baseline);

    // Untagged CBR resumes through the index and scans an exact length.
    let file = include_bytes!("fixtures/codecs/untagged.mp3");
    let mut d = Driver::new(file, AudioCodec::Mp3);
    let baseline = d.all();
    d.scan(usize::MAX);
    assert_eq!(
        d.stream.duration(&d.storage),
        SourceDuration::Scanned(baseline.len() as u64)
    );
    cold_seek(file, true, 150, &baseline);

    let mut tagged = Driver::new(LONG_VBR, AudioCodec::Mp3);
    tagged.block();
    tagged.scan(usize::MAX);
    let SourceDuration::Exact(frames) = tagged.stream.duration(&tagged.storage) else {
        panic!("Xing frame count is exact");
    };
    assert_eq!(
        frames,
        Driver::new(LONG_VBR, AudioCodec::Mp3).all().len() as u64
    );
}

#[test]
fn partial_index_serves_covered_targets_and_falls_back_beyond_it() {
    let baseline = Driver::new(LONG_VBR, AudioCodec::Mp3).all();
    let rate = 44_100;
    for (ms, partial_reads) in [(1500, 60), (7000, 60), (7000, 0)] {
        let mut d = Driver::new(LONG_VBR, AudioCodec::Mp3);
        d.block();
        d.scan(partial_reads);
        assert!(!d.storage.mp3_index().complete());
        d.change(2, StreamChange::SeekMilliseconds(ms));
        let (_, first) = d.first_block_reads();
        let mut decoded = first.frames().to_vec();
        decoded.extend(d.all());
        assert_eq!(decoded, baseline[ms as usize * rate / 1000..], "ms={ms}");
    }
}

fn views(slots: &mut [Box<Mp3CheckpointStorage>; 3]) -> [&mut Mp3CheckpointStorage; 3] {
    let [a, b, c] = slots;
    [a.as_mut(), b.as_mut(), c.as_mut()]
}
