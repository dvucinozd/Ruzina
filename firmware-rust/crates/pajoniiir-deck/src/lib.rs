#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_controller_core::{
    BeatFxTarget, ControlEvent, ControlValue, DeckExtAction, DeckId, PadMode, SemanticControl,
};
use pajoniiir_core::MediaTrackId;
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_track_analysis::{
    TrackAnalysis, beat_jump_target_ms, beat_loop_duration_ms, phase_align_target_ms,
};

pub const PITCH_CENTER: u16 = 8192;
pub const PITCH_MAX: u16 = 16383;
pub const DEFAULT_TEMPO_RANGE_PERCENT: u16 = 10;
pub const BEAT_SYNC_MAX_PERCENT: i16 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeatFxEffect {
    Filter,
    Echo,
    Flanger,
    Delay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum BeatFxBeat {
    Quarter,
    Half,
    One,
    Two,
    Four,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeatFxState {
    pub effect: BeatFxEffect,
    pub beat: BeatFxBeat,
    pub target: BeatFxTarget,
    pub depth: u8,
    pub enabled: bool,
}

impl BeatFxState {
    pub const fn new() -> Self {
        Self {
            effect: BeatFxEffect::Filter,
            beat: BeatFxBeat::One,
            target: BeatFxTarget::Both,
            depth: 64,
            enabled: false,
        }
    }
}

impl Default for BeatFxState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadFxBank {
    One,
    Two,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivePadFx {
    pub bank: PadFxBank,
    pub pad: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerformanceMode {
    HotCue,
    LoopRoll,
    BeatJump,
    KeyShift,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopAdjustMode {
    None,
    In,
    Out,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopRegion {
    pub start_ms: u32,
    pub end_ms: u32,
}

impl LoopRegion {
    pub const fn new(start_ms: u32, end_ms: u32) -> Option<Self> {
        if end_ms > start_ms {
            Some(Self { start_ms, end_ms })
        } else {
            None
        }
    }

    pub const fn duration_ms(self) -> u32 {
        self.end_ms - self.start_ms
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopState {
    pub active: Option<LoopRegion>,
    pub pending_in_ms: Option<u32>,
    pub last: Option<LoopRegion>,
}

impl LoopState {
    pub const fn new() -> Self {
        Self {
            active: None,
            pending_in_ms: None,
            last: None,
        }
    }
}

impl Default for LoopState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeatJumpPage {
    Fractional,
    Default,
    Large,
}

const BEAT_JUMP_FRACTIONAL: [(i32, u16); 8] = [
    (-1, 16),
    (1, 16),
    (-1, 8),
    (1, 8),
    (-1, 4),
    (1, 4),
    (-1, 2),
    (1, 2),
];
const BEAT_JUMP_DEFAULT: [(i32, u16); 8] = [
    (-1, 1),
    (1, 1),
    (-2, 1),
    (2, 1),
    (-4, 1),
    (4, 1),
    (-8, 1),
    (8, 1),
];
const BEAT_JUMP_LARGE: [(i32, u16); 8] = [
    (-16, 1),
    (16, 1),
    (-32, 1),
    (32, 1),
    (-64, 1),
    (64, 1),
    (-128, 1),
    (128, 1),
];

const BEAT_LOOP_LENGTHS: [(u16, u16); 8] = [
    (1, 32),
    (1, 16),
    (1, 8),
    (1, 4),
    (1, 2),
    (1, 1),
    (2, 1),
    (4, 1),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingPlayback {
    request_id: u32,
    playing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShiftedLoopRoll {
    active: bool,
    previous: Option<LoopRegion>,
}

impl ShiftedLoopRoll {
    const fn new() -> Self {
        Self {
            active: false,
            previous: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckState {
    pub playing: bool,
    pub position_ms: u32,
    pub cue_point_ms: u32,
    pub pitch_raw: u16,
    pub pitch_centipercent: i16,
    pub tempo_range_percent: u16,
    pub perf_mode: PerformanceMode,
    pub pad_mode: PadMode,
    pub active_pad_fx: Option<ActivePadFx>,
    pub sync_enabled: bool,
    pub sync_master: bool,
    pub quantize_enabled: bool,
    pub loop_adjust_mode: LoopAdjustMode,
    pub loop_state: LoopState,
    pub censor_active: bool,
    pub master_tempo: bool,
    pub controller_connected: bool,
    pub shift_held: bool,
    pub jog_touched: bool,
    pub base_bpm_x100: u32,
    playback_generation: u32,
    pending_playback: Option<PendingPlayback>,
}

impl DeckState {
    pub const fn new() -> Self {
        Self {
            playing: false,
            position_ms: 0,
            cue_point_ms: 0,
            pitch_raw: PITCH_CENTER,
            pitch_centipercent: 0,
            tempo_range_percent: DEFAULT_TEMPO_RANGE_PERCENT,
            perf_mode: PerformanceMode::HotCue,
            pad_mode: PadMode::HotCue,
            active_pad_fx: None,
            sync_enabled: false,
            sync_master: false,
            quantize_enabled: false,
            loop_adjust_mode: LoopAdjustMode::None,
            loop_state: LoopState::new(),
            censor_active: false,
            master_tempo: false,
            controller_connected: false,
            shift_held: false,
            jog_touched: false,
            base_bpm_x100: 12_000,
            playback_generation: 0,
            pending_playback: None,
        }
    }
}

impl Default for DeckState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckEffectTarget {
    Transport,
    Persistence,
    AudioDsp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckEffect {
    PlaybackRequest {
        deck: DeckId,
        playing: bool,
        request_id: u32,
    },
    Pause {
        deck: DeckId,
    },
    Seek {
        deck: DeckId,
        position_ms: u32,
    },
    SetPitchCentipercent {
        deck: DeckId,
        value: i16,
    },
    SetLoop {
        deck: DeckId,
        start_ms: u32,
        end_ms: u32,
    },
    ClearLoop {
        deck: DeckId,
    },
    PersistHotCues {
        deck: DeckId,
        bank: HotCueBank,
    },
    ApplyBeatFx {
        state: BeatFxState,
        delay_ms: u32,
        flanger_period_ms: u32,
    },
    ApplyPadFx {
        deck: DeckId,
        bank: PadFxBank,
        pad: u8,
        active: bool,
    },
}

impl DeckEffect {
    pub const fn target(self) -> DeckEffectTarget {
        match self {
            Self::PlaybackRequest { .. }
            | Self::Pause { .. }
            | Self::Seek { .. }
            | Self::SetPitchCentipercent { .. }
            | Self::SetLoop { .. }
            | Self::ClearLoop { .. } => DeckEffectTarget::Transport,
            Self::PersistHotCues { .. } => DeckEffectTarget::Persistence,
            Self::ApplyBeatFx { .. } | Self::ApplyPadFx { .. } => DeckEffectTarget::AudioDsp,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckEffects {
    pub items: [Option<DeckEffect>; 2],
}

impl DeckEffects {
    pub const NONE: Self = Self {
        items: [None, None],
    };

    pub const fn one(effect: DeckEffect) -> Self {
        Self {
            items: [Some(effect), None],
        }
    }

    pub const fn two(first: DeckEffect, second: DeckEffect) -> Self {
        Self {
            items: [Some(first), Some(second)],
        }
    }

    pub const fn len(self) -> usize {
        self.items[0].is_some() as usize + self.items[1].is_some() as usize
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

pub struct DeckEffectQueue<const N: usize> {
    items: [Option<DeckEffect>; N],
    head: usize,
    tail: usize,
    count: usize,
}

impl<const N: usize> Default for DeckEffectQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DeckEffectQueue<N> {
    pub const fn new() -> Self {
        Self {
            items: [None; N],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn remaining_capacity(&self) -> usize {
        N.saturating_sub(self.count)
    }

    pub fn try_push_all(&mut self, effects: DeckEffects) -> bool {
        let needed = effects.len();
        if needed > self.remaining_capacity() {
            return false;
        }

        for effect in effects.items.into_iter().flatten() {
            if N == 0 {
                return false;
            }
            self.items[self.tail] = Some(effect);
            self.tail = (self.tail + 1) % N;
            self.count += 1;
        }
        true
    }

    pub fn front(&self) -> Option<DeckEffect> {
        if self.count == 0 || N == 0 {
            None
        } else {
            self.items[self.head]
        }
    }

    /// Completes the currently visible effect only after the downstream owner
    /// reports whether it accepted the request.
    ///
    /// A failed completion intentionally keeps the same front item for retry.
    pub fn complete_front(&mut self, succeeded: bool) -> bool {
        if self.count == 0 || N == 0 {
            return false;
        }

        if succeeded {
            self.items[self.head] = None;
            self.head = (self.head + 1) % N;
            self.count -= 1;
        }
        true
    }
}

pub struct DeckEffectBroker<const T: usize, const P: usize, const A: usize> {
    transport: DeckEffectQueue<T>,
    persistence: DeckEffectQueue<P>,
    audio_dsp: DeckEffectQueue<A>,
}

impl<const T: usize, const P: usize, const A: usize> Default for DeckEffectBroker<T, P, A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const T: usize, const P: usize, const A: usize> DeckEffectBroker<T, P, A> {
    pub const fn new() -> Self {
        Self {
            transport: DeckEffectQueue::new(),
            persistence: DeckEffectQueue::new(),
            audio_dsp: DeckEffectQueue::new(),
        }
    }

    pub const fn len(&self) -> usize {
        self.transport.len() + self.persistence.len() + self.audio_dsp.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn transport_len(&self) -> usize {
        self.transport.len()
    }

    pub const fn persistence_len(&self) -> usize {
        self.persistence.len()
    }

    pub const fn audio_dsp_len(&self) -> usize {
        self.audio_dsp.len()
    }

    pub fn try_push_all(&mut self, effects: DeckEffects) -> bool {
        let mut transport = 0usize;
        let mut persistence = 0usize;
        let mut audio_dsp = 0usize;

        for effect in effects.items.into_iter().flatten() {
            match effect.target() {
                DeckEffectTarget::Transport => transport += 1,
                DeckEffectTarget::Persistence => persistence += 1,
                DeckEffectTarget::AudioDsp => audio_dsp += 1,
            }
        }

        if transport > self.transport.remaining_capacity()
            || persistence > self.persistence.remaining_capacity()
            || audio_dsp > self.audio_dsp.remaining_capacity()
        {
            return false;
        }

        for effect in effects.items.into_iter().flatten() {
            let accepted = match effect.target() {
                DeckEffectTarget::Transport => {
                    self.transport.try_push_all(DeckEffects::one(effect))
                }
                DeckEffectTarget::Persistence => {
                    self.persistence.try_push_all(DeckEffects::one(effect))
                }
                DeckEffectTarget::AudioDsp => self.audio_dsp.try_push_all(DeckEffects::one(effect)),
            };
            debug_assert!(accepted);
        }
        true
    }

    pub fn transport_front(&self) -> Option<DeckEffect> {
        self.transport.front()
    }

    pub fn complete_transport(&mut self, succeeded: bool) -> bool {
        self.transport.complete_front(succeeded)
    }

    pub fn persistence_front(&self) -> Option<DeckEffect> {
        self.persistence.front()
    }

    pub fn complete_persistence(&mut self, succeeded: bool) -> bool {
        self.persistence.complete_front(succeeded)
    }

    pub fn audio_dsp_front(&self) -> Option<DeckEffect> {
        self.audio_dsp.front()
    }

    pub fn complete_audio_dsp(&mut self, succeeded: bool) -> bool {
        self.audio_dsp.complete_front(succeeded)
    }
}

#[derive(Clone, Copy)]
pub struct DeckProductState {
    decks: [DeckState; 2],
    sync_master: Option<DeckId>,
    beat_jump_page: BeatJumpPage,
    shifted_loop_roll: [ShiftedLoopRoll; 2],
    hot_cue_sources: [Option<HotCueBank>; 2],
    hot_cue_locals: [Option<HotCueBank>; 2],
    beat_fx: BeatFxState,
}

impl Default for DeckProductState {
    fn default() -> Self {
        Self::new()
    }
}

impl DeckProductState {
    pub const fn new() -> Self {
        Self {
            decks: [DeckState::new(), DeckState::new()],
            sync_master: None,
            beat_jump_page: BeatJumpPage::Default,
            shifted_loop_roll: [ShiftedLoopRoll::new(), ShiftedLoopRoll::new()],
            hot_cue_sources: [None, None],
            hot_cue_locals: [None, None],
            beat_fx: BeatFxState::new(),
        }
    }

    pub fn deck(&self, deck: DeckId) -> &DeckState {
        &self.decks[deck_index(deck)]
    }

    pub const fn beat_jump_page(&self) -> BeatJumpPage {
        self.beat_jump_page
    }

    pub const fn beat_fx(&self) -> BeatFxState {
        self.beat_fx
    }

    pub fn load_hot_cues(
        &mut self,
        deck: DeckId,
        track_id: MediaTrackId,
        persisted: Option<HotCueBank>,
    ) -> bool {
        self.load_hot_cues_with_source(deck, track_id, HotCueBank::empty(track_id), persisted)
    }

    pub fn load_hot_cues_with_source(
        &mut self,
        deck: DeckId,
        track_id: MediaTrackId,
        source: HotCueBank,
        persisted: Option<HotCueBank>,
    ) -> bool {
        if source.track_id() != track_id {
            return false;
        }
        let local = match persisted {
            Some(bank) if bank.track_id() != track_id => return false,
            Some(bank) => bank,
            None => HotCueBank::empty(track_id),
        };
        let index = deck_index(deck);
        self.hot_cue_sources[index] = Some(source.as_source());
        self.hot_cue_locals[index] = Some(local);
        true
    }

    pub fn commit_track_load(
        &mut self,
        deck: DeckId,
        track_id: MediaTrackId,
        bpm_x100: u32,
        source: HotCueBank,
        persisted: Option<HotCueBank>,
    ) -> bool {
        if source.track_id() != track_id
            || persisted.is_some_and(|bank| bank.track_id() != track_id)
        {
            return false;
        }

        let local = persisted.unwrap_or_else(|| HotCueBank::empty(track_id));
        let index = deck_index(deck);
        let state = &mut self.decks[index];

        // Track-lifecycle state is replaced atomically only after all identities
        // have validated. User/controller configuration intentionally survives.
        state.playing = false;
        state.position_ms = 0;
        state.cue_point_ms = 0;
        state.active_pad_fx = None;
        state.sync_enabled = false;
        state.sync_master = false;
        state.loop_adjust_mode = LoopAdjustMode::None;
        state.loop_state = LoopState::new();
        state.censor_active = false;
        state.base_bpm_x100 = if bpm_x100 == 0 { 12_000 } else { bpm_x100 };
        state.playback_generation = state.playback_generation.wrapping_add(1);
        state.pending_playback = None;

        if self.sync_master == Some(deck) {
            self.sync_master = None;
        }
        self.shifted_loop_roll[index] = ShiftedLoopRoll::new();
        self.hot_cue_sources[index] = Some(source.as_source());
        self.hot_cue_locals[index] = Some(local);
        true
    }

    pub fn unload_track(&mut self, deck: DeckId) {
        let index = deck_index(deck);
        let state = &mut self.decks[index];

        state.playing = false;
        state.position_ms = 0;
        state.cue_point_ms = 0;
        state.active_pad_fx = None;
        state.sync_enabled = false;
        state.sync_master = false;
        state.loop_adjust_mode = LoopAdjustMode::None;
        state.loop_state = LoopState::new();
        state.censor_active = false;
        state.base_bpm_x100 = 12_000;
        state.playback_generation = state.playback_generation.wrapping_add(1);
        state.pending_playback = None;

        if self.sync_master == Some(deck) {
            self.sync_master = None;
        }
        self.shifted_loop_roll[index] = ShiftedLoopRoll::new();
        self.hot_cue_sources[index] = None;
        self.hot_cue_locals[index] = None;
    }

    pub fn clear_loaded_track(&mut self, deck: DeckId) {
        let index = deck_index(deck);
        self.hot_cue_sources[index] = None;
        self.hot_cue_locals[index] = None;
    }

    pub fn hot_cues(&self, deck: DeckId) -> Option<HotCueBank> {
        let index = deck_index(deck);
        HotCueBank::merged(self.hot_cue_sources[index]?, self.hot_cue_locals[index]?)
    }

    pub fn local_hot_cues(&self, deck: DeckId) -> Option<HotCueBank> {
        self.hot_cue_locals[deck_index(deck)]
    }

    pub fn set_base_bpm_x100(&mut self, deck: DeckId, bpm_x100: u32) {
        self.decks[deck_index(deck)].base_bpm_x100 = if bpm_x100 == 0 { 12_000 } else { bpm_x100 };
    }

    pub fn set_position_ms(&mut self, deck: DeckId, position_ms: u32) {
        self.decks[deck_index(deck)].position_ms = position_ms;
    }

    pub fn finish_playback(&mut self, deck: DeckId, position_ms: u32) {
        let state = &mut self.decks[deck_index(deck)];
        state.playing = false;
        state.position_ms = position_ms;
        state.pending_playback = None;
    }

    pub fn set_controller_connected(&mut self, connected: bool) {
        for state in &mut self.decks {
            state.controller_connected = connected;
            if !connected {
                state.jog_touched = false;
                state.loop_adjust_mode = LoopAdjustMode::None;
            }
        }
    }

    pub fn handle_control(&mut self, event: ControlEvent) -> DeckEffects {
        self.handle_control_with_analysis(event, [None, None])
    }

    pub fn handle_control_with_analysis(
        &mut self,
        event: ControlEvent,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        if is_beat_fx_control(event.control) {
            return self.handle_beat_fx_control(event);
        }

        let Some(deck) = event.deck else {
            return DeckEffects::NONE;
        };

        match event.control {
            SemanticControl::Play => self.handle_play(deck, event.value),
            SemanticControl::Cue => self.handle_cue(deck, event.value),
            SemanticControl::ToStart => self.handle_to_start(deck, event.value),
            SemanticControl::Tempo => self.handle_tempo(deck, event.value),
            SemanticControl::TempoRange => self.handle_tempo_range(deck, event.value),
            SemanticControl::JogScratch | SemanticControl::JogBend => {
                self.handle_loop_adjust_jog(deck, event.value)
            }
            SemanticControl::Sync => self.handle_sync(deck, event.value, analysis),
            SemanticControl::BeatJumpBack => {
                self.handle_beat_jump(deck, event.value, -1, 1, analysis)
            }
            SemanticControl::BeatJumpForward => {
                self.handle_beat_jump(deck, event.value, 1, 1, analysis)
            }
            SemanticControl::LoopIn => self.handle_loop_in(deck, event.value, analysis),
            SemanticControl::LoopOut => self.handle_loop_out(deck, event.value, analysis),
            SemanticControl::ReloopExit => self.handle_reloop_exit(deck, event.value),
            SemanticControl::LoopHalve => self.handle_loop_resize(deck, event.value, false),
            SemanticControl::LoopDouble => self.handle_loop_resize(deck, event.value, true),
            SemanticControl::LoopSize => self.handle_loop_size(deck, event.value),
            SemanticControl::PadAction => self.handle_pad_action(deck, event.value, analysis),
            SemanticControl::Shift => {
                if let ControlValue::Pressed(pressed) = event.value {
                    self.decks[deck_index(deck)].shift_held = pressed;
                }
                DeckEffects::NONE
            }
            SemanticControl::JogTouch => {
                if let ControlValue::Pressed(pressed) = event.value {
                    self.decks[deck_index(deck)].jog_touched = pressed;
                }
                DeckEffects::NONE
            }
            SemanticControl::PadModeHotCue => {
                self.set_pad_mode_if_pressed(
                    deck,
                    event.value,
                    PadMode::HotCue,
                    Some(PerformanceMode::HotCue),
                );
                DeckEffects::NONE
            }
            SemanticControl::PadModeBeatLoop => {
                self.set_pad_mode_if_pressed(
                    deck,
                    event.value,
                    PadMode::BeatLoop,
                    Some(PerformanceMode::LoopRoll),
                );
                DeckEffects::NONE
            }
            SemanticControl::PadModeBeatJump => {
                self.set_pad_mode_if_pressed(
                    deck,
                    event.value,
                    PadMode::BeatJump,
                    Some(PerformanceMode::BeatJump),
                );
                DeckEffects::NONE
            }
            SemanticControl::PadModePadFx1 => {
                self.set_pad_mode_if_pressed(deck, event.value, PadMode::PadFx1, None);
                DeckEffects::NONE
            }
            SemanticControl::PadModePadFx2 => {
                self.set_pad_mode_if_pressed(deck, event.value, PadMode::PadFx2, None);
                DeckEffects::NONE
            }
            SemanticControl::PadModeKeyboard
            | SemanticControl::PadModeKeyShift
            | SemanticControl::PadModeSampler => DeckEffects::NONE,
            SemanticControl::DeckExtAction => self.handle_deck_ext_action(deck, event.value),
            _ => DeckEffects::NONE,
        }
    }

    fn handle_beat_fx_control(&mut self, event: ControlEvent) -> DeckEffects {
        let changed = match event.control {
            SemanticControl::BeatFxSelectNext if event.value == ControlValue::Pressed(true) => {
                self.beat_fx.effect = next_beat_fx_effect(self.beat_fx.effect);
                true
            }
            SemanticControl::BeatFxSelectPrev if event.value == ControlValue::Pressed(true) => {
                self.beat_fx.effect = previous_beat_fx_effect(self.beat_fx.effect);
                true
            }
            SemanticControl::BeatFxBeatDec if event.value == ControlValue::Pressed(true) => {
                let next = beat_fx_step(self.beat_fx.beat, -1);
                update_value(&mut self.beat_fx.beat, next)
            }
            SemanticControl::BeatFxBeatInc if event.value == ControlValue::Pressed(true) => {
                let next = beat_fx_step(self.beat_fx.beat, 1);
                update_value(&mut self.beat_fx.beat, next)
            }
            SemanticControl::BeatFxBeatDecShift if event.value == ControlValue::Pressed(true) => {
                let next = beat_fx_step(self.beat_fx.beat, -2);
                update_value(&mut self.beat_fx.beat, next)
            }
            SemanticControl::BeatFxBeatIncShift if event.value == ControlValue::Pressed(true) => {
                let next = beat_fx_step(self.beat_fx.beat, 2);
                update_value(&mut self.beat_fx.beat, next)
            }
            SemanticControl::BeatFxTarget => {
                let ControlValue::BeatFxTarget(target) = event.value else {
                    return DeckEffects::NONE;
                };
                self.beat_fx.target = target;
                true
            }
            SemanticControl::BeatFxDepth => {
                let Some(depth) = normalize_beat_fx_depth(event.value) else {
                    return DeckEffects::NONE;
                };
                self.beat_fx.depth = depth;
                true
            }
            SemanticControl::BeatFxOn if event.value == ControlValue::Pressed(true) => {
                self.beat_fx.enabled = !self.beat_fx.enabled;
                true
            }
            SemanticControl::BeatFxClear if event.value == ControlValue::Pressed(true) => {
                self.beat_fx = BeatFxState::new();
                true
            }
            _ => false,
        };

        if changed {
            self.beat_fx_effect()
        } else {
            DeckEffects::NONE
        }
    }

    fn beat_fx_effect(&self) -> DeckEffects {
        DeckEffects::one(DeckEffect::ApplyBeatFx {
            state: self.beat_fx,
            delay_ms: beat_fx_delay_ms(self.beat_fx, &self.decks),
            flanger_period_ms: beat_fx_flanger_period_ms(self.beat_fx, &self.decks),
        })
    }

    pub fn resolve_playback_request(
        &mut self,
        deck: DeckId,
        request_id: u32,
        succeeded: bool,
    ) -> bool {
        let state = &mut self.decks[deck_index(deck)];
        let Some(pending) = state.pending_playback else {
            return false;
        };
        if pending.request_id != request_id {
            return false;
        }

        state.pending_playback = None;
        if succeeded {
            state.playing = pending.playing;
        }
        true
    }

    fn handle_play(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let state = &mut self.decks[deck_index(deck)];
        state.playback_generation = state.playback_generation.wrapping_add(1);
        let request_id = state.playback_generation;
        let desired_now = state
            .pending_playback
            .map(|pending| pending.playing)
            .unwrap_or(state.playing);
        let playing = !desired_now;
        state.pending_playback = Some(PendingPlayback {
            request_id,
            playing,
        });

        DeckEffects::one(DeckEffect::PlaybackRequest {
            deck,
            playing,
            request_id,
        })
    }

    fn handle_cue(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let state = &mut self.decks[deck_index(deck)];
        state.pending_playback = None;
        state.playing = false;
        state.position_ms = state.cue_point_ms;

        DeckEffects::two(
            DeckEffect::Pause { deck },
            DeckEffect::Seek {
                deck,
                position_ms: state.cue_point_ms,
            },
        )
    }

    fn handle_to_start(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let state = &mut self.decks[deck_index(deck)];
        state.pending_playback = None;
        state.playing = false;
        state.position_ms = 0;
        state.cue_point_ms = 0;

        DeckEffects::two(
            DeckEffect::Pause { deck },
            DeckEffect::Seek {
                deck,
                position_ms: 0,
            },
        )
    }

    fn handle_tempo(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        let ControlValue::Absolute { value, max } = value else {
            return DeckEffects::NONE;
        };
        if max == 0 {
            return DeckEffects::NONE;
        }

        let raw = scale_absolute(value, max, PITCH_MAX);
        let state = &mut self.decks[deck_index(deck)];
        state.sync_enabled = false;
        state.pitch_raw = raw;
        state.pitch_centipercent = tempo_centipercent_from_raw(raw, state.tempo_range_percent);

        DeckEffects::one(DeckEffect::SetPitchCentipercent {
            deck,
            value: state.pitch_centipercent,
        })
    }

    fn handle_tempo_range(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let state = &mut self.decks[deck_index(deck)];
        state.sync_enabled = false;
        state.tempo_range_percent = next_tempo_range_percent(state.tempo_range_percent);
        state.pitch_centipercent =
            tempo_centipercent_from_raw(state.pitch_raw, state.tempo_range_percent);

        DeckEffects::one(DeckEffect::SetPitchCentipercent {
            deck,
            value: state.pitch_centipercent,
        })
    }

    fn handle_sync(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let deck_idx = deck_index(deck);
        if self.decks[deck_idx].sync_enabled {
            self.decks[deck_idx].sync_enabled = false;
            return DeckEffects::NONE;
        }

        let reference = match self.sync_master {
            Some(master) if master != deck => master,
            _ => other_deck(deck),
        };
        let reference_idx = deck_index(reference);
        let target_bpm_x100 = analysis[deck_idx]
            .map(|item| item.bpm_x100())
            .filter(|bpm| *bpm > 0)
            .unwrap_or(self.decks[deck_idx].base_bpm_x100);
        let mut reference_state = self.decks[reference_idx];
        reference_state.base_bpm_x100 = analysis[reference_idx]
            .map(|item| item.bpm_x100())
            .filter(|bpm| *bpm > 0)
            .unwrap_or(reference_state.base_bpm_x100);
        let target_centipercent = centipercent_for_bpm_match(target_bpm_x100, reference_state);

        let pitch_effect = DeckEffect::SetPitchCentipercent {
            deck,
            value: target_centipercent,
        };

        let target_position_ms = self.decks[deck_idx].position_ms;
        let reference_position_ms = self.decks[reference_idx].position_ms;
        let aligned_ms = match (analysis[deck_idx], analysis[reference_idx]) {
            (Some(target_analysis), Some(reference_analysis)) => {
                if let (Some(target_grid), Some(reference_grid)) =
                    (target_analysis.beat_grid(), reference_analysis.beat_grid())
                {
                    phase_align_target_ms(
                        target_position_ms,
                        target_grid,
                        reference_position_ms,
                        reference_grid,
                    )
                } else {
                    None
                }
            }
            _ => None,
        };

        let state = &mut self.decks[deck_idx];
        state.sync_enabled = true;
        state.pitch_centipercent = target_centipercent;

        if let Some(position_ms) = aligned_ms {
            state.position_ms = position_ms;
            DeckEffects::two(pitch_effect, DeckEffect::Seek { deck, position_ms })
        } else {
            DeckEffects::one(pitch_effect)
        }
    }

    fn handle_loop_in(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let position_ms = self.quantized_position_ms(deck, analysis);
        self.decks[deck_index(deck)].loop_state.pending_in_ms = Some(position_ms);
        DeckEffects::NONE
    }

    fn handle_loop_out(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let position_ms = self.quantized_position_ms(deck, analysis);
        let index = deck_index(deck);
        let Some(start_ms) = self.decks[index].loop_state.pending_in_ms else {
            return DeckEffects::NONE;
        };
        let Some(region) = LoopRegion::new(start_ms, position_ms) else {
            return DeckEffects::NONE;
        };

        self.decks[index].loop_state.pending_in_ms = None;
        self.decks[index].loop_state.active = Some(region);
        self.decks[index].loop_state.last = Some(region);

        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: region.start_ms,
            end_ms: region.end_ms,
        })
    }

    fn handle_reloop_exit(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let state = &mut self.decks[deck_index(deck)];
        if let Some(active) = state.loop_state.active.take() {
            state.loop_state.last = Some(active);
            state.loop_adjust_mode = LoopAdjustMode::None;
            return DeckEffects::one(DeckEffect::ClearLoop { deck });
        }

        let Some(last) = state.loop_state.last else {
            return DeckEffects::NONE;
        };
        state.loop_state.active = Some(last);

        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: last.start_ms,
            end_ms: last.end_ms,
        })
    }

    fn handle_loop_resize(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        double: bool,
    ) -> DeckEffects {
        if value != ControlValue::Pressed(true) {
            return DeckEffects::NONE;
        }

        let index = deck_index(deck);
        let Some(active) = self.decks[index].loop_state.active else {
            return DeckEffects::NONE;
        };
        let Some(next) = resize_loop_region(active, double) else {
            return DeckEffects::NONE;
        };

        self.decks[index].loop_state.active = Some(next);
        self.decks[index].loop_state.last = Some(next);

        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: next.start_ms,
            end_ms: next.end_ms,
        })
    }

    fn handle_loop_size(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        let ControlValue::Relative(delta) = value else {
            return DeckEffects::NONE;
        };
        if delta == 0 {
            return DeckEffects::NONE;
        }

        let index = deck_index(deck);
        let Some(mut region) = self.decks[index].loop_state.active else {
            return DeckEffects::NONE;
        };
        let steps = (delta as i32).unsigned_abs().min(32);
        let double = delta > 0;
        let mut changed = false;

        for _ in 0..steps {
            let Some(next) = resize_loop_region(region, double) else {
                break;
            };
            region = next;
            changed = true;
        }

        if !changed {
            return DeckEffects::NONE;
        }

        self.decks[index].loop_state.active = Some(region);
        self.decks[index].loop_state.last = Some(region);

        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: region.start_ms,
            end_ms: region.end_ms,
        })
    }

    fn quantized_position_ms(&self, deck: DeckId, analysis: [Option<TrackAnalysis<'_>>; 2]) -> u32 {
        let index = deck_index(deck);
        let position_ms = self.decks[index].position_ms;
        if !self.decks[index].quantize_enabled {
            return position_ms;
        }

        let Some(grid) = analysis[index].and_then(|item| item.beat_grid()) else {
            return position_ms;
        };
        let Some(beat_index) = grid.nearest_index(position_ms) else {
            return position_ms;
        };
        grid.beats()[beat_index].time_ms
    }

    fn handle_beat_jump(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        beat_numerator: i32,
        beat_denominator: u16,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        if value != ControlValue::Pressed(true) || beat_numerator == 0 {
            return DeckEffects::NONE;
        }

        self.apply_beat_jump(deck, beat_numerator, beat_denominator, analysis)
    }

    fn handle_pad_action(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        let ControlValue::PadAction(action) = value else {
            return DeckEffects::NONE;
        };

        match action.mode {
            PadMode::HotCue => {
                if action.pressed {
                    self.handle_hot_cue_pad_action(deck, action.pad, action.shifted)
                } else {
                    DeckEffects::NONE
                }
            }
            PadMode::BeatJump => {
                if !action.pressed {
                    return DeckEffects::NONE;
                }

                if action.shifted {
                    match action.pad {
                        6 => self.change_beat_jump_page(-1),
                        7 => self.change_beat_jump_page(1),
                        _ => {}
                    }
                    return DeckEffects::NONE;
                }

                let Some((numerator, denominator)) = self.beat_jump_size_for_pad(action.pad) else {
                    return DeckEffects::NONE;
                };
                self.apply_beat_jump(deck, numerator, denominator, analysis)
            }
            PadMode::BeatLoop => {
                if action.shifted {
                    if action.pressed {
                        self.handle_shifted_beat_loop_press(deck, action.pad, analysis)
                    } else {
                        self.handle_shifted_beat_loop_release(deck)
                    }
                } else if action.pressed {
                    self.handle_beat_loop_pad_action(deck, action.pad, analysis)
                } else {
                    DeckEffects::NONE
                }
            }
            PadMode::PadFx1 | PadMode::PadFx2 => {
                self.handle_pad_fx_action(deck, action.mode, action.pad, action.pressed)
            }
            _ => DeckEffects::NONE,
        }
    }

    fn handle_pad_fx_action(
        &mut self,
        deck: DeckId,
        mode: PadMode,
        pad: u8,
        active: bool,
    ) -> DeckEffects {
        let bank = match mode {
            PadMode::PadFx1 => PadFxBank::One,
            PadMode::PadFx2 => PadFxBank::Two,
            _ => return DeckEffects::NONE,
        };

        let state = &mut self.decks[deck_index(deck)];
        if active {
            state.active_pad_fx = Some(ActivePadFx { bank, pad });
        } else if state.active_pad_fx == Some(ActivePadFx { bank, pad }) {
            state.active_pad_fx = None;
        }

        DeckEffects::one(DeckEffect::ApplyPadFx {
            deck,
            bank,
            pad,
            active,
        })
    }

    fn handle_hot_cue_pad_action(&mut self, deck: DeckId, pad: u8, shifted: bool) -> DeckEffects {
        let index = deck_index(deck);
        let (Some(source), Some(mut local)) =
            (self.hot_cue_sources[index], self.hot_cue_locals[index])
        else {
            return DeckEffects::NONE;
        };
        let Some(effective) = HotCueBank::merged(source, local) else {
            return DeckEffects::NONE;
        };

        if shifted {
            if effective.slot(pad).is_none() || !local.clear(pad) {
                return DeckEffects::NONE;
            }
            self.hot_cue_locals[index] = Some(local);
            return DeckEffects::one(DeckEffect::PersistHotCues { deck, bank: local });
        }

        if let Some(cue) = effective.slot(pad) {
            self.decks[index].position_ms = cue.pos_ms;
            return DeckEffects::one(DeckEffect::Seek {
                deck,
                position_ms: cue.pos_ms,
            });
        }

        let position_ms = self.decks[index].position_ms;
        if !local.set_single(pad, position_ms) {
            return DeckEffects::NONE;
        }
        self.hot_cue_locals[index] = Some(local);
        DeckEffects::one(DeckEffect::PersistHotCues { deck, bank: local })
    }

    fn handle_beat_loop_pad_action(
        &mut self,
        deck: DeckId,
        pad: u8,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        let Some((numerator, denominator)) = BEAT_LOOP_LENGTHS.get(pad as usize).copied() else {
            return DeckEffects::NONE;
        };

        let index = deck_index(deck);
        let state = self.decks[index];
        let track_analysis = analysis[index];
        let bpm_x100 = track_analysis
            .map(|item| item.bpm_x100())
            .filter(|value| *value > 0)
            .unwrap_or(state.base_bpm_x100);
        let beat_grid = track_analysis.and_then(|item| item.beat_grid());
        let duration_ms = beat_loop_duration_ms(
            state.position_ms,
            bpm_x100,
            numerator,
            denominator,
            beat_grid,
        );
        let Some(end_ms) = state.position_ms.checked_add(duration_ms) else {
            return DeckEffects::NONE;
        };
        let Some(region) = LoopRegion::new(state.position_ms, end_ms) else {
            return DeckEffects::NONE;
        };

        self.decks[index].loop_state.active = Some(region);
        self.decks[index].loop_state.last = Some(region);

        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: region.start_ms,
            end_ms: region.end_ms,
        })
    }

    fn handle_shifted_beat_loop_press(
        &mut self,
        deck: DeckId,
        pad: u8,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        let index = deck_index(deck);
        if !self.shifted_loop_roll[index].active {
            self.shifted_loop_roll[index] = ShiftedLoopRoll {
                active: true,
                previous: self.decks[index].loop_state.active,
            };
        }

        self.handle_beat_loop_pad_action(deck, pad, analysis)
    }

    fn handle_shifted_beat_loop_release(&mut self, deck: DeckId) -> DeckEffects {
        let index = deck_index(deck);
        let roll = self.shifted_loop_roll[index];
        if !roll.active {
            return DeckEffects::NONE;
        }
        self.shifted_loop_roll[index] = ShiftedLoopRoll::new();

        if let Some(previous) = roll.previous {
            self.decks[index].loop_state.active = Some(previous);
            self.decks[index].loop_state.last = Some(previous);
            return DeckEffects::one(DeckEffect::SetLoop {
                deck,
                start_ms: previous.start_ms,
                end_ms: previous.end_ms,
            });
        }

        self.decks[index].loop_state.active = None;
        self.decks[index].loop_adjust_mode = LoopAdjustMode::None;
        DeckEffects::one(DeckEffect::ClearLoop { deck })
    }

    fn apply_beat_jump(
        &mut self,
        deck: DeckId,
        beat_numerator: i32,
        beat_denominator: u16,
        analysis: [Option<TrackAnalysis<'_>>; 2],
    ) -> DeckEffects {
        let index = deck_index(deck);
        let state = self.decks[index];
        let track_analysis = analysis[index];
        let bpm_x100 = track_analysis
            .map(|item| item.bpm_x100())
            .filter(|value| *value > 0)
            .unwrap_or(state.base_bpm_x100);
        let beat_grid = track_analysis.and_then(|item| item.beat_grid());
        let target_ms = beat_jump_target_ms(
            state.position_ms,
            bpm_x100,
            beat_numerator,
            beat_denominator,
            beat_grid,
        );

        self.decks[index].position_ms = target_ms;
        DeckEffects::one(DeckEffect::Seek {
            deck,
            position_ms: target_ms,
        })
    }

    fn beat_jump_size_for_pad(&self, pad: u8) -> Option<(i32, u16)> {
        if pad >= 8 {
            return None;
        }

        let sizes = match self.beat_jump_page {
            BeatJumpPage::Fractional => &BEAT_JUMP_FRACTIONAL,
            BeatJumpPage::Default => &BEAT_JUMP_DEFAULT,
            BeatJumpPage::Large => &BEAT_JUMP_LARGE,
        };
        Some(sizes[pad as usize])
    }

    fn change_beat_jump_page(&mut self, delta: i8) {
        self.beat_jump_page = match (self.beat_jump_page, delta.cmp(&0)) {
            (BeatJumpPage::Fractional, core::cmp::Ordering::Greater) => BeatJumpPage::Default,
            (BeatJumpPage::Default, core::cmp::Ordering::Greater) => BeatJumpPage::Large,
            (BeatJumpPage::Large, core::cmp::Ordering::Greater) => BeatJumpPage::Large,
            (BeatJumpPage::Large, core::cmp::Ordering::Less) => BeatJumpPage::Default,
            (BeatJumpPage::Default, core::cmp::Ordering::Less) => BeatJumpPage::Fractional,
            (BeatJumpPage::Fractional, core::cmp::Ordering::Less) => BeatJumpPage::Fractional,
            (page, core::cmp::Ordering::Equal) => page,
        };
    }

    fn set_pad_mode_if_pressed(
        &mut self,
        deck: DeckId,
        value: ControlValue,
        pad_mode: PadMode,
        perf_mode: Option<PerformanceMode>,
    ) {
        if value != ControlValue::Pressed(true) {
            return;
        }
        let state = &mut self.decks[deck_index(deck)];
        state.pad_mode = pad_mode;
        if let Some(perf_mode) = perf_mode {
            state.perf_mode = perf_mode;
        }
    }

    fn handle_loop_adjust_jog(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        let ControlValue::Relative(delta) = value else {
            return DeckEffects::NONE;
        };

        let index = deck_index(deck);
        let mode = self.decks[index].loop_adjust_mode;
        if mode == LoopAdjustMode::None {
            return DeckEffects::NONE;
        }

        let Some(active) = self.decks[index].loop_state.active else {
            self.decks[index].loop_adjust_mode = LoopAdjustMode::None;
            return DeckEffects::NONE;
        };
        if delta == 0 {
            return DeckEffects::NONE;
        }

        let movement_ms = delta as i64;
        let next = match mode {
            LoopAdjustMode::In => {
                let target =
                    (active.start_ms as i64 + movement_ms).clamp(0, active.end_ms as i64 - 1);
                LoopRegion::new(target as u32, active.end_ms)
            }
            LoopAdjustMode::Out => {
                let target = (active.end_ms as i64 + movement_ms)
                    .clamp(active.start_ms as i64 + 1, u32::MAX as i64);
                LoopRegion::new(active.start_ms, target as u32)
            }
            LoopAdjustMode::None => None,
        };
        let Some(next) = next else {
            self.decks[index].loop_adjust_mode = LoopAdjustMode::None;
            return DeckEffects::NONE;
        };

        self.decks[index].loop_state.active = Some(next);
        self.decks[index].loop_state.last = Some(next);
        DeckEffects::one(DeckEffect::SetLoop {
            deck,
            start_ms: next.start_ms,
            end_ms: next.end_ms,
        })
    }

    fn set_loop_adjust_mode(&mut self, deck: DeckId, requested: LoopAdjustMode) {
        let index = deck_index(deck);
        if self.decks[index].loop_state.active.is_none() {
            self.decks[index].loop_adjust_mode = LoopAdjustMode::None;
            return;
        }

        self.decks[index].loop_adjust_mode = if self.decks[index].loop_adjust_mode == requested {
            LoopAdjustMode::None
        } else {
            requested
        };
    }

    fn handle_deck_ext_action(&mut self, deck: DeckId, value: ControlValue) -> DeckEffects {
        let ControlValue::DeckExtAction(action) = value else {
            return DeckEffects::NONE;
        };

        match action.action {
            DeckExtAction::SyncMaster if action.pressed => {
                self.sync_master = Some(deck);
                for candidate in [DeckId::One, DeckId::Two] {
                    self.decks[deck_index(candidate)].sync_master = candidate == deck;
                }
                self.decks[deck_index(deck)].sync_enabled = false;
            }
            DeckExtAction::Quantize if action.pressed => {
                let state = &mut self.decks[deck_index(deck)];
                state.quantize_enabled = !state.quantize_enabled;
            }
            DeckExtAction::SyncOff if action.pressed => {
                self.decks[deck_index(deck)].sync_enabled = false;
            }
            DeckExtAction::ReloopStop if action.pressed => {
                let state = &mut self.decks[deck_index(deck)];
                state.loop_state = LoopState::new();
                state.loop_adjust_mode = LoopAdjustMode::None;
                return DeckEffects::one(DeckEffect::ClearLoop { deck });
            }
            DeckExtAction::LoopAdjustIn if action.pressed => {
                self.set_loop_adjust_mode(deck, LoopAdjustMode::In);
            }
            DeckExtAction::LoopAdjustOut if action.pressed => {
                self.set_loop_adjust_mode(deck, LoopAdjustMode::Out);
            }
            DeckExtAction::Censor => {
                // Censor remains an audio-runtime slice.
            }
            _ => {}
        }

        DeckEffects::NONE
    }
}

fn resize_loop_region(region: LoopRegion, double: bool) -> Option<LoopRegion> {
    let duration = region.duration_ms();
    let next_duration = if double {
        duration.checked_mul(2)?
    } else {
        if duration < 2 {
            return None;
        }
        duration / 2
    };
    let end_ms = region.start_ms.checked_add(next_duration)?;
    LoopRegion::new(region.start_ms, end_ms)
}

fn is_beat_fx_control(control: SemanticControl) -> bool {
    matches!(
        control,
        SemanticControl::BeatFxSelectNext
            | SemanticControl::BeatFxSelectPrev
            | SemanticControl::BeatFxBeatDec
            | SemanticControl::BeatFxBeatInc
            | SemanticControl::BeatFxTarget
            | SemanticControl::BeatFxDepth
            | SemanticControl::BeatFxOn
            | SemanticControl::BeatFxClear
            | SemanticControl::BeatFxBeatDecShift
            | SemanticControl::BeatFxBeatIncShift
    )
}

fn next_beat_fx_effect(effect: BeatFxEffect) -> BeatFxEffect {
    match effect {
        BeatFxEffect::Filter => BeatFxEffect::Echo,
        BeatFxEffect::Echo => BeatFxEffect::Flanger,
        BeatFxEffect::Flanger => BeatFxEffect::Delay,
        BeatFxEffect::Delay => BeatFxEffect::Filter,
    }
}

fn previous_beat_fx_effect(effect: BeatFxEffect) -> BeatFxEffect {
    match effect {
        BeatFxEffect::Filter => BeatFxEffect::Delay,
        BeatFxEffect::Delay => BeatFxEffect::Flanger,
        BeatFxEffect::Flanger => BeatFxEffect::Echo,
        BeatFxEffect::Echo => BeatFxEffect::Filter,
    }
}

fn beat_fx_step(beat: BeatFxBeat, delta: i8) -> BeatFxBeat {
    let index = match beat {
        BeatFxBeat::Quarter => 0i8,
        BeatFxBeat::Half => 1,
        BeatFxBeat::One => 2,
        BeatFxBeat::Two => 3,
        BeatFxBeat::Four => 4,
    };
    match (index + delta).clamp(0, 4) {
        0 => BeatFxBeat::Quarter,
        1 => BeatFxBeat::Half,
        2 => BeatFxBeat::One,
        3 => BeatFxBeat::Two,
        _ => BeatFxBeat::Four,
    }
}

fn beat_fx_ratio(beat: BeatFxBeat) -> (u32, u32) {
    match beat {
        BeatFxBeat::Quarter => (1, 4),
        BeatFxBeat::Half => (1, 2),
        BeatFxBeat::One => (1, 1),
        BeatFxBeat::Two => (2, 1),
        BeatFxBeat::Four => (4, 1),
    }
}

fn beat_fx_target_bpm_x100(state: BeatFxState, decks: &[DeckState; 2]) -> u32 {
    let deck = match state.target {
        BeatFxTarget::ChannelTwo => decks[1],
        BeatFxTarget::ChannelOne | BeatFxTarget::Both => decks[0],
    };
    let factor = 10_000i64 + deck.pitch_centipercent as i64;
    let effective = ((deck.base_bpm_x100 as i64 * factor) + 5_000) / 10_000;
    if !(4_000..=30_000).contains(&effective) {
        12_000
    } else {
        effective as u32
    }
}

fn beat_fx_unclamped_time_ms(state: BeatFxState, decks: &[DeckState; 2]) -> u32 {
    let (numerator, denominator) = beat_fx_ratio(state.beat);
    let bpm_x100 = beat_fx_target_bpm_x100(state, decks) as u64;
    let divisor = bpm_x100 * denominator as u64;
    let scaled = 6_000_000u64 * numerator as u64;
    ((scaled + divisor / 2) / divisor)
        .max(1)
        .min(u32::MAX as u64) as u32
}

fn beat_fx_delay_ms(state: BeatFxState, decks: &[DeckState; 2]) -> u32 {
    beat_fx_unclamped_time_ms(state, decks).min(1_000)
}

fn beat_fx_flanger_period_ms(state: BeatFxState, decks: &[DeckState; 2]) -> u32 {
    beat_fx_unclamped_time_ms(state, decks).clamp(100, 8_000)
}

fn normalize_beat_fx_depth(value: ControlValue) -> Option<u8> {
    let ControlValue::Absolute { value, max } = value else {
        return None;
    };
    if max == 0 {
        return None;
    }
    let bounded = value.min(max) as u32;
    let scaled = (bounded * 127 + max as u32 / 2) / max as u32;
    Some(scaled.min(127) as u8)
}

fn update_value<T: Copy + PartialEq>(target: &mut T, next: T) -> bool {
    if *target == next {
        false
    } else {
        *target = next;
        true
    }
}

pub const fn tempo_centipercent_from_raw(raw: u16, range_percent: u16) -> i16 {
    let raw = if raw > PITCH_MAX { PITCH_MAX } else { raw } as i32;
    let centi_range = range_percent as i32 * 100;
    ((PITCH_CENTER as i32 - raw) * centi_range / PITCH_CENTER as i32) as i16
}

pub const fn next_tempo_range_percent(current: u16) -> u16 {
    match current {
        6 => 10,
        10 => 16,
        _ => 6,
    }
}

fn scale_absolute(value: u16, max: u16, target_max: u16) -> u16 {
    if max == target_max {
        return value.min(target_max);
    }
    let scaled = (value as u32 * target_max as u32 + max as u32 / 2) / max as u32;
    scaled.min(target_max as u32) as u16
}

fn centipercent_for_bpm_match(target_base_x100: u32, reference: DeckState) -> i16 {
    let target_base_x100 = if target_base_x100 == 0 {
        12_000
    } else {
        target_base_x100
    };
    let reference_base_x100 = if reference.base_bpm_x100 == 0 {
        12_000
    } else {
        reference.base_bpm_x100
    };
    let reference_factor = 10_000i64 + reference.pitch_centipercent as i64;
    let numerator = reference_base_x100 as i64 * reference_factor;
    let ratio_x10_000 = div_round_nearest_positive(numerator, target_base_x100 as i64);
    let centipercent = ratio_x10_000 - 10_000;
    centipercent.clamp(
        -(BEAT_SYNC_MAX_PERCENT as i64) * 100,
        BEAT_SYNC_MAX_PERCENT as i64 * 100,
    ) as i16
}

fn div_round_nearest_positive(numerator: i64, denominator: i64) -> i64 {
    (numerator + denominator / 2) / denominator
}

const fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

const fn other_deck(deck: DeckId) -> DeckId {
    match deck {
        DeckId::One => DeckId::Two,
        DeckId::Two => DeckId::One,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_controller_core::{BeatFxTarget, DeckExtActionValue, PadAction, SemanticControl};
    use pajoniiir_hot_cues::HotCueSlot;
    use pajoniiir_track_analysis::{AnalysisProvider, Beat, BeatGrid};

    fn system_pressed(control: SemanticControl, down: bool) -> ControlEvent {
        ControlEvent {
            deck: None,
            control,
            value: ControlValue::Pressed(down),
        }
    }

    fn beat_fx_depth(value: u16, max: u16) -> ControlEvent {
        ControlEvent {
            deck: None,
            control: SemanticControl::BeatFxDepth,
            value: ControlValue::Absolute { value, max },
        }
    }

    fn beat_fx_target(target: BeatFxTarget) -> ControlEvent {
        ControlEvent {
            deck: None,
            control: SemanticControl::BeatFxTarget,
            value: ControlValue::BeatFxTarget(target),
        }
    }

    fn beat_fx_command(effects: DeckEffects) -> (BeatFxState, u32, u32) {
        match effects.items[0].unwrap() {
            DeckEffect::ApplyBeatFx {
                state,
                delay_ms,
                flanger_period_ms,
            } => (state, delay_ms, flanger_period_ms),
            other => panic!("expected Beat FX command, got {other:?}"),
        }
    }

    fn pressed(deck: DeckId, control: SemanticControl, value: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Pressed(value),
        }
    }

    fn relative(deck: DeckId, control: SemanticControl, value: i16) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Relative(value),
        }
    }

    fn loop_region(start_ms: u32, end_ms: u32) -> LoopRegion {
        LoopRegion::new(start_ms, end_ms).unwrap()
    }

    fn absolute(deck: DeckId, control: SemanticControl, value: u16) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Absolute {
                value,
                max: PITCH_MAX,
            },
        }
    }

    fn ext(deck: DeckId, action: DeckExtAction, pressed: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::DeckExtAction,
            value: ControlValue::DeckExtAction(DeckExtActionValue { action, pressed }),
        }
    }

    fn playback_request(effects: DeckEffects) -> (DeckId, bool, u32) {
        match effects.items[0].unwrap() {
            DeckEffect::PlaybackRequest {
                deck,
                playing,
                request_id,
            } => (deck, playing, request_id),
            other => panic!("expected playback request, got {other:?}"),
        }
    }

    #[test]
    fn beat_fx_defaults_match_released_product_state() {
        let state = DeckProductState::new();
        assert_eq!(
            state.beat_fx(),
            BeatFxState {
                effect: BeatFxEffect::Filter,
                beat: BeatFxBeat::One,
                target: BeatFxTarget::Both,
                depth: 64,
                enabled: false,
            }
        );
    }

    #[test]
    fn beat_fx_effect_selector_cycles_without_none() {
        let mut state = DeckProductState::new();

        state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        assert_eq!(state.beat_fx().effect, BeatFxEffect::Echo);
        state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        assert_eq!(state.beat_fx().effect, BeatFxEffect::Flanger);
        state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        assert_eq!(state.beat_fx().effect, BeatFxEffect::Delay);
        state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        assert_eq!(state.beat_fx().effect, BeatFxEffect::Filter);

        state.handle_control(system_pressed(SemanticControl::BeatFxSelectPrev, true));
        assert_eq!(state.beat_fx().effect, BeatFxEffect::Delay);
    }

    #[test]
    fn beat_fx_beat_buttons_and_shifted_buttons_clamp_like_released_core() {
        let mut state = DeckProductState::new();

        state.handle_control(system_pressed(SemanticControl::BeatFxBeatInc, true));
        assert_eq!(state.beat_fx().beat, BeatFxBeat::Two);
        state.handle_control(system_pressed(SemanticControl::BeatFxBeatIncShift, true));
        assert_eq!(state.beat_fx().beat, BeatFxBeat::Four);
        assert_eq!(
            state.handle_control(system_pressed(SemanticControl::BeatFxBeatInc, true)),
            DeckEffects::NONE
        );

        state.handle_control(system_pressed(SemanticControl::BeatFxBeatDecShift, true));
        assert_eq!(state.beat_fx().beat, BeatFxBeat::One);
        state.handle_control(system_pressed(SemanticControl::BeatFxBeatDecShift, true));
        assert_eq!(state.beat_fx().beat, BeatFxBeat::Quarter);
    }

    #[test]
    fn beat_fx_target_depth_on_and_clear_emit_authoritative_commands() {
        let mut state = DeckProductState::new();

        let target = state.handle_control(beat_fx_target(BeatFxTarget::ChannelTwo));
        let (beat_fx, delay_ms, flanger_ms) = beat_fx_command(target);
        assert_eq!(beat_fx.target, BeatFxTarget::ChannelTwo);
        assert_eq!(delay_ms, 500);
        assert_eq!(flanger_ms, 500);

        let depth = state.handle_control(beat_fx_depth(127, 127));
        let (beat_fx, _, _) = beat_fx_command(depth);
        assert_eq!(beat_fx.depth, 127);

        let on = state.handle_control(system_pressed(SemanticControl::BeatFxOn, true));
        let (beat_fx, _, _) = beat_fx_command(on);
        assert!(beat_fx.enabled);
        assert_eq!(
            state.handle_control(system_pressed(SemanticControl::BeatFxOn, false)),
            DeckEffects::NONE
        );
        assert!(state.beat_fx().enabled);

        let clear = state.handle_control(system_pressed(SemanticControl::BeatFxClear, true));
        let (beat_fx, _, _) = beat_fx_command(clear);
        assert_eq!(beat_fx, BeatFxState::new());
    }

    #[test]
    fn beat_fx_target_depth_and_clear_reemit_for_state_replay() {
        let mut state = DeckProductState::new();

        assert!(matches!(
            state
                .handle_control(beat_fx_target(BeatFxTarget::Both))
                .items[0],
            Some(DeckEffect::ApplyBeatFx { .. })
        ));
        assert!(matches!(
            state.handle_control(beat_fx_depth(64, 127)).items[0],
            Some(DeckEffect::ApplyBeatFx { .. })
        ));
        assert!(matches!(
            state
                .handle_control(system_pressed(SemanticControl::BeatFxClear, true))
                .items[0],
            Some(DeckEffect::ApplyBeatFx { .. })
        ));
    }

    #[test]
    fn beat_fx_timing_uses_target_effective_bpm_and_released_caps() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 10_000);
        state.handle_control(absolute(DeckId::Two, SemanticControl::Tempo, 0));

        state.handle_control(beat_fx_target(BeatFxTarget::ChannelTwo));
        let effects = state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        let (_, delay_ms, flanger_ms) = beat_fx_command(effects);
        assert_eq!(delay_ms, 545);
        assert_eq!(flanger_ms, 545);

        state.set_base_bpm_x100(DeckId::Two, 4_000);
        state.handle_control(absolute(DeckId::Two, SemanticControl::Tempo, PITCH_CENTER));
        state.handle_control(system_pressed(SemanticControl::BeatFxBeatIncShift, true));
        state.handle_control(system_pressed(SemanticControl::BeatFxBeatInc, true));
        let effects = state.handle_control(system_pressed(SemanticControl::BeatFxSelectNext, true));
        let (_, delay_ms, flanger_ms) = beat_fx_command(effects);
        assert_eq!(delay_ms, 1_000);
        assert_eq!(flanger_ms, 6_000);

        state.set_base_bpm_x100(DeckId::Two, 35_000);
        let effects = state.handle_control(beat_fx_depth(64, 127));
        let (_, delay_ms, flanger_ms) = beat_fx_command(effects);
        assert_eq!(delay_ms, 1_000);
        assert_eq!(flanger_ms, 2_000);
    }

    #[test]
    fn beat_fx_depth_scales_absolute_sources_to_seven_bit_domain() {
        let mut state = DeckProductState::new();
        let effects = state.handle_control(beat_fx_depth(64, 255));
        let (beat_fx, _, _) = beat_fx_command(effects);
        assert_eq!(beat_fx.depth, 32);
    }

    #[test]
    fn released_defaults_are_preserved() {
        let state = DeckProductState::new();
        for deck in [DeckId::One, DeckId::Two] {
            assert_eq!(state.deck(deck).tempo_range_percent, 10);
            assert_eq!(state.deck(deck).pitch_raw, PITCH_CENTER);
            assert_eq!(state.deck(deck).pitch_centipercent, 0);
            assert_eq!(state.deck(deck).pad_mode, PadMode::HotCue);
        }
    }

    #[test]
    fn decks_track_transport_independently() {
        let mut state = DeckProductState::new();

        let (_, target, request) = playback_request(state.handle_control(pressed(
            DeckId::One,
            SemanticControl::Play,
            true,
        )));
        assert!(target);
        assert!(state.resolve_playback_request(DeckId::One, request, true));
        assert!(state.deck(DeckId::One).playing);
        assert!(!state.deck(DeckId::Two).playing);

        let (_, target, request) = playback_request(state.handle_control(pressed(
            DeckId::Two,
            SemanticControl::Play,
            true,
        )));
        assert!(target);
        assert!(state.resolve_playback_request(DeckId::Two, request, true));
        assert!(state.deck(DeckId::One).playing);
        assert!(state.deck(DeckId::Two).playing);

        state.handle_control(pressed(DeckId::Two, SemanticControl::Cue, true));
        assert!(state.deck(DeckId::One).playing);
        assert!(!state.deck(DeckId::Two).playing);
    }

    #[test]
    fn failed_playback_request_does_not_mark_deck_playing() {
        let mut state = DeckProductState::new();
        let (_, _, request) = playback_request(state.handle_control(pressed(
            DeckId::Two,
            SemanticControl::Play,
            true,
        )));

        assert!(state.resolve_playback_request(DeckId::Two, request, false));
        assert!(!state.deck(DeckId::Two).playing);
    }

    #[test]
    fn stale_playback_completion_cannot_overwrite_newer_request() {
        let mut state = DeckProductState::new();
        let (_, _, first) = playback_request(state.handle_control(pressed(
            DeckId::One,
            SemanticControl::Play,
            true,
        )));
        let (_, second_target, second) = playback_request(state.handle_control(pressed(
            DeckId::One,
            SemanticControl::Play,
            true,
        )));

        assert!(!second_target);
        assert!(!state.resolve_playback_request(DeckId::One, first, true));
        assert!(!state.deck(DeckId::One).playing);
        assert!(state.resolve_playback_request(DeckId::One, second, true));
        assert!(!state.deck(DeckId::One).playing);
    }

    #[test]
    fn decks_track_pitch_independently() {
        let mut state = DeckProductState::new();

        state.handle_control(absolute(DeckId::One, SemanticControl::Tempo, 7000));
        state.handle_control(absolute(DeckId::Two, SemanticControl::Tempo, 9600));

        assert_eq!(state.deck(DeckId::One).pitch_raw, 7000);
        assert_eq!(state.deck(DeckId::Two).pitch_raw, 9600);
    }

    #[test]
    fn tempo_range_cycles_requested_deck_only() {
        let mut state = DeckProductState::new();
        let d1 = pressed(DeckId::One, SemanticControl::TempoRange, true);
        let d2 = pressed(DeckId::Two, SemanticControl::TempoRange, true);

        state.handle_control(d1);
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 16);
        assert_eq!(state.deck(DeckId::Two).tempo_range_percent, 10);

        state.handle_control(d1);
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 6);
        state.handle_control(d1);
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 10);

        state.handle_control(d2);
        assert_eq!(state.deck(DeckId::Two).tempo_range_percent, 16);
    }

    #[test]
    fn tempo_range_release_does_not_cycle() {
        let mut state = DeckProductState::new();
        state.handle_control(pressed(DeckId::One, SemanticControl::TempoRange, false));
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 10);
    }

    #[test]
    fn pitch_mapping_matches_released_integer_contract() {
        let mut state = DeckProductState::new();

        state.handle_control(absolute(DeckId::One, SemanticControl::Tempo, 0));
        state.handle_control(absolute(DeckId::Two, SemanticControl::Tempo, PITCH_MAX));

        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 1000);
        assert_eq!(state.deck(DeckId::Two).pitch_centipercent, -999);

        state.handle_control(pressed(DeckId::One, SemanticControl::TempoRange, true));
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 16);
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 1600);
    }

    #[test]
    fn tempo_range_change_reapplies_current_pitch() {
        let mut state = DeckProductState::new();

        state.handle_control(absolute(DeckId::One, SemanticControl::Tempo, 4096));
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 500);

        let effects = state.handle_control(pressed(DeckId::One, SemanticControl::TempoRange, true));
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 800);
        assert_eq!(
            effects.items[0],
            Some(DeckEffect::SetPitchCentipercent {
                deck: DeckId::One,
                value: 800,
            })
        );
    }

    #[test]
    fn sync_toggles_requested_deck_only() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 12_000);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert!(state.deck(DeckId::One).sync_enabled);
        assert!(!state.deck(DeckId::Two).sync_enabled);

        state.handle_control(pressed(DeckId::Two, SemanticControl::Sync, true));
        assert!(state.deck(DeckId::One).sync_enabled);
        assert!(state.deck(DeckId::Two).sync_enabled);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert!(!state.deck(DeckId::One).sync_enabled);
        assert!(state.deck(DeckId::Two).sync_enabled);
    }

    #[test]
    fn sync_master_marks_requested_deck_as_reference() {
        let mut state = DeckProductState::new();
        state.handle_control(ext(DeckId::One, DeckExtAction::SyncMaster, true));

        assert!(state.deck(DeckId::One).sync_master);
        assert!(!state.deck(DeckId::Two).sync_master);
        assert!(!state.deck(DeckId::One).sync_enabled);
    }

    #[test]
    fn sync_matches_other_deck_bpm_and_can_exceed_selected_range() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 12_800);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 667);

        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 10_000);
        state.set_base_bpm_x100(DeckId::Two, 11_700);
        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert_eq!(state.deck(DeckId::One).tempo_range_percent, 10);
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 1700);
    }

    #[test]
    fn sync_clamps_to_twenty_percent_safe_limit() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 10_000);
        state.set_base_bpm_x100(DeckId::Two, 13_000);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 2000);
    }

    #[test]
    fn sync_phase_aligns_with_provider_neutral_beatgrids() {
        use pajoniiir_track_analysis::{AnalysisProvider, Beat, BeatGrid};

        let target_beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 1500,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2500,
                phase: 3,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3500,
                phase: 1,
                bpm_x100: 12_000,
            },
        ];
        let reference_beats = [
            Beat {
                time_ms: 8000,
                phase: 0,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 8469,
                phase: 1,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 8938,
                phase: 2,
                bpm_x100: 12_800,
            },
            Beat {
                time_ms: 9407,
                phase: 3,
                bpm_x100: 12_800,
            },
        ];

        let target = TrackAnalysis::new(
            AnalysisProvider::RekordboxImport,
            1,
            12_000,
            Some(BeatGrid::new(&target_beats)),
        );
        let reference = TrackAnalysis::new(
            AnalysisProvider::AptaNative,
            4,
            12_800,
            Some(BeatGrid::new(&reference_beats)),
        );

        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 2600);
        state.set_position_ms(DeckId::Two, 8900);

        let effects = state.handle_control_with_analysis(
            pressed(DeckId::One, SemanticControl::Sync, true),
            [Some(target), Some(reference)],
        );

        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 667);
        assert_eq!(state.deck(DeckId::One).position_ms, 1962);
        assert_eq!(
            effects,
            DeckEffects::two(
                DeckEffect::SetPitchCentipercent {
                    deck: DeckId::One,
                    value: 667,
                },
                DeckEffect::Seek {
                    deck: DeckId::One,
                    position_ms: 1962,
                },
            )
        );
    }

    #[test]
    fn sync_without_both_beatgrids_keeps_phase_position_unchanged() {
        use pajoniiir_track_analysis::{AnalysisProvider, Beat, BeatGrid};

        let target_beats = [Beat {
            time_ms: 1000,
            phase: 0,
            bpm_x100: 12_000,
        }];
        let target = TrackAnalysis::new(
            AnalysisProvider::RekordboxImport,
            1,
            12_000,
            Some(BeatGrid::new(&target_beats)),
        );
        let reference = TrackAnalysis::new(AnalysisProvider::AptaCache, 2, 12_800, None);

        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 2600);
        state.set_position_ms(DeckId::Two, 8900);

        let effects = state.handle_control_with_analysis(
            pressed(DeckId::One, SemanticControl::Sync, true),
            [Some(target), Some(reference)],
        );

        assert_eq!(state.deck(DeckId::One).position_ms, 2600);
        assert_eq!(
            effects.items[0],
            Some(DeckEffect::SetPitchCentipercent {
                deck: DeckId::One,
                value: 667,
            })
        );
        assert_eq!(effects.items[1], None);
    }

    #[test]
    fn sync_uses_other_decks_effective_bpm() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 10_000);
        state.handle_control(absolute(DeckId::Two, SemanticControl::Tempo, 0));
        assert_eq!(state.deck(DeckId::Two).pitch_centipercent, 1000);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, -833);
    }

    #[test]
    fn selected_sync_master_is_used_as_reference() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 10_000);
        state.set_base_bpm_x100(DeckId::Two, 12_500);
        state.handle_control(ext(DeckId::One, DeckExtAction::SyncMaster, true));

        state.handle_control(pressed(DeckId::Two, SemanticControl::Sync, true));
        assert!(state.deck(DeckId::Two).sync_enabled);
        assert_eq!(state.deck(DeckId::Two).pitch_centipercent, -2000);
    }

    #[test]
    fn sync_toggle_off_does_not_reapply_pitch() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 12_800);

        let first = state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert!(first.items[0].is_some());
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 667);

        let second = state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert_eq!(second, DeckEffects::NONE);
        assert!(!state.deck(DeckId::One).sync_enabled);
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 667);
    }

    #[test]
    fn manual_pitch_disables_sync_state() {
        let mut state = DeckProductState::new();
        state.set_base_bpm_x100(DeckId::One, 12_000);
        state.set_base_bpm_x100(DeckId::Two, 12_800);

        state.handle_control(pressed(DeckId::One, SemanticControl::Sync, true));
        assert!(state.deck(DeckId::One).sync_enabled);

        state.handle_control(absolute(DeckId::One, SemanticControl::Tempo, PITCH_CENTER));
        assert!(!state.deck(DeckId::One).sync_enabled);
        assert_eq!(state.deck(DeckId::One).pitch_centipercent, 0);
    }

    #[test]
    fn loop_in_out_sets_requested_deck_loop_from_product_position() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::Two, 1000);

        assert_eq!(
            state.handle_control(pressed(DeckId::Two, SemanticControl::LoopIn, true)),
            DeckEffects::NONE
        );
        assert_eq!(state.deck(DeckId::Two).loop_state.pending_in_ms, Some(1000));
        assert_eq!(state.deck(DeckId::Two).loop_state.active, None);

        state.set_position_ms(DeckId::Two, 2600);
        let effects = state.handle_control(pressed(DeckId::Two, SemanticControl::LoopOut, true));

        assert_eq!(state.deck(DeckId::One).loop_state.active, None);
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(1000, 2600))
        );
        assert_eq!(
            effects.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::Two,
                start_ms: 1000,
                end_ms: 2600,
            })
        );
    }

    #[test]
    fn quantized_loop_in_out_snaps_to_nearest_neutral_beat() {
        let mut state = DeckProductState::new();
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 4000,
                phase: 3,
                bpm_x100: 12_000,
            },
        ];
        let analysis = beat_jump_analysis(&beats, 12_000);

        state.handle_control(ext(DeckId::One, DeckExtAction::Quantize, true));
        state.set_position_ms(DeckId::One, 1850);
        state.handle_control_with_analysis(
            pressed(DeckId::One, SemanticControl::LoopIn, true),
            [Some(analysis), None],
        );
        state.set_position_ms(DeckId::One, 4230);
        state.handle_control_with_analysis(
            pressed(DeckId::One, SemanticControl::LoopOut, true),
            [Some(analysis), None],
        );

        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(2000, 4000))
        );
    }

    #[test]
    fn reloop_exit_clears_and_restores_last_loop() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(500, 2500));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(500, 2500));

        let clear = state.handle_control(pressed(DeckId::One, SemanticControl::ReloopExit, true));
        assert_eq!(
            clear.items[0],
            Some(DeckEffect::ClearLoop { deck: DeckId::One })
        );
        assert_eq!(state.deck(DeckId::One).loop_state.active, None);

        let restore = state.handle_control(pressed(DeckId::One, SemanticControl::ReloopExit, true));
        assert_eq!(
            restore.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 500,
                end_ms: 2500,
            })
        );
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(500, 2500))
        );
    }

    #[test]
    fn loop_halve_double_and_relative_size_match_released_final_state() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::Two)].loop_state.active = Some(loop_region(1000, 5000));
        state.decks[deck_index(DeckId::Two)].loop_state.last = Some(loop_region(1000, 5000));

        state.handle_control(pressed(DeckId::Two, SemanticControl::LoopHalve, true));
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(1000, 3000))
        );

        state.handle_control(pressed(DeckId::Two, SemanticControl::LoopDouble, true));
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(1000, 5000))
        );

        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(1000, 5000));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(1000, 5000));
        state.handle_control(relative(DeckId::One, SemanticControl::LoopSize, -1));
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(1000, 3000))
        );
        state.handle_control(relative(DeckId::One, SemanticControl::LoopSize, 2));
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(1000, 9000))
        );
    }

    #[test]
    fn loop_release_edges_do_not_mutate_state() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 1000);

        assert_eq!(
            state.handle_control(pressed(DeckId::One, SemanticControl::LoopIn, false)),
            DeckEffects::NONE
        );
        assert_eq!(
            state.handle_control(pressed(DeckId::One, SemanticControl::LoopOut, false)),
            DeckEffects::NONE
        );
        assert_eq!(state.deck(DeckId::One).loop_state, LoopState::new());
    }

    #[test]
    fn reloop_stop_clears_active_and_remembered_loop() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(1000, 4000));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(1000, 4000));
        state.decks[deck_index(DeckId::One)]
            .loop_state
            .pending_in_ms = Some(7000);

        let effects = state.handle_control(ext(DeckId::One, DeckExtAction::ReloopStop, true));
        assert_eq!(
            effects.items[0],
            Some(DeckEffect::ClearLoop { deck: DeckId::One })
        );
        assert_eq!(state.deck(DeckId::One).loop_state, LoopState::new());

        assert_eq!(
            state.handle_control(pressed(DeckId::One, SemanticControl::ReloopExit, true,)),
            DeckEffects::NONE
        );
    }

    fn beat_jump_analysis<'a>(beats: &'a [Beat], bpm_x100: u32) -> TrackAnalysis<'a> {
        TrackAnalysis::new(
            AnalysisProvider::RekordboxImport,
            1,
            bpm_x100,
            Some(BeatGrid::new(beats)),
        )
    }

    fn beat_jump_pad(deck: DeckId, pad: u8, shifted: bool, pressed: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::BeatJump,
                shifted,
                pressed,
            }),
        }
    }

    fn pad_fx_pad(
        deck: DeckId,
        mode: PadMode,
        pad: u8,
        shifted: bool,
        pressed: bool,
    ) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad,
                mode,
                shifted,
                pressed,
            }),
        }
    }

    fn beat_loop_pad(deck: DeckId, pad: u8, shifted: bool, pressed: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::BeatLoop,
                shifted,
                pressed,
            }),
        }
    }

    fn hot_cue_pad(deck: DeckId, pad: u8, shifted: bool, pressed: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad,
                mode: PadMode::HotCue,
                shifted,
                pressed,
            }),
        }
    }

    fn media_id(seed: u8) -> MediaTrackId {
        MediaTrackId([seed; 16])
    }

    #[test]
    fn beat_jump_buttons_use_grid_and_preserve_playing_state() {
        let mut state = DeckProductState::new();
        let beats = [
            Beat {
                time_ms: 1000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2000,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 3000,
                phase: 2,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 4000,
                phase: 3,
                bpm_x100: 12_000,
            },
        ];
        let analysis = beat_jump_analysis(&beats, 12_000);
        state.decks[deck_index(DeckId::Two)].position_ms = 4200;
        state.decks[deck_index(DeckId::Two)].playing = true;

        let back = state.handle_control_with_analysis(
            pressed(DeckId::Two, SemanticControl::BeatJumpBack, true),
            [None, Some(analysis)],
        );
        assert_eq!(
            back.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::Two,
                position_ms: 3000,
            })
        );
        assert_eq!(state.deck(DeckId::Two).position_ms, 3000);
        assert!(state.deck(DeckId::Two).playing);

        let forward = state.handle_control_with_analysis(
            pressed(DeckId::Two, SemanticControl::BeatJumpForward, true),
            [None, Some(analysis)],
        );
        assert_eq!(
            forward.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::Two,
                position_ms: 4000,
            })
        );
        assert!(state.deck(DeckId::Two).playing);
    }

    #[test]
    fn beat_jump_pad_default_page_matches_released_sizes() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 20_000);

        let pad4 = state.handle_control(beat_jump_pad(DeckId::One, 3, false, true));
        assert_eq!(
            pad4.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::One,
                position_ms: 21_000,
            })
        );

        let pad5 = state.handle_control(beat_jump_pad(DeckId::One, 4, false, true));
        assert_eq!(
            pad5.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::One,
                position_ms: 19_000,
            })
        );
    }

    #[test]
    fn shifted_beat_jump_changes_global_size_page() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 10_000);
        state.set_position_ms(DeckId::Two, 20_000);

        state.handle_control(beat_jump_pad(DeckId::One, 1, false, true));
        assert_eq!(state.deck(DeckId::One).position_ms, 10_500);

        state.handle_control(beat_jump_pad(DeckId::One, 7, true, true));
        state.handle_control(beat_jump_pad(DeckId::One, 7, true, true));
        assert_eq!(state.beat_jump_page(), BeatJumpPage::Large);

        state.handle_control(beat_jump_pad(DeckId::Two, 1, false, true));
        assert_eq!(state.deck(DeckId::Two).position_ms, 28_000);

        state.handle_control(beat_jump_pad(DeckId::Two, 6, true, true));
        state.handle_control(beat_jump_pad(DeckId::Two, 6, true, true));
        assert_eq!(state.beat_jump_page(), BeatJumpPage::Fractional);

        state.handle_control(beat_jump_pad(DeckId::One, 1, false, true));
        assert_eq!(state.deck(DeckId::One).position_ms, 10_532);
    }

    #[test]
    fn beat_jump_release_events_do_not_seek_or_change_page() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 20_000);

        assert_eq!(
            state.handle_control(beat_jump_pad(DeckId::One, 4, false, false)),
            DeckEffects::NONE
        );
        assert_eq!(
            state.handle_control(beat_jump_pad(DeckId::One, 7, true, false)),
            DeckEffects::NONE
        );
        assert_eq!(state.deck(DeckId::One).position_ms, 20_000);
        assert_eq!(state.beat_jump_page(), BeatJumpPage::Default);
    }

    #[test]
    fn hot_cue_requires_loaded_track_identity() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 4_000);

        assert_eq!(
            state.handle_control(hot_cue_pad(DeckId::One, 0, false, true)),
            DeckEffects::NONE
        );
        assert_eq!(state.hot_cues(DeckId::One), None);
    }

    #[test]
    fn hot_cue_sets_persists_and_recalls_by_media_track_id() {
        let mut state = DeckProductState::new();
        let track_id = media_id(0x21);
        assert!(state.load_hot_cues(DeckId::One, track_id, None));
        state.set_position_ms(DeckId::One, 4_250);

        let set = state.handle_control(hot_cue_pad(DeckId::One, 2, false, true));
        let bank = state.hot_cues(DeckId::One).unwrap();
        let local = state.local_hot_cues(DeckId::One).unwrap();
        assert_eq!(bank.track_id(), track_id);
        assert_eq!(bank.slot(2), Some(HotCueSlot::single(4_250)));
        assert_eq!(
            set.items[0],
            Some(DeckEffect::PersistHotCues {
                deck: DeckId::One,
                bank: local,
            })
        );

        state.set_position_ms(DeckId::One, 9_000);
        let recall = state.handle_control(hot_cue_pad(DeckId::One, 2, false, true));
        assert_eq!(
            recall.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::One,
                position_ms: 4_250,
            })
        );
        assert_eq!(state.deck(DeckId::One).position_ms, 4_250);
    }

    #[test]
    fn shifted_hot_cue_clears_slot_and_persists_bank() {
        let mut bank = HotCueBank::empty(media_id(0x44));
        assert!(bank.set_single(6, 12_000));

        let mut state = DeckProductState::new();
        assert!(state.load_hot_cues(DeckId::Two, media_id(0x44), Some(bank)));

        let cleared = state.handle_control(hot_cue_pad(DeckId::Two, 6, true, true));
        let updated = state.hot_cues(DeckId::Two).unwrap();
        let local = state.local_hot_cues(DeckId::Two).unwrap();
        assert_eq!(updated.slot(6), None);
        assert_eq!(
            cleared.items[0],
            Some(DeckEffect::PersistHotCues {
                deck: DeckId::Two,
                bank: local,
            })
        );
        assert_eq!(
            state.handle_control(hot_cue_pad(DeckId::Two, 6, true, true)),
            DeckEffects::NONE
        );
    }

    #[test]
    fn track_load_commits_source_and_local_atomically() {
        let first = media_id(0x31);
        let second = media_id(0x32);
        let mut state = DeckProductState::new();

        let mut first_source = HotCueBank::empty(first);
        assert!(first_source.set_single(0, 1_000));
        assert!(state.commit_track_load(DeckId::One, first, 12_800, first_source, None));
        state.set_position_ms(DeckId::One, 12_345);

        let mut second_source = HotCueBank::empty(second);
        assert!(second_source.set_single(1, 2_000));
        let mut second_local = HotCueBank::empty(second);
        assert!(second_local.clear(1));
        assert!(second_local.set_single(3, 4_000));

        assert!(state.commit_track_load(
            DeckId::One,
            second,
            13_400,
            second_source,
            Some(second_local),
        ));

        let deck = state.deck(DeckId::One);
        assert!(!deck.playing);
        assert_eq!(deck.position_ms, 0);
        assert_eq!(deck.cue_point_ms, 0);
        assert_eq!(deck.base_bpm_x100, 13_400);
        let effective = state.hot_cues(DeckId::One).unwrap();
        assert_eq!(effective.slot(1), None);
        assert_eq!(effective.slot(3), Some(HotCueSlot::single(4_000)));
    }

    #[test]
    fn failed_track_load_identity_validation_preserves_previous_deck() {
        let first = media_id(0x41);
        let second = media_id(0x42);
        let wrong = media_id(0x43);
        let mut state = DeckProductState::new();

        let mut source = HotCueBank::empty(first);
        assert!(source.set_single(0, 1_500));
        assert!(state.commit_track_load(DeckId::Two, first, 12_000, source, None));
        state.set_position_ms(DeckId::Two, 7_000);
        let before = *state.deck(DeckId::Two);
        let before_cues = state.hot_cues(DeckId::Two);

        let second_source = HotCueBank::empty(second);
        let wrong_local = HotCueBank::empty(wrong);
        assert!(!state.commit_track_load(
            DeckId::Two,
            second,
            14_000,
            second_source,
            Some(wrong_local),
        ));

        assert_eq!(*state.deck(DeckId::Two), before);
        assert_eq!(state.hot_cues(DeckId::Two), before_cues);
    }

    #[test]
    fn unload_track_clears_track_lifecycle_without_losing_pitch_configuration() {
        let track = media_id(0x51);
        let mut source = HotCueBank::empty(track);
        assert!(source.set_single(0, 1_000));
        let mut state = DeckProductState::new();
        assert!(state.commit_track_load(DeckId::One, track, 12_800, source, None));

        state.decks[0].pitch_centipercent = 275;
        state.decks[0].tempo_range_percent = 16;
        state.decks[0].master_tempo = true;
        state.set_position_ms(DeckId::One, 9_000);
        state.decks[0].playing = true;

        state.unload_track(DeckId::One);

        let deck = state.deck(DeckId::One);
        assert!(!deck.playing);
        assert_eq!(deck.position_ms, 0);
        assert_eq!(deck.base_bpm_x100, 12_000);
        assert_eq!(deck.pitch_centipercent, 275);
        assert_eq!(deck.tempo_range_percent, 16);
        assert!(deck.master_tempo);
        assert_eq!(state.hot_cues(DeckId::One), None);
        assert_eq!(state.local_hot_cues(DeckId::One), None);
    }

    #[test]
    fn deleting_imported_hot_cue_persists_tombstone_across_reload() {
        let track_id = media_id(0x45);
        let mut source = HotCueBank::empty(track_id);
        assert!(source.set_single(1, 5_000));

        let mut state = DeckProductState::new();
        assert!(state.load_hot_cues_with_source(DeckId::One, track_id, source, None));
        assert_eq!(
            state.hot_cues(DeckId::One).unwrap().slot(1),
            Some(HotCueSlot::single(5_000))
        );

        let effect = state.handle_control(hot_cue_pad(DeckId::One, 1, true, true));
        let local = state.local_hot_cues(DeckId::One).unwrap();
        assert!(local.is_overridden(1));
        assert_eq!(local.slot(1), None);
        assert_eq!(
            effect.items[0],
            Some(DeckEffect::PersistHotCues {
                deck: DeckId::One,
                bank: local,
            })
        );
        assert_eq!(state.hot_cues(DeckId::One).unwrap().slot(1), None);

        let mut reloaded = DeckProductState::new();
        assert!(reloaded.load_hot_cues_with_source(DeckId::One, track_id, source, Some(local),));
        assert_eq!(reloaded.hot_cues(DeckId::One).unwrap().slot(1), None);
    }

    #[test]
    fn hot_cue_bank_rejects_wrong_track_identity() {
        let mut state = DeckProductState::new();
        let persisted = HotCueBank::empty(media_id(1));

        assert!(!state.load_hot_cues(DeckId::One, media_id(2), Some(persisted)));
        assert_eq!(state.hot_cues(DeckId::One), None);
    }

    #[test]
    fn persisted_loop_hot_cue_recalls_its_start_position() {
        let track_id = media_id(0x55);
        let mut bank = HotCueBank::empty(track_id);
        assert!(bank.set_loop(3, 10_000, 12_000));

        let mut state = DeckProductState::new();
        assert!(state.load_hot_cues(DeckId::One, track_id, Some(bank)));
        state.set_position_ms(DeckId::One, 30_000);

        let recall = state.handle_control(hot_cue_pad(DeckId::One, 3, false, true));
        assert_eq!(
            recall.items[0],
            Some(DeckEffect::Seek {
                deck: DeckId::One,
                position_ms: 10_000,
            })
        );
    }

    #[test]
    fn hot_cue_release_edges_are_inert() {
        let mut state = DeckProductState::new();
        let track_id = media_id(0x77);
        assert!(state.load_hot_cues(DeckId::One, track_id, None));
        state.set_position_ms(DeckId::One, 2_000);

        assert_eq!(
            state.handle_control(hot_cue_pad(DeckId::One, 0, false, false)),
            DeckEffects::NONE
        );
        assert_eq!(state.hot_cues(DeckId::One).unwrap().exists_mask(), 0);
    }

    #[test]
    fn beat_loop_pads_match_released_lengths() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 1_000);

        let one_beat = state.handle_control(beat_loop_pad(DeckId::One, 5, false, true));
        assert_eq!(
            one_beat.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 1_000,
                end_ms: 1_500,
            })
        );
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(1_000, 1_500))
        );

        state.set_position_ms(DeckId::Two, 2_000);
        let four_beats = state.handle_control(beat_loop_pad(DeckId::Two, 7, false, true));
        assert_eq!(
            four_beats.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::Two,
                start_ms: 2_000,
                end_ms: 4_000,
            })
        );
    }

    #[test]
    fn beat_loop_uses_local_neutral_grid_spacing() {
        let beats = [
            Beat {
                time_ms: 1_000,
                phase: 0,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 1_501,
                phase: 1,
                bpm_x100: 12_000,
            },
            Beat {
                time_ms: 2_000,
                phase: 2,
                bpm_x100: 12_000,
            },
        ];
        let analysis = beat_jump_analysis(&beats, 12_000);
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 1_010);

        let effects = state.handle_control_with_analysis(
            beat_loop_pad(DeckId::One, 5, false, true),
            [Some(analysis), None],
        );

        assert_eq!(
            effects.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 1_010,
                end_ms: 1_511,
            })
        );
    }

    #[test]
    fn shifted_beat_loop_restores_previous_loop_on_release() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(5_000, 9_000));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(5_000, 9_000));
        state.set_position_ms(DeckId::One, 12_000);

        let pressed = state.handle_control(beat_loop_pad(DeckId::One, 5, true, true));
        assert_eq!(
            pressed.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 12_000,
                end_ms: 12_500,
            })
        );

        let released = state.handle_control(beat_loop_pad(DeckId::One, 5, true, false));
        assert_eq!(
            released.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 5_000,
                end_ms: 9_000,
            })
        );
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(5_000, 9_000))
        );
    }

    #[test]
    fn shifted_beat_loop_without_previous_loop_clears_on_release() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::Two, 2_000);

        state.handle_control(beat_loop_pad(DeckId::Two, 4, true, true));
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(2_000, 2_250))
        );

        let released = state.handle_control(beat_loop_pad(DeckId::Two, 4, true, false));
        assert_eq!(
            released.items[0],
            Some(DeckEffect::ClearLoop { deck: DeckId::Two })
        );
        assert_eq!(state.deck(DeckId::Two).loop_state.active, None);
        assert_eq!(
            state.deck(DeckId::Two).loop_state.last,
            Some(loop_region(2_000, 2_250))
        );
    }

    #[test]
    fn unshifted_beat_loop_release_is_inert() {
        let mut state = DeckProductState::new();
        state.set_position_ms(DeckId::One, 3_000);
        state.handle_control(beat_loop_pad(DeckId::One, 5, false, true));
        let before = state.deck(DeckId::One).loop_state;

        assert_eq!(
            state.handle_control(beat_loop_pad(DeckId::One, 5, false, false)),
            DeckEffects::NONE
        );
        assert_eq!(state.deck(DeckId::One).loop_state, before);
    }

    #[test]
    fn pad_fx1_press_and_release_route_to_audio_without_transport_side_effects() {
        let mut state = DeckProductState::new();

        let pressed_fx =
            state.handle_control(pad_fx_pad(DeckId::One, PadMode::PadFx1, 2, false, true));
        assert_eq!(
            pressed_fx.items[0],
            Some(DeckEffect::ApplyPadFx {
                deck: DeckId::One,
                bank: PadFxBank::One,
                pad: 2,
                active: true,
            })
        );
        assert_eq!(
            state.deck(DeckId::One).active_pad_fx,
            Some(ActivePadFx {
                bank: PadFxBank::One,
                pad: 2,
            })
        );
        assert!(!state.deck(DeckId::One).playing);

        let released =
            state.handle_control(pad_fx_pad(DeckId::One, PadMode::PadFx1, 2, false, false));
        assert_eq!(
            released.items[0],
            Some(DeckEffect::ApplyPadFx {
                deck: DeckId::One,
                bank: PadFxBank::One,
                pad: 2,
                active: false,
            })
        );
        assert_eq!(state.deck(DeckId::One).active_pad_fx, None);
    }

    #[test]
    fn pad_fx2_routes_by_mode_even_when_shifted_flag_is_set() {
        let mut state = DeckProductState::new();

        let effects = state.handle_control(pad_fx_pad(DeckId::Two, PadMode::PadFx2, 3, true, true));
        assert_eq!(
            effects.items[0],
            Some(DeckEffect::ApplyPadFx {
                deck: DeckId::Two,
                bank: PadFxBank::Two,
                pad: 3,
                active: true,
            })
        );
        assert_eq!(
            state.deck(DeckId::Two).active_pad_fx,
            Some(ActivePadFx {
                bank: PadFxBank::Two,
                pad: 3,
            })
        );
    }

    #[test]
    fn mismatched_pad_fx_release_does_not_clear_newer_active_pad() {
        let mut state = DeckProductState::new();
        state.handle_control(pad_fx_pad(DeckId::One, PadMode::PadFx1, 2, false, true));
        state.handle_control(pad_fx_pad(DeckId::One, PadMode::PadFx1, 3, false, true));

        let release =
            state.handle_control(pad_fx_pad(DeckId::One, PadMode::PadFx1, 2, false, false));
        assert_eq!(
            release.items[0],
            Some(DeckEffect::ApplyPadFx {
                deck: DeckId::One,
                bank: PadFxBank::One,
                pad: 2,
                active: false,
            })
        );
        assert_eq!(
            state.deck(DeckId::One).active_pad_fx,
            Some(ActivePadFx {
                bank: PadFxBank::One,
                pad: 3,
            })
        );
    }

    #[test]
    fn pad_mode_behavior_matches_released_scope() {
        let mut state = DeckProductState::new();

        state.handle_control(pressed(DeckId::One, SemanticControl::PadModeBeatLoop, true));
        state.handle_control(pressed(DeckId::Two, SemanticControl::PadModeBeatJump, true));
        assert_eq!(state.deck(DeckId::One).perf_mode, PerformanceMode::LoopRoll);
        assert_eq!(state.deck(DeckId::Two).perf_mode, PerformanceMode::BeatJump);
        assert_eq!(state.deck(DeckId::One).pad_mode, PadMode::BeatLoop);
        assert_eq!(state.deck(DeckId::Two).pad_mode, PadMode::BeatJump);

        state.handle_control(pressed(DeckId::One, SemanticControl::PadModePadFx1, true));
        state.handle_control(pressed(DeckId::Two, SemanticControl::PadModeSampler, true));
        state.handle_control(pressed(DeckId::One, SemanticControl::PadModeKeyboard, true));
        state.handle_control(pressed(DeckId::Two, SemanticControl::PadModeKeyShift, true));

        assert_eq!(state.deck(DeckId::One).pad_mode, PadMode::PadFx1);
        assert_eq!(state.deck(DeckId::One).perf_mode, PerformanceMode::LoopRoll);
        assert_eq!(state.deck(DeckId::Two).pad_mode, PadMode::BeatJump);
        assert_eq!(state.deck(DeckId::Two).perf_mode, PerformanceMode::BeatJump);
    }

    #[test]
    fn loop_adjust_requires_an_active_loop_and_toggles_mode() {
        let mut state = DeckProductState::new();

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));
        assert_eq!(
            state.deck(DeckId::One).loop_adjust_mode,
            LoopAdjustMode::None
        );

        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(1_000, 2_000));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(1_000, 2_000));

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));
        assert_eq!(state.deck(DeckId::One).loop_adjust_mode, LoopAdjustMode::In);

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));
        assert_eq!(
            state.deck(DeckId::One).loop_adjust_mode,
            LoopAdjustMode::None
        );

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustOut, true));
        assert_eq!(
            state.deck(DeckId::One).loop_adjust_mode,
            LoopAdjustMode::Out
        );

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));
        assert_eq!(state.deck(DeckId::One).loop_adjust_mode, LoopAdjustMode::In);

        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustOut, false));
        assert_eq!(state.deck(DeckId::One).loop_adjust_mode, LoopAdjustMode::In);
    }

    #[test]
    fn loop_adjust_in_uses_one_ms_per_jog_tick_and_clamps() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(100, 200));
        state.decks[deck_index(DeckId::One)].loop_state.last = Some(loop_region(100, 200));
        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));

        let moved = state.handle_control(relative(DeckId::One, SemanticControl::JogScratch, 25));
        assert_eq!(
            moved.items[0],
            Some(DeckEffect::SetLoop {
                deck: DeckId::One,
                start_ms: 125,
                end_ms: 200,
            })
        );

        state.handle_control(relative(DeckId::One, SemanticControl::JogBend, i16::MAX));
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(199, 200))
        );

        state.handle_control(relative(DeckId::One, SemanticControl::JogScratch, i16::MIN));
        assert_eq!(
            state.deck(DeckId::One).loop_state.active,
            Some(loop_region(0, 200))
        );
    }

    #[test]
    fn loop_adjust_out_uses_one_ms_per_jog_tick_and_clamps() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::Two)].loop_state.active = Some(loop_region(1_000, 2_000));
        state.decks[deck_index(DeckId::Two)].loop_state.last = Some(loop_region(1_000, 2_000));
        state.handle_control(ext(DeckId::Two, DeckExtAction::LoopAdjustOut, true));

        state.handle_control(relative(DeckId::Two, SemanticControl::JogBend, -1_500));
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(1_000, 1_001))
        );

        state.decks[deck_index(DeckId::Two)].loop_state.active =
            Some(loop_region(u32::MAX - 100, u32::MAX - 50));
        state.handle_control(relative(DeckId::Two, SemanticControl::JogScratch, 500));
        assert_eq!(
            state.deck(DeckId::Two).loop_state.active,
            Some(loop_region(u32::MAX - 100, u32::MAX))
        );
    }

    #[test]
    fn missing_loop_drops_adjust_mode_before_jog() {
        let mut state = DeckProductState::new();
        state.decks[deck_index(DeckId::One)].loop_state.active = Some(loop_region(1_000, 2_000));
        state.handle_control(ext(DeckId::One, DeckExtAction::LoopAdjustIn, true));
        state.decks[deck_index(DeckId::One)].loop_state.active = None;

        assert_eq!(
            state.handle_control(relative(DeckId::One, SemanticControl::JogScratch, 10,)),
            DeckEffects::NONE
        );
        assert_eq!(
            state.deck(DeckId::One).loop_adjust_mode,
            LoopAdjustMode::None
        );
    }

    #[test]
    fn disconnect_clears_physical_hold_state_without_stopping_playback() {
        let mut state = DeckProductState::new();
        state.decks[0].playing = true;
        state.decks[0].jog_touched = true;
        state.decks[0].loop_adjust_mode = LoopAdjustMode::In;

        state.set_controller_connected(false);

        assert!(state.deck(DeckId::One).playing);
        assert!(!state.deck(DeckId::One).jog_touched);
        assert_eq!(
            state.deck(DeckId::One).loop_adjust_mode,
            LoopAdjustMode::None
        );
        assert!(!state.deck(DeckId::One).controller_connected);
        assert!(!state.deck(DeckId::Two).controller_connected);
    }

    #[test]
    fn deck_effect_queue_rejects_multi_effect_delivery_atomically() {
        let pause = DeckEffect::Pause { deck: DeckId::One };
        let seek = DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 1_234,
        };
        let mut queue = DeckEffectQueue::<1>::new();

        assert!(!queue.try_push_all(DeckEffects::two(pause, seek)));
        assert!(queue.is_empty());

        assert!(queue.try_push_all(DeckEffects::one(pause)));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.front(), Some(pause));
        assert!(queue.complete_front(true));
        assert!(queue.is_empty());
    }

    #[test]
    fn deck_effect_queue_preserves_effect_order() {
        let first = DeckEffect::Pause { deck: DeckId::Two };
        let second = DeckEffect::Seek {
            deck: DeckId::Two,
            position_ms: 9_876,
        };
        let mut queue = DeckEffectQueue::<2>::new();

        assert!(queue.try_push_all(DeckEffects::two(first, second)));
        assert_eq!(queue.front(), Some(first));
        assert!(queue.complete_front(true));
        assert_eq!(queue.front(), Some(second));
        assert!(queue.complete_front(true));
        assert_eq!(queue.front(), None);
        assert!(!queue.complete_front(true));
    }

    #[test]
    fn deck_effect_target_classification_is_explicit() {
        assert_eq!(
            DeckEffect::Pause { deck: DeckId::One }.target(),
            DeckEffectTarget::Transport
        );
        assert_eq!(
            DeckEffect::ApplyPadFx {
                deck: DeckId::One,
                bank: PadFxBank::One,
                pad: 0,
                active: true,
            }
            .target(),
            DeckEffectTarget::AudioDsp
        );
    }

    #[test]
    fn effect_broker_reserves_all_lanes_before_committing_any_effect() {
        let transport = DeckEffect::Pause { deck: DeckId::One };
        let audio = DeckEffect::ApplyPadFx {
            deck: DeckId::One,
            bank: PadFxBank::One,
            pad: 2,
            active: true,
        };
        let mut broker = DeckEffectBroker::<1, 1, 0>::new();

        assert!(!broker.try_push_all(DeckEffects::two(transport, audio)));
        assert!(broker.is_empty());

        let mut broker = DeckEffectBroker::<1, 1, 1>::new();
        assert!(broker.try_push_all(DeckEffects::two(transport, audio)));
        assert_eq!(broker.transport_len(), 1);
        assert_eq!(broker.audio_dsp_len(), 1);
        assert_eq!(broker.transport_front(), Some(transport));
        assert!(broker.complete_transport(true));
        assert_eq!(broker.audio_dsp_front(), Some(audio));
        assert!(broker.complete_audio_dsp(true));
        assert!(broker.is_empty());
    }

    #[test]
    fn failed_effect_completion_retains_exact_front_for_retry() {
        let first = DeckEffect::Pause { deck: DeckId::One };
        let second = DeckEffect::Seek {
            deck: DeckId::One,
            position_ms: 4_321,
        };
        let mut queue = DeckEffectQueue::<2>::new();

        assert!(queue.try_push_all(DeckEffects::two(first, second)));
        assert_eq!(queue.front(), Some(first));

        assert!(queue.complete_front(false));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.front(), Some(first));

        assert!(queue.complete_front(true));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.front(), Some(second));
    }

    #[test]
    fn broker_lane_retry_does_not_advance_or_disturb_other_lanes() {
        let transport = DeckEffect::Pause { deck: DeckId::One };
        let audio = DeckEffect::ApplyPadFx {
            deck: DeckId::Two,
            bank: PadFxBank::Two,
            pad: 4,
            active: true,
        };
        let mut broker = DeckEffectBroker::<2, 1, 2>::new();

        assert!(broker.try_push_all(DeckEffects::two(transport, audio)));
        assert_eq!(broker.transport_front(), Some(transport));
        assert_eq!(broker.audio_dsp_front(), Some(audio));

        assert!(broker.complete_transport(false));
        assert_eq!(broker.transport_front(), Some(transport));
        assert_eq!(broker.audio_dsp_front(), Some(audio));

        assert!(broker.complete_audio_dsp(true));
        assert_eq!(broker.audio_dsp_front(), None);
        assert_eq!(broker.transport_front(), Some(transport));
    }
}
