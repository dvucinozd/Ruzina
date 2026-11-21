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
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
#[cfg(feature = "deck-engine")]
use ruzina_controller_core::ControllerLedFrame;
use ruzina_controller_core::{
    ControlEvent, ControlValue, DeckId, LedState, LedUpdate, SemanticControl, SemanticLedId,
};
use ruzina_controller_profile::{OutputKind, Profile};
use ruzina_controller_session::{ControllerSession, SessionDelivery, UsbMidiBatchReport};

/// USB-MIDI packets queued for Bulk-OUT; the FLX4 lamp test alone exceeds
/// one 512-byte transfer, so C drains the queue across several.
const OUT_PACKETS: usize = 256;

/// The owner's controller: the M2.5 golden DDJ-FLX4 profile (S3CP v2).
pub(crate) const FLX4_PROFILE: &[u8] = include_bytes!(
    "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/pioneer_ddj_flx4.v2-golden.s3bin"
);

/// A controller the deck knows: its name, for the Settings page, and its profile, which
/// names its USB VID:PID.
pub(crate) struct Controller {
    pub(crate) name: &'static str,
    profile: &'static [u8],
}

/// Every controller the deck knows: the FLX4, then the donor's other profiles
/// (`ruzina-controller-profile/fixtures/README.md` says where each comes from). Only the
/// FLX4 is tested on this hardware.
pub(crate) const CONTROLLERS: [Controller; 5] = [
    Controller {
        name: "Pioneer DDJ-FLX4",
        profile: FLX4_PROFILE,
    },
    Controller {
        name: "Pioneer DDJ-400",
        profile: include_bytes!(
            "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/pioneer_ddj_400.s3bin"
        ),
    },
    Controller {
        name: "Numark DJ2GO2 Touch",
        profile: include_bytes!(
            "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/numark_dj2go2_touch.s3bin"
        ),
    },
    Controller {
        name: "NI Traktor Kontrol S2 MK3",
        profile: include_bytes!(
            "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/ni_traktor_s2_mk3_full.s3bin"
        ),
    },
    Controller {
        name: "Hercules DJControl Inpulse 500",
        profile: include_bytes!(
            "../../../../firmware-rust/crates/ruzina-controller-profile/fixtures/hercules_djcontrol_inpulse_500.v4.s3bin"
        ),
    },
];
/// The FLX4's place in [`CONTROLLERS`]: its SAMPLER button and pads are read and lit
/// outside the profile.
const FLX4: usize = 0;

/// The controller in [`CONTROLLERS`] whose profile names `vid:pid`.
#[cfg(feature = "idf-controller-midi")]
fn controller_for(vid: u16, pid: u16) -> Option<(usize, Profile<'static>)> {
    CONTROLLERS
        .iter()
        .enumerate()
        .find_map(|(index, controller)| {
            Profile::parse(controller.profile)
                .ok()
                .filter(|profile| profile.vid() == vid && profile.pid() == pid)
                .map(|profile| (index, profile))
        })
}

/// The attached controller's VID:PID (0 when none), and its place in [`CONTROLLERS`] plus 1
/// (0 when none or unknown), for the screen.
static ATTACHED: AtomicU32 = AtomicU32::new(0);
static KNOWN: AtomicU8 = AtomicU8::new(0);

/// The attached controller: `None` when none is, else its VID:PID and, when the deck knows
/// it, its name.
#[cfg_attr(not(feature = "slint-ui"), allow(dead_code))]
pub(crate) fn attached() -> Option<(u16, u16, Option<&'static str>)> {
    let id = ATTACHED.load(Ordering::Acquire);
    let known = usize::from(KNOWN.load(Ordering::Acquire));
    (id != 0).then(|| {
        let name = known.checked_sub(1).map(|index| CONTROLLERS[index].name);
        ((id >> 16) as u16, id as u16, name)
    })
}

/// A SysEx message as USB-MIDI 1.0 packets on cable 0: code index 0x4 for each three bytes
/// that start or continue it, then 0x5, 0x6 or 0x7 for the last one to three.
#[cfg(feature = "idf-controller-midi")]
fn sysex_packets(message: &[u8]) -> impl Iterator<Item = [u8; 4]> + '_ {
    let chunks = message.len().div_ceil(3);
    message.chunks(3).enumerate().map(move |(index, chunk)| {
        let last = index + 1 == chunks;
        let code = if last { 0x4 + chunk.len() as u8 } else { 0x4 };
        let mut packet = [code, 0, 0, 0];
        packet[1..=chunk.len()].copy_from_slice(chunk);
        packet
    })
}

/// The FLX4's SAMPLER button and pads, which the golden profile does not map: the button
/// is note 0x22 on a deck's channel, and the pads notes 0x30 to 0x37 on the pad channels,
/// as the other pad modes are mapped.
#[cfg(feature = "deck-engine")]
const SAMPLER_BUTTON_NOTE: u8 = 0x22;
#[cfg(feature = "deck-engine")]
const SAMPLER_PAD_NOTE: u8 = 0x30;
/// Each deck's pad status (note on), without Shift; with Shift the next channel.
#[cfg(feature = "deck-engine")]
const SAMPLER_PADS: [u8; 2] = [0x97, 0x99];

/// A SAMPLER button or pad message as the semantic event the profile would give it.
#[cfg(feature = "deck-engine")]
pub(crate) fn sampler_event(packet: [u8; 4]) -> Option<ControlEvent> {
    use ruzina_controller_core::{PadAction, PadMode};
    let [_, status, data1, data2] = packet;
    let pressed = match status & 0xf0 {
        0x90 => data2 != 0,
        0x80 => false,
        _ => return None,
    };
    let channel = status & 0x0f;
    match (channel, data1) {
        (0 | 1, SAMPLER_BUTTON_NOTE) => Some(ControlEvent {
            deck: Some(if channel == 0 {
                DeckId::One
            } else {
                DeckId::Two
            }),
            control: SemanticControl::PadModeSampler,
            value: ControlValue::Pressed(pressed),
        }),
        (7..=10, SAMPLER_PAD_NOTE..=0x37) => Some(ControlEvent {
            deck: Some(if channel <= 8 {
                DeckId::One
            } else {
                DeckId::Two
            }),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad: data1 - SAMPLER_PAD_NOTE,
                mode: PadMode::Sampler,
                shifted: channel == 8 || channel == 10,
                pressed,
            }),
        }),
        _ => None,
    }
}

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
    /// What the controller's LEDs show, and which still have to be sent.
    #[cfg(feature = "deck-engine")]
    leds: ControllerLedFrame,
    /// The lamp test owns the LEDs until it clears them.
    #[cfg(feature = "deck-engine")]
    lamp_lit: bool,
    /// The channel meters' levels and when they were last set.
    #[cfg(feature = "deck-engine")]
    meters: [u8; 2],
    #[cfg(feature = "deck-engine")]
    meters_at_ms: u32,
    /// The SAMPLER pads' LEDs as sent, one bit a pad; `None` sends them all again.
    #[cfg(feature = "deck-engine")]
    sampler_leds: [Option<u8>; 2],
    /// Last Headphone Level position and its range; `None` until it moves.
    headphone: Option<(u16, u16)>,
    /// The bound controller's place in [`CONTROLLERS`].
    controller: Option<usize>,
    out: [[u8; 4]; OUT_PACKETS],
    out_len: usize,
    /// Packets that did not fit the queue.
    out_overflow: u32,
}

impl State {
    fn push_out(&mut self, message: [u8; 3]) {
        // Cable 0; the code index number is the channel-voice status nibble.
        self.push_packet([message[0] >> 4, message[0], message[1], message[2]]);
    }

    fn push_packet(&mut self, packet: [u8; 4]) {
        if self.out_len == OUT_PACKETS {
            self.out_overflow = self.out_overflow.saturating_add(1);
            return;
        }
        self.out[self.out_len] = packet;
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
        // Clearing the outputs clears what the LEDs showed: send it all again.
        #[cfg(feature = "deck-engine")]
        {
            self.lamp_lit = on;
            if !on {
                self.leds.mark_all_dirty();
                self.sampler_leds = [None; 2];
            }
        }
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
        #[cfg_attr(feature = "deck-engine", allow(unused_variables))]
        let slot = usize::from(deck == DeckId::Two);
        match event.control {
            #[cfg(not(feature = "deck-engine"))]
            SemanticControl::Play if pressed => {
                self.playing[slot] = !self.playing[slot];
                self.led(SemanticLedId::PLAY, Some(deck), self.playing[slot]);
            }
            SemanticControl::Cue => {
                #[cfg(all(feature = "deck-play", not(feature = "deck-engine")))]
                if pressed {
                    self.stop(deck, slot);
                    crate::deck_play::COMMANDS[slot].cue();
                }
                self.led(SemanticLedId::CUE, Some(deck), pressed);
            }
            #[cfg(all(feature = "deck-play", not(feature = "deck-engine")))]
            SemanticControl::Load if pressed => self.stop(deck, slot),
            _ => {}
        }
    }
}

#[cfg(all(feature = "deck-play", not(feature = "deck-engine")))]
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
    #[cfg(feature = "deck-engine")]
    leds: ControllerLedFrame::new(),
    #[cfg(feature = "deck-engine")]
    lamp_lit: false,
    #[cfg(feature = "deck-engine")]
    meters: [0; 2],
    #[cfg(feature = "deck-engine")]
    meters_at_ms: 0,
    #[cfg(feature = "deck-engine")]
    sampler_leds: [None; 2],
    headphone: None,
    controller: None,
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
    let playing = with(|state| state.playing);
    #[cfg(all(feature = "deck-play", not(feature = "deck-engine")))]
    crate::media_signal::playback(playing, [0; 2]);
    playing
}

#[cfg(feature = "deck-engine")]
impl State {
    /// The channel meters, as the donor drives them: every 30 ms, from the loudest
    /// pre-fader sample since the last update.
    fn meter_leds(&mut self, now_ms: u32) {
        const METER_MS: u32 = 30;
        if now_ms.wrapping_sub(self.meters_at_ms) < METER_MS {
            return;
        }
        self.meters_at_ms = now_ms;
        for (i, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            let peak = crate::deck_view::LED_PEAKS.take(i);
            self.meters[i] = crate::deck_view::meter_led(self.meters[i], peak);
            self.leds.set(LedUpdate {
                led: SemanticLedId::VU_METER,
                deck: Some(deck),
                state: LedState::Value(self.meters[i]),
            });
        }
    }

    /// The LEDs follow the deck engine and the mixer, as on the native firmware.
    /// Only the ones that changed are sent; a new binding sends them all.
    fn sync_leds(&mut self) {
        self.meter_leds(now_ms());
        let memory;
        (self.playing, memory) = crate::deck_mix::with(|mix| {
            mix.refresh_leds(&mut self.leds);
            (mix.engine().playing(), mix.engine().memory_masks())
        });
        if self.lamp_lit {
            return;
        }
        while let Some(update) = self.leds.take_next_dirty() {
            if let Some(message) = self.profile.and_then(|p| p.map_led_update(update)) {
                self.push_out(message);
            }
        }
        // The FLX4's SAMPLER pads light where the track has a memory cue; the profile has
        // no map for them, so their notes go out as they are.
        if self.controller != Some(FLX4) {
            return;
        }
        for (deck, mask) in memory.into_iter().enumerate() {
            let sent = self.sampler_leds[deck];
            for pad in 0..8u8 {
                let on = mask >> pad & 1 != 0;
                if sent.is_none_or(|sent| (sent >> pad & 1 != 0) != on) {
                    let status = SAMPLER_PADS[deck];
                    self.push_out([status, SAMPLER_PAD_NOTE + pad, if on { 0x7f } else { 0 }]);
                }
            }
            self.sampler_leds[deck] = Some(mask);
        }
    }
}

/// Milliseconds since boot; 0 in host tests.
#[cfg(feature = "deck-engine")]
fn now_ms() -> u32 {
    #[cfg(all(feature = "idf-deck-engine", not(test)))]
    {
        unsafe extern "C" {
            fn esp_timer_get_time() -> i64;
        }
        // SAFETY: IDF monotonic task-safe clock.
        (unsafe { esp_timer_get_time() } / 1_000) as u32
    }
    #[cfg(not(all(feature = "idf-deck-engine", not(test))))]
    0
}

/// Brings the LEDs up to date with the deck engine (a track that played out
/// stops without a controller event). USB client task, before
/// the periodic Bulk-OUT flush.
#[cfg(feature = "idf-deck-engine")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_refresh() {
    refresh_mix();
    with(|state| {
        if state.profile.is_some() {
            state.sync_leds();
        }
    });
}

/// The mix's turn on the USB client task. Kept out of line, as the mix's frame (about
/// 2.3 KiB) was before the channel meters, so the export's own frame stays small.
#[cfg(feature = "idf-deck-engine")]
#[inline(never)]
fn refresh_mix() {
    crate::deck_mix::with(|mix| {
        // Taps on the deck screen, as the controller's buttons would send them.
        #[cfg(feature = "slint-ui")]
        while let Some((deck, control)) = crate::deck_view::TOUCH.pop() {
            // The FX page picks an effect or a beat; the engine steps to it from where
            // it is now.
            let fx = mix.engine().fx_view();
            for event in touch_events(deck, control, fx).into_iter().flatten() {
                mix.control(&event);
            }
        }
        // The Web Remote's requests, the same ways, and its view of the mix.
        #[cfg(feature = "web-remote")]
        crate::web_remote::service(mix);
        mix.apply_settings();
        mix.tick();
        mix.attach_fx();
        mix.attach_keylocks();
        mix.engine().flush_hot_cues();
        mix.engine().take_grids();
        mix.engine().publish_marks();
        #[cfg(feature = "slint-ui")]
        crate::deck_view::publish_stats(mix.stats());
        #[cfg(not(test))]
        if let Some((deck, event, value)) = mix.engine().take_phase_report() {
            unsafe extern "C" {
                fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
            }
            // SAFETY: diagnostic telemetry passes three u32 values only.
            unsafe { ruzina_bridge_load_report(deck, event, value) };
        }
        // The soak DJ runs here, outside the USB audio fill's stack chain.
        #[cfg(feature = "soak-autoplay")]
        mix.engine().autoplay();
        #[cfg(all(feature = "soak-autoplay", not(test)))]
        {
            unsafe extern "C" {
                fn esp_timer_get_time() -> i64;
                fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
            }
            // SAFETY: IDF monotonic task-safe clock.
            let now_ms = (unsafe { esp_timer_get_time() } / 1_000) as u32;
            if let Some(gesture) = mix.exercise(now_ms) {
                // SAFETY: diagnostic telemetry passes three u32 values only.
                unsafe { ruzina_bridge_load_report(0, 20, gesture) };
            }
        }
    });
}

#[cfg(feature = "deck-play")]
#[cfg_attr(feature = "deck-mix", allow(dead_code))]
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

/// The most controller events one tap stands for: four beat steps and a release.
#[cfg(all(feature = "deck-engine", any(feature = "slint-ui", test)))]
pub(crate) const TOUCH_EVENTS: usize = 8;

/// The controller events a tap on the deck screen stands for, in order. `fx` is the
/// Beat FX as the engine has it now: the controller only steps the effect and the beat,
/// so a picked one is reached in steps from there.
#[cfg(all(feature = "deck-engine", any(feature = "slint-ui", test)))]
pub(crate) fn touch_events(
    deck: usize,
    control: crate::deck_view::TouchControl,
    fx: crate::deck_view::FxSnapshot,
) -> [Option<ControlEvent>; TOUCH_EVENTS] {
    use crate::deck_view::{FxTouch, TouchControl};
    use ruzina_controller_core::{
        BeatFxTarget, DeckExtAction, DeckExtActionValue, PadAction, PadMode,
    };
    struct Events([Option<ControlEvent>; TOUCH_EVENTS], usize);
    impl Events {
        fn push(&mut self, deck: Option<DeckId>, control: SemanticControl, value: ControlValue) {
            if let Some(slot) = self.0.get_mut(self.1) {
                *slot = Some(ControlEvent {
                    deck,
                    control,
                    value,
                });
                self.1 += 1;
            }
        }

        /// A button pressed `times` times and then released; nothing for no presses.
        fn presses(&mut self, deck: Option<DeckId>, control: SemanticControl, times: u8) {
            for _ in 0..times {
                self.push(deck, control, ControlValue::Pressed(true));
            }
            if times != 0 {
                self.push(deck, control, ControlValue::Pressed(false));
            }
        }
    }
    let mut events = Events([None; TOUCH_EVENTS], 0);
    let on = Some(if deck == 0 { DeckId::One } else { DeckId::Two });
    match control {
        TouchControl::Play => events.presses(on, SemanticControl::Play, 1),
        TouchControl::Cue(down) => {
            events.push(on, SemanticControl::Cue, ControlValue::Pressed(down))
        }
        TouchControl::Load => events.presses(on, SemanticControl::Load, 1),
        TouchControl::LibraryOpen => events.presses(None, SemanticControl::BrowsePress, 1),
        TouchControl::LibraryBack => events.presses(None, SemanticControl::ShiftBrowsePress, 1),
        // The pad in Hot Cue mode, whatever mode the controller's pads are in.
        TouchControl::HotCue { pad, delete, down } => events.push(
            on,
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::HotCue,
                shifted: delete,
                pressed: down,
            }),
        ),
        // A SAMPLER pad, 1 to 8, and with Shift 9 to 16: it calls that memory cue.
        TouchControl::Memory(n) => events.push(
            on,
            SemanticControl::PadAction,
            ControlValue::PadAction(PadAction {
                pad: n & 7,
                mode: PadMode::Sampler,
                shifted: n & 8 != 0,
                pressed: true,
            }),
        ),
        TouchControl::Fx(FxTouch::Effect(effect)) => {
            let steps = (effect & 3).wrapping_sub(fx.effect) & 3;
            events.presses(None, SemanticControl::BeatFxSelectNext, steps);
        }
        TouchControl::Fx(FxTouch::Beat(beat)) => {
            let beat = beat.min(4);
            if beat > fx.beat {
                events.presses(None, SemanticControl::BeatFxBeatInc, beat - fx.beat);
            } else {
                events.presses(None, SemanticControl::BeatFxBeatDec, fx.beat - beat);
            }
        }
        TouchControl::Fx(FxTouch::Target(target)) => events.push(
            None,
            SemanticControl::BeatFxTarget,
            ControlValue::BeatFxTarget(match target {
                1 => BeatFxTarget::ChannelOne,
                2 => BeatFxTarget::ChannelTwo,
                _ => BeatFxTarget::Both,
            }),
        ),
        TouchControl::Fx(FxTouch::Depth(depth)) => events.push(
            None,
            SemanticControl::BeatFxDepth,
            ControlValue::Absolute {
                value: u16::from(depth.min(127)),
                max: 127,
            },
        ),
        TouchControl::Fx(FxTouch::On) => events.presses(None, SemanticControl::BeatFxOn, 1),
        TouchControl::MasterTempo => {
            for pressed in [true, false] {
                events.push(
                    on,
                    SemanticControl::DeckExtAction,
                    ControlValue::DeckExtAction(DeckExtActionValue {
                        action: DeckExtAction::MasterTempo,
                        pressed,
                    }),
                );
            }
        }
    }
    events.0
}

/// The library action a semantic event asks for: `(action, deck, value)`.
#[cfg_attr(feature = "slint-ui", allow(dead_code))]
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
    #[cfg(not(feature = "slint-ui"))]
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
/// profile matches `vid:pid`, 0 when it does not (events then stay unmapped). A profile's
/// initial SysEx is queued first, ahead of the lamp test and the LEDs.
#[cfg(feature = "idf-controller-midi")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_midi_bind(vid: u16, pid: u16) -> i32 {
    with(|state| {
        let found = controller_for(vid, pid);
        state.profile = found.map(|(_, profile)| profile);
        state.controller = found.map(|(index, _)| index);
        ATTACHED.store(u32::from(vid) << 16 | u32::from(pid), Ordering::Release);
        KNOWN.store(
            found.map_or(0, |(index, _)| index as u8 + 1),
            Ordering::Release,
        );
        state.session.reset_profile_state();
        state.session.set_connected(true);
        state.totals = UsbMidiBatchReport::default();
        state.playing = [false; 2];
        #[cfg(feature = "deck-engine")]
        {
            state.leds = ControllerLedFrame::new();
            state.lamp_lit = false;
            state.sampler_leds = [None; 2];
        }
        state.headphone = None;
        state.out_len = 0;
        state.out_overflow = 0;
        if let Some(profile) = state.profile {
            for packet in sysex_packets(profile.init_sysex()) {
                state.push_packet(packet);
            }
        }
        // A profile may let the channel filter work without Smart CFX (the DDJ-400).
        #[cfg(feature = "deck-engine")]
        crate::deck_mix::with(|mix| {
            mix.set_channel_filter_always(state.profile.is_some_and(|p| p.channel_filter_always()))
        });
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
        state.controller = None;
        ATTACHED.store(0, Ordering::Release);
        KNOWN.store(0, Ordering::Release);
        #[cfg(feature = "deck-engine")]
        crate::deck_mix::with(|mix| mix.set_channel_filter_always(false));
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
    with(|state| ingest(state, payload))
}

/// The SAMPLER button and pads, which the profile leaves unmapped, are
/// delivered in packet order among the profile's events: the transfer is cut
/// after each SAMPLER packet, the session delivers what came before it, then
/// the SAMPLER event follows. A transfer without one stays a single batch.
#[cfg(feature = "idf-controller-midi")]
fn ingest(state: &mut State, payload: &[u8]) -> i32 {
    let Some(profile) = state.profile else {
        return -1;
    };
    if !payload.len().is_multiple_of(4) {
        return -1;
    }
    let mut delivered = 0;
    let mut rest = payload;
    loop {
        let (batch, sampler) = split_at_sampler(rest, state.controller == Some(FLX4));
        match state.session.process_usb_midi_transfer(&profile, batch) {
            Ok(report) => state.totals.accumulate(report),
            Err(_) => return -1,
        }
        delivered += drain(state);
        if let Some(event) = sampler {
            deliver(state, &event);
            delivered += 1;
        }
        rest = &rest[batch.len()..];
        if rest.is_empty() {
            return delivered;
        }
    }
}

/// The packets up to and including the first SAMPLER packet, with its event,
/// or the whole payload when it has none. Only the FLX4 has these notes; another
/// controller's payload stays whole, as it does without the deck engine, which alone
/// reads SAMPLER events.
#[cfg(feature = "idf-controller-midi")]
fn split_at_sampler(payload: &[u8], flx4: bool) -> (&[u8], Option<ControlEvent>) {
    #[cfg(feature = "deck-engine")]
    if flx4 {
        for (index, packet) in payload.chunks_exact(4).enumerate() {
            if let Some(event) = sampler_event([packet[0], packet[1], packet[2], packet[3]]) {
                return (&payload[..(index + 1) * 4], Some(event));
            }
        }
    }
    #[cfg(not(feature = "deck-engine"))]
    let _ = flx4;
    (payload, None)
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
        #[cfg(feature = "deck-engine")]
        if let SessionDelivery::Connection(connection) = delivery {
            crate::deck_mix::with(|mix| {
                mix.engine()
                    .connection(connection == ruzina_controller_session::ConnectionState::Connected)
            });
        }
        if let SessionDelivery::Control(event) = delivery {
            deliver(state, &event);
            delivered += 1;
        }
        state.session.complete_delivery(true);
    }
    delivered
}

/// One control event: logged in C, echoed, and handed to the mix and the deck engine. A
/// press that only wakes the screensaver goes no further (`screen_idle`).
#[cfg(feature = "idf-controller-midi")]
fn deliver(state: &mut State, event: &ControlEvent) {
    let (deck, kind, value, max) = encode(event);
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
    #[cfg(feature = "deck-engine")]
    if !crate::screen_idle::controller(event) {
        return;
    }
    state.echo(event);
    #[cfg(all(feature = "deck-mix", not(feature = "web-remote")))]
    crate::deck_mix::with(|mix| mix.control(event));
    #[cfg(feature = "web-remote")]
    crate::deck_mix::with(|mix| mix.controller_control(event));
    #[cfg(feature = "deck-engine")]
    state.sync_leds();
    // The deck screen's Library takes browse and load in Rust; the old load
    // screen keeps its own selection in C.
    #[cfg(not(feature = "slint-ui"))]
    if let Some((action, deck, value)) = ui_action(event) {
        // SAFETY: scalars only.
        unsafe { ruzina_bridge_midi_ui(action, deck, value) };
    }
}

#[cfg(test)]
mod tests;
