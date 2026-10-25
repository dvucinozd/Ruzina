use super::*;
use ruzina_controller_core::DeckId;

fn loaded() -> Session {
    let mut s = Session::new();
    assert!(s.load(DeckId::One));
    s
}
fn press(s: &mut Session, action: DeckAction) {
    s.control(action.event(DeckId::One, true));
}

// M2.5 HOLD RESTORE: only a 400 ms hold restores the Rekordbox cues; an early
// release or a lost press (page change) does nothing.
#[test]
fn hold_restore_brings_back_source_cues_only_after_the_long_press() {
    let mut s = loaded();
    let source = s.presentation(0).cues[0];
    assert!(source.is_some());
    s.control(cue_event(DeckId::One, 0, true, true).unwrap());
    assert!(s.presentation(0).cues[0].is_none(), "local delete");

    s.restore_hold(Some(0), true);
    s.tick(200);
    s.restore_hold(Some(0), false);
    s.tick(500);
    s.restore_hold(Some(0), true);
    s.restore_hold(None, false);
    s.tick(500);
    assert!(s.presentation(0).cues[0].is_none());

    s.restore_hold(Some(0), true);
    s.tick(399);
    assert!(s.presentation(0).cues[0].is_none());
    s.tick(1);
    assert_eq!(s.presentation(0).cues[0], source);
    assert_eq!(s.message, "Rekordbox cues restored for this session");
    s.restore_hold(Some(0), false);

    // Another deck's release does not end the hold; nothing to restore is a no-op.
    s.restore_hold(Some(0), true);
    s.restore_hold(Some(1), false);
    s.tick(400);
    assert_eq!(s.presentation(0).cues[0], source);
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
    press(&mut s, DeckAction::JumpBack);
    press(&mut s, DeckAction::Play);
    assert!(s.presentation(0).transport.playing);
}

fn cue(s: &mut Session, down: bool) {
    s.control(DeckAction::Cue.event(DeckId::One, down));
}

#[test]
fn touch_cue_sets_previews_while_held_and_play_commits_the_preview() {
    let mut s = loaded();
    s.decks.set_position_ms(DeckId::One, 2_000);
    cue(&mut s, true);
    cue(&mut s, false);
    assert_eq!(s.decks.deck(DeckId::One).cue_point_ms, 2_000);

    // Held at the cue: the preview plays and release returns to the cue.
    cue(&mut s, true);
    assert!(s.presentation(0).transport.playing);
    s.tick(700);
    assert_eq!(s.presentation(0).transport.position_ms, 2_700);
    cue(&mut s, false);
    assert!(!s.presentation(0).transport.playing);
    assert_eq!(s.presentation(0).transport.position_ms, 2_000);

    // PLAY during the preview commits playback; release no longer stops it.
    cue(&mut s, true);
    press(&mut s, DeckAction::Play);
    cue(&mut s, false);
    s.tick(500);
    assert!(s.presentation(0).transport.playing);
    assert_eq!(s.presentation(0).transport.position_ms, 2_500);

    // CUE while playing returns to the cue and pauses.
    cue(&mut s, true);
    cue(&mut s, false);
    assert!(!s.presentation(0).transport.playing);
    assert_eq!(s.presentation(0).transport.position_ms, 2_000);
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
fn deck_artwork_shows_the_decoded_cover_or_logo_and_never_an_old_cover() {
    use ruzina_media_catalog::artwork::{ArtworkFallback, ArtworkView};
    let mut s = loaded();
    let ArtworkView::Cover { pixels, .. } = s.artwork(0) else {
        panic!("track 01 carries a baseline cover");
    };
    assert!(pixels.iter().any(|pixel| *pixel != pixels[0]));
    assert_eq!(s.artwork(1), ArtworkView::Empty);

    s.selected = 3;
    assert!(s.load(DeckId::Two));
    assert_eq!(
        s.artwork(1),
        ArtworkView::Fallback(ArtworkFallback::NotBaseline)
    );
    // Replacing deck 1's track drops its cover before anything else shows.
    s.selected = 2;
    assert!(s.load(DeckId::One));
    assert_eq!(
        s.artwork(0),
        ArtworkView::Fallback(ArtworkFallback::NoArtwork)
    );
    s.detach();
    assert_eq!(s.artwork(0), ArtworkView::Empty);
    assert_eq!(s.artwork(1), ArtworkView::Empty);
}

#[test]
fn memory_cues_are_marked_apart_from_hot_cues_and_pwv4_colors_the_preview() {
    use ruzina_track_analysis::{AnalysisProvider, ColorPreviewEntry, TrackAnalysis};
    let s = loaded();
    // Track A's fixture carries one memory point at 750 ms (of 10 s).
    let pixels = compositor::compose(s.presentation(0), s.analysis(0), false);
    let memory_x = 750 * (compositor::WIDTH - 1) / 10_000;
    let bottom = (compositor::HEIGHT - 1) * compositor::WIDTH;
    assert_eq!(pixels[bottom + memory_x], 0xf800);
    assert_ne!(pixels[memory_x], 0xf800);
    let hot_x = 1000 * (compositor::WIDTH - 1) / 10_000;
    assert_ne!(pixels[bottom + hot_x], 0xf800);

    // Without PWV4 the PWAV preview stays; with it the bands set the color.
    let pure_bass = [ColorPreviewEntry([31, 0, 0, 200, 0, 0]); 4];
    let colored = TrackAnalysis::new(AnalysisProvider::RekordboxImport, 1, 12_000, None)
        .with_color_preview(&pure_bass, false);
    let pixels = compositor::compose(s.presentation(0), Some(colored), false);
    let center = (8 + (compositor::HEIGHT - 16) / 2) * compositor::WIDTH;
    assert_eq!(pixels[center + 300], 0xf800);
}

// M2.5 Library rows and deck labels: export columns, cover-or-logo row
// thumbnails, "--" for a deck without a key, CUE/LOOP pad names.
#[test]
fn library_columns_thumbnails_and_deck_key_follow_m25() {
    use ruzina_media_catalog::default_artwork::DEFAULT_ARTWORK_ROW;
    let mut s = loaded();
    assert_eq!(s.library_row_tracks(), [Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(s.track_columns(0), ("Grid Lab", "Am", 120, 10_000));
    assert_eq!(s.track_columns(1), ("Offset Unit", "Fm", 128, 14_000));
    // Track 1 has a baseline cover; 2 and 3 none; 4 a progressive file.
    assert_ne!(s.row_artwork(0), &DEFAULT_ARTWORK_ROW[..]);
    for track in [1, 2, 3] {
        assert_eq!(s.row_artwork(track), &DEFAULT_ARTWORK_ROW[..]);
    }
    assert_eq!(deck_key(s.key(0)), "Am");
    assert_eq!(deck_key(s.key(1)), "--", "empty deck");
    s.selected = 2;
    assert!(s.load(DeckId::Two));
    assert_eq!(deck_key(s.key(1)), "--", "track without a key");
    // Folder and playlist rows carry no track columns.
    s.library_back();
    assert!(s.library_row_tracks().iter().all(Option::is_none));
}

// M2.5 LOAD LOCK (v293): a playing deck's LOAD key reads "D1 LOCKED" on
// track rows, not on folder rows, and the switch stays on.
#[test]
fn load_lock_marks_the_playing_deck_on_track_rows_only() {
    let mut s = loaded();
    assert!(s.load_lock());
    assert!(!s.load_locked(0));
    press(&mut s, DeckAction::Play);
    assert!(s.load_locked(0));
    assert!(!s.load_locked(1));
    s.library_select(1);
    assert!(!s.library_load(DeckId::One), "the owner refuses");
    assert_eq!(s.title(0), session::TITLES[0]);
    s.library_back();
    assert!(!s.load_locked(0), "LOAD opens folders and playlists");
    s.library_back();
    press(&mut s, DeckAction::Play);
    assert!(!s.load_locked(0));
}

// Ruzina LOAD LOCK switch: committed to the RP-11 settings record before the
// decks use it, and read back after a settings power cycle.
#[test]
fn load_lock_switch_is_saved_and_survives_a_settings_reload() {
    let mut s = loaded();
    press(&mut s, DeckAction::Play);
    assert!(s.set_load_lock(false));
    assert!(!s.load_lock() && !s.load_locked(0));
    s.library_select(1);
    assert!(s.library_load(DeckId::One), "a playing deck is replaced");
    assert_eq!(s.title(0), session::TITLES[1]);

    s.reload_settings();
    assert!(!s.load_lock(), "stored off");
    assert!(s.set_load_lock(true));
    s.reload_settings();
    assert!(s.load_lock());
    press(&mut s, DeckAction::Play);
    assert!(s.load_locked(0));
}

// M2.5 keeps elapsed/remaining in its settings; so does the RP-11 record.
#[test]
fn time_display_is_saved_with_the_settings() {
    let mut s = Session::new();
    assert!(!s.time_remaining());
    assert!(s.set_time_remaining(true));
    s.reload_settings();
    assert!(s.time_remaining());
    assert!(s.set_load_lock(false));
    s.reload_settings();
    assert!(s.time_remaining() && !s.load_lock(), "both fields kept");
    assert!(s.set_time_remaining(false));
    s.reload_settings();
    assert!(!s.time_remaining());
}

// M2.5 MEMORY panel: export order, points and loops, truncation, empty deck.
#[test]
fn memory_panel_lists_imported_points_and_loops_like_m25() {
    use ruzina_track_analysis::{AnalysisMemoryCue, AnalysisProvider};
    let s = loaded();
    assert_eq!(
        memory_cue_text(0, s.analysis(0)),
        ("D1 MEMORY CUES: 1".into(), "01  00:00:00  CUE".into())
    );
    assert_eq!(
        memory_cue_text(1, s.analysis(1)),
        ("D2 MEMORY CUES: 0".into(), "No memory cues".into())
    );
    let cues = [
        AnalysisMemoryCue {
            start_ms: 95_000,
            end_ms: Some(103_500),
        },
        AnalysisMemoryCue {
            start_ms: 61_000,
            end_ms: None,
        },
    ];
    let analysis = TrackAnalysis::new(AnalysisProvider::RekordboxImport, 1, 12_000, None)
        .with_memory_cues(&cues, true);
    assert_eq!(
        memory_cue_text(1, Some(analysis)),
        (
            "D2 MEMORY CUES: 2 (TRUNCATED)".into(),
            "01  00:01:35  LOOP - 00:01:43\n02  00:01:01  CUE".into()
        )
    );
}

#[test]
fn imported_pqtz_bar_one_renders_as_the_tall_downbeat_tick() {
    // track-a.DAT stores PQTZ bar positions 1,2,3,4,1,... at 1000 + n*500 ms.
    // Cues cover 1000/2000 ms, so in this window only the 3000 ms bar-one beat
    // may reach the tall-tick row; the 1500..4500 ms neighbours stay short.
    let mut s = loaded();
    s.decks.set_position_ms(DeckId::One, 3200);
    let pixels = compositor::compose(s.presentation(0), s.analysis(0), true);
    let ticks = |row: usize| -> Vec<usize> {
        (0..compositor::WIDTH)
            .filter(|x| pixels[row * compositor::WIDTH + x] == 0x94b2)
            .collect()
    };
    let short = ticks(2);
    assert_eq!(short.len(), 6);
    assert_eq!(ticks(7), vec![short[2]]);
}

#[test]
fn invalid_action_and_cue_codes_are_rejected() {
    assert!(DeckAction::from_code(-1).is_none());
    assert!(DeckAction::from_code(10).is_none());
    assert!(cue_event(DeckId::One, 8, false, true).is_none());
    assert_eq!(format_time(93_427), "01:33.427");
}

#[test]
fn review_same_track_cue_edits_share_tombstones_without_disturbing_playback() {
    let mut s = loaded();
    assert!(s.load(DeckId::Two));
    s.control(DeckAction::Play.event(DeckId::Two, true));
    s.tick(1500);
    let position = s.presentation(1).transport.position_ms;
    s.control(cue_event(DeckId::One, 0, true, true).unwrap());
    assert!(s.presentation(1).cues[0].is_none());
    assert!(s.presentation(1).transport.playing);
    assert_eq!(s.presentation(1).transport.position_ms, position);
    s.control(cue_event(DeckId::Two, 2, false, true).unwrap());
    assert!(s.presentation(0).cues[2].is_some());
    assert!(s.presentation(0).cues[0].is_none());
    assert!(s.load(DeckId::One));
    assert!(s.presentation(0).cues[0].is_none());
    assert!(s.presentation(0).cues[2].is_some());
    s.control(DeckAction::Play.event(DeckId::Two, true));
    assert!(s.load(DeckId::Two));
    assert!(s.presentation(1).cues[0].is_none());
    assert!(s.presentation(1).cues[2].is_some());
    s.selected = 1;
    assert!(s.load(DeckId::Two));
    let other = s.presentation(1).cues;
    s.control(cue_event(DeckId::One, 3, false, true).unwrap());
    assert_eq!(s.presentation(1).cues, other);
}

#[test]
fn review_track_end_is_counted_only_when_entering_eof() {
    let mut s = Session::new();
    s.selected = 2; // BPM fallback permits seeks beyond the known duration.
    assert!(s.load(DeckId::One));
    press(&mut s, DeckAction::Play);
    s.tick(20_000);
    for _ in 0..40 {
        press(&mut s, DeckAction::JumpForward);
    }
    assert_eq!(s.diagnostics().counters.end_of_track, 1);
    assert_eq!(s.diagnostics().events.len(), 5); // boot, connect, load attempt, loaded, one end
    press(&mut s, DeckAction::JumpBack);
    assert!(s.presentation(0).transport.position_ms < s.presentation(0).transport.duration_ms);
    press(&mut s, DeckAction::JumpForward);
    assert_eq!(s.diagnostics().counters.end_of_track, 2);
    press(&mut s, DeckAction::Play);
    assert_eq!(s.diagnostics().counters.end_of_track, 2);
    press(&mut s, DeckAction::JumpBack);
    press(&mut s, DeckAction::Play);
    s.tick(20_000);
    assert_eq!(s.diagnostics().counters.end_of_track, 3);
}

#[test]
fn mixer_controls_work_unloaded_and_survive_media_removal_as_configuration() {
    use ruzina_ui_model::{MixerParameter, MixerToggle, present_mixer};
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
    use ruzina_ui_model::{BeatFxAction, fx_depth_event, fx_target_event, pad_fx_event};
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
        ruzina_controller_core::BeatFxTarget::ChannelTwo
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
    assert_eq!(s.decks.beat_fx(), ruzina_deck::BeatFxState::new());
    s.control(pad_fx_event(DeckId::One, 0, 1, true).unwrap());
    assert!(s.decks.deck(DeckId::One).active_pad_fx.is_none());
}

#[test]
fn load_lock_refusals_preserve_the_loaded_track_and_report_the_reason() {
    use ruzina_controller_core::{ControlEvent, ControlValue, SemanticControl};
    let jog = |down| ControlEvent {
        deck: Some(DeckId::One),
        control: SemanticControl::JogTouch,
        value: ControlValue::Pressed(down),
    };
    let mut s = loaded();
    let title = s.title(0).to_string();
    s.selected = 1;

    s.control(jog(true));
    assert!(!s.load(DeckId::One));
    assert!(s.message.contains("jog"), "{}", s.message);
    assert_eq!(s.title(0), title);
    assert!(s.analysis(0).is_some());

    s.control(jog(false));
    press(&mut s, DeckAction::Play);
    assert!(!s.load(DeckId::One));
    assert!(s.message.contains("pause"), "{}", s.message);
    assert_eq!(s.title(0), title);
    assert_eq!(s.diagnostics().counters.loads_blocked, 2);

    // The other deck is independent, and pausing reopens the destination.
    assert!(s.load(DeckId::Two));
    press(&mut s, DeckAction::Play);
    assert!(s.load(DeckId::One));
    assert_ne!(s.title(0), title);
}

#[test]
fn service_counts_real_load_outcomes_and_recovers_without_erasing_history() {
    use ruzina_service::{AnalysisStatus, Health};
    let mut s = loaded();
    press(&mut s, DeckAction::Play);
    assert!(!s.load(DeckId::One));
    assert_eq!(s.diagnostics().counters.loads_blocked, 1);
    assert_eq!(s.diagnostics().runtime.analysis[0], AnalysisStatus::Ready);
    press(&mut s, DeckAction::Play);
    s.selected = 3;
    assert!(s.load(DeckId::One));
    assert_eq!(s.diagnostics().health(), Health::AnalysisFallback);
    assert_eq!(s.diagnostics().counters.analysis_rejected, 1);
    s.detach();
    assert_eq!(s.diagnostics().health(), Health::MediaUnavailable);
    assert!(!s.load(DeckId::One));
    s.reconnect();
    s.reconnect();
    let d = s.diagnostics();
    assert_eq!(d.health(), Health::DemoMediaReady);
    assert_eq!(d.counters.media_connects, 2);
    assert_eq!(d.counters.analysis_rejected, 1);
    assert_eq!(d.counters.load_attempts, 4);
    assert!(d.events.len() > 5);
    assert_eq!(d.runtime.analysis, [AnalysisStatus::Unloaded; 2]);
    assert!(d.identity.chip_revision.is_none());
}

#[test]
fn service_clock_advances_while_paused_but_eof_is_recorded_once() {
    let mut s = loaded();
    s.tick(100);
    assert_eq!(s.diagnostics().elapsed_ms, 100);
    assert_eq!(s.presentation(0).transport.position_ms, 0);
    press(&mut s, DeckAction::Play);
    s.tick(20_000);
    s.tick(20_000);
    assert_eq!(s.diagnostics().counters.end_of_track, 1);
    press(&mut s, DeckAction::Play);
    assert_eq!(s.diagnostics().counters.playback_rejected, 1);
    assert_eq!(s.diagnostics().elapsed_ms, 40_100);
}

#[test]
fn runtime_heartbeat_is_independent_of_playback_and_refresh_reads() {
    use ruzina_service::{
        Health,
        publication::{Activity, Freshness},
    };
    let mut s = loaded();
    let initial = s.diagnostics().observation.unwrap();
    assert_eq!(initial.stamp.unwrap().activity, Activity::Idle);
    s.observe_elapsed(session::RUNTIME_MAX_AGE_MS as u32);
    assert_eq!(
        s.diagnostics().observation.unwrap().freshness,
        Freshness::Fresh
    );
    s.observe_elapsed(1);
    assert_eq!(s.diagnostics().health(), Health::RuntimeStale);
    assert_eq!(s.diagnostics().observation.unwrap().stamp, initial.stamp);
    // Repeated snapshot/UI reads cannot renew the publication.
    assert_eq!(s.diagnostics().health(), Health::RuntimeStale);
    s.tick(0);
    assert_eq!(s.diagnostics().health(), Health::DemoMediaReady);
    assert_eq!(
        s.diagnostics().observation.unwrap().stamp.unwrap().activity,
        Activity::Idle
    );
    press(&mut s, DeckAction::Play);
    assert_eq!(
        s.diagnostics().observation.unwrap().stamp.unwrap().activity,
        Activity::Active
    );
    s.detach();
    let removed = s.diagnostics().observation.unwrap();
    assert!(removed.generation > initial.generation);
    assert_eq!(removed.stamp.unwrap().activity, Activity::Idle);
    assert_eq!(s.diagnostics().health(), Health::MediaUnavailable);
    s.reconnect();
    let connected = s.diagnostics().observation.unwrap();
    assert!(connected.generation > removed.generation);
    s.reconnect();
    assert_eq!(
        s.diagnostics().observation.unwrap().generation,
        connected.generation
    );
    assert_eq!(s.diagnostics().counters.media_removals, 1);
}

#[test]
fn playlist_levels_never_survive_a_media_reconnect() {
    let mut s = Session::new();
    assert!(s.library_back());
    s.library_select(0);
    assert!(!s.library_load(DeckId::One), "a folder opens");
    s.library_select(1);
    assert!(!s.library_load(DeckId::One), "a playlist opens");
    assert_eq!(s.library_path(), "PLAYLISTS / Sets / Peak time");
    s.detach();
    assert_eq!(s.library_rows().len(), session::TITLES.len());
    s.reconnect();
    assert_eq!(
        s.library_path(),
        "DEMO LIBRARY",
        "a new publication starts at the track list"
    );
    s.library_select(2);
    assert!(s.library_load(DeckId::Two));
    assert_eq!(s.title(1), session::TITLES[2]);
}

#[test]
fn jog_nudges_a_playing_deck_and_scrubs_a_paused_one() {
    use ruzina_controller_core::{ControlEvent, ControlValue, SemanticControl};
    let jog = |control, ticks| ControlEvent {
        deck: Some(DeckId::One),
        control,
        value: ControlValue::Relative(ticks),
    };
    let mut plain = loaded();
    let mut nudged = loaded();
    for s in [&mut plain, &mut nudged] {
        press(s, DeckAction::Play);
    }
    nudged.control(jog(SemanticControl::JogBend, 15));
    for _ in 0..40 {
        plain.tick(5);
        nudged.tick(5);
    }
    let ahead =
        nudged.presentation(0).transport.position_ms - plain.presentation(0).transport.position_ms;
    assert!(
        (5..=30).contains(&ahead),
        "the bend pushed the deck {ahead} ms ahead, then sprang back"
    );
    plain.tick(100);
    nudged.tick(100);
    let later =
        nudged.presentation(0).transport.position_ms - plain.presentation(0).transport.position_ms;
    assert!(later.abs_diff(ahead) <= 1, "no lasting tempo change");
    assert_eq!(
        nudged.presentation(0).transport.pitch_centi_percent,
        0,
        "the fader is unchanged"
    );

    press(&mut nudged, DeckAction::Play);
    let paused = nudged.presentation(0).transport.position_ms;
    nudged.control(jog(SemanticControl::JogScratch, 10));
    assert_eq!(
        nudged.presentation(0).transport.position_ms,
        paused + 30,
        "3 ms per tick"
    );
    nudged.control(jog(SemanticControl::JogScratch, -20));
    assert_eq!(nudged.presentation(0).transport.position_ms, paused - 30);
}
