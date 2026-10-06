use pajoniiir_audio_dsp::{
    DspFrame, EqState, FilterState, PcmFrame, ResamplerState,
};
use pajoniiir_mixer::{LimiterStats, limit_main_sample};

pub const RENDER_SAMPLE_RATE_HZ: u32 = 48_000;
pub const RENDER_BLOCK_FRAMES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderDeckControl {
    pub playing: bool,
    pub source_sample_rate_hz: u32,
    pub pitch_centipercent: i16,
    pub master_tempo: bool,
    pub scratch_active: bool,
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
            playing: false,
            source_sample_rate_hz: RENDER_SAMPLE_RATE_HZ,
            pitch_centipercent: 0,
            master_tempo: false,
            scratch_active: false,
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
        self.master_tempo || self.scratch_active
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
    resampler: ResamplerState,
    eq: EqState,
    filter: FilterState,
}

impl RenderDeckState {
    fn new() -> Self {
        Self {
            resampler: ResamplerState::new(),
            eq: EqState::new(RENDER_SAMPLE_RATE_HZ),
            filter: FilterState::new(RENDER_SAMPLE_RATE_HZ),
        }
    }

    fn reset(&mut self) {
        self.resampler.reset();
        self.eq.reset_filters();
        self.filter.reset();
    }
}

pub struct FixedRateRenderGraph {
    decks: [RenderDeckState; 2],
    health: RenderHealth,
}

impl FixedRateRenderGraph {
    pub fn new() -> Self {
        Self {
            decks: [RenderDeckState::new(), RenderDeckState::new()],
            health: RenderHealth::default(),
        }
    }

    pub fn reset_deck(&mut self, deck: usize) {
        if let Some(state) = self.decks.get_mut(deck) {
            state.reset();
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
        let mut report = RenderBlockReport {
            rendered_frames: RENDER_BLOCK_FRAMES as u16,
            ..RenderBlockReport::default()
        };
        output.len = RENDER_BLOCK_FRAMES as u16;

        for deck in 0..2 {
            let deck_control = control.decks[deck];
            let state = &mut self.decks[deck];
            state
                .eq
                .set_raw(deck_control.eq_low, deck_control.eq_mid, deck_control.eq_high);
            state.filter.set_raw(deck_control.filter_raw);
        }

        for frame_index in 0..RENDER_BLOCK_FRAMES {
            let mut deck_frames = [DspFrame::default(); 2];

            for deck in 0..2 {
                let deck_control = control.decks[deck];
                output.pfl[deck][frame_index] = PcmFrame::default();

                if !deck_control.playing {
                    continue;
                }

                if deck_control.requires_deferred_source_arbitration() {
                    report.deferred_mode_frames[deck] =
                        report.deferred_mode_frames[deck].saturating_add(1);
                    continue;
                }

                let step = source_step_factor(
                    deck_control.source_sample_rate_hz,
                    RENDER_SAMPLE_RATE_HZ,
                    deck_control.pitch_centipercent,
                );
                let mut missed = false;
                let (pcm, consumed) = self.decks[deck].resampler.next(step, || {
                    let frame = pop_source(deck);
                    if frame.is_none() {
                        missed = true;
                    }
                    frame
                });
                report.source_frames_consumed[deck] =
                    report.source_frames_consumed[deck].saturating_add(consumed);

                if missed {
                    report.underflow_frames[deck] =
                        report.underflow_frames[deck].saturating_add(1);
                    continue;
                }

                let mut dsp = DspFrame::from(pcm);
                let pre_gain = sanitize_gain(deck_control.pre_gain);
                dsp.left *= pre_gain;
                dsp.right *= pre_gain;
                dsp = self.decks[deck].eq.process_frame(dsp);
                dsp = self.decks[deck].filter.process_frame(true, dsp);

                if deck_control.pfl_enabled {
                    output.pfl[deck][frame_index] = PcmFrame::from(dsp);
                }

                let post_gain = sanitize_gain(deck_control.post_gain);
                dsp.left *= post_gain;
                dsp.right *= post_gain;
                deck_frames[deck] = dsp;
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

impl Default for FixedRateRenderGraph {
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
            playing: true,
            source_sample_rate_hz: RENDER_SAMPLE_RATE_HZ,
            pitch_centipercent: 0,
            master_tempo: false,
            scratch_active: false,
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
        assert!(output.main.iter().all(|frame| *frame == PcmFrame::default()));

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
        assert!(output.main[8..].iter().all(|frame| *frame == PcmFrame::default()));
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

        assert!(output.main.iter().all(|frame| *frame == PcmFrame::default()));
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
        assert!(output.main[1].left <= i16::MAX);
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
        assert!(output.main.iter().all(|frame| *frame == PcmFrame::default()));

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
