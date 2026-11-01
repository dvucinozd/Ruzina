use super::*;
use ruzina_controller_core::SemanticControl;
use std::string::String;
use std::sync::Mutex;
use std::vec;
use std::vec::Vec;

/// `(deck, control, kind, value, name)` as delivered to the C bridge.
type Delivered = (u32, u32, u32, i32, String);

static EVENTS: Mutex<Vec<Delivered>> = Mutex::new(Vec::new());
static UI: Mutex<Vec<(u32, u32, i32)>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_midi_ui(action: u32, deck: u32, value: i32) {
    UI.lock().unwrap().push((action, deck, value));
}

#[unsafe(no_mangle)]
extern "C" fn ruzina_bridge_midi_event(
    deck: u32,
    control: u32,
    kind: u32,
    value: i32,
    _max: u32,
    name: *const u8,
    name_len: u32,
) {
    // SAFETY: the test bridge reads the lent name within the call.
    let name = unsafe { core::slice::from_raw_parts(name, name_len as usize) };
    EVENTS.lock().unwrap().push((
        deck,
        control,
        kind,
        value,
        String::from_utf8(name.to_vec()).unwrap(),
    ));
}

fn ingest(bytes: &[u8]) -> i32 {
    // SAFETY: `bytes` outlives the call.
    unsafe { ruzina_legacy_midi_ingest(bytes.as_ptr(), bytes.len() as u32) }
}

fn take_out(capacity: usize) -> Vec<[u8; 4]> {
    let mut buffer = vec![0u8; capacity];
    // SAFETY: `buffer` has `capacity` writable bytes for the call.
    let len = unsafe { ruzina_legacy_midi_take_out(buffer.as_mut_ptr(), capacity as u32) } as usize;
    assert_eq!(len % 4, 0);
    buffer[..len]
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

// One test: the session is a single process-wide static, as on the device.
#[test]
fn flx4_play_presses_reach_the_semantic_session() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(ruzina_legacy_midi_bind(0x1234, 0x0045), 0);
    assert_eq!(ingest(&[0x09, 0x90, 0x0b, 0x7f]), -1);

    assert_eq!(ruzina_legacy_midi_bind(0x2b73, 0x0045), 1);
    EVENTS.lock().unwrap().clear();
    // USB-MIDI CIN 9 (note on): deck 1 play press, deck 2 play press, then
    // deck 1 play release, in one Bulk-IN payload.
    let delivered = ingest(&[
        0x09, 0x90, 0x0b, 0x7f, //
        0x09, 0x91, 0x0b, 0x7f, //
        0x09, 0x90, 0x0b, 0x00,
    ]);
    assert!(delivered >= 2);
    let events = EVENTS.lock().unwrap().clone();
    let play = SemanticControl::Play as u32;
    assert!(events.contains(&(1, play, kind::PRESSED, 1, "Play".into())));
    assert!(events.contains(&(2, play, kind::PRESSED, 1, "Play".into())));

    // The echo latched both decks' Play LEDs on (the deck 1 release does not
    // toggle): note-on 0x90/0x91 0x0B with a non-zero value, CIN 9, cable 0.
    // With the deck engine, Play on an empty deck is refused: LEDs stay off.
    let out = take_out(512);
    let engine = cfg!(feature = "deck-engine");
    let lit = |status: u8| {
        out.iter()
            .rev()
            .find(|p| p[1] == status && p[2] == 0x0b)
            .map(|p| (p[0], p[3]))
    };
    if engine {
        assert_eq!((lit(0x90), lit(0x91)), (None, None));
    } else {
        assert!(matches!(lit(0x90), Some((0x09, value)) if value != 0));
        assert!(matches!(lit(0x91), Some((0x09, value)) if value != 0));
    }
    assert!(take_out(512).is_empty());

    // A second deck 1 press turns its Play LED off; Cue follows the hold.
    ingest(&[
        0x09, 0x90, 0x0b, 0x7f, 0x09, 0x90, 0x0c, 0x7f, 0x09, 0x90, 0x0c, 0x00,
    ]);
    let out = take_out(512);
    let deck1 = |data1: u8| {
        out.iter()
            .filter(|p| p[1] == 0x90 && p[2] == data1)
            .map(|p| p[3])
            .collect::<Vec<_>>()
    };
    assert_eq!(deck1(0x0b).last(), if engine { None } else { Some(&0) });
    let cue = deck1(0x0c);
    assert_eq!(cue.len(), 2);
    assert!(cue[0] != 0 && cue[1] == 0);

    // The lamp test emits every profile output; a small buffer drains in parts.
    ruzina_legacy_midi_lamp(1);
    let first = take_out(16);
    assert_eq!(first.len(), 4);
    let rest = [take_out(512), take_out(512)].concat();
    // 129 FLX4 outputs: more than one 512-byte Bulk-OUT.
    assert_eq!(first.len() + rest.len(), 129);
    assert!(first.iter().chain(&rest).all(|p| p[0] == p[1] >> 4));
    assert_eq!(ruzina_legacy_midi_out_overflow(), 0);

    assert_eq!(ingest(&[0x09, 0x90, 0x0b]), -1);
    let mut totals = [0u32; 8];
    // SAFETY: eight writable slots.
    unsafe { ruzina_legacy_midi_totals(totals.as_mut_ptr()) };
    assert_eq!(totals[0], 6);
    assert_eq!(totals[1], 6);

    // Library UI: browse encoder +1 and -1 (CC 0xB6 0x40, two's complement),
    // browse press (0x96 0x41) and load deck 1 / deck 2 (0x96 0x46 / 0x47).
    UI.lock().unwrap().clear();
    ingest(&[
        0x0b, 0xb6, 0x40, 0x01, //
        0x0b, 0xb6, 0x40, 0x7f, //
        0x09, 0x96, 0x41, 0x7f, //
        0x09, 0x96, 0x41, 0x00, //
        0x09, 0x96, 0x46, 0x7f, //
        0x09, 0x96, 0x47, 0x7f,
    ]);
    let actions = UI.lock().unwrap().clone();
    assert_eq!(
        actions,
        [
            (ui::BROWSE, 0, 1),
            (ui::BROWSE, 0, -1),
            (ui::CHOOSE, 0, 0),
            (ui::LOAD, 1, 0),
            (ui::LOAD, 2, 0),
        ]
    );

    #[cfg(all(feature = "deck-play", not(feature = "deck-engine")))]
    {
        use crate::deck_play::COMMANDS;
        // Load paused deck 2 above; deck 1 plays, then Cue pauses it and
        // asks the producer to return to the start; deck 2's Cue asks its own.
        assert_eq!(playing(), [false, false]);
        COMMANDS[0].take();
        COMMANDS[1].take();
        ingest(&[0x09, 0x90, 0x0b, 0x7f, 0x09, 0x91, 0x0c, 0x7f]);
        assert_eq!(playing(), [true, false]);
        assert_eq!(COMMANDS[0].take(), (0, false));
        assert_eq!(COMMANDS[1].take(), (0, true));
        take_out(512);
        ingest(&[0x09, 0x90, 0x0c, 0x7f]);
        assert_eq!(playing(), [false, false]);
        assert_eq!(COMMANDS[0].take(), (0, true));
        assert!(take_out(512).iter().any(|p| p[1..] == [0x90, 0x0b, 0x00]));
        // Load on a playing deck pauses it too.
        ingest(&[0x09, 0x90, 0x0b, 0x7f, 0x09, 0x96, 0x46, 0x7f]);
        assert_eq!(playing(), [false, false]);
        // The Headphone Level knob reaches the gain.
        assert_eq!(headphone_level(), None);
        with(|state| {
            state.echo(&ControlEvent {
                deck: None,
                control: SemanticControl::HeadphoneLevel,
                value: ControlValue::Absolute {
                    value: 100,
                    max: 0x3fff,
                },
            })
        });
        assert_eq!(headphone_level(), Some((100, 0x3fff)));
    }

    ruzina_legacy_midi_unbind();
    assert_eq!(ingest(&[0x09, 0x90, 0x0b, 0x7f]), -1);
}
