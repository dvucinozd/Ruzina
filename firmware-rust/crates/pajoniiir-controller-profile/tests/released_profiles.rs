use pajoniiir_controller_core::{
    ControlEvent, ControlValue, DeckId, LedState, LedUpdate, SemanticControl, SemanticLedId,
};
use pajoniiir_controller_profile::{
    PROFILE_FLAG_JOG_TOUCH, PROFILE_FLAG_LED_FEEDBACK, PROFILE_FLAG_PITCH_14BIT,
    PROFILE_FLAG_USB_AUDIO, Profile, ProfileEvent, ProfileRuntime, adapt_profile_event,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

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
    let output: PathBuf =
        std::env::temp_dir().join(format!("pajoniiir-{name}-{}.s3bin", process::id()));

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
