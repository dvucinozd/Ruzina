//! Two-deck mix to MAIN and the headphones, through the product render graph.
//!
//! Both decks decode into their `deck_play` rings. On the USB client task,
//! `controller_audio` takes each packet's frames from 128-frame blocks of
//! `ruzina-audio-runtime`'s `FixedRateRenderGraph`: per-deck resampling to
//! 48 kHz with the Tempo fader as pitch, trim, EQ and filter, the channel
//! faders and crossfader, master gain and the MAIN limiter, plus the
//! headphone mix of the PFL taps and MAIN. The FLX4's mixer controls reach
//! `ruzina-mixer`'s `MixerState` as on the native firmware.
//!
//! Two departures from the product, for a diagnostic on headphones:
//! - Master Volume and Headphone Level start at a quarter until each knob
//!   first moves, instead of the product's full scale.
//! - The headphone output keeps a fixed -12 dB ceiling.
use crate::deck_play::{RINGS, Reader};
use core::cell::UnsafeCell;
use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::render::{
    FixedRateRenderGraph, RENDER_BLOCK_FRAMES, RENDER_SAMPLE_RATE_HZ, RenderControl,
    RenderDeckControl, RenderOutputBlock,
};
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_mixer::{MIXER_CONTROL_MAX, MixerState};

/// Fixed headphone headroom on top of the product mix (-12 dB).
const HEADPHONE_CEILING: f32 = 0.25;

/// Master Volume and Headphone Level until each knob first moves.
const START_LEVEL: u16 = MIXER_CONTROL_MAX / 4;

/// Tempo fader range, as the deck's default (+-10 %).
const TEMPO_RANGE_PERCENT: u16 = 10;

pub(crate) struct Mix {
    graph: Option<FixedRateRenderGraph<'static>>,
    mixer: MixerState,
    /// Start levels are applied once, before the first control or block.
    started: bool,
    pitch: [i16; 2],
    readers: [Reader; 2],
    block: RenderOutputBlock,
    /// Next frame of `block` to hand out; a full block is consumed.
    position: usize,
    rendered: u32,
    /// Blocks in which a playing deck ran dry before its track ended.
    underruns: [u32; 2],
    limited: u32,
}

impl Mix {
    pub(crate) const fn new() -> Self {
        Self {
            graph: None,
            mixer: MixerState::new(),
            started: false,
            pitch: [0; 2],
            readers: [Reader::new(), Reader::new()],
            block: RenderOutputBlock::silence(),
            position: RENDER_BLOCK_FRAMES,
            rendered: 0,
            underruns: [0; 2],
            limited: 0,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        for control in [
            SemanticControl::MasterVolume,
            SemanticControl::HeadphoneLevel,
        ] {
            self.mixer.handle_control(ControlEvent {
                deck: None,
                control,
                value: ControlValue::Absolute {
                    value: START_LEVEL,
                    max: MIXER_CONTROL_MAX,
                },
            });
        }
    }

    /// One controller event into the mixer (and the Tempo faders).
    pub(crate) fn control(&mut self, event: &ControlEvent) {
        self.start();
        if let (SemanticControl::Tempo, Some(deck), ControlValue::Absolute { value, .. }) =
            (event.control, event.deck, event.value)
        {
            self.pitch[usize::from(deck == DeckId::Two)] =
                ruzina_deck::tempo_centipercent_from_raw(value, TEMPO_RANGE_PERCENT);
        }
        self.mixer.handle_control(*event);
    }

    /// The render control for this block, built as the native render owner
    /// builds it from the mixer.
    fn render_control(&self, playing: [bool; 2]) -> RenderControl {
        let gains = self.mixer.stage_gains();
        let mut control = RenderControl::idle();
        control.master_gain = gains.master;
        control.headphones = self.mixer.headphone_gains();
        for (index, deck) in [DeckId::One, DeckId::Two].into_iter().enumerate() {
            let mixer_deck = self.mixer.deck(deck);
            let rate = self.readers[index].rate();
            control.decks[index] = RenderDeckControl {
                source_epoch: 1,
                playing: playing[index] && rate != 0,
                stream_generation: 1,
                source_sample_rate_hz: if rate != 0 {
                    rate
                } else {
                    RENDER_SAMPLE_RATE_HZ
                },
                pitch_centipercent: self.pitch[index],
                pre_gain: gains.pre[index],
                post_gain: gains.post[index],
                eq_low: mixer_deck.eq_low,
                eq_mid: mixer_deck.eq_mid,
                eq_high: mixer_deck.eq_high,
                filter_raw: self.mixer.effective_filter_raw(deck),
                filter_enabled: self.mixer.channel_filter_enabled(),
                pfl_enabled: mixer_deck.pfl_enabled,
                ..RenderDeckControl::idle()
            };
        }
        control
    }

    fn render(&mut self, playing: [bool; 2]) {
        let graph = self.graph.get_or_insert_with(FixedRateRenderGraph::new);
        for (deck, reader) in self.readers.iter_mut().enumerate() {
            if reader.sync(&RINGS[deck]) {
                graph.reset_deck(deck);
            }
        }
        let control = self.render_control(playing);
        let graph = self.graph.as_mut().expect("graph built above");
        let readers = &mut self.readers;
        let report = graph.render_block(control, &mut self.block, |deck| {
            readers[deck].pop(&RINGS[deck])
        });
        for (deck, reader) in readers.iter().enumerate() {
            reader.commit(&RINGS[deck]);
            if report.underflow_frames[deck] != 0 && !reader.ended(&RINGS[deck]) {
                self.underruns[deck] = self.underruns[deck].wrapping_add(1);
            }
        }
        self.rendered = self
            .rendered
            .wrapping_add(u32::from(report.rendered_frames));
        self.limited = self.limited.wrapping_add(report.limiter.limited_samples);
        self.position = 0;
    }

    /// Fills one packet of MAIN and headphone frames; silent unless the
    /// device runs at the render rate.
    pub(crate) fn fill(
        &mut self,
        main: &mut [PcmFrame],
        headphones: &mut [PcmFrame],
        device_rate: u32,
        playing: [bool; 2],
    ) {
        self.start();
        if device_rate != RENDER_SAMPLE_RATE_HZ {
            main.fill(PcmFrame::default());
            headphones.fill(PcmFrame::default());
            return;
        }
        for (main, headphones) in main.iter_mut().zip(headphones.iter_mut()) {
            if self.position >= RENDER_BLOCK_FRAMES {
                self.render(playing);
            }
            *main = self.block.main[self.position];
            let monitor = self.block.headphones[self.position];
            *headphones = PcmFrame {
                left: (f32::from(monitor.left) * HEADPHONE_CEILING) as i16,
                right: (f32::from(monitor.right) * HEADPHONE_CEILING) as i16,
            };
            self.position += 1;
        }
    }

    /// Rendered frames, underrun blocks per deck, buffered frames and rate
    /// per deck, and limited MAIN samples.
    pub(crate) fn stats(&self) -> [u32; 8] {
        [
            self.rendered,
            self.underruns[0],
            self.underruns[1],
            self.readers[0].buffered(&RINGS[0]),
            self.readers[1].buffered(&RINGS[1]),
            self.readers[0].rate(),
            self.readers[1].rate(),
            self.limited,
        ]
    }
}

struct Shared(UnsafeCell<Mix>);
// SAFETY: only the C USB client task renders, routes controls and reads the
// counters (the MIDI and UAC exports share that task).
unsafe impl Sync for Shared {}
static MIX: Shared = Shared(UnsafeCell::new(Mix::new()));

pub(crate) fn with<R>(f: impl FnOnce(&mut Mix) -> R) -> R {
    // SAFETY: single caller task (see `Shared`); no reference escapes `f`.
    f(unsafe { &mut *MIX.0.get() })
}

/// Writes the `Mix::stats` counters to `out[0..8]`.
///
/// # Safety
/// `out` must be valid for eight `u32` writes. USB client task only.
#[cfg(feature = "idf-deck-mix")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_mix_stats(out: *mut u32) {
    if out.is_null() {
        return;
    }
    let values = with(|mix| mix.stats());
    // SAFETY: the caller provides eight writable u32 slots.
    unsafe { core::ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
}

#[cfg(test)]
mod tests;
