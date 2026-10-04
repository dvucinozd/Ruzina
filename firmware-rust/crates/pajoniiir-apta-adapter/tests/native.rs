use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{CancellationToken, Session, SessionConfig, SessionState, WorkBudget},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{Error, bpm_x100, convert, milliseconds};
use pajoniiir_controller_core::{ControlEvent, ControlValue, SemanticControl};
use pajoniiir_core::DeckId;
use pajoniiir_deck::DeckProductState;
use pajoniiir_track_analysis::Beat;

const EMPTY: Beat = Beat {
    time_ms: 0,
    phase: 0,
    bpm_x100: 0,
};

#[test]
fn pcm_to_deck_sync_and_jump() {
    let mut beats = [EMPTY; 128];
    let (track, position) = {
        let mut queue = [NormalizedSample::default(); 256];
        let mut columns = vec![WaveformColumn::default(); 1250];
        let mut bins = vec![OnsetBin::default(); BIN_CAPACITY];
        let mut flux = vec![0.0; BIN_CAPACITY];
        let mut session = Session::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 320000,
                frames_per_column: 256,
            },
            &mut queue,
            &mut columns,
        )
        .unwrap();
        session.enable_tempo(&mut bins, &mut flux).unwrap();
        session.enable_meter().unwrap();
        let identity = libapta::session::SourceIdentity::new(1, [42; 32]).unwrap();
        session.set_source_identity(identity).unwrap();
        let cancel = CancellationToken::new();
        let budget = WorkBudget {
            maximum_input_frames: 256,
            maximum_steps: 1,
        };
        let mut offset = 0;
        while offset < 320000 {
            let mut pcm = [0.0; 256];
            for (i, value) in pcm.iter_mut().enumerate() {
                let frame = offset + i;
                let phase = frame % 4000;
                if phase < 64 {
                    *value =
                        (64 - phase) as f32 / 64.0 * if frame / 4000 % 4 == 0 { 0.75 } else { 0.2 };
                }
            }
            let n = session
                .push_interleaved(&pcm[..256.min(320000 - offset)])
                .unwrap();
            offset += n;
            let progress = session.process(budget, &cancel).unwrap();
            assert!(progress.consumed_input_frames <= 256);
            assert!(progress.completed_steps <= 1);
        }
        session.finish_input().unwrap();
        for _ in 0..10000 {
            if session.state() == SessionState::Complete {
                break;
            }
            session.process(budget, &cancel).unwrap();
        }
        assert_eq!(session.state(), SessionState::Complete);
        let snapshot = session.snapshot(7).unwrap();
        let input = snapshot.view();

        let analysis = convert(&input, &mut beats).unwrap();
        let track = analysis.track();
        let grid = track.beat_grid().unwrap();
        assert!(grid.beats().len() > 30);
        assert_eq!(track.bpm_x100(), 12000);
        assert_eq!(grid.beats().len(), 80);
        assert_eq!(
            grid.beats()[0],
            Beat {
                time_ms: 160,
                phase: 1,
                bpm_x100: 12000
            }
        );
        let mut deck = DeckProductState::new();
        let position = grid.beats()[10].time_ms;
        deck.set_position_ms(DeckId::One, position);
        let event = |control| ControlEvent {
            deck: Some(DeckId::One),
            control,
            value: ControlValue::Pressed(true),
        };
        deck.handle_control_with_analysis(
            event(SemanticControl::BeatJumpForward),
            [Some(track), None],
        );
        assert_eq!(deck.deck(DeckId::One).position_ms, grid.beats()[11].time_ms);
        deck.set_position_ms(DeckId::Two, position + 17);
        deck.handle_control_with_analysis(event(SemanticControl::Sync), [Some(track), Some(track)]);
        assert!(deck.deck(DeckId::One).sync_enabled);
        assert_eq!(deck.deck(DeckId::One).position_ms, position + 17);
        let mut short = [EMPTY; 1];
        assert!(matches!(convert(&input, &mut short), Err(Error::Capacity)));
        assert_eq!(short, [EMPTY]);
        let mut next = [EMPTY; 128];
        let mut changed = input;
        changed.info.generation = u32::MAX as u64 + 1;
        assert!(matches!(
            convert(&changed, &mut next),
            Err(Error::Generation)
        ));
        changed.info.generation = 8;
        let newer = convert(&changed, &mut next).unwrap();
        assert_eq!(newer.track().generation(), 8);
        assert_eq!(track.generation(), 7);
        assert_eq!(track.beat_grid().unwrap().beats()[10].time_ms, position);
        let mut pinned = analysis;
        assert_eq!(pinned.upgrade(newer, true), Err(Error::Playing));
        assert_eq!(pinned.track().generation(), 7);
        pinned
            .upgrade(newer, deck.deck(DeckId::Two).playing)
            .unwrap();
        assert_eq!(pinned.track().generation(), 8);
        assert_eq!(pinned.upgrade(analysis, false), Err(Error::Generation));
        let mut wrong = newer;
        wrong.source.fingerprint[0] ^= 1;
        assert_eq!(pinned.upgrade(wrong, false), Err(Error::Identity));

        let mut waveform_columns = vec![
            pajoniiir_waveform::WaveformColumn {
                peak: 0,
                color_rgb565: 0
            };
            1250
        ];
        let wave = pajoniiir_apta_adapter::waveform(
            input.overview.unwrap(),
            &mut waveform_columns,
            0x07e0,
        )
        .unwrap();
        assert_eq!(wave.columns[0].peak, 49151);
        let mut pixels = [0; 80 * 32];
        pajoniiir_waveform::Rgb565Surface::new(&mut pixels, 80, 32)
            .unwrap()
            .draw_centered_waveform(0, 0, 80, 32, wave);
        assert!(pixels.contains(&0x07e0));

        let wire = libapta::result::from_session_snapshot(
            &snapshot,
            &mut [],
            libapta::NativeLimits::default(),
        )
        .unwrap();
        let mut bytes = vec![0; libapta::result::serialized_size(&wire).unwrap()];
        libapta::result::write(&wire, &mut bytes, libapta::result::Limits::default()).unwrap();
        let mut cached_beats = [EMPTY; 128];
        let cached = pajoniiir_apta_adapter::convert_cache(
            &bytes,
            identity,
            9,
            libapta::result::Limits::default(),
            &mut cached_beats,
        )
        .unwrap();
        assert_eq!(
            cached.track().provider(),
            pajoniiir_track_analysis::AnalysisProvider::AptaCache
        );
        assert_eq!(cached.track().beat_grid().unwrap().beats(), grid.beats());
        assert_eq!(cached.source, analysis.source);
        assert!(matches!(
            pajoniiir_apta_adapter::convert_cache(
                &bytes,
                libapta::session::SourceIdentity::default(),
                9,
                libapta::result::Limits::default(),
                &mut short
            ),
            Err(Error::Identity)
        ));
        bytes[0] ^= 1;
        assert!(matches!(
            pajoniiir_apta_adapter::convert_cache(
                &bytes,
                identity,
                9,
                libapta::result::Limits::default(),
                &mut short
            ),
            Err(Error::Invalid)
        ));
        let segment = input.local_grid.unwrap().segments[0];
        let mut explicit = Vec::new();
        for i in 0..segment.beat_count {
            let ordinal = segment.anchor_ordinal + i64::from(i);
            explicit.push(libapta::Beat {
                position: libapta::grid::segment_position_at_ordinal(&segment, ordinal).unwrap(),
                ordinal,
                revision: segment.revision,
                flags: 0,
                confidence: segment.confidence,
            });
        }
        let mut explicit_grid = input.local_grid.unwrap();
        explicit_grid.representation = libapta::GridRepresentation::Explicit;
        explicit_grid.segments = &[];
        explicit_grid.beats = &explicit;
        let mut explicit_input = input;
        explicit_input.global_grid = Some(explicit_grid);
        let mut explicit_out = [EMPTY; 128];
        assert_eq!(
            convert(&explicit_input, &mut explicit_out)
                .unwrap()
                .track()
                .beat_grid()
                .unwrap()
                .beats(),
            grid.beats()
        );
        let before = explicit_out;
        let mut different_grid = explicit.clone();
        let downbeat = different_grid
            .iter()
            .position(|b| b.ordinal == input.meter.unwrap().downbeat_ordinal)
            .unwrap();
        different_grid[downbeat].position.whole_frame += 1;
        explicit_grid.beats = &different_grid;
        explicit_input.global_grid = Some(explicit_grid);
        assert!(matches!(
            convert(&explicit_input, &mut explicit_out),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(explicit_out, before);
        explicit_grid.representation = libapta::GridRepresentation::Hybrid;
        explicit_input.global_grid = Some(explicit_grid);
        assert!(matches!(
            convert(&explicit_input, &mut explicit_out),
            Err(Error::UnsupportedGrid)
        ));

        // Split actual PCM timing; second anchor is context before applicability.
        let mut split = [segment; 2];
        split[0].applicability_range.end_frame = 160000;
        split[0].beat_count = 40;
        split[1].applicability_range.first_frame = 160000;
        split[1].beat_count = 40;
        split[1].segment_id += 1;
        let mut segmented = input.local_grid.unwrap();
        segmented.segments = &split;
        let mut split_input = input;
        split_input.global_grid = Some(segmented);
        let mut split_out = [EMPTY; 128];
        assert_eq!(
            convert(&split_input, &mut split_out)
                .unwrap()
                .track()
                .beat_grid()
                .unwrap()
                .beats(),
            grid.beats()
        );
        let before = split_out;
        let mut ordinal_gap = split;
        ordinal_gap[1].anchor_ordinal += 1;
        segmented.segments = &ordinal_gap;
        split_input.global_grid = Some(segmented);
        assert!(matches!(
            convert(&split_input, &mut split_out),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(split_out, before);
        let mut coverage_gap = split;
        coverage_gap[1].applicability_range.first_frame += 1;
        segmented.segments = &coverage_gap;
        split_input.global_grid = Some(segmented);
        assert!(matches!(
            convert(&split_input, &mut split_out),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(split_out, before);

        let mut partial = input;
        let mut partial_grid = input.local_grid.unwrap();
        partial_grid.state = libapta::FeatureState::Provisional;
        partial.local_grid = Some(partial_grid);
        assert!(
            convert(&partial, &mut short)
                .unwrap()
                .track()
                .beat_grid()
                .is_none()
        );
        let mut bad_segment = [input.local_grid.unwrap().segments[0]];
        bad_segment[0].anchor_position.whole_frame = 319999;
        let mut bad_grid = input.local_grid.unwrap();
        bad_grid.segments = &bad_segment;
        partial.local_grid = Some(bad_grid);
        let mut untouched = [EMPTY; 128];
        assert!(matches!(
            convert(&partial, &mut untouched),
            Err(Error::Coordinate)
        ));
        assert_eq!(untouched, [EMPTY; 128]);
        changed.meter = None;
        assert!(
            convert(&changed, &mut short)
                .unwrap()
                .track()
                .beat_grid()
                .is_none()
        );
        changed.tempo = None;
        changed.local_grid = None;
        assert_eq!(convert(&changed, &mut short).unwrap().track().bpm_x100(), 0);
        (track, position)
    };
    assert_eq!(track.beat_grid().unwrap().beats()[10].time_ms, position);
}

#[test]
fn unit_rounding_and_overflow() {
    use libapta::FractionalFrame as F;
    assert_eq!(
        milliseconds(
            F {
                whole_frame: 22050,
                fraction_q32: 0
            },
            44100
        ),
        Ok(500)
    );
    assert_eq!(
        milliseconds(
            F {
                whole_frame: 0,
                fraction_q32: 1 << 31
            },
            1000
        ),
        Ok(1)
    );
    assert_eq!(
        milliseconds(
            F {
                whole_frame: u64::MAX,
                fraction_q32: u32::MAX
            },
            1
        ),
        Err(Error::Coordinate)
    );
    assert_eq!(milliseconds(F::default(), 0), Err(Error::Invalid));
    assert_eq!(bpm_x100(120004), 12000);
    assert_eq!(bpm_x100(120005), 12001);
    assert_eq!(bpm_x100(u32::MAX), 429496730);
}

#[test]
fn pcm_backpressure_invalid_input_and_cancellation() {
    let mut queue = [NormalizedSample::default(); 2];
    let mut output = [WaveformColumn::default(); 1];
    let mut s = Session::new(
        SessionConfig {
            sample_rate: 44100,
            channel_count: 1,
            total_frames: 64,
            frames_per_column: 64,
        },
        &mut queue,
        &mut output,
    )
    .unwrap();
    assert_eq!(s.push_interleaved(&[0.25; 4]), Ok(2));
    assert_eq!(s.push_interleaved(&[0.5]), Ok(0));
    assert_eq!(s.accepted_frames(), 2);
    // Invalid accepted PCM must not advance input after the queue drains.
    let token = CancellationToken::new();
    s.process(
        WorkBudget {
            maximum_input_frames: 2,
            maximum_steps: 1,
        },
        &token,
    )
    .unwrap();
    assert_eq!(
        s.push_interleaved(&[f32::NAN]),
        Err(libapta::Error::InvalidArgument)
    );
    assert_eq!(s.accepted_frames(), 2);
    assert_eq!(s.push_interleaved(&[0.5]), Ok(1));
    token.cancel();
    assert_eq!(
        s.process(WorkBudget::default(), &token),
        Err(libapta::Error::Cancelled)
    );
    assert_eq!(s.processed_frames(), 2);
}

#[test]
fn waveform_capacity_and_signed_endpoints_are_transactional() {
    use libapta::{FeatureState, NativeOverview, WaveformSpan};
    let columns = [WaveformColumn {
        minimum: i16::MIN,
        maximum: i16::MAX,
        flags: 1,
        ..Default::default()
    }];
    let spans = [WaveformSpan {
        first_frame: 0,
        end_frame: 64,
        first_column_index: 0,
        column_count: 1,
        data_column_offset: 0,
    }];
    let overview = NativeOverview {
        frames_per_column: 64,
        origin_frame: 0,
        state: FeatureState::Final,
        confidence: 100,
        spans: &spans,
        columns: &columns,
    };
    assert!(matches!(
        pajoniiir_apta_adapter::waveform(overview, &mut [], 1),
        Err(Error::Capacity)
    ));
    let empty = pajoniiir_waveform::WaveformColumn {
        peak: 42,
        color_rgb565: 42,
    };
    let mut out = [empty];
    let mut partial = overview;
    partial.state = FeatureState::Partial;
    assert!(matches!(
        pajoniiir_apta_adapter::waveform(partial, &mut out, 1),
        Err(Error::UnsupportedGrid)
    ));
    assert_eq!(out, [empty]);
    assert_eq!(
        pajoniiir_apta_adapter::waveform(overview, &mut out, 1)
            .unwrap()
            .columns[0]
            .peak,
        u16::MAX
    );
}
