use super::*;
use pajoniiir_controller_core::DeckId;

fn loaded() -> Session {
    let mut s = Session::new();
    assert!(s.load(DeckId::One));
    s
}
fn press(s: &mut Session, action: DeckAction) {
    s.control(action.event(DeckId::One, true));
}

#[test]
fn transport_uses_confirmed_state_and_load_lock_then_stops_at_end() {
    let mut s = loaded();
    press(&mut s, DeckAction::Play);
    s.tick(1750);
    assert_eq!(s.presentation(0).transport.position_ms, 1750);
    s.selected = 1;
    assert!(!s.load(DeckId::One));
    assert!(s.title(0).contains("120"));
    s.tick(20_000);
    assert!(!s.presentation(0).transport.playing);
    press(&mut s, DeckAction::Play);
    assert!(!s.presentation(0).transport.playing);
    press(&mut s, DeckAction::Cue);
    press(&mut s, DeckAction::Play);
    assert!(s.presentation(0).transport.playing);
}

#[test]
fn imported_grid_and_quantized_loop_drive_host_transport() {
    let mut s = loaded();
    s.decks.set_position_ms(DeckId::One, 1770);
    press(&mut s, DeckAction::JumpForward);
    assert_eq!(s.presentation(0).transport.position_ms, 2500);
    press(&mut s, DeckAction::Quantize);
    s.decks.set_position_ms(DeckId::One, 1770);
    press(&mut s, DeckAction::LoopIn);
    s.decks.set_position_ms(DeckId::One, 2740);
    press(&mut s, DeckAction::LoopOut);
    assert_eq!(s.presentation(0).loop_region, Some((2000, 2500)));
    press(&mut s, DeckAction::Play);
    s.tick(1300);
    assert_eq!(s.presentation(0).transport.position_ms, 2040);
}

#[test]
fn cue_tombstone_survives_reload_and_missing_analysis_clears_waveform() {
    let mut s = loaded();
    assert!(s.presentation(0).cues[0].is_some());
    s.control(cue_event(DeckId::One, 0, true, true).unwrap());
    assert!(s.load(DeckId::One));
    assert!(s.presentation(0).cues[0].is_none());
    for selection in [2, 3] {
        s.selected = selection;
        assert!(s.load(DeckId::One));
        assert!(s.analysis(0).is_none());
        assert!(s.presentation(0).cues.iter().all(Option::is_none));
        let pixels = compositor::compose(s.presentation(0), s.analysis(0), false);
        assert!(pixels.iter().all(|p| *p == 0x0842));
    }
}

#[test]
fn detach_clears_both_decks_and_reconnect_requires_explicit_load() {
    let mut s = loaded();
    s.load(DeckId::Two);
    press(&mut s, DeckAction::Play);
    s.detach();
    assert!(!s.load(DeckId::One));
    s.reconnect();
    for i in 0..2 {
        assert!(!s.presentation(i).transport.loaded);
        assert!(s.analysis(i).is_none());
    }
    assert!(s.load(DeckId::One));
    assert!(s.analysis(0).is_some());
}

#[test]
fn overview_cue_and_playhead_are_aligned_to_track_time() {
    let mut s = loaded();
    s.decks.set_position_ms(DeckId::One, 5000);
    let pixels = compositor::compose(s.presentation(0), s.analysis(0), false);
    let cue_x = 1000 * (compositor::WIDTH - 1) / 10_000;
    let head_x = 5000 * (compositor::WIDTH - 1) / 10_000;
    assert_eq!(pixels[cue_x], 0xfd20);
    assert_eq!(pixels[head_x], 0xffff);
    assert_eq!(
        pixels[(compositor::HEIGHT - 1) * compositor::WIDTH + head_x],
        0xffff
    );
}

#[test]
fn invalid_action_and_cue_codes_are_rejected() {
    assert!(DeckAction::from_code(-1).is_none());
    assert!(DeckAction::from_code(10).is_none());
    assert!(cue_event(DeckId::One, 8, false, true).is_none());
    assert_eq!(format_time(93_427), "01:33.427");
}

#[test]
fn mixer_controls_work_unloaded_and_survive_media_removal_as_configuration() {
    use pajoniiir_ui_model::{MixerParameter, MixerToggle, present_mixer};
    let mut s = Session::new();
    s.control(MixerParameter::Crossfader.event(None, 0).unwrap());
    s.control(
        MixerParameter::ChannelVolume
            .event(Some(DeckId::Two), 0)
            .unwrap(),
    );
    s.control(MixerToggle::Pfl.event(Some(DeckId::Two), true).unwrap());
    s.control(MixerParameter::HeadphoneMix.event(None, 0).unwrap());
    s.control(MixerParameter::HeadphoneLevel.event(None, 4096).unwrap());
    let before = present_mixer(&s.mixer);
    s.detach();
    assert_eq!(present_mixer(&s.mixer), before);
    assert!(before.channels[1].pfl_enabled);
    assert_eq!(before.channels[1].channel_volume, 0);
    assert_eq!(before.master_volume, 16383);
    assert!(!s.presentation(0).transport.loaded);
}

#[test]
fn fx_uses_global_state_and_pad_cancellation_cannot_stop_another_deck() {
    use pajoniiir_ui_model::{BeatFxAction, fx_depth_event, fx_target_event, pad_fx_event};
    let mut s = loaded();
    s.load(DeckId::Two);
    s.control(BeatFxAction::Next.event(true));
    s.control(fx_target_event(1).unwrap());
    s.control(fx_depth_event(96).unwrap());
    s.control(BeatFxAction::Toggle.event(true));
    assert!(s.decks.beat_fx().enabled);
    assert_eq!(s.decks.beat_fx().depth, 96);
    assert_eq!(
        s.decks.beat_fx().target,
        pajoniiir_controller_core::BeatFxTarget::ChannelTwo
    );
    s.control(pad_fx_event(DeckId::One, 0, 2, true).unwrap());
    s.control(pad_fx_event(DeckId::Two, 1, 4, true).unwrap());
    s.select_pad_bank(DeckId::One, 1);
    assert!(s.decks.deck(DeckId::One).active_pad_fx.is_none());
    assert!(s.decks.deck(DeckId::Two).active_pad_fx.is_some());
    s.cancel_pads();
    assert!(s.decks.deck(DeckId::Two).active_pad_fx.is_none());
    assert!(s.decks.beat_fx().enabled);
    s.control(pad_fx_event(DeckId::One, 0, 1, true).unwrap());
    s.detach();
    assert!(s.decks.deck(DeckId::One).active_pad_fx.is_none());
    assert_eq!(s.decks.beat_fx(), pajoniiir_deck::BeatFxState::new());
    s.control(pad_fx_event(DeckId::One, 0, 1, true).unwrap());
    assert!(s.decks.deck(DeckId::One).active_pad_fx.is_none());
}
