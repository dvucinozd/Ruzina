use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::{
    pcm_stream::{PcmBlockDisposition, PcmStreamBlock, PcmTransportCursor},
    playback::PlaybackOutput,
    render::{FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock},
    sink::{BlockProgress, SinkBlock, SinkError, SinkRole},
    source::{CanonicalSource, IngestResult, SOURCE_HISTORY_FRAMES, SourceControl},
    timeline::{FrameLoop, PcmTimeline},
    wav::{WavPcm16Format, WavPcm16SourceDescriptor},
    wav_stream::{WavStreamChange, WavStreamDecoder},
};

fn sample(frame: u64, channels: u16) -> PcmFrame {
    let left = (1000 + frame % 20_000) as i16;
    PcmFrame {
        left,
        right: if channels == 1 { left } else { -left },
    }
}

struct Reader {
    data: Vec<u8>,
    decoder: WavStreamDecoder,
    offset: usize,
    chunk: usize,
    reads: usize,
    rewinds: usize,
    terminal: bool,
    frame_bytes: usize,
}
impl Reader {
    fn new(channels: u16, rate: u32, total: u64, region: Option<(u64, u64)>, origin: u64) -> Self {
        let format = WavPcm16Format {
            channels,
            sample_rate_hz: rate,
            byte_rate: rate * u32::from(channels) * 2,
            block_align: channels * 2,
        };
        let descriptor =
            WavPcm16SourceDescriptor::new(format, 44, total * u64::from(format.block_align))
                .unwrap();
        let mut data = Vec::new();
        for frame in 0..total {
            let pcm = sample(frame, channels);
            data.extend_from_slice(&pcm.left.to_le_bytes());
            if channels == 2 {
                data.extend_from_slice(&pcm.right.to_le_bytes());
            }
        }
        let decoder = WavStreamDecoder::new(
            descriptor,
            7,
            origin,
            region.map(|(start, end)| FrameLoop::new(start, end, total).unwrap()),
        )
        .unwrap();
        let offset = decoder.media_frame() as usize * format.frame_bytes();
        Self {
            data,
            decoder,
            offset,
            chunk: 512,
            reads: 0,
            rewinds: 0,
            terminal: false,
            frame_bytes: format.frame_bytes(),
        }
    }
    fn change(&mut self, generation: u32, change: WavStreamChange) {
        self.decoder = self.decoder.changed(generation, change).unwrap();
        self.terminal = false;
        self.offset = self.decoder.media_frame() as usize * self.frame_bytes;
    }
    fn next(&mut self, channels: u16) -> PcmStreamBlock {
        assert!(!self.terminal);
        for _ in 0..1024 {
            if let Some(block) = self.decoder.cached_block() {
                self.terminal = block.eof || block.faulted;
                return block;
            }
            if let Some(frame) = self.decoder.rewind_frame() {
                self.offset = frame as usize * usize::from(channels) * 2;
                self.rewinds += 1;
                assert!(self.decoder.complete_rewind());
            }
            let end = (self.offset + self.chunk).min(self.data.len());
            let block = self.decoder.decode(&self.data[self.offset..end]);
            self.offset = end;
            self.reads += 1;
            if let Some(block) = block {
                self.terminal = block.eof || block.faulted;
                return block;
            }
        }
        panic!("bounded split-read progress");
    }
}

fn control() -> SourceControl {
    SourceControl {
        playing: true,
        tempo_factor: 1.0,
        output_sample_rate_hz: 48_000,
        ..SourceControl::default()
    }
}

fn fill(
    source: &mut CanonicalSource<'_>,
    reader: &mut Reader,
    pending: &mut Option<PcmStreamBlock>,
    channels: u16,
) {
    for _ in 0..16 {
        let block = match pending.take() {
            Some(block) => block,
            None if !reader.terminal => reader.next(channels),
            None => break,
        };
        match source.ingest(&block) {
            IngestResult::Accepted => {}
            IngestResult::Backpressure => {
                *pending = Some(block);
                break;
            }
            result => panic!("unexpected ingest {result:?}"),
        }
    }
}

#[test]
fn decoded_loop_samples_are_exact_across_partial_blocks_mono_and_stereo() {
    for channels in [1, 2] {
        for (start, end) in [(17, 530), (0, 1), (9, 10), (0, 256), (0, 257), (100, 1001)] {
            let mut reader = Reader::new(channels, 48_000, 1001, Some((start, end)), 0);
            let timeline = reader.decoder.timeline();
            let mut next = 0;
            for _ in 0..70 {
                let block = reader.next(channels);
                assert_eq!(block.generation, 7);
                assert_eq!(block.first_frame, next);
                assert!(!block.eof && !block.faulted);
                for (offset, pcm) in block.frames().iter().enumerate() {
                    let virtual_frame = next + offset as u64;
                    let expected = if virtual_frame < end {
                        virtual_frame
                    } else {
                        start + (virtual_frame - end) % (end - start)
                    };
                    assert_eq!(*pcm, sample(expected, channels));
                    assert_eq!(timeline.media_frame(virtual_frame), Some(expected));
                }
                next = block.end_frame();
            }
            if end - start <= 256 {
                assert!(reader.reads <= 5, "short loop uses bounded cache");
            }
        }
    }
}

#[test]
fn split_byte_reads_keep_carry_at_loop_end_and_output_capacity() {
    for channels in [1, 2] {
        for chunk in [1, 3, 511, 512] {
            let mut reader = Reader::new(channels, 48_000, 1500, Some((13, 777)), 13);
            reader.chunk = chunk;
            let mut frame = 13;
            for _ in 0..80 {
                let block = reader.next(channels);
                assert!(!block.faulted && !block.eof);
                for pcm in block.frames() {
                    assert_eq!(*pcm, sample(frame, channels));
                    frame += 1;
                    if frame == 777 {
                        frame = 13;
                    }
                }
            }
        }
    }
}

#[test]
fn full_mono_block_keeps_the_extra_half_sample_after_an_initial_split_read() {
    let mut reader = Reader::new(1, 48_000, 1500, Some((13, 777)), 13);
    assert!(
        reader
            .decoder
            .decode(&reader.data[reader.offset..reader.offset + 1])
            .is_none()
    );
    reader.offset += 1;
    let mut frame = 13;
    for _ in 0..20 {
        let block = reader.next(1);
        assert!(!block.faulted && !block.eof);
        for pcm in block.frames() {
            assert_eq!(*pcm, sample(frame, 1));
            frame += 1;
            if frame == 777 {
                frame = 13;
            }
        }
    }
}

#[test]
fn canonical_loop_longer_than_history_repeats_without_underflow_or_eof() {
    let start = 333;
    let end = SOURCE_HISTORY_FRAMES as u64 * 3 + 19;
    let mut reader = Reader::new(2, 48_000, end + 99, Some((start, end)), 0);
    let mut history = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut history).unwrap();
    source.reset(7);
    let mut pending = None;
    for virtual_frame in 0..end * 3 {
        if virtual_frame % 128 == 0 {
            fill(&mut source, &mut reader, &mut pending, 2);
        }
        let pcm = source.next_sample(control());
        assert!(!pcm.starved && !pcm.silent);
        let frame = if virtual_frame < end {
            virtual_frame
        } else {
            start + (virtual_frame - end) % (end - start)
        };
        assert_eq!(pcm.frame, sample(frame, 2));
    }
    let progress = source.take_progress(control()).unwrap();
    assert!(!progress.eof && !progress.faulted);
    assert_eq!(
        progress.end_frame,
        reader
            .decoder
            .timeline()
            .media_frame(source.position())
            .unwrap()
    );
    assert!(reader.rewinds >= 2);
    assert!(core::mem::size_of::<WavStreamDecoder>() < 2048);
}

#[test]
fn one_frame_loop_prefills_full_blocks_in_a_fixed_owner_budget() {
    let mut reader = Reader::new(2, 48_000, 300, Some((17, 18)), 17);
    let mut history = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut history).unwrap();
    source.reset(7);
    let mut pending = None;
    fill(&mut source, &mut reader, &mut pending, 2);
    for _ in 0..128 {
        let pcm = source.next_sample(control());
        assert!(!pcm.starved);
        assert_eq!(pcm.frame, sample(17, 2));
    }
    assert_eq!(source.take_progress(control()).unwrap().end_frame, 17);
    assert_eq!(reader.reads, 1);
}

#[test]
fn loop_exit_resize_seek_and_replacement_revoke_old_pcm_and_receipts() {
    let mut reader = Reader::new(2, 48_000, 4800, Some((48, 480)), 48);
    let old = reader.next(2);
    reader.change(
        8,
        WavStreamChange::Loop {
            position_frame: 100,
            region_ms: Some((2, 4)),
        },
    );
    assert_eq!(
        reader.decoder.timeline().loop_region(),
        FrameLoop::new(96, 192, 4800)
    );
    let resized = reader.next(2);
    assert_eq!(resized.frames()[0], sample(100, 2));
    reader.change(9, WavStreamChange::SeekMilliseconds(50));
    assert_eq!(reader.decoder.media_frame(), 96); // 2400 maps back into [96,192)
    reader.change(
        10,
        WavStreamChange::Loop {
            position_frame: 150,
            region_ms: None,
        },
    );
    let linear = reader.next(2);
    assert_eq!(linear.first_frame, 150);
    assert_eq!(linear.frames()[0], sample(150, 2));
    assert!(linear.timeline.loop_region().is_none());
    let mut next = linear.end_frame();
    while !reader.terminal {
        let block = reader.next(2);
        assert_eq!(block.first_frame, next);
        assert!(!block.faulted);
        for (offset, pcm) in block.frames().iter().enumerate() {
            assert_eq!(*pcm, sample(next + offset as u64, 2));
        }
        next = block.end_frame();
    }
    assert_eq!(next, 4800);
    let mut storage = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(10);
    assert_eq!(source.ingest(&old), IngestResult::Stale);
    assert_eq!(source.ingest(&resized), IngestResult::Stale);
    assert_eq!(source.ingest(&linear), IngestResult::Accepted);
    let mut main = [SinkBlock::EMPTY; 1];
    let mut phones = [SinkBlock::EMPTY; 1];
    let mut playback = PlaybackOutput::new_reference(&mut main, &mut phones).unwrap();
    playback.sync_sources([7, 0], 0).unwrap();
    let output = RenderOutputBlock::silence();
    assert_eq!(
        playback.submit_render(
            playback.render_permit(),
            0,
            &output,
            BlockProgress::default()
        )[0],
        Ok(())
    );
    let token = playback.lane(SinkRole::Main).token().unwrap();
    let mut pcm = output.main;
    let receipt = playback
        .lane_mut(SinkRole::Main)
        .issue(token, 0, 128, &mut pcm)
        .unwrap();
    playback.sync_sources([10, 0], 0).unwrap();
    assert_eq!(playback.complete(receipt), Err(SinkError::Stale));
}

#[test]
fn two_looped_decks_publish_wrapped_position_only_after_completed_render() {
    let mut readers = [
        Reader::new(2, 48_000, 2000, Some((17, 530)), 17),
        Reader::new(2, 48_000, 2000, Some((100, 877)), 100),
    ];
    let mut ha = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut hb = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut sources = [
        CanonicalSource::new(&mut ha).unwrap(),
        CanonicalSource::new(&mut hb).unwrap(),
    ];
    for source in &mut sources {
        source.reset(7);
    }
    let mut pending = [None, None];
    let mut graph = FixedRateRenderGraph::new();
    let mut a = [SinkBlock::EMPTY; 2];
    let mut b = [SinkBlock::EMPTY; 1];
    let mut playback = PlaybackOutput::new_reference(&mut a, &mut b).unwrap();
    playback.sync_sources([7, 7], 0).unwrap();
    let deck = RenderDeckControl {
        playing: true,
        stream_generation: 7,
        post_gain: 0.25,
        pfl_enabled: true,
        ..RenderDeckControl::idle()
    };
    let control = RenderControl {
        decks: [deck; 2],
        master_gain: 1.0,
        ..RenderControl::idle()
    };
    let mut cursors = [PcmTransportCursor::new(); 2];
    for block_index in 0..80u64 {
        for i in 0..2 {
            fill(&mut sources[i], &mut readers[i], &mut pending[i], 2);
        }
        let mut block = RenderOutputBlock::silence();
        let report = graph.render_canonical_block(control, &mut block, &mut sources);
        assert_eq!(report.underflow_frames, [0, 0]);
        let progress = BlockProgress {
            decks: [
                sources[0].take_progress(control.decks[0].source_control()),
                sources[1].take_progress(control.decks[1].source_control()),
            ],
            source_frames: report.source_frames_consumed.map(|v| v as u16),
        };
        let first = block_index * 128;
        assert_eq!(
            playback.submit_render(playback.render_permit(), first, &block, progress)[0],
            Ok(())
        );
        if block_index == 0 {
            assert!(playback.confirmed_progress(0).is_none());
        }
        let token = playback.lane(SinkRole::Main).token().unwrap();
        let mut pcm = block.main;
        let receipt = playback
            .lane_mut(SinkRole::Main)
            .issue(token, first, 128, &mut pcm)
            .unwrap();
        playback.complete(receipt).unwrap();
        for i in 0..2 {
            let (mark, frames) = playback.pending_progress(i).unwrap();
            assert!(!mark.eof && !mark.faulted);
            assert!(matches!(
                cursors[i].accept_active_progress(mark, frames, 7),
                PcmBlockDisposition::Accepted { eof: false, .. }
            ));
            assert_eq!(
                mark.end_frame,
                readers[i]
                    .decoder
                    .timeline()
                    .media_frame(sources[i].position())
                    .unwrap()
            );
            playback.clear_progress(i);
        }
    }
    assert!(!cursors[0].ended() && !cursors[1].ended());
}

#[test]
fn keylock_and_pitch_use_continuous_loop_pcm_and_wrap_only_media_position() {
    for (rate, tempo, master_tempo) in [
        (48_000, 1.0, false),
        (44_100, 1.5, false),
        (48_000, 0.75, true),
        (48_000, 1.5, true),
        (44_100, 1.25, true),
        (96_000, 2.0, true),
    ] {
        let mut reader = Reader::new(2, rate, 3000, Some((100, 600)), 100);
        let mut storage = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
        let mut source = CanonicalSource::new(&mut storage).unwrap();
        source.reset(7);
        let mode = SourceControl {
            tempo_factor: tempo,
            master_tempo,
            ..control()
        };
        let mut pending = None;
        for i in 0..8000 {
            if i % 64 == 0 {
                fill(&mut source, &mut reader, &mut pending, 2);
            }
            let pcm = source.next_sample(mode);
            assert!(
                !pcm.starved && !pcm.deferred && !pcm.silent,
                "rate={rate} tempo={tempo} at {i}"
            );
        }
        let progress = source.take_progress(mode).unwrap();
        assert_eq!(
            progress.end_frame,
            reader
                .decoder
                .timeline()
                .media_frame(source.position())
                .unwrap()
        );
        assert!(!progress.eof && !progress.faulted);
    }
}

#[test]
fn censor_and_paused_scratch_keep_loop_media_mapping_and_release_state() {
    use ruzina_audio_scratch::ScratchConfig;
    let mut reader = Reader::new(2, 48_000, 3000, Some((100, 600)), 100);
    let mut storage = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(7);
    let mut pending = None;
    for i in 0..900 {
        if i % 64 == 0 {
            fill(&mut source, &mut reader, &mut pending, 2);
        }
        source.next_sample(control());
    }
    let before = source.position();
    let censor = SourceControl {
        censor: true,
        ..control()
    };
    for offset in 0..100 {
        let pcm = source.next_sample(censor);
        let media = reader
            .decoder
            .timeline()
            .media_frame(before - 1 - offset)
            .unwrap();
        assert_eq!(pcm.frame, sample(media, 2));
        assert!(!pcm.starved);
    }
    assert_eq!(source.position(), before + 100);
    let media = reader
        .decoder
        .timeline()
        .media_frame(source.position())
        .unwrap();
    assert_eq!(source.next_sample(control()).frame, sample(media, 2));
    source.configure_scratch(ScratchConfig::sanitized(1.0, 1, 1.0, 6.0, 100));
    let parked = source.position();
    let scratch = SourceControl {
        playing: false,
        scratch: true,
        scratch_epoch: 4,
        ..control()
    };
    source.jog(7, 4, -2);
    for _ in 0..4 {
        source.next_sample(scratch);
    }
    assert_eq!(source.position(), parked);
    let progress = source.take_progress(scratch).unwrap();
    assert_eq!(
        progress.end_frame,
        reader
            .decoder
            .timeline()
            .media_frame(source.audible_position())
            .unwrap()
    );
    assert!(!progress.eof);
    source.next_sample(control());
    assert!(!source.take_progress(control()).unwrap().eof);
}

#[test]
fn changed_loop_metadata_in_same_generation_faults_instead_of_reusing_history() {
    let mut reader = Reader::new(2, 48_000, 3000, Some((100, 600)), 100);
    let mut storage = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(7);
    let first = reader.next(2);
    assert_eq!(source.ingest(&first), IngestResult::Accepted);
    let mut next = reader.next(2);
    next.timeline = PcmTimeline::linear();
    assert_eq!(source.ingest(&next), IngestResult::Fault);
    assert!(source.take_progress(control()).unwrap().faulted);
}

#[test]
fn failed_rewind_read_or_media_replacement_stops_only_its_generation() {
    let mut reader = Reader::new(2, 48_000, 600, Some((17, 530)), 17);
    while reader.decoder.rewind_frame().is_none() {
        assert!(!reader.next(2).faulted);
    }
    // Filesystem seek failed: publish the exact producer's terminal fault.
    let fault = reader.decoder.fault_block();
    let mut cursor = PcmTransportCursor::new();
    assert!(matches!(
        cursor.accept(&fault),
        PcmBlockDisposition::Fault { .. }
    ));
    assert!(cursor.ended());
    let mut other = PcmTransportCursor::new();
    let mut other_reader = Reader::new(2, 48_000, 2000, Some((0, 1000)), 0);
    assert!(matches!(
        other.accept(&other_reader.next(2)),
        PcmBlockDisposition::Accepted { eof: false, .. }
    ));
    assert!(!other.ended());
    let mut storage = vec![PcmFrame::default(); SOURCE_HISTORY_FRAMES];
    let mut source = CanonicalSource::new(&mut storage).unwrap();
    source.reset(8);
    assert_eq!(source.ingest(&fault), IngestResult::Stale);
    let mut fresh = reader
        .decoder
        .changed(9, WavStreamChange::SeekMilliseconds(1))
        .unwrap();
    assert!(fresh.decode(&[]).unwrap().faulted);
}

#[test]
fn loop_validation_and_timeline_arithmetic_fail_closed() {
    assert!(FrameLoop::new(0, 0, 100).is_none());
    assert!(FrameLoop::new(90, 80, 100).is_none());
    assert!(FrameLoop::from_ms(0, 20, 48000, 100).is_none());
    assert!(FrameLoop::from_ms(0, 10, 0, 100).is_none());
    assert_eq!(
        FrameLoop::from_ms(1, 4, 44100, 1000),
        FrameLoop::new(44, 176, 1000)
    );
    let region = FrameLoop::new(10, 20, 100).unwrap();
    let timeline = PcmTimeline::at(12, Some(region));
    assert_eq!(timeline.media_frame(11), None);
    assert_eq!(timeline.media_frame(20), Some(10));
    assert_eq!(
        timeline.media_frame(u64::MAX),
        Some(10 + (u64::MAX - 20) % 10)
    );
    let reader = Reader::new(2, 48000, 100, None, 0);
    assert!(
        reader
            .decoder
            .changed(
                8,
                WavStreamChange::Loop {
                    position_frame: 0,
                    region_ms: Some((10, 20))
                }
            )
            .is_none()
    );
}
