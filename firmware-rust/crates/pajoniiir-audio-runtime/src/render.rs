use crate::fx::{DeckFx, DeckFxControl, FxBufferError};
use crate::source::{CanonicalSource, SourceControl, SourceSample};
use pajoniiir_audio_dsp::{DspFrame, EqState, FilterState, PcmFrame, ResamplerState};
use pajoniiir_mixer::{LimiterStats, limit_main_sample};

pub const RENDER_SAMPLE_RATE_HZ: u32 = 48_000;
pub const RENDER_BLOCK_FRAMES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderDeckControl {
    pub source_epoch: u32,
    pub fx: DeckFxControl,
    pub playing: bool,
    pub stream_generation: u32,
    pub source_sample_rate_hz: u32,
    pub pitch_centipercent: i16,
    pub master_tempo: bool,
    pub scratch_active: bool,
    pub scratch_epoch: u32,
    pub censor_active: bool,
    pub pre_gain: f32,
    pub post_gain: f32,
    pub eq_low: u16,
    pub eq_mid: u16,
    pub eq_high: u16,
    pub filter_raw: u16,
    pub pfl_enabled: bool,
}

impl RenderDeckControl {
    pub const fn idle() -> Self {
        Self {
            source_epoch: 0,
            fx: DeckFxControl::bypass(),
            playing: false,
            stream_generation: 0,
            source_sample_rate_hz: RENDER_SAMPLE_RATE_HZ,
            pitch_centipercent: 0,
            master_tempo: false,
            scratch_active: false,
            scratch_epoch: 0,
            censor_active: false,
            pre_gain: 1.0,
            post_gain: 1.0,
            eq_low: pajoniiir_audio_dsp::EQ_RAW_CENTER,
            eq_mid: pajoniiir_audio_dsp::EQ_RAW_CENTER,
            eq_high: pajoniiir_audio_dsp::EQ_RAW_CENTER,
            filter_raw: pajoniiir_audio_dsp::FILTER_RAW_CENTER,
            pfl_enabled: false,
        }
    }

    pub const fn requires_deferred_source_arbitration(self) -> bool {
        self.master_tempo || self.scratch_active || self.censor_active
    }

    pub fn source_control(self) -> SourceControl {
        SourceControl {
            playing: self.playing,
            master_tempo: self.master_tempo,
            scratch: self.scratch_active,
            scratch_epoch: self.scratch_epoch,
            censor: self.censor_active,
            tempo_factor: tempo_factor_from_centipercent(self.pitch_centipercent),
            output_sample_rate_hz: RENDER_SAMPLE_RATE_HZ,
        }
    }
}

impl Default for RenderDeckControl {
    fn default() -> Self {
        Self::idle()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderControl {
    pub decks: [RenderDeckControl; 2],
    pub master_gain: f32,
}

impl RenderControl {
    pub const fn idle() -> Self {
        Self {
            decks: [RenderDeckControl::idle(), RenderDeckControl::idle()],
            master_gain: 1.0,
        }
    }
}

impl Default for RenderControl {
    fn default() -> Self {
        Self::idle()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOutputBlock {
    pub main: [PcmFrame; RENDER_BLOCK_FRAMES],
    pub pfl: [[PcmFrame; RENDER_BLOCK_FRAMES]; 2],
    pub len: u16,
}

impl RenderOutputBlock {
    pub const fn silence() -> Self {
        Self {
            main: [PcmFrame { left: 0, right: 0 }; RENDER_BLOCK_FRAMES],
            pfl: [[PcmFrame { left: 0, right: 0 }; RENDER_BLOCK_FRAMES]; 2],
            len: RENDER_BLOCK_FRAMES as u16,
        }
    }
}

impl Default for RenderOutputBlock {
    fn default() -> Self {
        Self::silence()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderBlockReport {
    pub rendered_frames: u16,
    pub source_frames_consumed: [u32; 2],
    pub underflow_frames: [u32; 2],
    pub deferred_mode_frames: [u32; 2],
    pub limiter: LimiterStats,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderHealth {
    pub rendered_blocks: u64,
    pub rendered_frames: u64,
    pub source_frames_consumed: [u64; 2],
    pub underflow_frames: [u64; 2],
    pub deferred_mode_frames: [u64; 2],
    pub limited_samples: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderDeckState {
    scope: Option<(u32, u32)>,
    resampler: ResamplerState,
    source_starved: bool,
    eq: EqState,
    filter: FilterState,
}

impl RenderDeckState {
    fn new() -> Self {
        Self {
            scope: None,
            resampler: ResamplerState::new(),
            source_starved: false,
            eq: EqState::new(RENDER_SAMPLE_RATE_HZ),
            filter: FilterState::new(RENDER_SAMPLE_RATE_HZ),
        }
    }

    fn reset(&mut self) {
        self.scope = None;
        self.resampler.reset();
        self.source_starved = false;
        self.eq.reset_filters();
        self.filter.reset();
    }
}

pub struct FixedRateRenderGraph<'a> {
    decks: [RenderDeckState; 2],
    health: RenderHealth,
    fx: Option<[DeckFx<'a>; 2]>,
}

impl<'a> FixedRateRenderGraph<'a> {
    /// Compatibility graph without an FX rack. Use `with_fx` for product FX.
    pub fn new() -> Self {
        Self {
            decks: [RenderDeckState::new(), RenderDeckState::new()],
            health: RenderHealth::default(),
            fx: None,
        }
    }

    /// Bind caller-owned effect memory for both decks before rendering starts.
    pub fn with_fx(storage: [&'a mut [f32]; 2]) -> Result<Self, FxBufferError> {
        let [a, b] = storage;
        let mut graph = Self::new();
        graph.fx = Some([DeckFx::new(a)?, DeckFx::new(b)?]);
        Ok(graph)
    }

    pub const fn has_fx_storage(&self) -> bool {
        self.fx.is_some()
    }

    pub fn reset_deck(&mut self, deck: usize) {
        if let Some(state) = self.decks.get_mut(deck) {
            state.reset();
        }
        if let Some(fx) = &mut self.fx
            && let Some(deck) = fx.get_mut(deck)
        {
            deck.reset();
        }
    }

    pub const fn health(&self) -> RenderHealth {
        self.health
    }

    pub fn render_block<F>(
        &mut self,
        control: RenderControl,
        output: &mut RenderOutputBlock,
        mut pop_source: F,
    ) -> RenderBlockReport
    where
        F: FnMut(usize) -> Option<PcmFrame>,
    {
        self.render_with(control, output, |deck, control, state| {
            if !control.playing {
                return SourceSample::default();
            }
            if control.requires_deferred_source_arbitration() {
                return SourceSample {
                    deferred: true,
                    ..SourceSample::default()
                };
            }
            let step = source_step_factor(
                control.source_sample_rate_hz,
                RENDER_SAMPLE_RATE_HZ,
                control.pitch_centipercent,
            );
            let mut missed = false;
            let mut received = false;
            let (pcm, consumed) = state.resampler.next(step, || {
                let frame = pop_source(deck);
                missed |= frame.is_none();
                received |= frame.is_some();
                frame
            });
            if missed {
                state.source_starved = true;
                state.resampler.silence_history();
            } else if received {
                state.source_starved = false;
            }
            SourceSample {
                frame: pcm,
                advanced: consumed,
                starved: state.source_starved,
                ..SourceSample::default()
            }
        })
    }

    /// Random-access live history path. The pop-only compatibility API above
    /// deliberately retains its guard for modes that need random access.
    pub fn render_canonical_block(
        &mut self,
        mut control: RenderControl,
        output: &mut RenderOutputBlock,
        sources: &mut [CanonicalSource<'_>; 2],
    ) -> RenderBlockReport {
        for (i, source) in sources.iter().enumerate() {
            if control.decks[i].stream_generation == 0
                || control.decks[i].stream_generation != source.generation()
                || source.faulted()
            {
                control.decks[i].source_epoch = 0; // revoke FX memory and tails too
            }
        }
        self.render_with(control, output, |deck, control, _| {
            if control.stream_generation == 0
                || control.stream_generation != sources[deck].generation()
            {
                let mut idle = control.source_control();
                idle.playing = false;
                idle.scratch = false;
                sources[deck].next_sample(idle);
                return SourceSample {
                    silent: true,
                    ..SourceSample::default()
                };
            }
            sources[deck].next_sample(control.source_control())
        })
    }

    fn render_with<F>(
        &mut self,
        control: RenderControl,
        output: &mut RenderOutputBlock,
        mut source: F,
    ) -> RenderBlockReport
    where
        F: FnMut(usize, RenderDeckControl, &mut RenderDeckState) -> SourceSample,
    {
        let mut report = RenderBlockReport {
            rendered_frames: RENDER_BLOCK_FRAMES as u16,
            ..RenderBlockReport::default()
        };
        output.len = RENDER_BLOCK_FRAMES as u16;

        for deck in 0..2 {
            let deck_control = control.decks[deck];
            let state = &mut self.decks[deck];
            let scope = (deck_control.source_epoch != 0 && deck_control.stream_generation != 0)
                .then_some((deck_control.source_epoch, deck_control.stream_generation));
            if self.fx.is_some() && state.scope != scope {
                state.reset();
                state.scope = scope;
            }
            state.eq.set_raw(
                deck_control.eq_low,
                deck_control.eq_mid,
                deck_control.eq_high,
            );
            state.filter.set_raw(deck_control.filter_raw);
            if let Some(fx) = &mut self.fx {
                fx[deck].configure(scope, deck_control.fx);
            }
        }

        for frame_index in 0..RENDER_BLOCK_FRAMES {
            let mut deck_frames = [DspFrame::default(); 2];

            for (deck, deck_frame) in deck_frames.iter_mut().enumerate() {
                let deck_control = control.decks[deck];
                output.pfl[deck][frame_index] = PcmFrame::default();

                let sample = source(deck, deck_control, &mut self.decks[deck]);
                report.source_frames_consumed[deck] =
                    report.source_frames_consumed[deck].saturating_add(sample.advanced);
                report.underflow_frames[deck] =
                    report.underflow_frames[deck].saturating_add(u32::from(sample.starved));
                report.deferred_mode_frames[deck] =
                    report.deferred_mode_frames[deck].saturating_add(u32::from(sample.deferred));
                let has_input = !((!deck_control.playing && !deck_control.scratch_active)
                    || sample.starved
                    || sample.deferred
                    || sample.silent);
                if !has_input && self.fx.is_none() {
                    continue;
                }

                let mut dsp = if has_input {
                    DspFrame::from(sample.frame)
                } else {
                    DspFrame::default()
                };
                let pre_gain = sanitize_gain(deck_control.pre_gain);
                dsp.left *= pre_gain;
                dsp.right *= pre_gain;
                dsp = self.decks[deck].eq.process_frame(dsp);
                dsp = self.decks[deck].filter.process_frame(true, dsp);
                if let Some(fx) = &mut self.fx {
                    dsp = fx[deck].process(dsp);
                }

                if deck_control.pfl_enabled {
                    output.pfl[deck][frame_index] = PcmFrame::from(dsp);
                }

                let post_gain = sanitize_gain(deck_control.post_gain);
                dsp.left *= post_gain;
                dsp.right *= post_gain;
                *deck_frame = dsp;
            }

            let master_gain = sanitize_gain(control.master_gain);
            let mixed_left = (deck_frames[0].left + deck_frames[1].left) * master_gain;
            let mixed_right = (deck_frames[0].right + deck_frames[1].right) * master_gain;
            output.main[frame_index] = PcmFrame {
                left: limit_main_sample(mixed_left, &mut report.limiter),
                right: limit_main_sample(mixed_right, &mut report.limiter),
            };
        }

        self.health.rendered_blocks = self.health.rendered_blocks.saturating_add(1);
        self.health.rendered_frames = self
            .health
            .rendered_frames
            .saturating_add(u64::from(report.rendered_frames));
        for deck in 0..2 {
            self.health.source_frames_consumed[deck] = self.health.source_frames_consumed[deck]
                .saturating_add(u64::from(report.source_frames_consumed[deck]));
            self.health.underflow_frames[deck] = self.health.underflow_frames[deck]
                .saturating_add(u64::from(report.underflow_frames[deck]));
            self.health.deferred_mode_frames[deck] = self.health.deferred_mode_frames[deck]
                .saturating_add(u64::from(report.deferred_mode_frames[deck]));
        }
        self.health.limited_samples = self
            .health
            .limited_samples
            .saturating_add(u64::from(report.limiter.limited_samples));

        report
    }
}

impl Default for FixedRateRenderGraph<'_> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn tempo_factor_from_centipercent(pitch_centipercent: i16) -> f32 {
    (1.0 + pitch_centipercent as f32 / 10_000.0).clamp(0.01, 16.0)
}

pub fn source_step_factor(
    source_sample_rate_hz: u32,
    output_sample_rate_hz: u32,
    pitch_centipercent: i16,
) -> f32 {
    if source_sample_rate_hz == 0 || output_sample_rate_hz == 0 {
        return 1.0;
    }
    (source_sample_rate_hz as f32 / output_sample_rate_hz as f32)
        * tempo_factor_from_centipercent(pitch_centipercent)
}

fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_nan() || gain <= 0.0 {
        0.0
    } else {
        gain.min(4.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceSink {
    frames: u64,
    digest: u64,
    peak_abs: i16,
}

impl ReferenceSink {
    pub const fn new() -> Self {
        Self {
            frames: 0,
            digest: 0xcbf2_9ce4_8422_2325,
            peak_abs: 0,
        }
    }

    pub fn write(&mut self, block: &RenderOutputBlock) {
        let len = usize::from(block.len).min(RENDER_BLOCK_FRAMES);
        for frame in &block.main[..len] {
            self.digest_sample(frame.left);
            self.digest_sample(frame.right);
            let peak = sample_abs(frame.left).max(sample_abs(frame.right));
            self.peak_abs = self.peak_abs.max(peak);
        }
        self.frames = self.frames.saturating_add(len as u64);
    }

    pub const fn frames(&self) -> u64 {
        self.frames
    }

    pub const fn digest(&self) -> u64 {
        self.digest
    }

    pub const fn peak_abs(&self) -> i16 {
        self.peak_abs
    }

    fn digest_sample(&mut self, sample: i16) {
        for byte in sample.to_le_bytes() {
            self.digest ^= u64::from(byte);
            self.digest = self.digest.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

impl Default for ReferenceSink {
    fn default() -> Self {
        Self::new()
    }
}

fn sample_abs(sample: i16) -> i16 {
    if sample == i16::MIN {
        i16::MAX
    } else {
        sample.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_audio_dsp::{EQ_RAW_CENTER, FILTER_RAW_CENTER};

    fn active_deck() -> RenderDeckControl {
        RenderDeckControl {
            source_epoch: 0,
            fx: DeckFxControl::bypass(),
            playing: true,
            stream_generation: 0,
            source_sample_rate_hz: RENDER_SAMPLE_RATE_HZ,
            pitch_centipercent: 0,
            master_tempo: false,
            scratch_active: false,
            scratch_epoch: 0,
            censor_active: false,
            pre_gain: 1.0,
            post_gain: 1.0,
            eq_low: EQ_RAW_CENTER,
            eq_mid: EQ_RAW_CENTER,
            eq_high: EQ_RAW_CENTER,
            filter_raw: FILTER_RAW_CENTER,
            pfl_enabled: false,
        }
    }

    #[test]
    fn idle_graph_is_deterministic_silence() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let report = graph.render_block(RenderControl::idle(), &mut output, |_| None);

        assert_eq!(report.rendered_frames as usize, RENDER_BLOCK_FRAMES);
        assert_eq!(report.underflow_frames, [0, 0]);
        assert!(
            output
                .main
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );

        let mut sink = ReferenceSink::new();
        sink.write(&output);
        assert_eq!(sink.frames(), RENDER_BLOCK_FRAMES as u64);
        assert_eq!(sink.peak_abs(), 0);
        assert_ne!(sink.digest(), 0);
    }

    #[test]
    fn neutral_single_deck_reaches_main_after_resampler_priming() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let control = RenderControl {
            decks: [active_deck(), RenderDeckControl::idle()],
            master_gain: 1.0,
        };
        let mut sequence = 0i16;
        let report = graph.render_block(control, &mut output, |deck| {
            if deck != 0 {
                return None;
            }
            sequence = sequence.saturating_add(100);
            Some(PcmFrame {
                left: sequence,
                right: -sequence,
            })
        });

        assert_eq!(report.source_frames_consumed[0], RENDER_BLOCK_FRAMES as u32);
        assert_eq!(report.underflow_frames[0], 0);
        assert_eq!(output.main[0], PcmFrame::default());
        assert_eq!(
            output.main[1],
            PcmFrame {
                left: 100,
                right: -100,
            }
        );
    }

    #[test]
    fn source_starvation_is_silence_and_counted_not_repeated_audio() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let control = RenderControl {
            decks: [active_deck(), RenderDeckControl::idle()],
            master_gain: 1.0,
        };
        let mut remaining = 4usize;
        let report = graph.render_block(control, &mut output, |deck| {
            if deck != 0 || remaining == 0 {
                return None;
            }
            remaining -= 1;
            Some(PcmFrame {
                left: 1_000,
                right: -1_000,
            })
        });

        assert!(report.underflow_frames[0] > 0);
        assert!(
            output.main[8..]
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );
    }

    #[test]
    fn sub_unity_starvation_stays_silent_on_main_and_pfl_across_blocks() {
        for (sample_rate, pitch) in [(24_000, 0), (44_100, 0), (48_000, -100), (48_000, -5_000)] {
            let mut graph = FixedRateRenderGraph::new();
            let mut output = RenderOutputBlock::default();
            let mut deck = active_deck();
            deck.source_sample_rate_hz = sample_rate;
            deck.pitch_centipercent = pitch;
            deck.pfl_enabled = true;
            let control = RenderControl {
                decks: [deck, RenderDeckControl::idle()],
                master_gain: 1.0,
            };
            let mut remaining = 4;
            graph.render_block(control, &mut output, |index| {
                if index != 0 || remaining == 0 {
                    return None;
                }
                remaining -= 1;
                Some(PcmFrame {
                    left: 1_000,
                    right: -1_000,
                })
            });
            assert!(
                output.main[16..]
                    .iter()
                    .all(|frame| *frame == PcmFrame::default()),
                "MAIN at {sample_rate}/{pitch}"
            );
            assert!(
                output.pfl[0][16..]
                    .iter()
                    .all(|frame| *frame == PcmFrame::default()),
                "PFL at {sample_rate}/{pitch}"
            );
            let report = graph.render_block(control, &mut output, |_| None);
            assert_eq!(report.underflow_frames, [RENDER_BLOCK_FRAMES as u32, 0]);
            assert!(
                output
                    .main
                    .iter()
                    .all(|frame| *frame == PcmFrame::default())
            );
            assert!(
                output.pfl[0]
                    .iter()
                    .all(|frame| *frame == PcmFrame::default())
            );
        }
    }

    #[test]
    fn sub_unity_starvation_recovers_with_new_samples_and_preserves_other_deck() {
        for sample_rate in [480, 24_000, 44_100] {
            let mut graph = FixedRateRenderGraph::new();
            let mut output = RenderOutputBlock::default();
            let mut one = active_deck();
            one.source_sample_rate_hz = sample_rate;
            one.pfl_enabled = true;
            let mut two = active_deck();
            two.pfl_enabled = true;
            let control = RenderControl {
                decks: [one, two],
                master_gain: 1.0,
            };
            // Prime with an old positive signal, then empty deck one's source.
            graph.render_block(control, &mut output, |_| {
                Some(PcmFrame {
                    left: 1_000,
                    right: 1_000,
                })
            });
            graph.render_block(control, &mut output, |index| {
                (index == 1).then_some(PcmFrame {
                    left: 200,
                    right: 200,
                })
            });
            let starved = graph.render_block(control, &mut output, |index| {
                (index == 1).then_some(PcmFrame {
                    left: 200,
                    right: 200,
                })
            });
            assert_eq!(starved.underflow_frames, [RENDER_BLOCK_FRAMES as u32, 0]);
            assert!(
                output.pfl[0]
                    .iter()
                    .all(|frame| *frame == PcmFrame::default())
            );
            assert!(output.pfl[1].iter().all(|frame| frame.left == 200));
            assert!(output.main.iter().all(|frame| frame.left == 200));

            let recovery = graph.render_block(control, &mut output, |index| {
                Some(if index == 0 {
                    PcmFrame {
                        left: -2_000,
                        right: -2_000,
                    }
                } else {
                    PcmFrame {
                        left: 200,
                        right: 200,
                    }
                })
            });
            assert!(
                recovery.source_frames_consumed[0] > 0,
                "source recovery at {sample_rate}"
            );
            assert!(
                output.pfl[0].iter().all(|frame| frame.left <= 0),
                "old positive audio at {sample_rate}"
            );
            assert!(
                output.pfl[0].iter().any(|frame| frame.left < 0),
                "silent recovery at {sample_rate}"
            );
            assert!(output.pfl[1].iter().all(|frame| frame.left == 200));
        }
    }

    #[test]
    fn reset_deck_discards_underflow_and_previous_source_history() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let mut deck = active_deck();
        deck.source_sample_rate_hz = 24_000;
        deck.pfl_enabled = true;
        let control = RenderControl {
            decks: [deck, RenderDeckControl::idle()],
            master_gain: 1.0,
        };
        graph.render_block(control, &mut output, |_| None);
        graph.reset_deck(0);
        let report = graph.render_block(control, &mut output, |index| {
            (index == 0).then_some(PcmFrame {
                left: -1_000,
                right: -1_000,
            })
        });
        assert_eq!(report.underflow_frames, [0, 0]);
        assert!(output.main.iter().any(|frame| frame.left < 0));
        assert!(output.main.iter().all(|frame| frame.left <= 0));
        assert_eq!(output.main, output.pfl[0]);
    }

    #[test]
    fn pfl_tap_is_before_channel_post_gain() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let mut deck = active_deck();
        deck.post_gain = 0.0;
        deck.pfl_enabled = true;
        let control = RenderControl {
            decks: [deck, RenderDeckControl::idle()],
            master_gain: 1.0,
        };

        let _ = graph.render_block(control, &mut output, |deck| {
            (deck == 0).then_some(PcmFrame {
                left: 2_000,
                right: -2_000,
            })
        });

        assert!(
            output
                .main
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );
        assert_eq!(
            output.pfl[0][1],
            PcmFrame {
                left: 2_000,
                right: -2_000,
            }
        );
    }

    #[test]
    fn master_limiter_is_main_only() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let mut one = active_deck();
        one.pfl_enabled = true;
        one.pre_gain = 2.0;
        let two = one;
        let control = RenderControl {
            decks: [one, two],
            master_gain: 2.0,
        };

        let report = graph.render_block(control, &mut output, |_| {
            Some(PcmFrame {
                left: 20_000,
                right: 20_000,
            })
        });

        assert!(report.limiter.limited_samples > 0);
        assert_ne!(output.main[1].left, 20_000);
        assert_eq!(output.pfl[0][1].left, i16::MAX);
    }

    #[test]
    fn master_tempo_and_scratch_fail_closed_until_arbitration_stage_exists() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let mut deck = active_deck();
        deck.master_tempo = true;
        let control = RenderControl {
            decks: [deck, RenderDeckControl::idle()],
            master_gain: 1.0,
        };

        let report = graph.render_block(control, &mut output, |_| {
            Some(PcmFrame {
                left: 12_000,
                right: -12_000,
            })
        });
        assert_eq!(report.deferred_mode_frames[0], RENDER_BLOCK_FRAMES as u32);
        assert!(
            output
                .main
                .iter()
                .all(|frame| *frame == PcmFrame::default())
        );

        deck.master_tempo = false;
        deck.scratch_active = true;
        let report = graph.render_block(
            RenderControl {
                decks: [deck, RenderDeckControl::idle()],
                master_gain: 1.0,
            },
            &mut output,
            |_| Some(PcmFrame { left: 1, right: 1 }),
        );
        assert_eq!(report.deferred_mode_frames[0], RENDER_BLOCK_FRAMES as u32);
    }

    #[test]
    fn source_rate_and_tempo_factor_are_explicit() {
        assert_eq!(tempo_factor_from_centipercent(0), 1.0);
        assert!((tempo_factor_from_centipercent(1_000) - 1.1).abs() < 0.0001);
        assert!((source_step_factor(44_100, 48_000, 0) - 0.91875).abs() < 0.0001);
    }

    #[test]
    fn reference_sink_digest_is_stable_for_same_render() {
        fn render_once() -> ReferenceSink {
            let mut graph = FixedRateRenderGraph::new();
            let mut output = RenderOutputBlock::default();
            let control = RenderControl {
                decks: [active_deck(), RenderDeckControl::idle()],
                master_gain: 1.0,
            };
            let mut sample = -4_000i16;
            let _ = graph.render_block(control, &mut output, |deck| {
                if deck != 0 {
                    return None;
                }
                sample = sample.saturating_add(31);
                Some(PcmFrame {
                    left: sample,
                    right: sample.wrapping_neg(),
                })
            });
            let mut sink = ReferenceSink::new();
            sink.write(&output);
            sink
        }

        let first = render_once();
        let second = render_once();
        assert_eq!(first, second);
        assert_ne!(first.digest(), ReferenceSink::new().digest());
    }
}
