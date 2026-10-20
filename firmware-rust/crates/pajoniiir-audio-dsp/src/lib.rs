#![no_std]
#![forbid(unsafe_code)]

pub const MIXER_CONTROL_MAX: u16 = 16_383;
pub const MIXER_CONTROL_CENTER: u16 = 8_192;
pub const EQ_RAW_MIN: u16 = 0;
pub const EQ_RAW_CENTER: u16 = MIXER_CONTROL_CENTER;
pub const EQ_RAW_MAX: u16 = MIXER_CONTROL_MAX;

const EQ_LOW_CUTOFF_HZ: f32 = 800.0;
const EQ_HIGH_CUTOFF_HZ: f32 = 4_000.0;
const EQ_GAIN_BLOCK: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PcmFrame {
    pub left: i16,
    pub right: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DspFrame {
    pub left: f32,
    pub right: f32,
}

impl From<PcmFrame> for DspFrame {
    fn from(frame: PcmFrame) -> Self {
        Self {
            left: frame.left as f32,
            right: frame.right as f32,
        }
    }
}

impl From<DspFrame> for PcmFrame {
    fn from(frame: DspFrame) -> Self {
        Self {
            left: pcm_sample_from_float(frame.left),
            right: pcm_sample_from_float(frame.right),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EqBand {
    Low,
    Mid,
    High,
}

impl EqBand {
    const fn index(self) -> usize {
        match self {
            Self::Low => 0,
            Self::Mid => 1,
            Self::High => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqState {
    low_lp: [f32; 2],
    low_lp2: [f32; 2],
    high_lp: [f32; 2],
    high_lp2: [f32; 2],
    low_alpha: f32,
    high_alpha: f32,
    raw: [u16; 3],
    applied_raw: [u16; 3],
    gain: [f32; 3],
    gain_frames_left: u32,
}

impl EqState {
    pub fn new(sample_rate_hz: u32) -> Self {
        let mut state = Self {
            low_lp: [0.0; 2],
            low_lp2: [0.0; 2],
            high_lp: [0.0; 2],
            high_lp2: [0.0; 2],
            low_alpha: 0.0,
            high_alpha: 0.0,
            raw: [EQ_RAW_CENTER; 3],
            applied_raw: [EQ_RAW_CENTER; 3],
            gain: [1.0; 3],
            gain_frames_left: 0,
        };
        state.set_sample_rate(sample_rate_hz);
        state
    }

    pub fn set_sample_rate(&mut self, sample_rate_hz: u32) {
        self.low_alpha = one_pole_alpha(EQ_LOW_CUTOFF_HZ, sample_rate_hz);
        self.high_alpha = one_pole_alpha(EQ_HIGH_CUTOFF_HZ, sample_rate_hz);
    }

    pub fn reset_filters(&mut self) {
        self.low_lp = [0.0; 2];
        self.low_lp2 = [0.0; 2];
        self.high_lp = [0.0; 2];
        self.high_lp2 = [0.0; 2];
    }

    pub fn set_raw(&mut self, low: u16, mid: u16, high: u16) {
        self.set_band_raw(EqBand::Low, low);
        self.set_band_raw(EqBand::Mid, mid);
        self.set_band_raw(EqBand::High, high);
    }

    pub fn set_band_raw(&mut self, band: EqBand, raw: u16) {
        self.raw[band.index()] = raw.min(EQ_RAW_MAX);
    }

    pub fn band_raw(&self, band: EqBand) -> u16 {
        self.raw[band.index()]
    }

    pub fn process_frame(&mut self, input: DspFrame) -> DspFrame {
        if self.gain_frames_left == 0 {
            self.refresh_gains();
            self.gain_frames_left = EQ_GAIN_BLOCK;
        }
        self.gain_frames_left -= 1;

        DspFrame {
            left: self.process_sample(input.left, 0),
            right: self.process_sample(input.right, 1),
        }
    }

    pub fn process_pcm_frame(&mut self, input: PcmFrame) -> PcmFrame {
        self.process_frame(input.into()).into()
    }

    fn refresh_gains(&mut self) {
        for band in [EqBand::Low, EqBand::Mid, EqBand::High] {
            let index = band.index();
            let raw = self.raw[index];
            if raw != self.applied_raw[index] {
                self.applied_raw[index] = raw;
                self.gain[index] = eq_raw_to_gain(raw);
            }
        }
    }

    fn process_sample(&mut self, sample: f32, channel: usize) -> f32 {
        self.low_lp[channel] += self.low_alpha * (sample - self.low_lp[channel]);
        self.low_lp2[channel] += self.low_alpha * (self.low_lp[channel] - self.low_lp2[channel]);
        self.high_lp[channel] += self.high_alpha * (sample - self.high_lp[channel]);
        self.high_lp2[channel] +=
            self.high_alpha * (self.high_lp[channel] - self.high_lp2[channel]);

        let low = self.low_lp2[channel];
        let high = sample - self.high_lp2[channel];
        let mid = sample - low - high;

        (low * self.gain[EqBand::Low.index()])
            + (mid * self.gain[EqBand::Mid.index()])
            + (high * self.gain[EqBand::High.index()])
    }
}

const RESAMPLER_MIN_FACTOR: f32 = 0.01;
const RESAMPLER_MAX_FACTOR: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResamplerState {
    previous: PcmFrame,
    current: PcmFrame,
    phase_q32: u32,
    pitch_factor_bits: u32,
    step_q32: u64,
}

impl ResamplerState {
    pub const fn new() -> Self {
        Self {
            previous: PcmFrame { left: 0, right: 0 },
            current: PcmFrame { left: 0, right: 0 },
            phase_q32: 0,
            pitch_factor_bits: u32::MAX,
            step_q32: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Discard cached audio after source starvation without restarting the
    /// fractional phase. Keeping that phase lets sub-unity rates reach their
    /// next source read and recover even while repeated reads are empty.
    pub fn silence_history(&mut self) {
        self.previous = PcmFrame::default();
        self.current = PcmFrame::default();
    }

    pub fn next<F>(&mut self, pitch_factor: f32, mut pop_source: F) -> (PcmFrame, u32)
    where
        F: FnMut() -> Option<PcmFrame>,
    {
        let factor = sanitize_pitch_factor(pitch_factor);
        let factor_bits = factor.to_bits();
        if factor_bits != self.pitch_factor_bits {
            self.pitch_factor_bits = factor_bits;
            self.step_q32 = pitch_step_q32(factor_bits);
        }

        let phase = self.phase_q32 as u64 + self.step_q32;
        let mut source_frames = (phase >> 32) as u32;
        self.phase_q32 = phase as u32;
        let mut consumed = 0u32;

        while source_frames > 0 {
            source_frames -= 1;
            self.previous = self.current;
            if let Some(next) = pop_source() {
                self.current = next;
                consumed = consumed.saturating_add(1);
            }
        }

        let t = phase_fraction_float(self.phase_q32);
        let inv = 1.0 - t;
        (
            PcmFrame {
                left: (inv * self.previous.left as f32 + t * self.current.left as f32) as i16,
                right: (inv * self.previous.right as f32 + t * self.current.right as f32) as i16,
            },
            consumed,
        )
    }

    pub const fn phase_q32(&self) -> u32 {
        self.phase_q32
    }

    pub const fn step_q32(&self) -> u64 {
        self.step_q32
    }
}

impl Default for ResamplerState {
    fn default() -> Self {
        Self::new()
    }
}

fn sanitize_pitch_factor(factor: f32) -> f32 {
    if !factor.is_finite() {
        1.0
    } else {
        factor.clamp(RESAMPLER_MIN_FACTOR, RESAMPLER_MAX_FACTOR)
    }
}

fn pitch_step_q32(bits: u32) -> u64 {
    let mantissa = (1u32 << 23) | (bits & 0x7f_ff_ff);
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    (mantissa as u64) << (exponent + 9) as u32
}

fn phase_fraction_float(phase_q32: u32) -> f32 {
    f32::from_bits(0x3f80_0000 | (phase_q32 >> 9)) - 1.0
}

pub const CENSOR_EDGE_FADE_FRAMES: u32 = 64;
const CENSOR_Q32_SCALE: f32 = 4_294_967_296.0;
const CENSOR_MAX_STEP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CensorRender {
    pub frame: PcmFrame,
    pub reverse_gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CensorState {
    active: bool,
    releasing: bool,
    exhausted: bool,
    origin_seq: u64,
    distance_q32: u64,
    step_q32: u64,
    release_frames: u32,
    release_remaining: u32,
    edge_fade_remaining: u32,
    edge_hits: u32,
    last_reverse: PcmFrame,
}

impl CensorState {
    pub const fn new() -> Self {
        Self {
            active: false,
            releasing: false,
            exhausted: false,
            origin_seq: 0,
            distance_q32: 0,
            step_q32: 0,
            release_frames: 0,
            release_remaining: 0,
            edge_fade_remaining: 0,
            edge_hits: 0,
            last_reverse: PcmFrame { left: 0, right: 0 },
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn begin(
        &mut self,
        origin_seq: u64,
        source_sample_rate: u32,
        output_sample_rate: u32,
        speed_factor: f32,
        release_frames: u32,
    ) -> bool {
        if release_frames == 0 {
            return false;
        }
        let step_q32 = censor_step_q32(source_sample_rate, output_sample_rate, speed_factor);
        if step_q32 == 0 {
            return false;
        }

        *self = Self::new();
        self.active = true;
        self.origin_seq = origin_seq;
        self.step_q32 = step_q32;
        self.release_frames = release_frames;
        true
    }

    pub fn set_rate(
        &mut self,
        source_sample_rate: u32,
        output_sample_rate: u32,
        speed_factor: f32,
    ) {
        if !self.active {
            return;
        }
        let step_q32 = censor_step_q32(source_sample_rate, output_sample_rate, speed_factor);
        if step_q32 != 0 {
            self.step_q32 = step_q32;
        }
    }

    pub fn release(&mut self) {
        if !self.active || self.releasing {
            return;
        }
        self.releasing = true;
        self.release_remaining = self.release_frames;
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    pub const fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub const fn edge_hits(&self) -> u32 {
        self.edge_hits
    }

    pub fn render<F>(&mut self, mut read_frame: F) -> Option<CensorRender>
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        if !self.active {
            return None;
        }

        let gain = if self.releasing {
            if self.release_remaining == 0 {
                self.active = false;
                self.releasing = false;
                return None;
            }
            self.release_remaining as f32 / self.release_frames as f32
        } else {
            1.0
        };

        let mut frame = PcmFrame::default();
        if !self.exhausted {
            if let Some(reverse) = self.read_reverse(&mut read_frame) {
                frame = reverse;
                self.last_reverse = reverse;
            } else {
                self.exhausted = true;
                self.edge_hits = self.edge_hits.saturating_add(1);
                self.edge_fade_remaining = CENSOR_EDGE_FADE_FRAMES;
            }
        }

        if self.exhausted && frame == PcmFrame::default() && self.edge_fade_remaining > 0 {
            frame = PcmFrame {
                left: scale_sample(
                    self.last_reverse.left,
                    self.edge_fade_remaining,
                    CENSOR_EDGE_FADE_FRAMES,
                ),
                right: scale_sample(
                    self.last_reverse.right,
                    self.edge_fade_remaining,
                    CENSOR_EDGE_FADE_FRAMES,
                ),
            };
            self.edge_fade_remaining -= 1;
        }

        self.distance_q32 = self.distance_q32.wrapping_add(self.step_q32);

        if self.releasing && self.release_remaining > 0 {
            self.release_remaining -= 1;
            if self.release_remaining == 0 {
                self.active = false;
                self.releasing = false;
            }
        }

        Some(CensorRender {
            frame,
            reverse_gain: gain,
        })
    }

    fn read_reverse<F>(&self, read_frame: &mut F) -> Option<PcmFrame>
    where
        F: FnMut(u64) -> Option<PcmFrame>,
    {
        let whole = self.distance_q32 >> 32;
        let frac = self.distance_q32 as u32;
        if whole > self.origin_seq {
            return None;
        }

        let newer_seq = self.origin_seq - whole;
        let newer = read_frame(newer_seq)?;
        if frac == 0 || newer_seq == 0 {
            return Some(newer);
        }

        let Some(older) = read_frame(newer_seq - 1) else {
            return Some(newer);
        };
        Some(PcmFrame {
            left: interpolate_censor_sample(newer.left, older.left, frac),
            right: interpolate_censor_sample(newer.right, older.right, frac),
        })
    }
}

impl Default for CensorState {
    fn default() -> Self {
        Self::new()
    }
}

fn censor_step_q32(source_sample_rate: u32, output_sample_rate: u32, speed_factor: f32) -> u64 {
    if source_sample_rate == 0 || output_sample_rate == 0 {
        return 0;
    }

    let speed_factor = if speed_factor > 0.0 {
        speed_factor
    } else {
        1.0
    };
    let step = (source_sample_rate as f32 / output_sample_rate as f32) * speed_factor;
    let step = step.min(CENSOR_MAX_STEP);
    if step <= 0.0 {
        return 0;
    }
    (step * CENSOR_Q32_SCALE + 0.5) as u64
}

fn interpolate_censor_sample(newer: i16, older: i16, frac: u32) -> i16 {
    let delta = older as i64 - newer as i64;
    let value = newer as i64 + (delta * frac as i64) / 0x1_0000_0000i64;
    value.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

fn scale_sample(sample: i16, numerator: u32, denominator: u32) -> i16 {
    if denominator == 0 || numerator == 0 {
        return 0;
    }
    ((sample as i32 * numerator as i32) / denominator as i32) as i16
}

pub const FILTER_RAW_MIN: u16 = 0;
pub const FILTER_RAW_CENTER: u16 = MIXER_CONTROL_CENTER;
pub const FILTER_RAW_MAX: u16 = MIXER_CONTROL_MAX;

const FILTER_LP_MAX_HZ: f32 = 18_000.0;
const FILTER_HP_MIN_HZ: f32 = 20.0;
const FILTER_RES_K: f32 = 0.8;
const FILTER_CENTER_DEAD_RAW: u16 = 96;
const FILTER_SMOOTH_BLOCK: u32 = 32;
const FILTER_SMOOTH_COEF: f32 = 0.2;
const FILTER_SMOOTH_SNAP_RAW: f32 = 0.5;
const FILTER_LP_LOG_RATIO: f32 = -5.703_782_6;
const FILTER_HP_LOG_RATIO: f32 = 5.991_464_6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterState {
    raw: u16,
    sample_rate_hz: u32,
    smoothed_raw: f32,
    block_frames_left: u32,
    coefficients_dirty: bool,
    bypassed: bool,
    hp_mode: bool,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1eq: [f32; 2],
    ic2eq: [f32; 2],
}

impl FilterState {
    pub fn new(sample_rate_hz: u32) -> Self {
        let mut state = Self {
            raw: FILTER_RAW_CENTER,
            sample_rate_hz: 44_100,
            smoothed_raw: FILTER_RAW_CENTER as f32,
            block_frames_left: 0,
            coefficients_dirty: true,
            bypassed: true,
            hp_mode: false,
            k: FILTER_RES_K,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1eq: [0.0; 2],
            ic2eq: [0.0; 2],
        };
        state.set_sample_rate(sample_rate_hz);
        state.reset();
        state
    }

    pub fn reset(&mut self) {
        self.ic1eq = [0.0; 2];
        self.ic2eq = [0.0; 2];
        self.smoothed_raw = self.raw as f32;
        self.block_frames_left = 0;
        self.coefficients_dirty = true;
        self.bypassed = true;
    }

    pub fn set_sample_rate(&mut self, sample_rate_hz: u32) {
        let next = if sample_rate_hz == 0 {
            44_100
        } else {
            sample_rate_hz
        };
        if next != self.sample_rate_hz {
            self.sample_rate_hz = next;
            self.coefficients_dirty = true;
            self.block_frames_left = 0;
        }
    }

    pub fn set_raw(&mut self, raw: u16) {
        self.raw = raw.min(FILTER_RAW_MAX);
    }

    pub const fn raw(&self) -> u16 {
        self.raw
    }

    pub fn process_frame(&mut self, enabled: bool, input: DspFrame) -> DspFrame {
        if !enabled {
            return input;
        }

        if self.block_frames_left == 0 {
            self.block_frames_left = FILTER_SMOOTH_BLOCK;
            self.update_coefficients();
        }
        self.block_frames_left -= 1;

        if self.bypassed {
            return input;
        }

        DspFrame {
            left: self.svf_process(input.left, 0),
            right: self.svf_process(input.right, 1),
        }
    }

    pub fn process_pcm_frame(&mut self, enabled: bool, input: PcmFrame) -> PcmFrame {
        self.process_frame(enabled, input.into()).into()
    }

    fn update_coefficients(&mut self) {
        let target_raw = self.raw;
        let target = target_raw as f32;
        let movement = target - self.smoothed_raw;
        let mut position_changed = false;

        if movement.abs() <= FILTER_SMOOTH_SNAP_RAW {
            if self.smoothed_raw != target {
                self.smoothed_raw = target;
                position_changed = true;
            }
        } else {
            self.smoothed_raw += movement * FILTER_SMOOTH_COEF;
            position_changed = true;
        }

        let delta = self.smoothed_raw - FILTER_RAW_CENTER as f32;
        let mag = delta.abs();
        let raw_dist = target_raw.abs_diff(FILTER_RAW_CENTER);
        if raw_dist <= FILTER_CENTER_DEAD_RAW && mag <= FILTER_CENTER_DEAD_RAW as f32 {
            self.bypassed = true;
            return;
        }

        let was_bypassed = self.bypassed;
        let next_hp_mode = delta >= 0.0;
        self.bypassed = false;

        if !position_changed
            && !self.coefficients_dirty
            && !was_bypassed
            && self.hp_mode == next_hp_mode
        {
            return;
        }

        let intensity = (mag / FILTER_RAW_CENTER as f32).min(1.0);
        let cutoff = if delta < 0.0 {
            self.hp_mode = false;
            FILTER_LP_MAX_HZ * libm::expf(FILTER_LP_LOG_RATIO * intensity)
        } else {
            self.hp_mode = true;
            FILTER_HP_MIN_HZ * libm::expf(FILTER_HP_LOG_RATIO * intensity)
        };

        let fs = self.sample_rate_hz as f32;
        let cutoff = cutoff.min(0.45 * fs);
        let g = libm::tanf(core::f32::consts::PI * cutoff / fs);

        self.k = FILTER_RES_K;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.coefficients_dirty = false;
    }

    fn svf_process(&mut self, sample: f32, channel: usize) -> f32 {
        let v3 = sample - self.ic2eq[channel];
        let v1 = self.a1 * self.ic1eq[channel] + self.a2 * v3;
        let v2 = self.ic2eq[channel] + self.a2 * self.ic1eq[channel] + self.a3 * v3;

        self.ic1eq[channel] = 2.0 * v1 - self.ic1eq[channel];
        self.ic2eq[channel] = 2.0 * v2 - self.ic2eq[channel];

        if self.hp_mode {
            sample - self.k * v1 - v2
        } else {
            v2
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelayMode {
    Echo,
    Delay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelayConfig {
    pub enabled: bool,
    pub mode: DelayMode,
    pub delay_ms: u32,
    pub wet_q15: u16,
    pub feedback_q15: u16,
}

impl Default for DelayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: DelayMode::Echo,
            delay_ms: 0,
            wet_q15: 0,
            feedback_q15: 0,
        }
    }
}

const DELAY_DAMP_OMEGA: u32 = 28_274;
const DELAY_TAIL_SECONDS: u32 = 2;
const DELAY_SMOOTH_SHIFT: i32 = 6;

pub struct DelayFx<'a> {
    left: &'a mut [f32],
    right: &'a mut [f32],
    capacity_frames: usize,
    sample_rate: u32,
    write_index: usize,
    delay_frames: usize,
    config: DelayConfig,
    allocated: bool,
    fb_lp: [f32; 2],
    damp_alpha_q15: u16,
    wet_cur_q15: u16,
    feedback_cur_q15: u16,
    tail_frames_remaining: u32,
}

impl<'a> DelayFx<'a> {
    pub fn new(left: &'a mut [f32], right: &'a mut [f32], sample_rate: u32) -> Self {
        let capacity_frames = left.len().min(right.len());
        let allocated = capacity_frames > 1 && sample_rate > 0;
        let mut state = Self {
            left,
            right,
            capacity_frames,
            sample_rate,
            write_index: 0,
            delay_frames: 0,
            config: DelayConfig::default(),
            allocated,
            fb_lp: [0.0; 2],
            damp_alpha_q15: 0,
            wet_cur_q15: 0,
            feedback_cur_q15: 0,
            tail_frames_remaining: 0,
        };
        state.reset();
        state
    }

    pub fn reset(&mut self) {
        self.write_index = 0;
        self.fb_lp = [0.0; 2];
        self.wet_cur_q15 = 0;
        self.feedback_cur_q15 = 0;
        self.tail_frames_remaining = 0;
        for sample in &mut self.left[..self.capacity_frames] {
            *sample = 0.0;
        }
        for sample in &mut self.right[..self.capacity_frames] {
            *sample = 0.0;
        }
    }

    pub fn configure(&mut self, config: DelayConfig) {
        let was_enabled = self.config.enabled;
        let was_ringing = self.tail_frames_remaining > 0;
        let previous_mode = self.config.mode;

        let mut next = config;
        next.wet_q15 = next.wet_q15.min(32_767);
        next.feedback_q15 = next.feedback_q15.min(24_576);
        if next.mode == DelayMode::Delay {
            next.feedback_q15 = 0;
        }

        if !next.enabled && (was_enabled || was_ringing) {
            next.mode = self.config.mode;
            next.delay_ms = self.config.delay_ms;
            next.wet_q15 = self.config.wet_q15;
            next.feedback_q15 = self.config.feedback_q15;
        }

        if next.enabled && (!was_enabled || next.mode != previous_mode) {
            self.reset();
            self.wet_cur_q15 = next.wet_q15;
            self.feedback_cur_q15 = next.feedback_q15;
        } else if !next.enabled && was_enabled && self.allocated {
            self.tail_frames_remaining = if previous_mode == DelayMode::Delay {
                self.delay_frames as u32
            } else {
                self.sample_rate.saturating_mul(DELAY_TAIL_SECONDS)
            };
        }

        self.config = next;

        let mut frames =
            (self.sample_rate as u64 * self.config.delay_ms as u64).div_ceil(1_000) as usize;
        frames = frames.max(1);
        if self.capacity_frames > 0 && frames >= self.capacity_frames {
            frames = self.capacity_frames - 1;
        }
        self.delay_frames = frames;

        let fs = if self.sample_rate == 0 {
            44_100
        } else {
            self.sample_rate
        };
        self.damp_alpha_q15 = ((32_768u32 * DELAY_DAMP_OMEGA) / (DELAY_DAMP_OMEGA + fs)) as u16;
    }

    pub const fn config(&self) -> DelayConfig {
        self.config
    }

    pub const fn is_allocated(&self) -> bool {
        self.allocated
    }

    pub const fn is_ringing(&self) -> bool {
        self.tail_frames_remaining > 0
    }

    pub const fn delay_ms(&self) -> u32 {
        self.config.delay_ms
    }

    pub const fn delay_frames(&self) -> usize {
        self.delay_frames
    }

    pub const fn tail_frames_remaining(&self) -> u32 {
        self.tail_frames_remaining
    }

    pub fn process_frame(&mut self, input: DspFrame) -> DspFrame {
        if !self.allocated || self.delay_frames == 0 {
            return input;
        }

        let active = self.config.enabled;
        let ringing = self.tail_frames_remaining > 0;
        if !active && !ringing {
            return input;
        }

        self.wet_cur_q15 = smooth_q15(self.wet_cur_q15, self.config.wet_q15);
        self.feedback_cur_q15 = smooth_q15(self.feedback_cur_q15, self.config.feedback_q15);

        let read_index = if self.write_index >= self.delay_frames {
            self.write_index - self.delay_frames
        } else {
            self.write_index + self.capacity_frames - self.delay_frames
        };

        let delayed_l = self.left[read_index];
        let delayed_r = self.right[read_index];

        let out_l = input.left + q15_mul_float(delayed_l, self.wet_cur_q15);
        let out_r = input.right + q15_mul_float(delayed_r, self.wet_cur_q15);

        let fb_l = q15_mul_float(
            self.damp_feedback_sample(delayed_l, 0),
            self.feedback_cur_q15,
        );
        let fb_r = q15_mul_float(
            self.damp_feedback_sample(delayed_r, 1),
            self.feedback_cur_q15,
        );

        self.left[self.write_index] = if active { input.left + fb_l } else { fb_l };
        self.right[self.write_index] = if active { input.right + fb_r } else { fb_r };

        self.write_index += 1;
        if self.write_index >= self.capacity_frames {
            self.write_index = 0;
        }

        if !active && ringing {
            self.tail_frames_remaining -= 1;
        }

        DspFrame {
            left: out_l,
            right: out_r,
        }
    }

    pub fn process_pcm_frame(&mut self, input: PcmFrame) -> PcmFrame {
        self.process_frame(input.into()).into()
    }

    fn damp_feedback_sample(&mut self, delayed: f32, channel: usize) -> f32 {
        self.fb_lp[channel] +=
            (delayed - self.fb_lp[channel]) * (self.damp_alpha_q15 as f32 / 32_768.0);
        self.fb_lp[channel]
    }
}

fn q15_mul_float(sample: f32, gain_q15: u16) -> f32 {
    sample * (gain_q15 as f32 / 32_768.0)
}

fn smooth_q15(current: u16, target: u16) -> u16 {
    let delta = target as i32 - current as i32;
    let mut step = delta / (1 << DELAY_SMOOTH_SHIFT);
    if step == 0 && delta != 0 {
        step = if delta > 0 { 1 } else { -1 };
    }
    (current as i32 + step) as u16
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlangerConfig {
    pub enabled: bool,
    pub period_ms: u32,
    pub depth_q15: u16,
}

impl Default for FlangerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            period_ms: 500,
            depth_q15: 0,
        }
    }
}

const FLANGER_MIN_DELAY_US: u32 = 250;
const FLANGER_MAX_DELAY_US: u32 = 6_000;
const FLANGER_MIN_PERIOD_MS: u32 = 100;
const FLANGER_MAX_PERIOD_MS: u32 = 8_000;
const FLANGER_WET_MAX_Q15: u16 = 22_938;
const FLANGER_FB_MAX_Q15: u16 = 24_576;

pub fn flanger_required_frames(sample_rate: u32) -> usize {
    let max_frames =
        (sample_rate as u64 * FLANGER_MAX_DELAY_US as u64).div_ceil(1_000_000) as usize;
    max_frames.saturating_add(4)
}

pub struct FlangerFx<'a> {
    left: &'a mut [f32],
    right: &'a mut [f32],
    capacity_frames: usize,
    sample_rate: u32,
    write_index: usize,
    config: FlangerConfig,
    allocated: bool,
    lfo_phase_q32: u32,
    lfo_step_q32: u32,
    min_delay_q16: u32,
    span_delay_q16: u32,
    wet_cur_q15: u16,
    feedback_cur_q15: u16,
}

impl<'a> FlangerFx<'a> {
    pub fn new(left: &'a mut [f32], right: &'a mut [f32], sample_rate: u32) -> Self {
        let capacity_frames = left.len().min(right.len());
        let allocated = sample_rate > 0 && capacity_frames >= flanger_required_frames(sample_rate);

        let mut state = Self {
            left,
            right,
            capacity_frames,
            sample_rate,
            write_index: 0,
            config: FlangerConfig::default(),
            allocated,
            lfo_phase_q32: 0,
            lfo_step_q32: 0,
            min_delay_q16: 0,
            span_delay_q16: 0,
            wet_cur_q15: 0,
            feedback_cur_q15: 0,
        };
        state.reset();
        state
    }

    pub fn reset(&mut self) {
        self.write_index = 0;
        self.lfo_phase_q32 = 0;
        self.wet_cur_q15 = 0;
        self.feedback_cur_q15 = 0;
        for sample in &mut self.left[..self.capacity_frames] {
            *sample = 0.0;
        }
        for sample in &mut self.right[..self.capacity_frames] {
            *sample = 0.0;
        }
    }

    pub fn configure(&mut self, config: FlangerConfig) {
        let was_enabled = self.config.enabled;
        let mut next = config;
        next.depth_q15 = next.depth_q15.min(32_767);
        next.period_ms = next
            .period_ms
            .clamp(FLANGER_MIN_PERIOD_MS, FLANGER_MAX_PERIOD_MS);

        if next.enabled && !was_enabled {
            self.reset();
            self.wet_cur_q15 = ((next.depth_q15 as u32 * FLANGER_WET_MAX_Q15 as u32) >> 15) as u16;
            self.feedback_cur_q15 =
                ((next.depth_q15 as u32 * FLANGER_FB_MAX_Q15 as u32) >> 15) as u16;
        }

        self.config = next;

        let fs = if self.sample_rate == 0 {
            44_100
        } else {
            self.sample_rate
        };
        let period_frames = ((fs as u64 * self.config.period_ms as u64) / 1_000).max(1);
        self.lfo_step_q32 = ((1u64 << 32) / period_frames) as u32;

        let min_q16 = ((fs as u64 * FLANGER_MIN_DELAY_US as u64) << 16) / 1_000_000;
        let max_q16 = ((fs as u64 * FLANGER_MAX_DELAY_US as u64) << 16) / 1_000_000;
        self.min_delay_q16 = min_q16 as u32;
        self.span_delay_q16 = (max_q16 - min_q16) as u32;
    }

    pub const fn config(&self) -> FlangerConfig {
        self.config
    }

    pub const fn is_allocated(&self) -> bool {
        self.allocated
    }

    pub fn process_frame(&mut self, input: DspFrame) -> DspFrame {
        if !self.allocated || !self.config.enabled {
            return input;
        }

        let wet_target = ((self.config.depth_q15 as u32 * FLANGER_WET_MAX_Q15 as u32) >> 15) as u16;
        let feedback_target =
            ((self.config.depth_q15 as u32 * FLANGER_FB_MAX_Q15 as u32) >> 15) as u16;
        self.wet_cur_q15 = smooth_q15(self.wet_cur_q15, wet_target);
        self.feedback_cur_q15 = smooth_q15(self.feedback_cur_q15, feedback_target);

        self.lfo_phase_q32 = self.lfo_phase_q32.wrapping_add(self.lfo_step_q32);
        let phase = self.lfo_phase_q32;
        let tri_q16 = if phase < 0x8000_0000 {
            phase >> 15
        } else {
            (u32::MAX - phase) >> 15
        };

        let delay_q16 =
            self.min_delay_q16 + (((self.span_delay_q16 as u64 * tri_q16 as u64) >> 16) as u32);
        let delay_int = (delay_q16 >> 16) as usize;
        let frac_q16 = delay_q16 & 0xffff;

        let idx0 = if self.write_index >= delay_int {
            self.write_index - delay_int
        } else {
            self.write_index + self.capacity_frames - delay_int
        };
        let idx1 = if idx0 == 0 {
            self.capacity_frames - 1
        } else {
            idx0 - 1
        };

        let delayed_l = read_delayed(&*self.left, idx0, idx1, frac_q16);
        let delayed_r = read_delayed(&*self.right, idx0, idx1, frac_q16);

        let wet_gain = self.wet_cur_q15 as f32 / 32_768.0;
        let feedback_gain = self.feedback_cur_q15 as f32 / 32_768.0;

        let out_l = input.left + delayed_l * wet_gain;
        let out_r = input.right + delayed_r * wet_gain;

        self.left[self.write_index] = input.left + delayed_l * feedback_gain;
        self.right[self.write_index] = input.right + delayed_r * feedback_gain;

        self.write_index += 1;
        if self.write_index >= self.capacity_frames {
            self.write_index = 0;
        }

        DspFrame {
            left: out_l,
            right: out_r,
        }
    }

    pub fn process_pcm_frame(&mut self, input: PcmFrame) -> PcmFrame {
        self.process_frame(input.into()).into()
    }
}

fn read_delayed(buffer: &[f32], idx0: usize, idx1: usize, frac_q16: u32) -> f32 {
    let fraction = frac_q16 as f32 / 65_536.0;
    buffer[idx0] + (buffer[idx1] - buffer[idx0]) * fraction
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadFxMode {
    PadFx1,
    PadFx2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadFxKind {
    None,
    Filter,
    Echo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PadFxConfig {
    pub mode: PadFxMode,
    pub pad: u8,
    pub active: bool,
}

impl Default for PadFxConfig {
    fn default() -> Self {
        Self {
            mode: PadFxMode::PadFx1,
            pad: 0,
            active: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PadFxPreset {
    kind: PadFxKind,
    filter_raw: u16,
    echo_delay_ms: u32,
    echo_wet_q15: u16,
    echo_feedback_q15: u16,
}

const fn pad_fx_preset(mode: PadFxMode, pad: u8) -> PadFxPreset {
    match (mode, pad) {
        (PadFxMode::PadFx2, 0) => PadFxPreset {
            kind: PadFxKind::Filter,
            filter_raw: 2_300,
            echo_delay_ms: 0,
            echo_wet_q15: 0,
            echo_feedback_q15: 0,
        },
        (PadFxMode::PadFx2, 1) => PadFxPreset {
            kind: PadFxKind::Filter,
            filter_raw: 14_000,
            echo_delay_ms: 0,
            echo_wet_q15: 0,
            echo_feedback_q15: 0,
        },
        (PadFxMode::PadFx2, 2) => PadFxPreset {
            kind: PadFxKind::Echo,
            filter_raw: FILTER_RAW_CENTER,
            echo_delay_ms: 125,
            echo_wet_q15: 9_830,
            echo_feedback_q15: 9_830,
        },
        (PadFxMode::PadFx2, 3) => PadFxPreset {
            kind: PadFxKind::Echo,
            filter_raw: FILTER_RAW_CENTER,
            echo_delay_ms: 1_000,
            echo_wet_q15: 9_830,
            echo_feedback_q15: 13_107,
        },
        (PadFxMode::PadFx1, 0) => PadFxPreset {
            kind: PadFxKind::Filter,
            filter_raw: 3_600,
            echo_delay_ms: 0,
            echo_wet_q15: 0,
            echo_feedback_q15: 0,
        },
        (PadFxMode::PadFx1, 1) => PadFxPreset {
            kind: PadFxKind::Filter,
            filter_raw: 12_700,
            echo_delay_ms: 0,
            echo_wet_q15: 0,
            echo_feedback_q15: 0,
        },
        (PadFxMode::PadFx1, 2) => PadFxPreset {
            kind: PadFxKind::Echo,
            filter_raw: FILTER_RAW_CENTER,
            echo_delay_ms: 250,
            echo_wet_q15: 8_192,
            echo_feedback_q15: 9_830,
        },
        (PadFxMode::PadFx1, 3) => PadFxPreset {
            kind: PadFxKind::Echo,
            filter_raw: FILTER_RAW_CENTER,
            echo_delay_ms: 500,
            echo_wet_q15: 8_192,
            echo_feedback_q15: 11_469,
        },
        _ => PadFxPreset {
            kind: PadFxKind::None,
            filter_raw: FILTER_RAW_CENTER,
            echo_delay_ms: 0,
            echo_wet_q15: 0,
            echo_feedback_q15: 0,
        },
    }
}

pub struct PadFx<'a> {
    filter: FilterState,
    echo: DelayFx<'a>,
    sample_rate: u32,
    config: PadFxConfig,
    kind: PadFxKind,
    echo_tail_frames_remaining: u32,
    active: bool,
    echo_tail_active: bool,
}

impl<'a> PadFx<'a> {
    pub fn new(sample_rate_hz: u32, echo_left: &'a mut [f32], echo_right: &'a mut [f32]) -> Self {
        let sample_rate = if sample_rate_hz == 0 {
            44_100
        } else {
            sample_rate_hz
        };
        Self {
            filter: FilterState::new(sample_rate),
            echo: DelayFx::new(echo_left, echo_right, sample_rate),
            sample_rate,
            config: PadFxConfig::default(),
            kind: PadFxKind::None,
            echo_tail_frames_remaining: 0,
            active: false,
            echo_tail_active: false,
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.echo_tail_active = false;
        self.echo_tail_frames_remaining = 0;
        self.kind = PadFxKind::None;
        self.config.active = false;
        self.filter.set_raw(FILTER_RAW_CENTER);
        self.filter.reset();
        self.echo.configure(DelayConfig::default());
        self.echo.reset();
    }

    pub fn set(&mut self, config: PadFxConfig) {
        if !config.active {
            if self.active && self.config.mode == config.mode && self.config.pad == config.pad {
                if self.kind == PadFxKind::Echo && self.echo.is_allocated() {
                    self.active = false;
                    self.config.active = false;
                    self.echo_tail_active = true;
                    self.echo_tail_frames_remaining = self.sample_rate.saturating_mul(2);
                    return;
                }
                self.reset();
            }
            return;
        }

        let preset = pad_fx_preset(config.mode, config.pad);
        self.config = config;
        self.kind = preset.kind;
        self.active = preset.kind != PadFxKind::None;
        self.config.active = self.active;
        self.echo_tail_active = false;
        self.echo_tail_frames_remaining = 0;

        match preset.kind {
            PadFxKind::Filter => {
                self.filter.set_raw(preset.filter_raw);
                self.echo.configure(DelayConfig::default());
            }
            PadFxKind::Echo => {
                self.filter.set_raw(FILTER_RAW_CENTER);
                self.filter.reset();
                self.echo.configure(DelayConfig {
                    enabled: self.echo.is_allocated(),
                    mode: DelayMode::Echo,
                    delay_ms: preset.echo_delay_ms,
                    wet_q15: preset.echo_wet_q15,
                    feedback_q15: preset.echo_feedback_q15,
                });
            }
            PadFxKind::None => self.reset(),
        }
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }
    pub const fn kind(&self) -> PadFxKind {
        self.kind
    }
    pub const fn config(&self) -> PadFxConfig {
        self.config
    }
    pub const fn echo_tail_active(&self) -> bool {
        self.echo_tail_active
    }

    pub fn process_frame(&mut self, input: DspFrame) -> DspFrame {
        if self.active && self.kind == PadFxKind::Filter {
            return self.filter.process_frame(true, input);
        }
        if self.active && self.kind == PadFxKind::Echo {
            return self.echo.process_frame(input);
        }
        if self.echo_tail_active {
            let tail = self.echo.process_frame(DspFrame::default());
            if self.echo_tail_frames_remaining > 0 {
                self.echo_tail_frames_remaining -= 1;
            }
            if self.echo_tail_frames_remaining == 0 {
                self.reset();
            }
            return DspFrame {
                left: input.left + tail.left,
                right: input.right + tail.right,
            };
        }
        input
    }

    pub fn process_pcm_frame(&mut self, input: PcmFrame) -> PcmFrame {
        self.process_frame(input.into()).into()
    }
}

pub fn smart_cfx_curve_raw(raw: u16) -> u16 {
    let raw = raw.min(FILTER_RAW_MAX);
    if raw == FILTER_RAW_CENTER || raw == FILTER_RAW_MIN || raw == FILTER_RAW_MAX {
        return raw;
    }

    let delta = raw as i32 - FILTER_RAW_CENTER as i32;
    let sign = if delta < 0 { -1 } else { 1 };
    let mag = delta.unsigned_abs().min(FILTER_RAW_CENTER as u32);
    let max = FILTER_RAW_CENTER as u32;

    let numerator = mag as u64 * mag as u64 * (3 * max - 2 * mag) as u64;
    let denominator = max as u64 * max as u64;
    let mut curved = ((numerator + denominator / 2) / denominator) as u32;

    let min_audible = max / 24;
    if mag > min_audible && curved < min_audible {
        curved = min_audible;
    }

    let out = FILTER_RAW_CENTER as i32 + sign * curved as i32;
    out.clamp(FILTER_RAW_MIN as i32, FILTER_RAW_MAX as i32) as u16
}

pub fn eq_raw_to_gain(raw: u16) -> f32 {
    let raw = raw.min(EQ_RAW_MAX);
    if raw <= EQ_RAW_CENTER {
        raw as f32 / EQ_RAW_CENTER as f32
    } else {
        1.0 + (raw - EQ_RAW_CENTER) as f32 / (EQ_RAW_MAX - EQ_RAW_CENTER) as f32
    }
}

pub fn pcm_sample_from_float(sample: f32) -> i16 {
    if sample.is_nan() {
        0
    } else if sample > i16::MAX as f32 {
        i16::MAX
    } else if sample < i16::MIN as f32 {
        i16::MIN
    } else if sample >= 0.0 {
        (sample + 0.5) as i16
    } else {
        (sample - 0.5) as i16
    }
}

fn one_pole_alpha(cutoff_hz: f32, sample_rate_hz: u32) -> f32 {
    let sample_rate_hz = if sample_rate_hz == 0 {
        44_100
    } else {
        sample_rate_hz
    };
    let omega = 2.0 * core::f32::consts::PI * cutoff_hz;
    omega / (omega + sample_rate_hz as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::PI;

    const SAMPLE_RATE: u32 = 44_100;
    const TEST_FRAMES: u32 = 44_100;

    fn rms_of_sine_after_eq(freq_hz: f32, low: u16, mid: u16, high: u16) -> f32 {
        let mut eq = EqState::new(SAMPLE_RATE);
        eq.set_raw(low, mid, high);

        let mut sum_sq = 0.0f64;
        for i in 0..TEST_FRAMES {
            let phase = 2.0 * PI * freq_hz * i as f32 / SAMPLE_RATE as f32;
            let sample = (phase.sin() * 12_000.0) as i16;
            let out = eq.process_pcm_frame(PcmFrame {
                left: sample,
                right: sample,
            });
            let normalized = out.left as f32 / 32_768.0;
            sum_sq += (normalized as f64) * (normalized as f64);
        }

        (sum_sq / TEST_FRAMES as f64).sqrt() as f32
    }

    fn peak_of_sine_after_wide_eq(freq_hz: f32, band: EqBand) -> f32 {
        let mut eq = EqState::new(SAMPLE_RATE);
        eq.set_band_raw(band, EQ_RAW_MAX);

        let mut peak = 0.0f32;
        for i in 0..TEST_FRAMES {
            let phase = 2.0 * PI * freq_hz * i as f32 / SAMPLE_RATE as f32;
            let sample = phase.sin() * 25_000.0;
            let out = eq.process_frame(DspFrame {
                left: sample,
                right: sample,
            });
            assert!(out.left.is_finite());
            peak = peak.max(out.left.abs());
        }
        peak
    }

    fn rms_after_filter(freq_hz: f32, raw: u16, enabled: bool) -> f32 {
        let mut filter = FilterState::new(SAMPLE_RATE);
        filter.set_raw(raw);

        let mut sum_sq = 0.0f64;
        for i in 0..TEST_FRAMES {
            let phase = 2.0 * PI * freq_hz * i as f32 / SAMPLE_RATE as f32;
            let sample = (libm::sinf(phase) * 12_000.0) as i16;
            let out = filter.process_pcm_frame(
                enabled,
                PcmFrame {
                    left: sample,
                    right: sample,
                },
            );
            let normalized = out.left as f32 / 32_768.0;
            sum_sq += (normalized as f64) * (normalized as f64);
        }

        libm::sqrt(sum_sq / TEST_FRAMES as f64) as f32
    }

    fn settle_filter(filter: &mut FilterState, raw: u16) {
        filter.set_raw(raw);
        for _ in 0..SAMPLE_RATE {
            filter.process_frame(true, DspFrame::default());
        }
    }

    fn programmed_cutoff_hz(filter: &FilterState) -> f32 {
        let g = filter.a2 / filter.a1;
        libm::atanf(g) * SAMPLE_RATE as f32 / PI
    }

    #[test]
    fn censor_reverse_walks_backward_at_unity_rate() {
        let frames = [
            PcmFrame {
                left: 100,
                right: -100,
            },
            PcmFrame {
                left: 200,
                right: -200,
            },
            PcmFrame {
                left: 300,
                right: -300,
            },
            PcmFrame {
                left: 400,
                right: -400,
            },
            PcmFrame {
                left: 500,
                right: -500,
            },
        ];
        let mut censor = CensorState::new();
        assert!(censor.begin(14, 48_000, 48_000, 1.0, 4));
        let mut read = |seq: u64| {
            let index = seq.checked_sub(10)? as usize;
            frames.get(index).copied()
        };

        for expected in [500, 400, 300] {
            let out = censor.render(&mut read).unwrap();
            assert_eq!(out.frame.left, expected);
            assert_eq!(out.frame.right, -expected);
            assert!((out.reverse_gain - 1.0).abs() < 0.0001);
        }
    }

    #[test]
    fn censor_mixed_rate_interpolates_fractional_reverse_head() {
        let frames = [
            PcmFrame {
                left: 100,
                right: 100,
            },
            PcmFrame {
                left: 200,
                right: 200,
            },
            PcmFrame {
                left: 300,
                right: 300,
            },
            PcmFrame {
                left: 400,
                right: 400,
            },
            PcmFrame {
                left: 500,
                right: 500,
            },
        ];
        let mut censor = CensorState::new();
        assert!(censor.begin(24, 24_000, 48_000, 1.0, 4));
        let mut read = |seq: u64| {
            let index = seq.checked_sub(20)? as usize;
            frames.get(index).copied()
        };

        for expected in [500, 450, 400] {
            assert_eq!(censor.render(&mut read).unwrap().frame.left, expected);
        }
    }

    #[test]
    fn censor_release_fades_without_seeking_forward_timeline() {
        let frames = [
            PcmFrame {
                left: 100,
                right: 100,
            },
            PcmFrame {
                left: 200,
                right: 200,
            },
            PcmFrame {
                left: 300,
                right: 300,
            },
            PcmFrame {
                left: 400,
                right: 400,
            },
            PcmFrame {
                left: 500,
                right: 500,
            },
        ];
        let mut censor = CensorState::new();
        assert!(censor.begin(34, 48_000, 48_000, 1.0, 4));
        censor.release();
        let mut read = |seq: u64| {
            let index = seq.checked_sub(30)? as usize;
            frames.get(index).copied()
        };

        for expected in [1.0, 0.75, 0.5, 0.25] {
            let out = censor.render(&mut read).unwrap();
            assert!((out.reverse_gain - expected).abs() < 0.0001);
        }
        assert!(!censor.is_active());
    }

    #[test]
    fn censor_bounded_history_edge_fades_once_to_silence() {
        let frames = [
            PcmFrame {
                left: 6_400,
                right: -6_400,
            },
            PcmFrame {
                left: 3_200,
                right: -3_200,
            },
        ];
        let mut censor = CensorState::new();
        assert!(censor.begin(1, 48_000, 48_000, 1.0, 4));
        let mut read = |seq: u64| frames.get(seq as usize).copied();

        assert_eq!(censor.render(&mut read).unwrap().frame.left, 3_200);
        assert_eq!(censor.render(&mut read).unwrap().frame.left, 6_400);
        assert_eq!(censor.render(&mut read).unwrap().frame.left, 6_400);
        assert_eq!(censor.edge_hits(), 1);
        assert!(censor.exhausted());

        assert_eq!(censor.render(&mut read).unwrap().frame.left, 6_300);
        let mut last = PcmFrame::default();
        for _ in 0..CENSOR_EDGE_FADE_FRAMES {
            last = censor.render(&mut read).unwrap().frame;
        }
        assert_eq!(last, PcmFrame::default());
        assert_eq!(censor.edge_hits(), 1);
    }

    #[test]
    fn censor_invalid_configuration_is_rejected() {
        let mut censor = CensorState::new();
        assert!(!censor.begin(0, 0, 48_000, 1.0, 4));
        assert!(!censor.begin(0, 48_000, 0, 1.0, 4));
        assert!(!censor.begin(0, 48_000, 48_000, 1.0, 0));
        assert!(!censor.is_active());
    }

    #[test]
    fn censor_rate_update_is_bounded_and_nonpositive_speed_defaults_to_unity() {
        let mut censor = CensorState::new();
        assert!(censor.begin(100, 48_000, 48_000, -1.0, 4));
        assert_eq!(censor.step_q32, 1u64 << 32);

        censor.set_rate(48_000, 48_000, 100.0);
        assert_eq!(censor.step_q32, 8u64 << 32);

        let before = censor.step_q32;
        censor.set_rate(0, 48_000, 1.0);
        assert_eq!(censor.step_q32, before);
    }

    #[test]
    fn resampler_reset_outputs_silence_without_source() {
        let mut state = ResamplerState::new();
        let (out, consumed) = state.next(1.0, || None);
        assert_eq!(out, PcmFrame::default());
        assert_eq!(consumed, 0);
    }

    #[test]
    fn unity_pitch_preserves_released_one_frame_latency() {
        let frames = [
            PcmFrame {
                left: 100,
                right: -100,
            },
            PcmFrame {
                left: 200,
                right: -200,
            },
        ];
        let mut index = 0usize;
        let mut state = ResamplerState::new();

        let (first, consumed) = state.next(1.0, || {
            let frame = frames.get(index).copied();
            if frame.is_some() {
                index += 1;
            }
            frame
        });
        assert_eq!(first, PcmFrame::default());
        assert_eq!(consumed, 1);

        let (second, consumed) = state.next(1.0, || {
            let frame = frames.get(index).copied();
            if frame.is_some() {
                index += 1;
            }
            frame
        });
        assert_eq!(second, frames[0]);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn fractional_pitch_interpolates_between_source_frames() {
        let mut available = Some(PcmFrame {
            left: 100,
            right: -100,
        });
        let mut state = ResamplerState::new();

        let (first, consumed) = state.next(0.5, || available.take());
        assert_eq!(first, PcmFrame::default());
        assert_eq!(consumed, 0);

        let (second, consumed) = state.next(0.5, || available.take());
        assert_eq!(second, PcmFrame::default());
        assert_eq!(consumed, 1);

        let (third, consumed) = state.next(0.5, || available.take());
        assert_eq!(
            third,
            PcmFrame {
                left: 50,
                right: -50,
            }
        );
        assert_eq!(consumed, 0);
    }

    #[test]
    fn underrun_holds_last_frame_instead_of_clicking_to_zero() {
        let mut available = Some(PcmFrame {
            left: 1_000,
            right: -1_000,
        });
        let mut state = ResamplerState::new();

        let (_, consumed) = state.next(1.0, || available.take());
        assert_eq!(consumed, 1);

        for _ in 0..2 {
            let (out, consumed) = state.next(1.0, || available.take());
            assert_eq!(
                out,
                PcmFrame {
                    left: 1_000,
                    right: -1_000,
                }
            );
            assert_eq!(consumed, 0);
        }
    }

    #[test]
    fn non_finite_pitch_is_sanitized_to_unity() {
        let mut state = ResamplerState::new();
        let (_, consumed) = state.next(f32::NAN, || {
            Some(PcmFrame {
                left: 123,
                right: -123,
            })
        });
        assert_eq!(consumed, 1);
        assert_eq!(state.step_q32(), 1u64 << 32);
    }

    #[test]
    fn q32_step_proves_released_five_minute_zero_drift_contract() {
        const OUTPUT_FRAMES: u64 = 5 * 60 * 48_000;

        for factor in [44_100.0f32 / 48_000.0, 1.1] {
            let sanitized = sanitize_pitch_factor(factor);
            let step = pitch_step_q32(sanitized.to_bits());
            let consumed = OUTPUT_FRAMES
                .checked_mul(step)
                .expect("five-minute Q32 product fits u64")
                >> 32;
            let expected = (OUTPUT_FRAMES as f64 * factor as f64) as u64;
            assert_eq!(consumed, expected);
        }
    }

    #[test]
    fn pitch_change_refreshes_cached_q32_step_and_consumption() {
        let mut state = ResamplerState::new();
        let mut total = 0u32;

        for _ in 0..10 {
            total += state.next(0.5, || Some(PcmFrame::default())).1;
        }
        let half_step = state.step_q32();

        for _ in 0..10 {
            total += state.next(1.5, || Some(PcmFrame::default())).1;
        }

        assert_eq!(total, 20);
        assert_ne!(state.step_q32(), half_step);
    }

    #[test]
    fn disabled_filter_is_bypass_even_at_extreme_raw() {
        let dry = rms_after_filter(8_000.0, FILTER_RAW_CENTER, false);
        let disabled = rms_after_filter(8_000.0, FILTER_RAW_MIN, false);
        assert!(disabled > dry * 0.98);
        assert!(disabled < dry * 1.02);
    }

    #[test]
    fn center_filter_is_bypass_when_enabled() {
        let dry = rms_after_filter(1_000.0, FILTER_RAW_CENTER, false);
        let center = rms_after_filter(1_000.0, FILTER_RAW_CENTER, true);
        assert!(center > dry * 0.98);
        assert!(center < dry * 1.02);
    }

    #[test]
    fn half_low_pass_keeps_bass_and_kills_treble() {
        let half_lp = FILTER_RAW_CENTER / 2;
        let bass = rms_after_filter(100.0, half_lp, true);
        let normal_bass = rms_after_filter(100.0, FILTER_RAW_CENTER, true);
        let treble = rms_after_filter(8_000.0, half_lp, true);
        let normal_treble = rms_after_filter(8_000.0, FILTER_RAW_CENTER, true);

        assert!(bass > normal_bass * 0.85);
        assert!(treble < normal_treble * 0.15);
    }

    #[test]
    fn full_low_pass_kills_mids_and_most_bass() {
        let mids = rms_after_filter(1_000.0, FILTER_RAW_MIN, true);
        let normal_mids = rms_after_filter(1_000.0, FILTER_RAW_CENTER, true);
        let bass = rms_after_filter(100.0, FILTER_RAW_MIN, true);
        let normal_bass = rms_after_filter(100.0, FILTER_RAW_CENTER, true);

        assert!(mids < normal_mids * 0.10);
        assert!(bass < normal_bass * 0.75);
    }

    #[test]
    fn low_pass_treble_cut_deepens_monotonically() {
        let normal = rms_after_filter(8_000.0, FILTER_RAW_CENTER, true);
        let quarter = rms_after_filter(8_000.0, FILTER_RAW_CENTER - FILTER_RAW_CENTER / 4, true);
        let half = rms_after_filter(8_000.0, FILTER_RAW_CENTER / 2, true);
        let full = rms_after_filter(8_000.0, FILTER_RAW_MIN, true);

        assert!(quarter < normal * 0.75);
        assert!(half < quarter * 0.5);
        assert!(full < half * 0.5);
    }

    #[test]
    fn half_high_pass_kills_bass_and_keeps_treble() {
        let half_hp = FILTER_RAW_CENTER + (FILTER_RAW_MAX - FILTER_RAW_CENTER) / 2;
        let bass = rms_after_filter(100.0, half_hp, true);
        let normal_bass = rms_after_filter(100.0, FILTER_RAW_CENTER, true);
        let treble = rms_after_filter(8_000.0, half_hp, true);
        let normal_treble = rms_after_filter(8_000.0, FILTER_RAW_CENTER, true);

        assert!(bass < normal_bass * 0.20);
        assert!(treble > normal_treble * 0.85);
    }

    #[test]
    fn full_high_pass_kills_mids_and_keeps_some_treble() {
        let mids = rms_after_filter(1_000.0, FILTER_RAW_MAX, true);
        let normal_mids = rms_after_filter(1_000.0, FILTER_RAW_CENTER, true);
        let treble = rms_after_filter(8_000.0, FILTER_RAW_MAX, true);
        let normal_treble = rms_after_filter(8_000.0, FILTER_RAW_CENTER, true);

        assert!(mids < normal_mids * 0.15);
        assert!(treble > normal_treble * 0.70);
    }

    #[test]
    fn resonant_bump_lifts_tone_at_cutoff() {
        let raw = FILTER_RAW_CENTER - (0.142 * FILTER_RAW_CENTER as f32) as u16;
        let at_cutoff = rms_after_filter(8_000.0, raw, true);
        let dry = rms_after_filter(8_000.0, FILTER_RAW_CENTER, true);

        assert!(at_cutoff > dry * 1.05);
        assert!(at_cutoff < dry * 1.60);
    }

    #[test]
    fn knob_sweep_follows_released_exponential_curve() {
        for intensity in [0.25f32, 0.5, 0.75, 1.0] {
            let travel = (intensity * FILTER_RAW_CENTER as f32) as u16;

            let mut lp = FilterState::new(SAMPLE_RATE);
            settle_filter(&mut lp, FILTER_RAW_CENTER - travel);
            assert!(!lp.hp_mode);
            let lp_expected = FILTER_LP_MAX_HZ * libm::powf(60.0 / FILTER_LP_MAX_HZ, intensity);
            let lp_actual = programmed_cutoff_hz(&lp);
            assert!(libm::fabsf(lp_actual - lp_expected) < lp_expected * 0.02);

            let mut hp = FilterState::new(SAMPLE_RATE);
            settle_filter(&mut hp, FILTER_RAW_CENTER + travel);
            assert!(hp.hp_mode);
            let hp_expected = FILTER_HP_MIN_HZ * libm::powf(8_000.0 / FILTER_HP_MIN_HZ, intensity);
            let hp_actual = programmed_cutoff_hz(&hp);
            assert!(libm::fabsf(hp_actual - hp_expected) < hp_expected * 0.02);
        }
    }

    #[test]
    fn stable_knob_skips_coefficient_recomputation() {
        let mut filter = FilterState::new(SAMPLE_RATE);
        settle_filter(&mut filter, FILTER_RAW_MIN);
        assert!(!filter.bypassed);

        let poison = -12_345.0;
        filter.a1 = poison;
        for _ in 0..4_096 {
            filter.process_frame(true, DspFrame::default());
        }
        assert_eq!(filter.a1, poison);

        filter.set_raw(FILTER_RAW_MAX);
        for _ in 0..4_096 {
            filter.process_frame(true, DspFrame::default());
        }
        assert_ne!(filter.a1, poison);
    }

    #[test]
    fn flanger_required_frames_cover_released_max_delay() {
        let frames = flanger_required_frames(48_000);
        assert!(frames >= 290);
        assert!(frames <= 512);
    }

    #[test]
    fn disabled_flanger_bypasses_input() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);

        let input = PcmFrame {
            left: 4_321,
            right: -1_234,
        };
        assert_eq!(fx.process_pcm_frame(input), input);
    }

    #[test]
    fn unallocated_flanger_bypasses_input() {
        let mut left = [];
        let mut right = [];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 500,
            depth_q15: 32_767,
        });

        let input = PcmFrame {
            left: 4_321,
            right: -1_234,
        };
        assert!(!fx.is_allocated());
        assert_eq!(fx.process_pcm_frame(input), input);
    }

    #[test]
    fn zero_depth_flanger_is_transparent() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 500,
            depth_q15: 0,
        });

        for i in 0..400 {
            let sample = (libm::sinf(i as f32 * 0.05) * 12_000.0) as i16;
            let out = fx.process_pcm_frame(PcmFrame {
                left: sample,
                right: sample,
            });
            assert_eq!(out.left, sample);
            assert_eq!(out.right, sample);
        }
    }

    #[test]
    fn impulse_reappears_inside_released_delay_bounds() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 2_000,
            depth_q15: 32_767,
        });

        let first = fx.process_pcm_frame(PcmFrame {
            left: 16_000,
            right: 16_000,
        });
        assert_eq!(first.left, 16_000);
        assert_eq!(first.right, 16_000);

        let mut first_wet_frame = None;
        for i in 1..400 {
            let out = fx.process_pcm_frame(PcmFrame::default());
            if out.left != 0 || out.right != 0 {
                first_wet_frame = Some(i);
                break;
            }
        }

        let frame = first_wet_frame.expect("impulse must return");
        assert!(frame >= 10);
        assert!(frame <= 292);
    }

    #[test]
    fn enabled_flanger_colours_a_tone() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 300,
            depth_q15: 32_767,
        });

        let mut changed = 0;
        for i in 0..4_800 {
            let sample = (libm::sinf(i as f32 * 0.13) * 10_000.0) as i16;
            let out = fx.process_pcm_frame(PcmFrame {
                left: sample,
                right: sample,
            });
            if out.left != sample {
                changed += 1;
            }
        }

        assert!(changed > 4_000);
    }

    #[test]
    fn flanger_sweep_produces_deep_notch_and_resonant_peak() {
        const SR: u32 = 48_000;
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, SR);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 300,
            depth_q15: 32_767,
        });

        let amplitude = 3_000.0f32;
        let omega = 2.0 * core::f32::consts::PI * 400.0 / SR as f32;
        let window = 128usize;
        let total = (SR as usize * 700) / 1_000;
        let lead_in = SR as usize / 10;

        let mut weakest = amplitude;
        let mut strongest = 0.0f32;
        let mut window_peak = 0.0f32;

        for i in 0..total {
            let sample = (libm::sinf(i as f32 * omega) * amplitude) as i16;
            let out = fx.process_pcm_frame(PcmFrame {
                left: sample,
                right: sample,
            });
            window_peak = window_peak.max((out.left as f32).abs());

            if i % window == window - 1 {
                if i > lead_in {
                    weakest = weakest.min(window_peak);
                    strongest = strongest.max(window_peak);
                }
                window_peak = 0.0;
            }
        }

        assert!(strongest > 0.0);
        assert!(weakest < strongest * 0.25);
        assert!(strongest > amplitude * 2.8);
        assert!(strongest <= amplitude * 3.6);
    }

    #[test]
    fn flanger_wide_path_preserves_headroom_without_internal_clamp() {
        const SR: u32 = 48_000;
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, SR);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 300,
            depth_q15: 32_767,
        });

        let quiet = fx.process_frame(DspFrame {
            left: 20_000.0,
            right: -20_000.0,
        });
        assert_eq!(quiet.left, 20_000.0);
        assert_eq!(quiet.right, -20_000.0);

        let omega = 2.0 * core::f32::consts::PI * 400.0 / SR as f32;
        let total = (SR as usize * 700) / 1_000;
        let mut saw_above_pcm_ceiling = false;

        for i in 0..total {
            let sample = libm::sinf(i as f32 * omega) * 26_000.0;
            let out = fx.process_frame(DspFrame {
                left: sample,
                right: sample,
            });
            assert!(out.left.is_finite());
            assert!(out.right.is_finite());
            assert!(out.left.abs() < 100_000.0);
            assert!(out.right.abs() < 100_000.0);

            if out.left.abs() > 32_768.0 || out.right.abs() > 32_768.0 {
                saw_above_pcm_ceiling = true;
            }
        }

        assert!(saw_above_pcm_ceiling);
    }

    #[test]
    fn flanger_reenable_clears_stale_buffer() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        let mut config = FlangerConfig {
            enabled: true,
            period_ms: 500,
            depth_q15: 32_767,
        };
        fx.configure(config);

        for _ in 0..64 {
            fx.process_pcm_frame(PcmFrame {
                left: 16_000,
                right: -16_000,
            });
        }

        config.enabled = false;
        fx.configure(config);
        config.enabled = true;
        fx.configure(config);

        for _ in 0..400 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }
    }

    #[test]
    fn flanger_fractional_interpolation_matches_wide_float_reference() {
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        let mut fx = FlangerFx::new(&mut left, &mut right, 48_000);
        fx.configure(FlangerConfig {
            enabled: true,
            period_ms: 333,
            depth_q15: 32_767,
        });

        for i in 0..fx.capacity_frames {
            let sample = if i & 1 == 0 {
                i16::MIN as f32
            } else {
                i16::MAX as f32
            };
            fx.left[i] = sample;
            fx.right[i] = sample;
        }

        let phase = fx.lfo_phase_q32.wrapping_add(fx.lfo_step_q32);
        let tri_q16 = if phase < 0x8000_0000 {
            phase >> 15
        } else {
            (u32::MAX - phase) >> 15
        };
        let delay_q16 =
            fx.min_delay_q16 + (((fx.span_delay_q16 as u64 * tri_q16 as u64) >> 16) as u32);
        let delay_int = (delay_q16 >> 16) as usize;
        let frac_q16 = delay_q16 & 0xffff;
        let idx0 = if fx.write_index >= delay_int {
            fx.write_index - delay_int
        } else {
            fx.write_index + fx.capacity_frames - delay_int
        };
        let idx1 = if idx0 == 0 {
            fx.capacity_frames - 1
        } else {
            idx0 - 1
        };
        let delayed = read_delayed(&*fx.left, idx0, idx1, frac_q16);
        let expected = delayed * (fx.wet_cur_q15 as f32 / 32_768.0);

        let out = fx.process_frame(DspFrame::default());
        assert!((out.left - expected).abs() < 1.0);
        assert!((out.right - expected).abs() < 1.0);
    }

    fn delay_config(
        enabled: bool,
        mode: DelayMode,
        delay_ms: u32,
        wet_q15: u16,
        feedback_q15: u16,
    ) -> DelayConfig {
        DelayConfig {
            enabled,
            mode,
            delay_ms,
            wet_q15,
            feedback_q15,
        }
    }

    #[test]
    fn disabled_delay_bypasses_input() {
        let mut left = [0.0; 16];
        let mut right = [0.0; 16];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        fx.configure(delay_config(false, DelayMode::Echo, 4, 16_384, 8_192));

        let input = PcmFrame {
            left: 1_234,
            right: -2_345,
        };
        assert_eq!(fx.process_pcm_frame(input), input);
    }

    #[test]
    fn one_shot_delay_reappears_once_after_configured_period() {
        let mut left = [0.0; 16];
        let mut right = [0.0; 16];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        fx.configure(delay_config(true, DelayMode::Delay, 4, 16_384, 24_576));

        assert_eq!(fx.config().feedback_q15, 0);
        assert_eq!(
            fx.process_pcm_frame(PcmFrame {
                left: 10_000,
                right: 10_000,
            }),
            PcmFrame {
                left: 10_000,
                right: 10_000,
            }
        );

        for _ in 0..3 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }

        let delayed = fx.process_pcm_frame(PcmFrame::default());
        assert!(delayed.left > 4_500 && delayed.left < 5_500);
        assert!(delayed.right > 4_500 && delayed.right < 5_500);

        for _ in 0..4 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }
    }

    #[test]
    fn echo_feedback_decays_and_reset_clears_line() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        fx.configure(delay_config(true, DelayMode::Echo, 2, 16_384, 8_192));

        fx.process_pcm_frame(PcmFrame {
            left: 12_000,
            right: 12_000,
        });
        fx.process_pcm_frame(PcmFrame::default());
        let first = fx.process_pcm_frame(PcmFrame::default());
        fx.process_pcm_frame(PcmFrame::default());
        let second = fx.process_pcm_frame(PcmFrame::default());

        assert!(first.left > second.left);
        assert!(second.left > 0);

        fx.reset();
        for _ in 0..8 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }
    }

    #[test]
    fn echo_switch_off_rings_bounded_tail() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        let active = delay_config(true, DelayMode::Echo, 4, 16_384, 8_192);
        fx.configure(active);

        fx.process_pcm_frame(PcmFrame {
            left: 10_000,
            right: 10_000,
        });
        for _ in 0..3 {
            fx.process_pcm_frame(PcmFrame::default());
        }

        fx.configure(DelayConfig {
            enabled: false,
            ..active
        });
        assert!(fx.is_ringing());

        let tail = fx.process_pcm_frame(PcmFrame::default());
        assert!(tail.left > 4_000);
        assert!(tail.right > 4_000);

        for _ in 0..2_100 {
            fx.process_pcm_frame(PcmFrame::default());
        }
        assert!(!fx.is_ringing());

        let dry = PcmFrame {
            left: 777,
            right: -777,
        };
        assert_eq!(fx.process_pcm_frame(dry), dry);
    }

    #[test]
    fn reenable_clears_stale_tail() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        let active = delay_config(true, DelayMode::Echo, 4, 16_384, 8_192);
        fx.configure(active);
        fx.process_pcm_frame(PcmFrame {
            left: 10_000,
            right: 10_000,
        });

        fx.configure(DelayConfig {
            enabled: false,
            ..active
        });
        assert!(fx.is_ringing());

        fx.configure(active);
        assert!(!fx.is_ringing());
        for _ in 0..8 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }
    }

    #[test]
    fn delay_tail_is_exactly_one_delay_period() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        let active = delay_config(true, DelayMode::Delay, 4, 32_767, 24_576);
        fx.configure(active);
        fx.process_pcm_frame(PcmFrame {
            left: 10_000,
            right: -10_000,
        });

        fx.configure(DelayConfig {
            enabled: false,
            ..active
        });
        assert_eq!(fx.tail_frames_remaining(), 4);

        for _ in 0..3 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
            assert!(fx.is_ringing());
        }

        let final_tap = fx.process_pcm_frame(PcmFrame::default());
        assert!(final_tap.left > 9_900 && final_tap.left <= 10_000);
        assert!(final_tap.right < -9_900 && final_tap.right >= -10_000);
        assert!(!fx.is_ringing());
    }

    #[test]
    fn disabled_commands_cannot_retime_ringing_delay() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        let active = delay_config(true, DelayMode::Delay, 4, 32_767, 0);
        fx.configure(active);
        fx.process_pcm_frame(PcmFrame {
            left: 10_000,
            right: -10_000,
        });

        fx.configure(delay_config(false, DelayMode::Echo, 2, 1_024, 20_000));
        assert_eq!(fx.config().mode, DelayMode::Delay);
        assert_eq!(fx.config().delay_ms, 4);
        assert_eq!(fx.config().wet_q15, 32_767);
        assert_eq!(fx.delay_frames(), 4);
        assert_eq!(fx.tail_frames_remaining(), 4);

        assert_eq!(
            fx.process_pcm_frame(PcmFrame::default()),
            PcmFrame::default()
        );
        fx.configure(delay_config(false, DelayMode::Echo, 8, 16_384, 20_000));
        assert_eq!(fx.config().delay_ms, 4);
        assert_eq!(fx.tail_frames_remaining(), 3);
    }

    #[test]
    fn live_echo_delay_mode_change_clears_shared_line() {
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);

        let echo = delay_config(true, DelayMode::Echo, 4, 32_767, 16_384);
        let delay = delay_config(true, DelayMode::Delay, 4, 32_767, 16_384);

        fx.configure(echo);
        fx.process_pcm_frame(PcmFrame {
            left: 12_000,
            right: -12_000,
        });
        for _ in 0..3 {
            fx.process_pcm_frame(PcmFrame::default());
        }

        fx.configure(delay);
        assert_eq!(fx.config().feedback_q15, 0);
        for _ in 0..8 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }

        fx.process_pcm_frame(PcmFrame {
            left: -9_000,
            right: 9_000,
        });
        for _ in 0..3 {
            fx.process_pcm_frame(PcmFrame::default());
        }

        fx.configure(echo);
        for _ in 0..12 {
            assert_eq!(
                fx.process_pcm_frame(PcmFrame::default()),
                PcmFrame::default()
            );
        }
    }

    #[test]
    fn zero_length_buffers_are_safe_bypass() {
        let mut left = [];
        let mut right = [];
        let mut fx = DelayFx::new(&mut left, &mut right, 1_000);
        fx.configure(delay_config(true, DelayMode::Delay, 10, 32_767, 32_767));
        assert!(!fx.is_allocated());

        let input = PcmFrame {
            left: -3_000,
            right: 3_000,
        };
        assert_eq!(fx.process_pcm_frame(input), input);
    }

    #[test]
    fn wide_echo_path_preserves_headroom_without_internal_clamp() {
        const CAP: usize = 4_410;
        const SR: u32 = 44_100;
        let mut left = [0.0; CAP];
        let mut right = [0.0; CAP];
        let mut fx = DelayFx::new(&mut left, &mut right, SR);
        fx.configure(delay_config(true, DelayMode::Echo, 50, 22_938, 22_282));

        let quiet = fx.process_frame(DspFrame {
            left: 20_000.0,
            right: -20_000.0,
        });
        assert_eq!(quiet.left, 20_000.0);
        assert_eq!(quiet.right, -20_000.0);

        let mut saw_above_pcm_ceiling = false;
        for i in 0..SR * 3 {
            let sample = if i % 200 < 100 { 16_000.0 } else { -16_000.0 };
            let out = fx.process_frame(DspFrame {
                left: sample,
                right: sample,
            });
            assert!(out.left.is_finite());
            assert!(out.right.is_finite());
            assert!(out.left.abs() < 80_000.0);
            assert!(out.right.abs() < 80_000.0);
            if out.left.abs() > 32_768.0 || out.right.abs() > 32_768.0 {
                saw_above_pcm_ceiling = true;
            }
        }
        assert!(saw_above_pcm_ceiling);
    }

    #[test]
    fn pad_fx_defaults_to_bypass() {
        let mut left = [];
        let mut right = [];
        let mut fx = PadFx::new(44_100, &mut left, &mut right);
        let input = PcmFrame {
            left: 1_200,
            right: -1_200,
        };
        assert_eq!(fx.process_pcm_frame(input), input);
        assert!(!fx.is_active());
        assert_eq!(fx.kind(), PadFxKind::None);
    }

    #[test]
    fn pad_fx_presets_match_released_tables() {
        let p10 = pad_fx_preset(PadFxMode::PadFx1, 0);
        let p11 = pad_fx_preset(PadFxMode::PadFx1, 1);
        let p12 = pad_fx_preset(PadFxMode::PadFx1, 2);
        let p13 = pad_fx_preset(PadFxMode::PadFx1, 3);
        let p20 = pad_fx_preset(PadFxMode::PadFx2, 0);
        let p21 = pad_fx_preset(PadFxMode::PadFx2, 1);
        let p22 = pad_fx_preset(PadFxMode::PadFx2, 2);
        let p23 = pad_fx_preset(PadFxMode::PadFx2, 3);

        assert_eq!((p10.kind, p10.filter_raw), (PadFxKind::Filter, 3_600));
        assert_eq!((p11.kind, p11.filter_raw), (PadFxKind::Filter, 12_700));
        assert_eq!((p20.kind, p20.filter_raw), (PadFxKind::Filter, 2_300));
        assert_eq!((p21.kind, p21.filter_raw), (PadFxKind::Filter, 14_000));
        assert_eq!(
            (
                p12.kind,
                p12.echo_delay_ms,
                p12.echo_wet_q15,
                p12.echo_feedback_q15
            ),
            (PadFxKind::Echo, 250, 8_192, 9_830)
        );
        assert_eq!(
            (
                p13.kind,
                p13.echo_delay_ms,
                p13.echo_wet_q15,
                p13.echo_feedback_q15
            ),
            (PadFxKind::Echo, 500, 8_192, 11_469)
        );
        assert_eq!(
            (
                p22.kind,
                p22.echo_delay_ms,
                p22.echo_wet_q15,
                p22.echo_feedback_q15
            ),
            (PadFxKind::Echo, 125, 9_830, 9_830)
        );
        assert_eq!(
            (
                p23.kind,
                p23.echo_delay_ms,
                p23.echo_wet_q15,
                p23.echo_feedback_q15
            ),
            (PadFxKind::Echo, 1_000, 9_830, 13_107)
        );
    }

    #[test]
    fn pad_fx_filter_pad_changes_signal_and_matching_release_resets() {
        let mut left = [];
        let mut right = [];
        let mut fx = PadFx::new(44_100, &mut left, &mut right);
        let cfg = PadFxConfig {
            mode: PadFxMode::PadFx1,
            pad: 0,
            active: true,
        };
        fx.set(cfg);

        let input = PcmFrame {
            left: 16_000,
            right: -16_000,
        };
        let out = fx.process_pcm_frame(input);
        assert!(fx.is_active());
        assert_eq!(fx.kind(), PadFxKind::Filter);
        assert!(out.left != input.left || out.right != input.right);

        fx.set(PadFxConfig {
            active: false,
            ..cfg
        });
        assert!(!fx.is_active());
        assert_eq!(fx.kind(), PadFxKind::None);
    }

    #[test]
    fn mismatched_pad_fx_release_is_ignored() {
        let mut left = [];
        let mut right = [];
        let mut fx = PadFx::new(44_100, &mut left, &mut right);
        let cfg = PadFxConfig {
            mode: PadFxMode::PadFx2,
            pad: 0,
            active: true,
        };
        fx.set(cfg);
        fx.set(PadFxConfig {
            mode: PadFxMode::PadFx2,
            pad: 1,
            active: false,
        });
        assert!(fx.is_active());
        assert_eq!(fx.config(), cfg);
    }

    #[test]
    fn unsupported_pad_fx_pad_is_inactive() {
        let mut left = [0.0; 1_100];
        let mut right = [0.0; 1_100];
        let mut fx = PadFx::new(1_000, &mut left, &mut right);
        fx.set(PadFxConfig {
            mode: PadFxMode::PadFx1,
            pad: 7,
            active: true,
        });
        assert!(!fx.is_active());
        assert_eq!(fx.kind(), PadFxKind::None);
    }

    #[test]
    fn pad_fx_echo_release_keeps_tail() {
        let mut left = [0.0; 1_100];
        let mut right = [0.0; 1_100];
        let mut fx = PadFx::new(1_000, &mut left, &mut right);
        let cfg = PadFxConfig {
            mode: PadFxMode::PadFx2,
            pad: 2,
            active: true,
        };
        fx.set(cfg);

        fx.process_pcm_frame(PcmFrame {
            left: 12_000,
            right: -12_000,
        });
        for _ in 0..8 {
            fx.process_pcm_frame(PcmFrame::default());
        }

        fx.set(PadFxConfig {
            active: false,
            ..cfg
        });
        assert!(!fx.is_active());
        assert!(fx.echo_tail_active());

        let mut saw_tail = false;
        for _ in 0..180 {
            let out = fx.process_pcm_frame(PcmFrame::default());
            if out.left != 0 || out.right != 0 {
                saw_tail = true;
                break;
            }
        }
        assert!(saw_tail);
    }

    #[test]
    fn smart_cfx_center_and_extremes_are_identity() {
        assert_eq!(smart_cfx_curve_raw(FILTER_RAW_CENTER), FILTER_RAW_CENTER);
        assert_eq!(smart_cfx_curve_raw(FILTER_RAW_MIN), FILTER_RAW_MIN);
        assert_eq!(smart_cfx_curve_raw(FILTER_RAW_MAX), FILTER_RAW_MAX);
        assert_eq!(smart_cfx_curve_raw(u16::MAX), FILTER_RAW_MAX);
    }

    #[test]
    fn smart_cfx_softens_near_center() {
        let input = FILTER_RAW_CENTER - 512;
        let curved = smart_cfx_curve_raw(input);
        assert!(curved < FILTER_RAW_CENTER);
        assert!(curved > input);
    }

    #[test]
    fn smart_cfx_half_turn_is_near_linear() {
        let half_low = FILTER_RAW_CENTER / 2;
        let curved_low = smart_cfx_curve_raw(half_low);
        assert!(curved_low >= half_low - 8);
        assert!(curved_low <= half_low + 8);

        let half_high = FILTER_RAW_CENTER + (FILTER_RAW_MAX - FILTER_RAW_CENTER) / 2;
        let curved_high = smart_cfx_curve_raw(half_high);
        assert!(curved_high >= half_high - 8);
        assert!(curved_high <= half_high + 8);
    }

    #[test]
    fn smart_cfx_past_half_turn_runs_ahead_of_linear() {
        let three_quarters_low = FILTER_RAW_CENTER / 4;
        assert!(smart_cfx_curve_raw(three_quarters_low) < three_quarters_low);
    }

    #[test]
    fn raw_gain_mapping_matches_released_contract() {
        assert_eq!(eq_raw_to_gain(EQ_RAW_MIN), 0.0);
        assert_eq!(eq_raw_to_gain(EQ_RAW_CENTER), 1.0);
        assert_eq!(eq_raw_to_gain(EQ_RAW_MAX), 2.0);
        assert_eq!(eq_raw_to_gain(u16::MAX), 2.0);
    }

    #[test]
    fn raw_values_are_clamped_and_reported_without_forcing_immediate_refresh() {
        let mut eq = EqState::new(SAMPLE_RATE);
        eq.set_band_raw(EqBand::Low, u16::MAX);
        assert_eq!(eq.band_raw(EqBand::Low), EQ_RAW_MAX);

        eq.process_frame(DspFrame::default());
        eq.set_band_raw(EqBand::Low, EQ_RAW_MIN);
        for _ in 0..31 {
            eq.process_frame(DspFrame::default());
        }
        assert_eq!(eq.gain[EqBand::Low.index()], 2.0);

        eq.process_frame(DspFrame::default());
        assert_eq!(eq.gain[EqBand::Low.index()], 0.0);
    }

    #[test]
    fn sample_rate_retune_preserves_filter_history_and_gains() {
        let mut eq = EqState::new(SAMPLE_RATE);
        eq.set_band_raw(EqBand::Mid, EQ_RAW_MAX);
        eq.process_frame(DspFrame {
            left: 10_000.0,
            right: -10_000.0,
        });

        let low_before = eq.low_lp;
        let gain_before = eq.gain;
        eq.set_sample_rate(48_000);

        assert_eq!(eq.low_lp, low_before);
        assert_eq!(eq.gain, gain_before);
        assert_ne!(eq.low_alpha, one_pole_alpha(EQ_LOW_CUTOFF_HZ, SAMPLE_RATE));
    }

    #[test]
    fn center_eq_keeps_signal_level_near_unity() {
        let dry = rms_of_sine_after_eq(1_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let centered = rms_of_sine_after_eq(1_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);
        assert!(centered > dry * 0.98);
        assert!(centered < dry * 1.02);
    }

    #[test]
    fn low_kill_reduces_bass_more_than_treble() {
        let killed_bass = rms_of_sine_after_eq(100.0, EQ_RAW_MIN, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let normal_bass = rms_of_sine_after_eq(100.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let killed_treble = rms_of_sine_after_eq(8_000.0, EQ_RAW_MIN, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let normal_treble =
            rms_of_sine_after_eq(8_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);

        assert!(killed_bass < normal_bass * 0.35);
        assert!(killed_treble > normal_treble * 0.80);
    }

    #[test]
    fn high_kill_reduces_treble_more_than_bass() {
        let killed_treble = rms_of_sine_after_eq(8_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_MIN);
        let normal_treble =
            rms_of_sine_after_eq(8_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let killed_bass = rms_of_sine_after_eq(100.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_MIN);
        let normal_bass = rms_of_sine_after_eq(100.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);

        assert!(killed_treble < normal_treble * 0.35);
        assert!(killed_bass > normal_bass * 0.80);
    }

    #[test]
    fn mid_boost_increases_level_and_pcm_conversion_clamps() {
        let normal_mid = rms_of_sine_after_eq(1_000.0, EQ_RAW_CENTER, EQ_RAW_CENTER, EQ_RAW_CENTER);
        let boosted_mid = rms_of_sine_after_eq(1_000.0, EQ_RAW_CENTER, EQ_RAW_MAX, EQ_RAW_CENTER);
        assert!(boosted_mid > normal_mid * 1.35);

        let mut eq = EqState::new(SAMPLE_RATE);
        eq.set_raw(EQ_RAW_MAX, EQ_RAW_MAX, EQ_RAW_MAX);
        let out = eq.process_pcm_frame(PcmFrame {
            left: 30_000,
            right: -30_000,
        });
        assert_eq!(out.left, i16::MAX);
        assert_eq!(out.right, i16::MIN);
    }

    #[test]
    fn each_band_boost_preserves_wide_headroom() {
        for (freq_hz, band) in [
            (100.0, EqBand::Low),
            (1_000.0, EqBand::Mid),
            (8_000.0, EqBand::High),
        ] {
            let peak = peak_of_sine_after_wide_eq(freq_hz, band);
            assert!(peak > 32_768.0);
            assert!(peak < 60_000.0);
        }
    }

    #[test]
    fn pcm_conversion_matches_released_nan_round_and_clamp_policy() {
        assert_eq!(pcm_sample_from_float(f32::NAN), 0);
        assert_eq!(pcm_sample_from_float(32_768.0), i16::MAX);
        assert_eq!(pcm_sample_from_float(-32_769.0), i16::MIN);
        assert_eq!(pcm_sample_from_float(10.49), 10);
        assert_eq!(pcm_sample_from_float(10.50), 11);
        assert_eq!(pcm_sample_from_float(-10.49), -10);
        assert_eq!(pcm_sample_from_float(-10.50), -11);
    }
}
