//! Fixed-rate, caller-owned per-deck effect state. No render-time allocation.
use crate::{BeatFxDeckConfig, BeatFxFilterConfig};
use pajoniiir_audio_dsp::{
    DelayConfig, DelayFx, DelayMode, DspFrame, FilterState, FlangerConfig, FlangerFx, PadFx,
    PadFxConfig, PadFxMode,
};

pub const FX_SAMPLE_RATE: u32 = 48_000;
pub const FX_DELAY_FRAMES: usize = FX_SAMPLE_RATE as usize + 1;
pub const FX_FLANGER_FRAMES: usize = 292; // 6 ms at 48 kHz + interpolation margin
pub const FX_DECK_STORAGE_SAMPLES: usize = 4 * FX_DELAY_FRAMES + 2 * FX_FLANGER_FRAMES;
pub const FX_TOTAL_STORAGE_SAMPLES: usize = 2 * FX_DECK_STORAGE_SAMPLES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckFxControl {
    pub beat: BeatFxDeckConfig,
    pub beat_selected: bool,
    pub beat_reset_epoch: u32,
    pub pad: PadFxConfig,
}
impl DeckFxControl {
    pub const fn bypass() -> Self {
        Self {
            beat: BeatFxDeckConfig {
                filter: BeatFxFilterConfig {
                    enabled: false,
                    raw: 8192,
                },
                time: DelayConfig {
                    enabled: false,
                    mode: DelayMode::Echo,
                    delay_ms: 1,
                    wet_q15: 0,
                    feedback_q15: 0,
                },
                flanger: FlangerConfig {
                    enabled: false,
                    period_ms: 500,
                    depth_q15: 0,
                },
            },
            beat_selected: false,
            beat_reset_epoch: 0,
            pad: PadFxConfig {
                mode: PadFxMode::PadFx1,
                pad: 0,
                active: false,
            },
        }
    }
}
impl Default for DeckFxControl {
    fn default() -> Self {
        Self::bypass()
    }
}

/// Read authoritative staged product state, not historical queued commands.
pub fn product_fx_controls(product: &pajoniiir_deck::DeckProductState) -> [DeckFxControl; 2] {
    use pajoniiir_controller_core::{BeatFxTarget, DeckId};
    use pajoniiir_deck::PadFxBank;
    let effect = product.beat_fx_effect().items[0].expect("Beat FX snapshot");
    let Some(crate::AudioEffectCommand::BeatFx { decks }) = crate::map_deck_effect(effect) else {
        unreachable!()
    };
    core::array::from_fn(|i| {
        let active = product
            .deck(if i == 0 { DeckId::One } else { DeckId::Two })
            .active_pad_fx;
        DeckFxControl {
            beat: decks[i],
            beat_selected: match product.beat_fx().target {
                BeatFxTarget::Both => true,
                BeatFxTarget::ChannelOne => i == 0,
                BeatFxTarget::ChannelTwo => i == 1,
            },
            beat_reset_epoch: product.beat_fx_reset_epoch(),
            pad: active.map_or(DeckFxControl::bypass().pad, |pad| PadFxConfig {
                mode: match pad.bank {
                    PadFxBank::One => PadFxMode::PadFx1,
                    PadFxBank::Two => PadFxMode::PadFx2,
                },
                pad: pad.pad,
                active: true,
            }),
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FxBufferError;

pub(crate) struct DeckFx<'a> {
    pad: PadFx<'a>,
    filter: FilterState,
    delay: DelayFx<'a>,
    flanger: FlangerFx<'a>,
    scope: Option<(u32, u32)>,
    control: Option<DeckFxControl>,
}
impl<'a> DeckFx<'a> {
    pub(crate) fn new(storage: &'a mut [f32]) -> Result<Self, FxBufferError> {
        if storage.len() < FX_DECK_STORAGE_SAMPLES {
            return Err(FxBufferError);
        }
        let (delay_l, rest) = storage.split_at_mut(FX_DELAY_FRAMES);
        let (delay_r, rest) = rest.split_at_mut(FX_DELAY_FRAMES);
        let (pad_l, rest) = rest.split_at_mut(FX_DELAY_FRAMES);
        let (pad_r, rest) = rest.split_at_mut(FX_DELAY_FRAMES);
        let (flanger_l, rest) = rest.split_at_mut(FX_FLANGER_FRAMES);
        let (flanger_r, _) = rest.split_at_mut(FX_FLANGER_FRAMES);
        Ok(Self {
            pad: PadFx::new(FX_SAMPLE_RATE, pad_l, pad_r),
            filter: FilterState::new(FX_SAMPLE_RATE),
            delay: DelayFx::new(delay_l, delay_r, FX_SAMPLE_RATE),
            flanger: FlangerFx::new(flanger_l, flanger_r, FX_SAMPLE_RATE),
            scope: None,
            control: None,
        })
    }

    fn reset_beat(&mut self) {
        self.filter.reset();
        self.filter.set_raw(8192);
        self.delay.configure(DelayConfig::default());
        self.delay.reset();
        self.flanger.configure(FlangerConfig::default());
        self.flanger.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.reset_beat();
        self.pad.reset();
        self.scope = None;
        self.control = None;
    }

    pub(crate) fn configure(&mut self, scope: Option<(u32, u32)>, control: DeckFxControl) {
        if scope != self.scope {
            self.reset();
            self.scope = scope;
        }
        if scope.is_none() {
            return;
        }
        let previous = self.control;
        if previous == Some(control) {
            return;
        }
        if previous.is_some_and(|p| p.beat_reset_epoch != control.beat_reset_epoch)
            || !control.beat_selected
        {
            self.reset_beat();
        }
        if control.beat_selected {
            if previous.is_some_and(|p| p.beat.filter.enabled && !control.beat.filter.enabled) {
                self.filter.reset();
            }
            self.filter.set_raw(control.beat.filter.raw);
            self.delay.configure(control.beat.time);
            self.flanger.configure(control.beat.flanger);
        }
        if previous.is_none_or(|p| p.pad != control.pad) {
            if control.pad.active {
                self.pad.reset();
                self.pad.set(control.pad);
            } else {
                // The snapshot no longer carries the identity of the released
                // pad. Release the actual held instance, then let its tail run.
                let released = PadFxConfig {
                    active: false,
                    ..self.pad.config()
                };
                self.pad.set(released);
            }
        }
        self.control = Some(control);
    }

    pub(crate) fn process(&mut self, input: DspFrame) -> DspFrame {
        if self.scope.is_none() {
            return DspFrame::default();
        }
        let mut out = self.pad.process_frame(input);
        if let Some(control) = self.control.filter(|c| c.beat_selected) {
            out = self.filter.process_frame(control.beat.filter.enabled, out);
            out = self.delay.process_frame(out);
            out = self.flanger.process_frame(out);
        }
        out
    }
}
