use ruzina_controller_core::{
    BeatFxTarget, ControlEvent, ControlValue, DeckId, LedState, LedUpdate, SemanticControl,
    SemanticLedId,
};
use ruzina_controller_profile::{
    PROFILE_FLAG_JOG_TOUCH, PROFILE_FLAG_LED_FEEDBACK, PROFILE_FLAG_PITCH_14BIT,
    PROFILE_FLAG_USB_AUDIO, Profile, ProfileEvent, ProfileRuntime, adapt_profile_event,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicUsize, Ordering};

fn python_program() -> &'static str {
    if Command::new("python3").arg("--version").output().is_ok() {
        "python3"
    } else {
        "python"
    }
}

fn compile_fixture(name: &str) -> Vec<u8> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest.join("fixtures");
    let compiler = fixture_dir.join("compile_profile.py");
    let input = fixture_dir.join(format!("{name}.json"));
    // Tests run in parallel and may compile the same fixture concurrently.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let output: PathBuf =
        std::env::temp_dir().join(format!("ruzina-{name}-{}-{unique}.s3bin", process::id()));

    let status = Command::new(python_program())
        .arg(compiler)
        .arg(input)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("run released profile compiler");
    assert!(status.success(), "released profile compiler failed");

    let bytes = fs::read(&output).expect("read generated S3CP fixture");
    let _ = fs::remove_file(output);
    bytes
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    fs::read(path).expect("read frozen S3CP fixture")
}

#[test]
fn m25_compiler_preserves_frozen_v2_v3_v4_bytes() {
    // The .s3bin files were produced by the pre-M2.5 fixture compiler (v2)
    // or by the pinned M2.5 donor compiler (v3/v4), never by Rust code.
    for (json, golden, version) in [
        ("pioneer_ddj_flx4", "pioneer_ddj_flx4.v2-golden.s3bin", 2u16),
        (
            "hercules_djcontrol_inpulse_500",
            "hercules_djcontrol_inpulse_500.v2-golden.s3bin",
            2,
        ),
        ("s3cp_v3_scaled_cc", "s3cp_v3_scaled_cc.golden.s3bin", 3),
        ("s3cp_v4_extended", "s3cp_v4_extended.golden.s3bin", 4),
    ] {
        let compiled = compile_fixture(json);
        assert_eq!(compiled, fixture_bytes(golden), "{json}");
        assert_eq!(u16::from_le_bytes([compiled[4], compiled[5]]), version);
        assert_eq!(Profile::parse(&compiled).unwrap().version(), version);
    }
}

#[test]
fn m25_v3_scaled_cc_maps_detents_and_replays_mixer_only() {
    let bytes = fixture_bytes("s3cp_v3_scaled_cc.golden.s3bin");
    let profile = Profile::parse(&bytes).unwrap();
    assert!(profile.init_sysex().is_empty());
    assert!(!profile.channel_filter_always());

    let mut runtime = ProfileRuntime::new();
    for (data2, expected) in [(0u8, 0u16), (64, 8192), (127, 16383)] {
        let tempo = runtime.process(&profile, 0xb0, 0x00, data2).unwrap();
        assert_eq!(
            adapt_profile_event(tempo).unwrap(),
            ControlEvent {
                deck: Some(DeckId::One),
                control: SemanticControl::Tempo,
                value: ControlValue::Absolute {
                    value: expected,
                    max: 0x3fff,
                },
            }
        );
    }
    let volume = runtime.process(&profile, 0xb0, 0x13, 64).unwrap();
    assert_eq!(volume.value, 8192);

    let mut replayed = Vec::new();
    runtime.emit_snapshot(&profile, |event| {
        replayed.push(event);
        true
    });
    assert_eq!(
        replayed,
        [ProfileEvent {
            semantic_type: 0x03,
            semantic_id: 0x50,
            value: 8192,
        }]
    );
}

#[test]
fn m25_v4_extensions_select_initialize_scale_and_declare_filter_policy() {
    let bytes = fixture_bytes("s3cp_v4_extended.golden.s3bin");
    let profile = Profile::parse(&bytes).unwrap();
    assert_eq!(
        profile.init_sysex(),
        &[0xf0, 0x00, 0x20, 0x7f, 0x03, 0x01, 0xf7]
    );
    assert!(profile.channel_filter_always());

    let mut runtime = ProfileRuntime::new();
    for (data1, target) in [
        (0x10, BeatFxTarget::ChannelOne),
        (0x11, BeatFxTarget::ChannelTwo),
        (0x12, BeatFxTarget::Both),
    ] {
        let event = runtime.process(&profile, 0x94, data1, 0x7f).unwrap();
        assert_eq!(
            adapt_profile_event(event).unwrap().value,
            ControlValue::BeatFxTarget(target)
        );
        assert_eq!(runtime.process(&profile, 0x94, data1, 0x00), None);
    }

    let filter = runtime.process(&profile, 0xb1, 0x17, 127).unwrap();
    assert_eq!(
        adapt_profile_event(filter).unwrap(),
        ControlEvent {
            deck: Some(DeckId::Two),
            control: SemanticControl::Filter,
            value: ControlValue::Absolute {
                value: 16383,
                max: 0x3fff,
            },
        }
    );
    // Existing cc7_abs semantics are unchanged in a v4 profile.
    assert_eq!(
        runtime.process(&profile, 0xb4, 0x02, 100).unwrap().value,
        100
    );

    assert_eq!(
        profile.map_led_update(LedUpdate {
            led: SemanticLedId::PLAY,
            deck: Some(DeckId::One),
            state: LedState::On,
        }),
        Some([0x90, 0x0b, 64])
    );
    assert_eq!(profile.map_led(5, 0, 0x30), Some([0xb0, 0x02, 96]));
    assert_eq!(profile.map_led(5, 0, 0x7f), Some([0xb0, 0x02, 127]));
    assert_eq!(profile.map_led(0, 0, 1), Some([0x90, 0x0c, 0x7f]));
}

fn assert_common_capabilities(profile: &Profile<'_>) {
    let expected = PROFILE_FLAG_LED_FEEDBACK
        | PROFILE_FLAG_USB_AUDIO
        | PROFILE_FLAG_JOG_TOUCH
        | PROFILE_FLAG_PITCH_14BIT;
    assert_eq!(profile.flags(), expected);
    assert_eq!(profile.decks(), 2);
    assert!(profile.input_count() > 50);
    assert!(profile.output_count() > 10);
}

#[test]
fn released_flx4_profile_parses_and_matches_core_mappings() {
    let bytes = compile_fixture("pioneer_ddj_flx4");
    let profile = Profile::parse(&bytes).expect("parse released FLX4 profile");

    assert_eq!(profile.vid(), 0x2b73);
    assert_eq!(profile.pid(), 0x0045);
    assert_common_capabilities(&profile);

    let mut runtime = ProfileRuntime::new();
    let play = runtime
        .process(&profile, 0x90, 0x0b, 0x7f)
        .expect("FLX4 play event");
    assert_eq!(
        play,
        ProfileEvent {
            semantic_type: 0x01,
            semantic_id: 0x10,
            value: 1,
        }
    );
    assert_eq!(
        adapt_profile_event(play).unwrap(),
        ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        }
    );
    assert_eq!(profile.map_led(1, 0, 1), Some([0x90, 0x0b, 0x7f]));
    assert_eq!(
        profile.map_led_update(LedUpdate {
            led: SemanticLedId::PLAY,
            deck: Some(DeckId::One),
            state: LedState::On,
        }),
        Some([0x90, 0x0b, 0x7f])
    );
    assert_eq!(profile.map_led(5, 1, 0x55), Some([0xb1, 0x02, 0x55]));

    assert_eq!(runtime.process(&profile, 0xb6, 0x1f, 0x40), None);
    let crossfader = runtime
        .process(&profile, 0xb6, 0x3f, 0x20)
        .expect("FLX4 crossfader pair");
    assert_eq!(
        crossfader,
        ProfileEvent {
            semantic_type: 0x03,
            semantic_id: 0x52,
            value: 0x2020,
        }
    );
    assert_eq!(
        adapt_profile_event(crossfader).unwrap(),
        ControlEvent {
            deck: None,
            control: SemanticControl::Crossfader,
            value: ControlValue::Absolute {
                value: 0x2020,
                max: 0x3fff,
            },
        }
    );

    let mut replayed_crossfader = None;
    let replay_count = runtime.emit_snapshot(&profile, |event| {
        if event.semantic_id == 0x52 {
            replayed_crossfader = Some(event);
        }
        true
    });
    assert!(replay_count >= 1);
    assert_eq!(
        replayed_crossfader,
        Some(ProfileEvent {
            semantic_type: 0x03,
            semantic_id: 0x52,
            value: 0x2020,
        })
    );
}

#[test]
fn released_hercules_profile_parses_and_matches_core_mappings() {
    let bytes = compile_fixture("hercules_djcontrol_inpulse_500");
    let profile = Profile::parse(&bytes).expect("parse released Hercules profile");

    assert_eq!(profile.vid(), 0x06f8);
    assert_eq!(profile.pid(), 0xb12b);
    assert_common_capabilities(&profile);

    let mut runtime = ProfileRuntime::new();
    let play = runtime
        .process(&profile, 0x91, 0x07, 0x7f)
        .expect("Hercules play event");
    assert_eq!(
        adapt_profile_event(play).unwrap(),
        ControlEvent {
            deck: Some(DeckId::One),
            control: SemanticControl::Play,
            value: ControlValue::Pressed(true),
        }
    );
    assert_eq!(profile.map_led(1, 0, 1), Some([0x91, 0x07, 0x7f]));
    assert_eq!(
        profile.map_led_update(LedUpdate {
            led: SemanticLedId::PLAY,
            deck: Some(DeckId::One),
            state: LedState::On,
        }),
        Some([0x91, 0x07, 0x7f])
    );
    assert_eq!(profile.map_led(5, 1, 0x44), Some([0xb2, 0x40, 0x44]));

    assert_eq!(runtime.process(&profile, 0xb0, 0x00, 0x40), None);
    let crossfader = runtime
        .process(&profile, 0xb0, 0x20, 0x20)
        .expect("Hercules crossfader pair");
    assert_eq!(
        adapt_profile_event(crossfader).unwrap(),
        ControlEvent {
            deck: None,
            control: SemanticControl::Crossfader,
            value: ControlValue::Absolute {
                value: 0x2020,
                max: 0x3fff,
            },
        }
    );
}
