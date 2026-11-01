//! USB-MIDI ingestion from an IDF-hosted controller through the shared
//! semantic controller session. C owns the USB host, the claimed MIDI
//! interface and every transfer; it lends each completed Bulk-IN payload to
//! [`ruzina_legacy_midi_ingest`] for the duration of the call. Every function
//! here runs only on the C USB client task, which serializes access to the
//! single session.
//!
//! Output is a diagnostic echo, not product LED state: a lamp test lights or
//! clears every profile output, Play toggles its deck's Play LED and the Play
//! LED follows that latch, and the Cue LED follows the held Cue button. With
//! `deck-play`, Cue and Load also clear deck 1's latch (Cue returns its track
//! to the start) and the Headphone Level knob sets the headphone gain. LED
//! updates go through the profile's output map into USB-MIDI event packets,
//! which C takes with [`ruzina_legacy_midi_take_out`] for one Bulk-OUT.
use core::cell::UnsafeCell;
use core::fmt::Write;
use ruzina_controller_core::{
    ControlEvent, ControlValue, DeckId, LedState, LedUpdate, SemanticControl, SemanticLedId,
};
use ruzina_controller_profile::{OutputKind, Profile};
use ruzina_controller_session::{ControllerSession, SessionDelivery, UsbMidiBatchReport};

/// USB-MIDI packets queued for Bulk-OUT; the FLX4 lamp test alone exceeds
/// one 512-byte transfer, so C drains the queue across several.
const OUT_PACKETS: usize = 256;

/// The owner's controller: the M2.5 golden DDJ-FLX4 profile (S3CP v2).
const FLX4_PROFILE: &[u8] = include_bytes!(
    "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/pioneer_ddj_flx4.v2-golden.s3bin"
);

/// Event value kinds passed to C (see [`encode`]).
pub(crate) mod kind {
    pub const PRESSED: u32 = 1;
    pub const RELATIVE: u32 = 2;
    pub const ABSOLUTE: u32 = 3;
    pub const PAD: u32 = 4;
    pub const DECK_EXT: u32 = 5;
    pub const BEAT_FX_TARGET: u32 = 6;
}

struct State {
    session: ControllerSession,
    profile: Option<Profile<'static>>,
    totals: UsbMidiBatchReport,
    /// Play latch per deck for the echo.
    playing: [bool; 2],
    /// Last Headphone Level position and its range; `None` until it moves.
    headphone: Option<(u16, u16)>,
    out: [[u8; 4]; OUT_PACKETS],
    out_len: usize,
    /// Packets that did not fit the queue.
    out_overflow: u32,
}

impl State {
    fn push_out(&mut self, message: [u8; 3]) {
        if self.out_len == OUT_PACKETS {
            self.out_overflow = self.out_overflow.saturating_add(1);
            return;
        }
        // Cable 0; the code index number is the channel-voice status nibble.
        self.out[self.out_len] = [message[0] >> 4, message[0], message[1], message[2]];
        self.out_len += 1;
    }

    fn led(&mut self, led: SemanticLedId, deck: Option<DeckId>, on: bool) {
        let state = if on { LedState::On } else { LedState::Off };
        if let Some(message) = self
            .profile
            .and_then(|p| p.map_led_update(LedUpdate { led, deck, state }))
        {
            self.push_out(message);
        }
    }

    /// Lights or clears every output in the profile.
    fn lamp(&mut self, on: bool) {
        let Some(profile) = self.profile else { return };
        for index in 0..profile.output_count() {
            let Some(entry) = profile.output(index) else {
                continue;
            };
            let value = match (entry.kind, on) {
                (OutputKind::NoteOnOff, true) => entry.on_value,
                (OutputKind::NoteOnOff, false) => entry.off_value,
                (OutputKind::CcValue, true) => 0x7f,
                (OutputKind::CcValue, false) => 0,
            };
            self.push_out([entry.status, entry.data1, value]);
        }
    }

    fn echo(&mut self, event: &ControlEvent) {
        if let (SemanticControl::HeadphoneLevel, ControlValue::Absolute { value, max }) =
            (event.control, event.value)
        {
            self.headphone = Some((value, max));
        }
        let ControlValue::Pressed(pressed) = event.value else {
            return;
        };
        let Some(deck) = event.deck else { return };
        let slot = usize::from(deck == DeckId::Two);
        match event.control {
            SemanticControl::Play if pressed => {
                self.playing[slot] = !self.playing[slot];
                self.led(SemanticLedId::PLAY, Some(deck), self.playing[slot]);
            }
            SemanticControl::Cue => {
                #[cfg(feature = "deck-play")]
                if pressed {
                    self.stop(deck, slot);
                    if slot == 0 {
                        crate::deck_play::COMMANDS.cue();
                    }
                }
                self.led(SemanticLedId::CUE, Some(deck), pressed);
            }
            #[cfg(feature = "deck-play")]
            SemanticControl::Load if pressed => self.stop(deck, slot),
            _ => {}
        }
    }
}

#[cfg(feature = "deck-play")]
impl State {
    /// Pauses a deck: its Play latch and LED go off.
    fn stop(&mut self, deck: DeckId, slot: usize) {
        if self.playing[slot] {
            self.playing[slot] = false;
            self.led(SemanticLedId::PLAY, Some(deck), false);
        }
    }
}

struct Shared(UnsafeCell<State>);
// SAFETY: only the C USB client task calls the exports below (module contract),
// so the cell is never accessed concurrently.
unsafe impl Sync for Shared {}
// Built at compile time: the session (~4 KiB) must never be constructed on the
// 8 KiB USB client task stack, which a lazy initializer did on the device.
static STATE: Shared = Shared(UnsafeCell::new(State {
    session: ControllerSession::new(),
    profile: None,
    totals: UsbMidiBatchReport {
        packets: 0,
        decoded: 0,
        scheduled: 0,
        dropped: 0,
        no_mapping: 0,
        ignored_non_channel_voice: 0,
        unsupported_cable: 0,
        semantic_errors: 0,
    },
    playing: [false; 2],
    headphone: None,
    out: [[0; 4]; OUT_PACKETS],
    out_len: 0,
    out_overflow: 0,
}));

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: single caller task (see `Shared`); no reference escapes `f`.
    f(unsafe { &mut *STATE.0.get() })
}

/// The Play latch of decks 1 and 2 (the echo state). Same task as audio.
#[cfg_attr(not(feature = "controller-audio"), allow(dead_code))]
pub(crate) fn playing() -> [bool; 2] {
    with(|state| state.playing)
}

#[cfg(feature = "deck-play")]
pub(crate) fn headphone_level() -> Option<(u16, u16)> {
    with(|state| state.headphone)
}

/// `(deck, kind, value, max)` scalars for one semantic event.
pub(crate) fn encode(event: &ControlEvent) -> (u32, u32, i32, u32) {
    let deck = match event.deck {
        None => 0,
        Some(DeckId::One) => 1,
        Some(DeckId::Two) => 2,
    };
    let (kind, value, max) = match event.value {
        ControlValue::Pressed(pressed) => (kind::PRESSED, i32::from(pressed), 1),
        ControlValue::Relative(delta) => (kind::RELATIVE, i32::from(delta), 0),
        ControlValue::Absolute { value, max } => (kind::ABSOLUTE, i32::from(value), u32::from(max)),
        ControlValue::PadAction(pad) => (
            kind::PAD,
            i32::from(pad.pad)
                | (pad.mode as i32) << 8
                | i32::from(pad.shifted) << 16
                | i32::from(pad.pressed) << 17,
            0,
        ),
        ControlValue::DeckExtAction(ext) => (
            kind::DECK_EXT,
            ext.action as i32 | i32::from(ext.pressed) << 8,
            0,
        ),
        ControlValue::BeatFxTarget(target) => (kind::BEAT_FX_TARGET, target as i32, 0),
    };
    (deck, kind, value, max)
}

/// A control's name, written without allocation into a fixed buffer.
struct Name {
    bytes: [u8; 32],
    len: usize,
}

impl Write for Name {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Library UI actions passed to C (see [`ui_action`]).
pub(crate) mod ui {
    /// Move the selection by `value` rows.
    pub const BROWSE: u32 = 1;
    /// Browse encoder press: choose the selected track.
    pub const CHOOSE: u32 = 2;
    /// Load the selected track on deck `deck`.
    pub const LOAD: u32 = 3;
}

/// The library action a semantic event asks for: `(action, deck, value)`.
pub(crate) fn ui_action(event: &ControlEvent) -> Option<(u32, u32, i32)> {
    let deck = match event.deck {
        None => 0,
        Some(DeckId::One) => 1,
        Some(DeckId::Two) => 2,
    };
    match (event.control, event.value) {
        (SemanticControl::BrowseDelta, ControlValue::Relative(delta)) if delta != 0 => {
            Some((ui::BROWSE, 0, i32::from(delta)))
        }
        (SemanticControl::BrowsePress, ControlValue::Pressed(true)) => Some((ui::CHOOSE, 0, 0)),
        (SemanticControl::Load, ControlValue::Pressed(true)) if deck != 0 => {
            Some((ui::LOAD, deck, 0))
        }
        _ => None,
    }
}

#[cfg(feature = "idf-controller-midi")]
unsafe extern "C" {
    /// One library UI action (browse, choose, load); scalars only.
    fn ruzina_bridge_midi_ui(action: u32, deck: u32, value: i32);
    /// One semantic event; `name` is lent for the call only.
    fn ruzina_bridge_midi_event(
        deck: u32,
        control: u32,
        kind: u32,
        value: i32,
        max: u32,
        name: *const u8,
        name_len: u32,
    );
}

/// Bind the session to a newly attached controller. Returns 1 when a built-in
/// profile matches `vid:pid`, 0 when it does not (events then stay unmapped).
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_bind(vid: u16, pid: u16) -> i32 {
    with(|state| {
        state.profile = Profile::parse(FLX4_PROFILE)
            .ok()
            .filter(|profile| profile.vid() == vid && profile.pid() == pid);
        state.session.reset_profile_state();
        state.session.set_connected(true);
        state.totals = UsbMidiBatchReport::default();
        state.playing = [false; 2];
        state.headphone = None;
        state.out_len = 0;
        state.out_overflow = 0;
        drain(state);
        i32::from(state.profile.is_some())
    })
}

/// Queue a lamp test: every profile output lit (`on` != 0) or cleared.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_lamp(on: i32) {
    with(|state| state.lamp(on != 0));
}

/// Move up to `capacity` bytes of queued USB-MIDI packets into `out` for one
/// Bulk-OUT; packets that do not fit stay queued. Returns the bytes written.
///
/// # Safety
/// `out` must be valid for `capacity` writes for the duration of the call.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_midi_take_out(out: *mut u8, capacity: u32) -> u32 {
    if out.is_null() {
        return 0;
    }
    with(|state| {
        let count = state.out_len.min(capacity as usize / 4);
        // SAFETY: `count * 4 <= capacity` writable bytes; the queue is a
        // contiguous array of 4-byte packets.
        unsafe { core::ptr::copy_nonoverlapping(state.out.as_ptr().cast::<u8>(), out, count * 4) };
        state.out.copy_within(count..state.out_len, 0);
        state.out_len -= count;
        (count * 4) as u32
    })
}

/// Packets dropped because the output queue was full.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_out_overflow() -> u32 {
    with(|state| state.out_overflow)
}

/// End the controller connection; pending events are retired.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_unbind() {
    with(|state| {
        state.session.set_connected(false);
        drain(state);
        state.profile = None;
    });
}

/// Ingest one Bulk-IN payload of USB-MIDI event packets and deliver every
/// resulting semantic event. Returns the number of events delivered, -1 when
/// unbound or the length is not a multiple of four.
///
/// # Safety
/// `bytes` must be valid for `len` reads for the duration of the call.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_midi_ingest(bytes: *const u8, len: u32) -> i32 {
    if bytes.is_null() {
        return -1;
    }
    // SAFETY: the caller lends `len` readable bytes for this call.
    let payload = unsafe { core::slice::from_raw_parts(bytes, len as usize) };
    with(|state| {
        let Some(profile) = state.profile else {
            return -1;
        };
        match state.session.process_usb_midi_transfer(&profile, payload) {
            Ok(report) => state.totals.accumulate(report),
            Err(_) => return -1,
        }
        drain(state)
    })
}

/// Copies the running batch totals into `out[0..8]`: packets, decoded,
/// scheduled, dropped, unmapped, non-channel-voice, other cable, errors.
///
/// # Safety
/// `out` must be valid for eight `u32` writes.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_midi_totals(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let t = with(|state| state.totals);
    let values = [
        t.packets,
        t.decoded,
        t.scheduled,
        t.dropped,
        t.no_mapping,
        t.ignored_non_channel_voice,
        t.unsupported_cable,
        t.semantic_errors,
    ];
    // SAFETY: the caller provides eight writable, aligned u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

/// Deliver every pending session item; control events go to C.
#[cfg(feature = "idf-controller-midi")]
fn drain(state: &mut State) -> i32 {
    let mut delivered = 0;
    while let Some(delivery) = state.session.next_delivery(state.profile.as_ref()) {
        if let SessionDelivery::Control(event) = delivery {
            let (deck, kind, value, max) = encode(&event);
            let mut name = Name {
                bytes: [0; 32],
                len: 0,
            };
            let _ = write!(name, "{:?}", event.control);
            // SAFETY: scalars plus `name`, lent for the call only.
            unsafe {
                ruzina_bridge_midi_event(
                    deck,
                    event.control as u32,
                    kind,
                    value,
                    max,
                    name.bytes.as_ptr(),
                    name.len as u32,
                )
            };
            state.echo(&event);
            if let Some((action, deck, value)) = ui_action(&event) {
                // SAFETY: scalars only.
                unsafe { ruzina_bridge_midi_ui(action, deck, value) };
            }
            delivered += 1;
        }
        state.session.complete_delivery(true);
    }
    delivered
}

#[cfg(test)]
mod tests;
