use ruzina_controller_core::{
    ControlEvent, ControlValue, DeckExtAction, DeckExtActionValue, DeckId, SemanticControl,
};
use ruzina_core::MediaTrackId;
use ruzina_deck::{DeckEffect, DeckProductState};
use ruzina_hot_cues::HotCueBank;
use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
use ruzina_rekordbox::anlz_analysis::{AnalysisFileKind, AnlzImporter};
use ruzina_track_analysis::owned::{AnalysisIdentity, AnalysisPins};
use ruzina_waveform::{Rgb565Surface, WaveformView};

const A: &[u8] = include_bytes!("../../ruzina-rekordbox/tests/fixtures/track-a.DAT");
const B: &[u8] = include_bytes!("../../ruzina-rekordbox/tests/fixtures/track-b.DAT");
const EXT: &[u8] = include_bytes!("../../ruzina-rekordbox/tests/fixtures/track-a.EXT");
fn identity(track: u8, generation: u32) -> AnalysisIdentity {
    let mut media = MediaSession::new();
    let ConnectResult::Accepted(lease) = media.on_connect(MediaSourceId::new(1).unwrap()) else {
        panic!()
    };
    AnalysisIdentity {
        track: MediaTrackId([track; 16]),
        lease,
        catalog_revision: 1,
        load_generation: generation,
    }
}
fn feed(importer: &mut AnlzImporter, kind: AnalysisFileKind, data: &[u8]) {
    importer.begin_file(kind, data.len() as u64).unwrap();
    while let Some(plan) = importer.read_plan() {
        importer
            .ingest(
                plan.offset,
                &data[plan.offset as usize..plan.offset as usize + plan.len],
            )
            .unwrap();
    }
}
fn pins() -> AnalysisPins {
    let mut pins = AnalysisPins::new();
    for (index, data) in [A, B].into_iter().enumerate() {
        let mut importer = AnlzImporter::new();
        feed(&mut importer, AnalysisFileKind::Dat, data);
        if index == 0 {
            feed(&mut importer, AnalysisFileKind::Ext, EXT);
        }
        let id = identity(index as u8 + 1, 1);
        assert!(pins.publish(index, id, importer.finish(id, 9999).unwrap()));
    }
    pins
}
fn press(deck: DeckId, control: SemanticControl) -> ControlEvent {
    ControlEvent {
        deck: Some(deck),
        control,
        value: ControlValue::Pressed(true),
    }
}

#[test]
fn imported_grids_drive_sync_phase_and_tempo_through_the_deck_consumer() {
    let pins = pins();
    let views = [pins.view(0, identity(1, 1)), pins.view(1, identity(2, 1))];
    let mut deck = DeckProductState::new();
    deck.set_position_ms(DeckId::One, 2600);
    deck.set_position_ms(DeckId::Two, 8900);
    let effects =
        deck.handle_control_with_analysis(press(DeckId::One, SemanticControl::Sync), views);
    assert_eq!(
        effects.items[0],
        Some(DeckEffect::SetPitchCentipercent {
            deck: DeckId::One,
            value: 667
        })
    );
    assert_eq!(
        effects.items[1],
        Some(DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 1962
        })
    );
}

#[test]
fn beat_jump_and_quantized_loop_use_imported_beats_instead_of_fallback_bpm() {
    let pins = pins();
    let views = [pins.view(0, identity(1, 1)), None];
    let mut deck = DeckProductState::new();
    deck.set_position_ms(DeckId::One, 1770);
    let jump = deck
        .handle_control_with_analysis(press(DeckId::One, SemanticControl::BeatJumpForward), views);
    assert_eq!(
        jump.items[0],
        Some(DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 2500
        })
    );
    deck.handle_control(ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::DeckExtAction,
        value: ControlValue::DeckExtAction(DeckExtActionValue {
            action: DeckExtAction::Quantize,
            pressed: true,
        }),
    });
    deck.set_position_ms(DeckId::One, 1770);
    deck.handle_control_with_analysis(press(DeckId::One, SemanticControl::LoopIn), views);
    deck.set_position_ms(DeckId::One, 2740);
    let out =
        deck.handle_control_with_analysis(press(DeckId::One, SemanticControl::LoopOut), views);
    assert_eq!(
        out.items[0],
        Some(DeckEffect::SetLoop {
            deck: DeckId::One,
            start_ms: 2000,
            end_ms: 2500
        })
    );
}

#[test]
fn reloading_and_detaching_drop_old_analysis_without_affecting_the_other_deck() {
    let mut pins = pins();
    pins.clear(0);
    let views = [pins.view(0, identity(1, 2)), pins.view(1, identity(2, 1))];
    assert!(views[0].is_none());
    assert!(views[1].is_some());
    let mut deck = DeckProductState::new();
    deck.set_position_ms(DeckId::One, 1770);
    let fallback = deck
        .handle_control_with_analysis(press(DeckId::One, SemanticControl::BeatJumpForward), views);
    assert_eq!(
        fallback.items[0],
        Some(DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 2270
        })
    );
    pins.clear(1);
    assert!(pins.view(1, identity(2, 1)).is_none());
}

#[test]
fn immutable_waveform_columns_feed_the_existing_rgb565_renderer() {
    let pins = pins();
    let analysis = pins.view(0, identity(1, 1)).unwrap();
    let mut pixels = [0u16; 32 * 24];
    let mut surface = Rgb565Surface::new(&mut pixels, 32, 24).unwrap();
    surface.draw_centered_waveform(
        0,
        0,
        32,
        24,
        WaveformView {
            columns: analysis.detail_window(1000, 32),
        },
    );
    assert!(pixels.iter().any(|pixel| *pixel != 0));
    assert_eq!(analysis.preview().len(), 400);
}

#[test]
fn imported_cues_preserve_local_hot_cue_overrides_and_tombstones() {
    let pins = pins();
    let analysis = pins.view(0, identity(1, 1)).unwrap();
    let id = identity(1, 1).track;
    let mut source = HotCueBank::empty(id);
    for cue in analysis.cues() {
        if let Some(end) = cue.end_ms {
            source.set_loop(cue.slot, cue.start_ms, end);
        } else {
            source.set_single(cue.slot, cue.start_ms);
        }
    }
    let mut local = HotCueBank::empty(id);
    local.set_single(0, 1777);
    local.clear(1);
    let mut deck = DeckProductState::new();
    assert!(
        deck.commit_track_load(DeckId::One, id, 12000, source.as_source(), Some(local))
            .is_ok()
    );
    let merged = deck.hot_cues(DeckId::One).unwrap();
    assert_eq!(merged.slot(0).unwrap().pos_ms, 1777);
    assert!(merged.slot(1).is_none());
}
