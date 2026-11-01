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
//! (`deck_play`) replaces the tone on both headphone sides. MAIN stays silent
//! (no MAIN route is qualified).
use core::cell::UnsafeCell;
use ruzina_audio_dsp::PcmFrame;
use ruzina_uac::descriptor::{Selected, UsbSpeed, parse_playback_formats, select};
use ruzina_uac::packet::{Cadence, ChannelMap, Pair, pack};
use ruzina_uac::stream::{ControlRequest, ControlStep};

/// Half period of a 1 kHz sine at 48 kHz, amplitude 2067 (-24 dBFS).
const SINE_HALF: [i16; 24] = [
    0, 270, 535, 791, 1033, 1258, 1462, 1640, 1790, 1910, 1997, 2049, 2067, 2049, 1997, 1910, 1790,
    1640, 1462, 1258, 1033, 791, 535, 270,
];

/// Frames in the largest 1 ms packet (48 kHz).
const MAX_FRAMES: usize = 48;

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
/// and frequency-control into `out[0..8]`; returns 1 when a format is
/// supported with a headphone pair, else 0.
///
/// # Safety
/// `config` must be valid for `len` reads and `out` for eight `u32` writes.
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
        state.selected = parse_playback_formats(bytes)
            .ok()
            .and_then(|formats| select(&formats, speed))
            .filter(|chosen| chosen.format.channels == 4);
        state.map = state
            .selected
            .and_then(|_| ChannelMap::new(4, Some(MAIN), Some(HEADPHONES)).ok());
        let Some(chosen) = state.selected.filter(|_| state.map.is_some()) else {
            return 0;
        };
        state.cadence = Cadence::new(chosen.rate);
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
        ];
        // SAFETY: the caller provides eight writable u32 slots.
        unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
        1
    })
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
///
/// # Safety
/// `buffer` must be valid for `capacity` writes and `sizes` for `packets`.
#[cfg(feature = "idf-controller-audio")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_uac_fill(
    buffer: *mut u8,
    capacity: u32,
    sizes: *mut u32,
    packets: u32,
) -> u32 {
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
    #[cfg(feature = "deck-play")]
    {
        let _ = right;
        let gain = crate::deck_play::headphone_gain_q15(crate::controller_midi::headphone_level());
        with(|state| {
            let rate = state.selected.map_or(0, |chosen| chosen.rate);
            fill_from(state, out, sizes, |_, pcm| {
                crate::deck_play::render(pcm, rate, left, gain);
                left
            })
        })
    }
}

/// Packs one 1 ms packet per entry of `sizes` into `out`, the tone on the
/// enabled headphone sides; returns the total bytes, 0 on any error.
#[cfg_attr(feature = "deck-play", allow(dead_code))]
fn fill(state: &mut State, out: &mut [u8], sizes: &mut [u32], left: bool, right: bool) -> u32 {
    fill_from(state, out, sizes, |state, pcm| {
        tone(&mut state.phase, pcm.len(), left, right, pcm);
        left || right
    })
}

/// Packs one 1 ms packet per entry of `sizes` into `out` with headphone PCM
/// from `source`, which reports whether the packet is audible; returns the
/// total bytes, 0 on any error.
fn fill_from(
    state: &mut State,
    out: &mut [u8],
    sizes: &mut [u32],
    mut source: impl FnMut(&mut State, &mut [PcmFrame]) -> bool,
) -> u32 {
    let (Some(chosen), Some(map)) = (state.selected, state.map) else {
        return 0;
    };
    let Some(format) = chosen.format.sample_format() else {
        return 0;
    };
    let mut offset = 0;
    let mut pcm = [PcmFrame::default(); MAX_FRAMES + 1];
    for size in sizes.iter_mut() {
        let frames = state.cadence.next_frames();
        let audible = source(state, &mut pcm[..frames]);
        let Ok(written) = pack(
            &mut out[offset..],
            format,
            &map,
            None,
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
