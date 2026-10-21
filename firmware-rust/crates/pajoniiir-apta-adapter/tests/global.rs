use libapta::{
    WaveformColumn,
    analysis::{BIN_CAPACITY, OnsetBin},
    session::{CancellationToken, Session, SessionConfig, SessionState, WorkBudget},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{Error, convert};
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
fn real_global_pcm_cache_and_consumers() {
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
    let mut global_bins = vec![OnsetBin::default(); libapta::global_analysis::BIN_CAPACITY];
    let mut global_flux = vec![0.; global_bins.len()];
    let mut global_beats = vec![libapta::Beat::default(); libapta::global_analysis::MAX_BEATS];
    session
        .enable_global_grid(false, &mut global_bins, &mut global_flux, &mut global_beats)
        .unwrap();
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
                *value = (64 - phase) as f32 / 64.0 * 0.75;
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

    // Preserve actual S6 rejection: this fixture's final global segment does
    // not span the source and does not bind to the local meter. No fallback.
    let global = input.global_grid.expect("real global processing");
    let mut rejected = [EMPTY; 128];
    assert!(matches!(
        convert(&input, &mut rejected),
        Err(Error::UnsupportedGrid)
    ));
    assert_eq!(rejected, [EMPTY; 128]);
    assert_ne!(global.segments[0].applicability_range.end_frame, 320000);

    // Exercise global wire transport with the actual PCM-derived local timing.
    // This is an explicit transport fixture, not acceptance of the S6 estimate.
    for mode in 0..3 {
        let mut input = input;
        let mut segments = vec![input.local_grid.unwrap().segments[0]];
        segments[0].revision = 1;
        let explicit: Vec<_> = (0..segments[0].beat_count)
            .map(|i| segments[0].beat_at(i).unwrap().unwrap())
            .collect();
        if mode == 1 {
            segments.push(segments[0]);
            segments[0].applicability_range.end_frame = 160000;
            segments[0].beat_count = 40;
            segments[1].applicability_range.first_frame = 160000;
            segments[1].beat_count = 40;
            segments[1].segment_id = 2;
        }
        let mut global = input.local_grid.unwrap();
        global.segments = &segments;
        if mode == 2 {
            global.representation = libapta::GridRepresentation::Explicit;
            global.segments = &[];
            global.beats = &explicit;
        }
        input.global_grid = Some(global);
        input.revision = Some(libapta::GridRevision {
            state: libapta::RevisionState::Applied,
            confidence: global.confidence,
            flags: 0,
            revision_id: 1,
            previous_revision_id: 0,
            proposed_representation: global.representation,
            proposed_segment_count: global.segments.len() as u32,
            proposed_beat_count: global.beats.len() as u32,
            affected_range: global.applicability_range,
        });
        let mut pending = input;
        pending.revision.as_mut().unwrap().state = libapta::RevisionState::Pending;
        assert!(matches!(
            convert(&pending, &mut rejected),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(rejected, [EMPTY; 128]);
        let mut output = [EMPTY; 128];
        let converted = convert(&input, &mut output).unwrap();
        let grid = converted.track().beat_grid().unwrap();
        assert_eq!(grid.beats().len(), 80);
        let mut deck = DeckProductState::new();
        deck.set_position_ms(DeckId::One, grid.beats()[10].time_ms);
        deck.handle_control_with_analysis(
            ControlEvent {
                deck: Some(DeckId::One),
                control: SemanticControl::BeatJumpForward,
                value: ControlValue::Pressed(true),
            },
            [Some(converted.track()), None],
        );
        assert!(deck.deck(DeckId::One).position_ms > grid.beats()[10].time_ms);
        assert_eq!(
            pajoniiir_track_analysis::beat_jump_target_ms(
                grid.beats()[10].time_ms,
                converted.track().bpm_x100(),
                4,
                1,
                Some(grid)
            ),
            grid.beats()[14].time_ms
        );
        let mut wire = libapta::result::from_session_snapshot(
            &snapshot,
            &mut [],
            libapta::NativeLimits::default(),
        )
        .unwrap();
        wire.global_grid = Some(libapta::GlobalGrid {
            state: global.state,
            confidence: global.confidence,
            flags: global.flags,
            representation: global.representation,
            requested_range: global.requested_range,
            evidence_range: global.evidence_range,
            applicability_range: global.applicability_range,
            coverage_range: global.coverage_ranges[0],
            segments: global.segments,
            beats: global.beats,
        });
        wire.revision = input.revision;
        let mut bytes = vec![0; libapta::result::serialized_size(&wire).unwrap()];
        libapta::result::write(&wire, &mut bytes, libapta::result::Limits::default()).unwrap();
        let mut segment_scratch =
            vec![input.local_grid.unwrap().segments[0]; global.segments.len()];
        let mut beat_scratch = vec![libapta::Beat::default(); global.beats.len()];
        let mut bank = [EMPTY; 128];
        let cached = pajoniiir_apta_adapter::convert_cache_with_scratch(
            &bytes,
            identity,
            8,
            libapta::result::Limits::default(),
            &mut segment_scratch,
            &mut beat_scratch,
            &mut bank,
        )
        .unwrap();
        assert_eq!(cached.track().beat_grid().unwrap().beats(), grid.beats());
        let mut short = [EMPTY; 128];
        assert!(matches!(
            pajoniiir_apta_adapter::convert_cache_with_scratch(
                &bytes,
                identity,
                8,
                libapta::result::Limits::default(),
                &mut [],
                &mut [],
                &mut short
            ),
            Err(Error::Capacity)
        ));
        assert_eq!(short, [EMPTY; 128]);
        drop(bytes);
        drop(segment_scratch);
        drop(beat_scratch);
        assert_eq!(cached.track().beat_grid().unwrap().beats(), grid.beats());
    }
}
