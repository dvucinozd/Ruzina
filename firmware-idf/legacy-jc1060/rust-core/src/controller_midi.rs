//! USB-MIDI ingestion from an IDF-hosted controller through the shared
//! semantic controller session. C owns the USB host, the claimed MIDI
//! interface and every transfer; it lends each completed Bulk-IN payload to
//! [`ruzina_legacy_midi_ingest`] for the duration of the call. Every function
//! here runs only on the C USB client task, which serializes access to the
//! single session.
use core::cell::UnsafeCell;
use core::fmt::Write;
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId};
use ruzina_controller_profile::Profile;
use ruzina_controller_session::{ControllerSession, SessionDelivery, UsbMidiBatchReport};

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
}));

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: single caller task (see `Shared`); no reference escapes `f`.
    f(unsafe { &mut *STATE.0.get() })
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

#[cfg(feature = "idf-controller-midi")]
unsafe extern "C" {
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
        drain(state);
        i32::from(state.profile.is_some())
    })
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
            delivered += 1;
        }
        state.session.complete_delivery(true);
    }
    delivered
}

#[cfg(test)]
mod tests;
