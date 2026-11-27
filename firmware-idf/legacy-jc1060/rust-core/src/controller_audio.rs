//! USB Audio Class 1 playback to the controller's headphone channels, over
//! the IDF USB host. C owns the claimed AudioStreaming interface, EP0 and the
//! isochronous transfers; Rust chooses the format (`ruzina_uac::descriptor`),
//! builds the SET_INTERFACE / SET_CUR requests (`ruzina_uac::stream`) and packs
//! each 1 ms packet (`ruzina_uac::packet`). Every function here runs only on
//! the C USB client task, like `controller_midi`.
//!
//! The source is a diagnostic tone, not the deck engine: a 1 kHz sine at
//! -24 dBFS on the headphone pair, left while deck 1's Play latch is on and
//! right while deck 2's is. With `deck-play`, deck 1's decoded track
//! (`deck_play`) replaces the tone on both headphone sides, and MAIN stays
//! silent. With `deck-mix`, the product render graph (`deck_mix`) fills
//! MAIN and the headphones from both decks.
use core::cell::UnsafeCell;
use ruzina_audio_dsp::PcmFrame;
use ruzina_uac::clock::{ClockFailure, ClockNegotiation, ClockNext, ClockRequest};
use ruzina_uac::descriptor::{DescriptorError, Rejection, Selected, UsbSpeed, choose_preferring};
#[cfg(all(test, feature = "idf-uac-profile"))]
use ruzina_uac::descriptor::{parse_playback_formats, select};
use ruzina_uac::packet::{Cadence, ChannelMap, Pair, pack};
use ruzina_uac::stream::{ControlRequest, ControlStep};
use ruzina_uac::uac2::Uac2Playback;

/// Half period of a 1 kHz sine at 48 kHz, amplitude 2067 (-24 dBFS).
const SINE_HALF: [i16; 24] = [
    0, 270, 535, 791, 1033, 1258, 1462, 1640, 1790, 1910, 1997, 2049, 2067, 2049, 1997, 1910, 1790,
    1640, 1462, 1258, 1033, 791, 535, 270,
];

/// Frames in the largest 1 ms packet: 48 kHz, or a device's feedback up to
/// 5 % above it (`ruzina_uac::feedback`), as far as its endpoint holds.
const MAX_FRAMES: usize = 51;

/// The rate tried first: the render rate, or 44.1 kHz to check the
/// conversion on the board (`uac-44k1`).
const PREFERRED_RATE: u32 = if cfg!(feature = "uac-44k1") {
    44_100
} else {
    48_000
};

/// A UAC2 clock is asked for the render rate first.
const PREFERRED_UAC2_RATE: u32 = 48_000;

/// FLX4 four-channel output: MAIN on 1/2, headphones on 3/4.
const MAIN: Pair = Pair { left: 0, right: 1 };
const HEADPHONES: Pair = Pair { left: 2, right: 3 };

struct State {
    selected: Option<Selected>,
    cadence: Cadence,
    map: Option<ChannelMap>,
    phase: usize,
    packets: u32,
    frames: u64,
    toned_packets: u32,
    /// Why the last configuration offered no format: [`reject_words`].
    rejection: [u32; 3],
    /// Explicit feedback readings: new rates, empty replies, refused ones.
    feedback: [u32; 3],
    /// A USB Audio 2.0 alternate to negotiate the clock of, and that
    /// negotiation with the request in flight.
    uac2: Option<Uac2Playback>,
    clock: Option<(ClockNegotiation, ClockRequest)>,
    /// The clock's outcome for the log: [`clock_words`].
    clock_outcome: [u32; 4],
}

struct Shared(UnsafeCell<State>);
// SAFETY: only the C USB client task calls the exports below.
unsafe impl Sync for Shared {}
static STATE: Shared = Shared(UnsafeCell::new(State {
    selected: None,
    cadence: Cadence::new(48_000),
    map: None,
    phase: 0,
    packets: 0,
    frames: 0,
    toned_packets: 0,
    rejection: [0; 3],
    feedback: [0; 3],
    uac2: None,
    clock: None,
    clock_outcome: [0; 4],
}));

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: single caller task (see `Shared`); no reference escapes `f`.
    f(unsafe { &mut *STATE.0.get() })
}

fn sine(index: usize) -> i16 {
    let index = index % 48;
    if index < 24 {
        SINE_HALF[index]
    } else {
        -SINE_HALF[index - 24]
    }
}

/// One packet of headphone PCM: the tone on the enabled sides, else silence.
fn tone(phase: &mut usize, frames: usize, left: bool, right: bool, out: &mut [PcmFrame]) {
    for frame in out.iter_mut().take(frames) {
        let sample = sine(*phase);
        *phase = (*phase + 1) % 48;
        *frame = PcmFrame {
            left: if left { sample } else { 0 },
            right: if right { sample } else { 0 },
        };
    }
}

/// Selects the playback format from a configuration descriptor. Writes
/// interface, alternate, endpoint, max packet, channels, subframe bytes, rate
/// and frequency-control into `out[0..8]`, then the explicit feedback endpoint
/// (0 for none) and its refresh exponent (UAC1) or interval (UAC2) into
/// `out[8..10]`, and the UAC2 clock source and its AudioControl interface into
/// `out[10..12]` (0 for UAC1). Returns 1 for a UAC1 format, 2 for a UAC2
/// alternate whose clock is to be negotiated (`ruzina_legacy_uac2_clock_*`),
/// 0 when nothing; a UAC1 format is
/// supported with a headphone pair, else 0.
///
/// # Safety
/// `config` must be valid for `len` reads and `out` for twelve `u32` writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_select(
    config: *const u8,
    len: u32,
    high_speed: i32,
    out: *mut u32,
) -> i32 {
    if config.is_null() || out.is_null() {
        return 0;
    }
    // SAFETY: the caller lends `len` readable bytes for this call.
    let bytes = unsafe { core::slice::from_raw_parts(config, len as usize) };
    let speed = if high_speed != 0 {
        UsbSpeed::High
    } else {
        UsbSpeed::Full
    };
    with(|state| {
        let chosen = choose_preferring(bytes, speed, PREFERRED_RATE);
        state.rejection = match chosen {
            Ok(chosen) if chosen.format.channels != 4 => [
                REJECT_CHANNELS,
                u32::from(chosen.format.interface),
                u32::from(chosen.format.alternate),
            ],
            Ok(_) => [0; 3],
            Err(rejection) => reject_words(rejection),
        };
        state.uac2 = None;
        state.clock = None;
        state.clock_outcome = [0; 4];
        if let Err(Rejection::Uac2 {
            interface,
            alternate,
        }) = chosen
            && let Some(format) =
                ruzina_uac::uac2::parse(bytes).ok().and_then(|formats| {
                    formats.as_slice().iter().copied().find(|format| {
                        (format.interface, format.alternate) == (interface, alternate)
                    })
                })
        {
            state.selected = None;
            state.map = None;
            state.uac2 = Some(format);
            state.rejection = [0; 3];
            let values = [
                u32::from(format.interface),
                u32::from(format.alternate),
                u32::from(format.endpoint),
                u32::from(format.max_packet),
                u32::from(format.channels),
                u32::from(format.subslot_bytes),
                PREFERRED_UAC2_RATE,
                0,
                u32::from(format.feedback_endpoint),
                u32::from(format.feedback_interval),
                u32::from(format.clock_source),
                u32::from(format.control_interface),
            ];
            // SAFETY: the caller provides twelve writable u32 slots.
            unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
            return 2;
        }
        state.selected = chosen.ok().filter(|chosen| chosen.format.channels == 4);
        state.map = state
            .selected
            .and_then(|_| ChannelMap::new(4, Some(MAIN), Some(HEADPHONES)).ok());
        let Some(chosen) = state.selected.filter(|_| state.map.is_some()) else {
            return 0;
        };
        let explicit = chosen.format.explicit_feedback();
        state.cadence = if explicit {
            Cadence::with_feedback(chosen.rate)
        } else {
            Cadence::new(chosen.rate)
        };
        state.feedback = [0; 3];
        state.phase = 0;
        state.packets = 0;
        state.frames = 0;
        state.toned_packets = 0;
        let f = chosen.format;
        let values = [
            u32::from(f.interface),
            u32::from(f.alternate),
            u32::from(f.endpoint),
            u32::from(f.max_packet),
            u32::from(f.channels),
            u32::from(f.subframe_bytes),
            chosen.rate,
            u32::from(f.frequency_control),
            u32::from(if explicit { f.feedback_endpoint } else { 0 }),
            u32::from(if explicit { f.feedback_refresh } else { 0 }),
            0,
            0,
        ];
        // SAFETY: the caller provides twelve writable u32 slots.
        unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
        1
    })
}

/// One reading of the selected format's explicit feedback endpoint (the
/// bytes it returned, none between refreshes): 1 a new pace, 0 nothing new,
/// -1 refused or no feedback to follow. The pace holds until the next reading.
///
/// # Safety
/// `bytes` must be valid for `len` reads (any pointer when `len` is 0).
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_feedback(bytes: *const u8, len: u32) -> i32 {
    let bytes = if len == 0 || bytes.is_null() {
        &[][..]
    } else {
        // SAFETY: the caller lends `len` readable bytes for this call.
        unsafe { core::slice::from_raw_parts(bytes, len as usize) }
    };
    with(|state| {
        let Some(chosen) = state
            .selected
            .filter(|chosen| chosen.format.explicit_feedback())
        else {
            return -1;
        };
        let max_frames = chosen.format.max_frames().min(MAX_FRAMES);
        match ruzina_uac::feedback::decode(bytes, chosen.rate, max_frames) {
            ruzina_uac::feedback::Feedback::Rate(q16) => {
                state.cadence.set_feedback_q16(q16);
                state.feedback[0] = state.feedback[0].wrapping_add(1);
                1
            }
            ruzina_uac::feedback::Feedback::Empty => {
                state.feedback[1] = state.feedback[1].wrapping_add(1);
                0
            }
            ruzina_uac::feedback::Feedback::Invalid => {
                state.feedback[2] = state.feedback[2].wrapping_add(1);
                -1
            }
        }
    })
}

/// Writes the feedback readings (new paces, empty, refused) and the current
/// pace in Q16 frames per ms (0 without feedback) to `out[0..4]`.
///
/// # Safety
/// `out` must be valid for four `u32` writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_feedback_stats(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = with(|state| {
        let [updates, empty, invalid] = state.feedback;
        [
            updates,
            empty,
            invalid,
            state.cadence.feedback_q16().unwrap_or(0),
        ]
    });
    // SAFETY: the caller provides four writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

/// `[state, rate, retries, detail]` for the C log: state 1 negotiating, 2
/// ready at `rate`, 3 failed with `detail` 1 transfer, 2 range, 3 no rate, 4
/// readback (then `rate` is the rate the clock reported), 5 invalid.
fn clock_words(clock: &ClockNegotiation, next: ClockNext) -> [u32; 4] {
    match next {
        ClockNext::Send(_) | ClockNext::RetryAt { .. } => [1, clock.rate(), clock.retries(), 0],
        ClockNext::Ready(rate) => [2, rate, clock.retries(), 0],
        ClockNext::Failed(failure) => {
            let (detail, rate) = match failure {
                ClockFailure::Transfer => (1, clock.rate()),
                ClockFailure::Range => (2, clock.rate()),
                ClockFailure::NoRate => (3, clock.rate()),
                ClockFailure::Readback { reported } => (4, reported),
                ClockFailure::Invalid => (5, clock.rate()),
            };
            [3, rate, clock.retries(), detail]
        }
    }
}

/// Writes `request`'s SETUP (and SET CUR data) for the selected clock to
/// `out[0..12]` and the bytes the device returns to `reply`; returns the
/// bytes to send.
fn write_setup(format: &Uac2Playback, request: ClockRequest, out: *mut u8, reply: *mut u32) -> u32 {
    let setup = request.setup(format.clock_source, format.control_interface);
    // SAFETY: the callers check both pointers and provide twelve bytes and
    // one u32.
    unsafe {
        core::ptr::copy_nonoverlapping(setup.bytes.as_ptr(), out, setup.len);
        *reply = u32::from(setup.reply);
    }
    setup.len as u32
}

/// Starts the selected UAC2 alternate's clock negotiation: its first request
/// into `out[0..12]` and the reply bytes it expects into `reply`; returns the
/// bytes to send, 0 without a UAC2 selection.
///
/// # Safety
/// `out` must be valid for twelve writes and `reply` for one.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac2_clock_begin(out: *mut u8, reply: *mut u32) -> u32 {
    if out.is_null() || reply.is_null() {
        return 0;
    }
    with(|state| {
        let Some(format) = state.uac2 else {
            return 0;
        };
        let (clock, request) = ClockNegotiation::new(PREFERRED_UAC2_RATE);
        state.clock = Some((clock, request));
        state.clock_outcome = [1, clock.rate(), 0, 0];
        write_setup(&format, request, out, reply)
    })
}

/// The request in flight came back (`ok` when it completed) with `len`
/// reply bytes at `now_ms`. Returns 0: send `out` (`out_len`, expecting
/// `reply` bytes) now; 1: send it after `wait_ms`; 2: ready; 3: failed;
/// -1 without a negotiation. [`ruzina_legacy_uac2_clock_status`] says which
/// rate or failure.
///
/// # Safety
/// `bytes` must be valid for `len` reads (any pointer when `len` is 0), `out`
/// for twelve writes, and `out_len`, `reply` and `wait_ms` for one each.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac2_clock_done(
    ok: i32,
    bytes: *const u8,
    len: u32,
    now_ms: u32,
    out: *mut u8,
    out_len: *mut u32,
    reply: *mut u32,
    wait_ms: *mut u32,
) -> i32 {
    if out.is_null() || out_len.is_null() || reply.is_null() || wait_ms.is_null() {
        return -1;
    }
    let mut copy = [0u8; ruzina_uac::clock::MAX_RANGE_BYTES];
    let len = (len as usize).min(copy.len());
    if len != 0 && !bytes.is_null() {
        // SAFETY: the caller lends `len` readable bytes for this call; they
        // are copied before `out` is written.
        unsafe { core::ptr::copy_nonoverlapping(bytes, copy.as_mut_ptr(), len) };
    }
    with(|state| {
        let (Some(format), Some((mut clock, request))) = (state.uac2, state.clock) else {
            return -1;
        };
        let next = clock.done(request, ok != 0, &copy[..len], u64::from(now_ms));
        state.clock_outcome = clock_words(&clock, next);
        let (action, send, wait) = match next {
            ClockNext::Send(request) => (0, Some(request), 0),
            ClockNext::RetryAt { at_ms, request } => (
                1,
                Some(request),
                at_ms.saturating_sub(u64::from(now_ms)) as u32,
            ),
            ClockNext::Ready(_) => (2, None, 0),
            ClockNext::Failed(_) => (3, None, 0),
        };
        state.clock = send.map(|request| (clock, request));
        // SAFETY: checked above; the caller provides the outputs.
        unsafe {
            *wait_ms = wait;
            *out_len = send.map_or(0, |request| write_setup(&format, request, out, reply));
        }
        action
    })
}

/// Writes the clock negotiation's `[state, rate, retries, detail]`
/// (`clock_words`) to `out[0..4]`.
///
/// # Safety
/// `out` must be valid for four `u32` writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac2_clock_status(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = with(|state| state.clock_outcome);
    // SAFETY: the caller provides four writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

/// `ruzina_legacy_uac_reject` code for a selected format without four
/// channels (MAIN and headphones need four).
const REJECT_CHANNELS: u32 = 11;

/// `[code, a, b]` for the C log; the codes index the names in
/// `uac_log_reject` (`uac_driver.inc`), 0 meaning none. Descriptor errors
/// carry the byte offset in `a`; format reasons carry interface and alternate.
fn reject_words(rejection: Rejection) -> [u32; 3] {
    let at = |code: u32, offset: usize| [code, offset as u32, 0];
    let alt = |code: u32, interface: u8, alternate: u8| {
        [code, u32::from(interface), u32::from(alternate)]
    };
    match rejection {
        Rejection::Descriptor(DescriptorError::Header) => [1, 0, 0],
        Rejection::Descriptor(DescriptorError::Malformed { offset }) => at(2, offset),
        Rejection::Descriptor(DescriptorError::DuplicateEndpoint { offset }) => at(3, offset),
        Rejection::Descriptor(DescriptorError::TooManyRates { offset }) => at(4, offset),
        Rejection::NoStreaming => [5, 0, 0],
        Rejection::Format {
            interface,
            alternate,
        } => alt(6, interface, alternate),
        Rejection::Feedback {
            interface,
            alternate,
        } => alt(7, interface, alternate),
        Rejection::Interval {
            interface,
            alternate,
        } => alt(8, interface, alternate),
        Rejection::Rate {
            interface,
            alternate,
        } => alt(9, interface, alternate),
        Rejection::PacketSize {
            interface,
            alternate,
        } => alt(10, interface, alternate),
        Rejection::Uac2 {
            interface,
            alternate,
        } => alt(12, interface, alternate),
        Rejection::Uac2Unsupported {
            interface,
            alternate,
        } => alt(13, interface, alternate),
    }
}

/// Writes why the last [`ruzina_legacy_uac_select`] chose nothing to
/// `out[0..3]`: code (0 after a selection), then offset or interface and
/// alternate.
///
/// # Safety
/// `out` must be valid for three `u32` writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_reject(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = with(|state| state.rejection);
    // SAFETY: the caller provides three writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

/// Builds the EP0 request for `step` (0 SET_INTERFACE to the selected
/// alternate, 1 SET_CUR sampling frequency, 2 SET_INTERFACE to zero
/// bandwidth) into `out[0..11]`; returns its length, 0 without a selection.
///
/// # Safety
/// `out` must be valid for eleven writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_control(step: u32, out: *mut u8) -> u32 {
    let step = match step {
        0 => ControlStep::SetAlternate,
        1 => ControlStep::SetSampleRate,
        2 => ControlStep::SetZeroAlternate,
        _ => return 0,
    };
    if out.is_null() {
        return 0;
    }
    with(|state| {
        let Some(chosen) = state.selected else {
            return 0;
        };
        let request = ControlRequest::build(step, &chosen.format, chosen.rate);
        // SAFETY: the caller provides eleven writable bytes.
        unsafe { core::ptr::copy_nonoverlapping(request.bytes.as_ptr(), out, request.len) };
        request.len as u32
    })
}

/// Fills `packets` consecutive 1 ms packets into `buffer` (`capacity` bytes)
/// and their sizes into `sizes`; returns the total bytes, 0 on any error.
/// Each call counts in `FILLS`.
///
/// # Safety
/// `buffer` must be valid for `capacity` writes and `sizes` for `packets`.
/// USB audio fills since boot. While it stands still, no audio is streaming to the
/// controller, so a long flash operation stalls nothing that is heard.
#[cfg(feature = "idf-controller-audio")]
pub(crate) static FILLS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_fill(
    buffer: *mut u8,
    capacity: u32,
    sizes: *mut u32,
    packets: u32,
) -> u32 {
    let _fill_profile = crate::uac_profile::fill_scope();
    FILLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if buffer.is_null() || sizes.is_null() {
        return 0;
    }
    // SAFETY: the caller lends both regions for this call.
    let (out, sizes) = unsafe {
        (
            core::slice::from_raw_parts_mut(buffer, capacity as usize),
            core::slice::from_raw_parts_mut(sizes, packets as usize),
        )
    };
    let [left, right] = crate::controller_midi::playing();
    #[cfg(not(feature = "deck-play"))]
    return with(|state| fill(state, out, sizes, left, right));
    #[cfg(all(feature = "deck-play", not(feature = "deck-mix")))]
    {
        let _ = right;
        let gain = crate::deck_play::headphone_gain_q15(crate::controller_midi::headphone_level());
        with(|state| {
            let rate = state.selected.map_or(0, |chosen| chosen.rate);
            fill_from(state, out, sizes, |_, _, pcm| {
                crate::deck_play::render(pcm, rate, left, gain);
                left
            })
        })
    }
    #[cfg(feature = "deck-mix")]
    with(|state| {
        let rate = state.selected.map_or(0, |chosen| chosen.rate);
        fill_from(state, out, sizes, |_, main, headphones| {
            crate::deck_mix::with(|mix| mix.fill(main, headphones, rate, [left, right]));
            left || right
        })
    })
}

/// Packs one 1 ms packet per entry of `sizes` into `out`, the tone on the
/// enabled headphone sides; returns the total bytes, 0 on any error.
#[cfg_attr(feature = "deck-play", allow(dead_code))]
fn fill(state: &mut State, out: &mut [u8], sizes: &mut [u32], left: bool, right: bool) -> u32 {
    fill_from(state, out, sizes, |state, _, pcm| {
        tone(&mut state.phase, pcm.len(), left, right, pcm);
        left || right
    })
}

/// Packs one 1 ms packet per entry of `sizes` into `out` with MAIN and
/// headphone PCM from `source` (MAIN stays silent unless it writes there),
/// which reports whether the packet is audible; returns the total bytes, 0
/// on any error.
fn fill_from(
    state: &mut State,
    out: &mut [u8],
    sizes: &mut [u32],
    mut source: impl FnMut(&mut State, &mut [PcmFrame], &mut [PcmFrame]) -> bool,
) -> u32 {
    let (Some(chosen), Some(map)) = (state.selected, state.map) else {
        return 0;
    };
    let Some(format) = chosen.format.sample_format() else {
        return 0;
    };
    let mut offset = 0;
    let mut main = [PcmFrame::default(); MAX_FRAMES + 1];
    let mut pcm = [PcmFrame::default(); MAX_FRAMES + 1];
    for size in sizes.iter_mut() {
        crate::uac_profile::packet();
        let frames = state.cadence.next_frames();
        let audible = source(state, &mut main[..frames], &mut pcm[..frames]);
        let _profile = crate::uac_profile::scope(crate::uac_profile::Stage::Pack);
        let Ok(written) = pack(
            &mut out[offset..],
            format,
            &map,
            Some(&main[..frames]),
            Some(&pcm[..frames]),
            frames,
        ) else {
            return 0;
        };
        *size = written as u32;
        offset += written;
        state.packets = state.packets.wrapping_add(1);
        state.frames += frames as u64;
        state.toned_packets = state.toned_packets.wrapping_add(u32::from(audible));
    }
    offset as u32
}

/// Writes packets, frames (low 32 bits) and audible packets (tone, or deck 1
/// playing) to `out[0..3]`.
///
/// # Safety
/// `out` must be valid for three `u32` writes.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_stats(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = with(|state| [state.packets, state.frames as u32, state.toned_packets]);
    // SAFETY: the caller provides three writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

#[cfg(test)]
mod tests;
