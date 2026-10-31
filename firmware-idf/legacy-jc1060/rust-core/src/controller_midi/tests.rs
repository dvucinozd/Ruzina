use super::*;
use ruzina_controller_core::SemanticControl;
use std::string::String;
use std::sync::Mutex;
use std::vec::Vec;

/// `(deck, control, kind, value, name)` as delivered to the C bridge.
type Delivered = (u32, u32, u32, i32, String);

static EVENTS: Mutex<Vec<Delivered>> = Mutex::new(Vec::new());

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

// One test: the session is a single process-wide static, as on the device.
#[test]
fn flx4_play_presses_reach_the_semantic_session() {
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

    assert_eq!(ingest(&[0x09, 0x90, 0x0b]), -1);
    let mut totals = [0u32; 8];
    // SAFETY: eight writable slots.
    unsafe { ruzina_legacy_midi_totals(totals.as_mut_ptr()) };
    assert_eq!(totals[0], 3);
    assert_eq!(totals[1], 3);

    ruzina_legacy_midi_unbind();
    assert_eq!(ingest(&[0x09, 0x90, 0x0b, 0x7f]), -1);
}
