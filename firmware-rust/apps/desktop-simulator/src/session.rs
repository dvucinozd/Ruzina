//! Host owner: real deck semantics and importer, simulated transport/media.
//! This does not pump audio, access USB, or persist settings/cues to disk.
use pajoniiir_board_profile::BoardId;
use pajoniiir_controller_core::{
    ControlDomain, ControlEvent, ControlValue, DeckId, SemanticControl,
};
use pajoniiir_core::MediaTrackId;
use pajoniiir_deck::{DeckEffect, DeckLoadGate, DeckProductState, LoadPermit, LoadRefusal};
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_media_session::{MediaLease, MediaSession, MediaSourceId};
use pajoniiir_mixer::MixerState;
use pajoniiir_rekordbox::anlz_analysis::{AnalysisFileKind, AnlzImporter};
use pajoniiir_service::publication::{Activity, Publication, Slot, Stamp};
use pajoniiir_service::{
    AnalysisStatus, EventKind, Identity, LoadBlock, MediaStatus, RuntimeStatus, Service, Snapshot,
};

/// Host owner heartbeat contract, independent of whether a deck is playing.
pub const RUNTIME_MAX_AGE_MS: u64 = 1000;
use pajoniiir_track_analysis::{
    TrackAnalysis,
    owned::{AnalysisIdentity, AnalysisPins},
};
use pajoniiir_ui_model::{
    BeatFxAction, DeckPresentation, pad_bank_event, pad_fx_event, present_deck,
};

const DAT_A: &[u8] =
    include_bytes!("../../../crates/pajoniiir-rekordbox/tests/fixtures/track-a.DAT");
const DAT_B: &[u8] =
    include_bytes!("../../../crates/pajoniiir-rekordbox/tests/fixtures/track-b.DAT");
const EXT: &[u8] = include_bytes!("../../../crates/pajoniiir-rekordbox/tests/fixtures/track-a.EXT");
pub const TITLES: [&str; 4] = [
    "01 / Grid study · 120 BPM",
    "02 / Offset beats · 128 BPM",
    "03 / No analysis · 120 BPM",
    "04 / Damaged analysis · recovery",
];
const DURATIONS: [u32; 4] = [10_000, 14_000, 10_000, 10_000];

pub struct Session {
    pub analysis_observation: pajoniiir_service::analysis::Observation,
    pub decks: DeckProductState,
    pub mixer: MixerState,
    pub selected: usize,
    pub message: String,
    media: MediaSession,
    pins: AnalysisPins,
    loaded: [Option<(usize, AnalysisIdentity)>; 2],
    load_gates: [DeckLoadGate; 2],
    local: [Option<HotCueBank>; 4],
    generation: u32,
    fraction: [u64; 2],
    service: Service,
    analysis_status: [AnalysisStatus; 2],
    health_publication: Slot<RuntimeStatus>,
    health_generation: u64,
    health_sequence: u64,
}

pub const fn deck(index: usize) -> DeckId {
    if index == 0 { DeckId::One } else { DeckId::Two }
}
const fn load_block(refusal: LoadRefusal) -> LoadBlock {
    match refusal {
        LoadRefusal::Playing => LoadBlock::Playing,
        LoadRefusal::PlayPending => LoadBlock::PlayPending,
        LoadRefusal::Scratching => LoadBlock::Scratching,
        LoadRefusal::LoadInFlight => LoadBlock::LoadInFlight,
        LoadRefusal::Superseded => LoadBlock::Superseded,
        LoadRefusal::TrackIdentity => LoadBlock::TrackIdentity,
    }
}
pub const fn index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

impl Session {
    pub fn new() -> Self {
        Self::for_board(BoardId::GuitionJc1060P470cIwY)
    }
    pub fn for_board(board: BoardId) -> Self {
        let mut result = Self {
            analysis_observation: pajoniiir_service::analysis::Observation::DISABLED,
            decks: DeckProductState::new(),
            mixer: MixerState::new(),
            selected: 0,
            message: "Choose a demo track in Library".into(),
            media: MediaSession::new(),
            pins: AnalysisPins::new(),
            loaded: [None, None],
            load_gates: [DeckLoadGate::new(), DeckLoadGate::new()],
            local: [None; 4],
            generation: 0,
            fraction: [0; 2],
            service: Service::new(Identity::host(board, option_env!("M1_IMAGE_ID"))),
            analysis_status: [AnalysisStatus::Unloaded; 2],
            health_publication: Slot::new(RUNTIME_MAX_AGE_MS),
            health_generation: 0,
            health_sequence: 0,
        };
        result.reconnect();
        result
    }
    pub fn connected(&self) -> bool {
        self.media.lease().is_some()
    }
    pub fn reconnect(&mut self) {
        let was_connected = self.connected();
        self.media.on_connect(MediaSourceId::new(1).unwrap());
        if !was_connected {
            self.service.record(EventKind::MediaConnected);
            self.restart_health_owner();
        }
        self.message = "Demo media connected · choose a track".into();
        self.publish_health();
    }
    pub fn detach(&mut self) {
        let was_connected = self.connected();
        self.cancel_pads();
        self.control(BeatFxAction::Clear.event(true));
        self.media.on_disconnect(None);
        if was_connected {
            self.service.record(EventKind::MediaRemoved);
        }
        for i in 0..2 {
            self.decks.unload_track(deck(i));
            self.pins.clear(i);
            self.loaded[i] = None;
            self.fraction[i] = 0;
            self.analysis_status[i] = AnalysisStatus::Unloaded;
        }
        self.message = "Media removed · decks stopped · reconnect in Library".into();
        if was_connected {
            self.restart_health_owner();
        }
        self.publish_health();
    }
    pub fn load(&mut self, target: DeckId) -> bool {
        let loaded = self.load_inner(target);
        self.publish_health();
        loaded
    }
    fn load_inner(&mut self, target: DeckId) -> bool {
        self.service.record(EventKind::LoadAttempt(target));
        let i = index(target);
        let Some(lease) = self.media.lease() else {
            self.service.record(EventKind::LoadBlocked {
                deck: target,
                reason: LoadBlock::NoMedia,
            });
            self.message = "Load unavailable · reconnect media".into();
            return false;
        };
        // Same admission path as firmware: verdict, single-flight permit,
        // final check before replacement and again inside the commit.
        let permit = match self
            .decks
            .load_admission(target)
            .and_then(|_| self.load_gates[i].reserve())
        {
            Ok(permit) => permit,
            Err(refusal) => return self.refuse_load(target, refusal),
        };
        let loaded = self.load_admitted(target, lease, permit);
        self.load_gates[i].finish(permit);
        loaded
    }
    fn refuse_load(&mut self, target: DeckId, refusal: LoadRefusal) -> bool {
        let reason = load_block(refusal);
        self.service.record(EventKind::LoadBlocked {
            deck: target,
            reason,
        });
        self.message = match reason {
            LoadBlock::TrackIdentity => "Load rejected · track identity mismatch".into(),
            LoadBlock::LoadInFlight => "Load busy · previous load still running".into(),
            LoadBlock::Scratching => "Load locked · release the jog".into(),
            _ => "Load locked · pause the destination deck".into(),
        };
        false
    }
    fn load_admitted(&mut self, target: DeckId, lease: MediaLease, permit: LoadPermit) -> bool {
        let i = index(target);
        let selection = self.selected;
        if selection >= TITLES.len() {
            self.service.record(EventKind::LoadBlocked {
                deck: target,
                reason: LoadBlock::InvalidSelection,
            });
            self.message = "Load rejected · invalid demo selection".into();
            return false;
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        let id = AnalysisIdentity {
            track: MediaTrackId([selection as u8 + 1; 16]),
            lease,
            catalog_revision: 1,
            load_generation: self.generation,
        };
        let mut importer = AnlzImporter::new();
        let data = match selection {
            0 => Some(DAT_A),
            1 => Some(DAT_B),
            3 => Some(&b"damaged"[..]),
            _ => None,
        };
        let publication = data.and_then(|data| {
            feed(&mut importer, AnalysisFileKind::Dat, data).ok()?;
            if selection == 0 {
                feed(&mut importer, AnalysisFileKind::Ext, EXT).ok()?;
            }
            importer
                .finish(id, if selection == 1 { 12800 } else { 12000 })
                .ok()
        });
        let mut source = HotCueBank::empty(id.track);
        if let Some(analysis) = publication.as_ref().map(|p| p.view()) {
            for cue in analysis.cues() {
                if let Some(end) = cue.end_ms {
                    source.set_loop(cue.slot, cue.start_ms, end);
                } else {
                    source.set_single(cue.slot, cue.start_ms);
                }
            }
        }
        if let Err(refusal) = self.load_gates[i].begin_replacement(permit) {
            return self.refuse_load(target, refusal);
        }
        if let Err(refusal) = self.decks.commit_track_load(
            target,
            id.track,
            if selection == 1 { 12800 } else { 12000 },
            source.as_source(),
            self.local[selection],
        ) {
            return self.refuse_load(target, refusal);
        }
        self.pins.clear(i);
        let available = publication.is_some();
        if let Some(publication) = publication {
            assert!(self.pins.publish(i, id, publication));
        }
        self.loaded[i] = Some((selection, id));
        // The host source length plays the role of the decoded duration.
        self.decks
            .set_decoded_duration_ms(target, Some(DURATIONS[selection]));
        self.analysis_status[i] = if available {
            AnalysisStatus::Ready
        } else if selection == 2 {
            AnalysisStatus::Missing
        } else {
            AnalysisStatus::Rejected
        };
        self.service.record(EventKind::Loaded {
            deck: target,
            analysis: self.analysis_status[i],
        });
        self.fraction[i] = 0;
        self.message = if available {
            format!("Deck {} loaded · Rekordbox grid ready", i + 1)
        } else {
            format!(
                "Deck {} loaded · {} · BPM fallback",
                i + 1,
                if selection == 3 {
                    "analysis rejected"
                } else {
                    "no analysis"
                }
            )
        };
        true
    }
    pub fn analysis(&self, i: usize) -> Option<TrackAnalysis<'_>> {
        let (_, id) = self.loaded.get(i).copied().flatten()?;
        if !self.media.validate(id.lease) {
            return None;
        }
        self.pins.view(i, id)
    }
    pub fn title(&self, i: usize) -> &str {
        self.loaded[i]
            .map(|(selection, _)| TITLES[selection])
            .unwrap_or("No track loaded")
    }
    pub fn presentation(&self, i: usize) -> DeckPresentation {
        let duration = self.loaded[i]
            .map(|(selection, _)| DURATIONS[selection])
            .unwrap_or(0);
        present_deck(
            deck(i),
            self.decks.deck(deck(i)),
            self.loaded[i].is_some(),
            duration,
            self.decks.hot_cues(deck(i)),
        )
    }
    pub fn control(&mut self, event: ControlEvent) {
        self.control_inner(event);
        self.publish_health();
    }
    fn control_inner(&mut self, event: ControlEvent) {
        self.service.note_control();
        if event.control.domain() == ControlDomain::Mixer {
            self.mixer.handle_control(event);
            return;
        }
        let global_fx = matches!(
            event.control,
            SemanticControl::BeatFxSelectNext
                | SemanticControl::BeatFxSelectPrev
                | SemanticControl::BeatFxBeatDec
                | SemanticControl::BeatFxBeatInc
                | SemanticControl::BeatFxTarget
                | SemanticControl::BeatFxDepth
                | SemanticControl::BeatFxOn
                | SemanticControl::BeatFxClear
        );
        if !global_fx && event.deck.is_none() {
            return;
        }
        if !global_fx
            && event
                .deck
                .is_some_and(|target| self.loaded[index(target)].is_none())
        {
            // A release queued before detach must not replace the recovery
            // status or recreate state after the deck was invalidated.
            if matches!(event.value, ControlValue::Pressed(false))
                || matches!(event.value, ControlValue::PadAction(action) if !action.pressed)
            {
                return;
            }
            self.message = "Load a track before using deck controls".into();
            if event.control == SemanticControl::Play && event.value == ControlValue::Pressed(true)
            {
                self.service
                    .record(EventKind::PlaybackRejected(event.deck.unwrap()));
            }
            return;
        }
        // Core control handling may already move the deck before returning Seek.
        let was_at_end: [bool; 2] = core::array::from_fn(|i| {
            let t = self.presentation(i).transport;
            t.loaded && t.position_ms >= t.duration_ms
        });
        let views = core::array::from_fn(|i| {
            self.loaded[i]
                .filter(|(_, id)| self.media.validate(id.lease))
                .and_then(|(_, id)| self.pins.view(i, id))
        });
        let effects = self.decks.handle_control_with_analysis(event, views);
        for effect in effects.items.into_iter().flatten() {
            match effect {
                DeckEffect::PlaybackRequest {
                    deck,
                    playing,
                    request_id,
                } => {
                    let p = self.presentation(index(deck));
                    let can_play = !playing || p.transport.position_ms < p.transport.duration_ms;
                    self.decks
                        .resolve_playback_request(deck, request_id, can_play);
                    if !can_play {
                        self.service.record(EventKind::PlaybackRejected(deck));
                        // CDJ CUE at the end would set a new cue there.
                        self.message = "End of track · jump back or recall a hot cue".into();
                    }
                }
                DeckEffect::Seek { deck, position_ms } => {
                    let i = index(deck);
                    let duration = self.presentation(i).transport.duration_ms as u32;
                    let position = position_ms.min(duration);
                    self.decks.set_position_ms(deck, position);
                    self.fraction[i] = 0;
                    if position == duration {
                        self.decks.finish_playback(deck, position);
                        if !was_at_end[i] {
                            self.service.record(EventKind::EndOfTrack(deck));
                        }
                    }
                }
                DeckEffect::PersistHotCues { deck, bank } => {
                    if let Some((selection, _)) = self.loaded[index(deck)] {
                        self.local[selection] = Some(bank);
                        for i in 0..2 {
                            if self.loaded[i].is_some_and(|(other, id)| {
                                other == selection
                                    && id.track == bank.track_id()
                                    && self.media.validate(id.lease)
                            }) {
                                self.decks.update_local_hot_cues(self::deck(i), bank);
                            }
                        }
                    }
                    self.message = "Local cue edit retained for this session".into();
                }
                _ => {} // Deck state already records pause/pitch/loop; no audio DSP in host.
            }
        }
    }
    pub fn cancel_pads(&mut self) {
        for i in 0..2 {
            if let Some(active) = self.decks.deck(deck(i)).active_pad_fx {
                let bank = if active.bank == pajoniiir_deck::PadFxBank::One {
                    0
                } else {
                    1
                };
                self.control(pad_fx_event(deck(i), bank, i32::from(active.pad), false).unwrap());
            }
        }
    }
    pub fn select_pad_bank(&mut self, target: DeckId, bank: i32) {
        if let Some(event) = pad_bank_event(target, bank) {
            if let Some(active) = self.decks.deck(target).active_pad_fx {
                let old_bank = if active.bank == pajoniiir_deck::PadFxBank::One {
                    0
                } else {
                    1
                };
                self.control(pad_fx_event(target, old_bank, i32::from(active.pad), false).unwrap());
            }
            self.control(event);
        }
    }
    /// Explicit host-only transport clock. Fractional pitch time is retained;
    /// real product timing continues to come from its PCM owner.
    pub fn tick(&mut self, elapsed_ms: u32) {
        self.observe_elapsed(elapsed_ms);
        for i in 0..2 {
            let p = self.presentation(i);
            if !p.transport.playing {
                continue;
            }
            let rate = (10_000 + p.transport.pitch_centi_percent).max(0) as u64;
            let scaled = u64::from(elapsed_ms) * rate + self.fraction[i];
            self.fraction[i] = scaled % 10_000;
            let mut next = p.transport.position_ms + scaled / 10_000;
            if let Some((start, end)) = p.loop_region {
                let end = u64::from(end).min(p.transport.duration_ms);
                let start = u64::from(start);
                if end > start && next >= end {
                    next = start + (next - end) % (end - start);
                }
            }
            if next >= p.transport.duration_ms {
                self.decks
                    .finish_playback(deck(i), p.transport.duration_ms as u32);
                self.service.record(EventKind::EndOfTrack(deck(i)));
            } else {
                self.decks.set_position_ms(deck(i), next as u32);
            }
        }
        self.publish_health();
    }
    /// Advance only the observer. Replay uses this to model a stopped publisher.
    /// The interactive timer normally calls tick(), which also runs the owner.
    pub fn observe_elapsed(&mut self, elapsed_ms: u32) {
        self.service.advance(u64::from(elapsed_ms));
    }
    fn restart_health_owner(&mut self) {
        self.health_publication.unbind();
        if let Some(next) = self.health_generation.checked_add(1) {
            self.health_generation = next;
            assert!(self.health_publication.bind(next));
            self.health_sequence = 0;
        }
    }
    fn publish_health(&mut self) {
        let Some(sequence) = self.health_sequence.checked_add(1) else {
            return;
        };
        let publication = Publication {
            stamp: Stamp {
                generation: self.health_generation,
                sequence,
                sampled_ms: self.service.elapsed_ms(),
                activity: if (0..2).any(|i| self.presentation(i).transport.playing) {
                    Activity::Active
                } else {
                    Activity::Idle
                },
            },
            value: self.runtime_status(),
        };
        if self
            .health_publication
            .accept(publication, self.service.elapsed_ms())
            .is_ok()
        {
            self.health_sequence = sequence;
        }
    }
    pub fn diagnostics(&self) -> Snapshot {
        self.service.observed_snapshot(&self.health_publication)
    }
    fn runtime_status(&self) -> RuntimeStatus {
        RuntimeStatus {
            media: if self.connected() {
                MediaStatus::Connected
            } else {
                MediaStatus::Absent
            },
            analysis: core::array::from_fn(|i| {
                if self.loaded[i].is_some() {
                    self.analysis_status[i]
                } else {
                    AnalysisStatus::Unloaded
                }
            }),
        }
    }
}

fn feed(importer: &mut AnlzImporter, kind: AnalysisFileKind, data: &[u8]) -> Result<(), ()> {
    importer
        .begin_file(kind, data.len() as u64)
        .map_err(|_| ())?;
    while let Some(plan) = importer.read_plan() {
        importer
            .ingest(
                plan.offset,
                &data[plan.offset as usize..plan.offset as usize + plan.len],
            )
            .map_err(|_| ())?;
    }
    Ok(())
}
