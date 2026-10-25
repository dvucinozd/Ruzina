//! Host owner: real deck semantics and importer, simulated transport/media.
//! This does not pump audio, access USB, or persist settings/cues to disk.
use ruzina_audio_runtime::nudge::TempoNudge;
use ruzina_board_profile::BoardId;
use ruzina_controller_core::{ControlDomain, ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_core::MediaTrackId;
use ruzina_deck::{DeckEffect, DeckLoadGate, DeckProductState, LoadPermit, LoadRefusal};
use ruzina_hot_cues::HotCueBank;
use ruzina_media_catalog::{
    CatalogBuilder, CatalogSnapshot, CatalogText, CatalogTrack,
    artwork::{
        ARTWORK_ROW_EDGE, ARTWORK_ROW_PIXELS, ArtworkDecks, ArtworkIdentity, ArtworkView,
        decode_thumbnail,
    },
    default_artwork::DEFAULT_ARTWORK_ROW,
    jpeg::BaselineJpegDecoder,
    playlists::{
        InlinePlaylistStore, LibraryActivation, LibraryBrowser, LibraryLevel, LibraryRow,
        PlaylistBuilder, PlaylistIndex,
    },
};
use ruzina_media_session::{MediaLease, MediaSession, MediaSourceId};
use ruzina_mixer::MixerState;
use ruzina_rekordbox::anlz_analysis::{AnalysisFileKind, AnlzImporter};
use ruzina_service::publication::{Activity, Publication, Slot, Stamp};
use ruzina_service::{
    AnalysisStatus, EventKind, Identity, LoadBlock, MediaStatus, RuntimeStatus, Service, Snapshot,
};

/// Host owner heartbeat contract, independent of whether a deck is playing.
pub const RUNTIME_MAX_AGE_MS: u64 = 1000;
use ruzina_settings::owner::SettingsOwner;
use ruzina_track_analysis::{
    TrackAnalysis,
    owned::{AnalysisIdentity, AnalysisPins},
};
use ruzina_ui_model::idle::{IDLE_DEFAULT_TIMEOUT_MS, IdleScreen, InputSource};
use ruzina_ui_model::long_press::LongPress;

use crate::settings_store::RamFlash;
use ruzina_ui_model::{
    BeatFxAction, DeckAction, DeckPresentation, pad_bank_event, pad_fx_event, present_deck,
};

const DAT_A: &[u8] = include_bytes!("../../../crates/ruzina-rekordbox/tests/fixtures/track-a.DAT");
const DAT_B: &[u8] = include_bytes!("../../../crates/ruzina-rekordbox/tests/fixtures/track-b.DAT");
const EXT: &[u8] = include_bytes!("../../../crates/ruzina-rekordbox/tests/fixtures/track-a.EXT");
pub const TITLES: [&str; 4] = [
    "01 / Grid study · 120 BPM",
    "02 / Offset beats · 128 BPM",
    "03 / No analysis · 120 BPM",
    "04 / Damaged analysis · recovery",
];
const DURATIONS: [u32; 4] = [10_000, 14_000, 10_000, 10_000];
/// Demo export rows: artist and key names as the PDB tables would give them
/// (empty when absent) and the export tempo.
const ARTISTS: [&str; 4] = ["Grid Lab", "Offset Unit", "", "Ruzina Demo"];
const KEYS: [&str; 4] = ["Am", "Fm", "", "Dm"];
const BPMS_X100: [u32; 4] = [12_000, 12_800, 12_000, 12_000];
/// Demo covers as the export would reference them: a baseline cover, two
/// tracks without one, and a progressive file the bounded decoder refuses.
const COVERS: [Option<&[u8]>; 4] = [
    Some(include_bytes!(concat!(
        "../../../crates/ruzina-media-catalog/tests/fixtures/artwork/",
        "cover-420.jpg"
    ))),
    None,
    None,
    Some(include_bytes!(concat!(
        "../../../crates/ruzina-media-catalog/tests/fixtures/artwork/",
        "cover-progressive.jpg"
    ))),
];

pub struct Session {
    pub analysis_observation: ruzina_service::analysis::Observation,
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
    artwork: ArtworkDecks,
    jpeg: Box<BaselineJpegDecoder>,
    idle: IdleScreen,
    idle_clock_ms: u32,
    catalog: Option<CatalogSnapshot<4>>,
    playlists: Option<DemoPlaylists>,
    library: LibraryBrowser,
    nudge: [TempoNudge; 2],
    /// Library row thumbnails per demo track: the cover or the logo (M2.5).
    row_artwork: Vec<[u16; ARTWORK_ROW_PIXELS]>,
    /// Touch HOLD RESTORE (M2.5 performance tab), on the idle clock.
    restore: LongPress<usize>,
    /// RP-11 product settings committed to a RAM settings partition.
    settings: SettingsOwner<RamFlash>,
}

type DemoPlaylists = PlaylistIndex<InlinePlaylistStore<8, 16>>;

/// What a Library row is, for presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowKind {
    Track,
    Folder,
    Playlist,
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
const fn artwork_identity(id: AnalysisIdentity) -> ArtworkIdentity {
    ArtworkIdentity {
        track: id.track,
        lease: id.lease,
        load_generation: id.load_generation,
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
            analysis_observation: ruzina_service::analysis::Observation::DISABLED,
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
            artwork: ArtworkDecks::new(),
            jpeg: Box::new(BaselineJpegDecoder::new()),
            idle: IdleScreen::new(IDLE_DEFAULT_TIMEOUT_MS, 0),
            idle_clock_ms: 0,
            catalog: None,
            playlists: None,
            library: LibraryBrowser::new(),
            nudge: [TempoNudge::new(); 2],
            row_artwork: Vec::new(),
            restore: LongPress::new(),
            settings: SettingsOwner::load(RamFlash::new())
                .expect("RAM settings partition")
                .0,
        };
        result.apply_settings();
        let mut jpeg = BaselineJpegDecoder::new();
        result.row_artwork = COVERS
            .iter()
            .map(|cover| {
                let mut pixels = [0; ARTWORK_ROW_PIXELS];
                match cover
                    .map(|bytes| decode_thumbnail(bytes, ARTWORK_ROW_EDGE, &mut pixels, &mut jpeg))
                {
                    Some(Ok(())) => pixels,
                    _ => DEFAULT_ARTWORK_ROW,
                }
            })
            .collect();
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
        self.publish_library();
        self.message = "Demo media connected · choose a track".into();
        self.publish_health();
    }
    pub fn detach(&mut self) {
        let was_connected = self.connected();
        self.cancel_pads();
        self.control(BeatFxAction::Clear.event(true));
        self.media.on_disconnect(None);
        self.catalog = None;
        self.playlists = None;
        self.library.clear();
        if was_connected {
            self.service.record(EventKind::MediaRemoved);
        }
        for i in 0..2 {
            self.decks.unload_track(deck(i));
            self.pins.clear(i);
            self.artwork.clear(i);
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
        self.note_activity();
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
            // A rejected EXT keeps the DAT analysis, as on the device.
            if selection == 0 && feed(&mut importer, AnalysisFileKind::Ext, EXT).is_err() {
                importer.discard_ext();
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
        // The committed load drops the previous cover at once; the demo
        // "read" completes immediately through the bounded decoder.
        let cover = COVERS[selection];
        if let Some(ticket) = self.artwork.begin(i, artwork_identity(id), cover.is_some()) {
            let bytes = cover.unwrap_or_default();
            if self.artwork.admit_file(ticket, bytes.len() as u64) {
                self.artwork.complete(ticket, bytes, self.jpeg.as_mut());
            }
        }
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
    /// The cover of exactly the loaded track; Empty for an unloaded deck.
    pub fn artwork(&self, i: usize) -> ArtworkView<'_> {
        match self.loaded.get(i).copied().flatten() {
            Some((_, id)) if self.media.validate(id.lease) => {
                self.artwork.view(i, artwork_identity(id))
            }
            _ => ArtworkView::Empty,
        }
    }
    pub fn title(&self, i: usize) -> &str {
        self.loaded[i]
            .map(|(selection, _)| TITLES[selection])
            .unwrap_or("No track loaded")
    }
    /// The loaded track's key name, if the export has one.
    pub fn key(&self, i: usize) -> Option<&str> {
        self.loaded[i].map(|(selection, _)| KEYS[selection])
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
        self.note_activity();
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
                DeckEffect::JogNudge { deck, ticks } => self.nudge[index(deck)].add(ticks),
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
                    self.message = if bank.override_mask() == 0 {
                        "Rekordbox cues restored for this session"
                    } else {
                        "Local cue edit retained for this session"
                    }
                    .into();
                }
                _ => {} // Deck state already records pause/pitch/loop; no audio DSP in host.
            }
        }
    }
    pub fn cancel_pads(&mut self) {
        for i in 0..2 {
            if let Some(active) = self.decks.deck(deck(i)).active_pad_fx {
                let bank = if active.bank == ruzina_deck::PadFxBank::One {
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
                let old_bank = if active.bank == ruzina_deck::PadFxBank::One {
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
            // A jog nudge rides on the fader pitch and springs back (B13a).
            let pitch = self.nudge[i].apply_centipercent(p.transport.pitch_centi_percent as i16);
            self.nudge[i].advance(elapsed_ms.saturating_mul(48), 48_000);
            let rate = (10_000 + i32::from(pitch)).max(0) as u64;
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
        self.idle_clock_ms = self.idle_clock_ms.wrapping_add(elapsed_ms);
        if let Some(i) = self.restore.tick(self.idle_clock_ms) {
            self.control(DeckAction::RestoreSourceCues.event(deck(i), true));
        }
        self.update_idle();
        self.publish_health();
    }
    /// HOLD RESTORE pressed or released on a deck; `None` is a lost press
    /// (page change). The restore fires only after the M2.5 long press.
    pub fn restore_hold(&mut self, deck: Option<usize>, pressed: bool) {
        self.note_activity();
        match (deck, pressed) {
            (Some(i), true) if i < 2 => self.restore.press(i, self.idle_clock_ms),
            (Some(i), false) => self.restore.release(i),
            _ => self.restore.cancel(),
        }
    }
    /// Advance only the observer. Replay uses this to model a stopped publisher.
    /// The interactive timer normally calls tick(), which also runs the owner.
    pub fn observe_elapsed(&mut self, elapsed_ms: u32) {
        self.service.advance(u64::from(elapsed_ms));
    }
    /// Touch, navigation or a control; restarts the screensaver countdown.
    pub fn note_activity(&mut self) {
        self.idle.input(InputSource::Physical, self.idle_clock_ms);
    }
    /// The input that dismisses the screensaver is spent on waking (M2.5).
    pub fn wake(&mut self) {
        self.note_activity();
        self.update_idle();
    }
    pub const fn screensaver(&self) -> bool {
        self.idle.shown()
    }

    /// Demo export for the connected media: the four demo tracks plus a
    /// small playlist tree, browsed with the product `LibraryBrowser`.
    fn publish_library(&mut self) {
        let Some(lease) = self.media.lease() else {
            return;
        };
        let mut builder = CatalogBuilder::<4>::new(lease);
        for (i, title) in TITLES.iter().enumerate() {
            builder
                .push(CatalogTrack {
                    track_id: MediaTrackId([i as u8 + 1; 16]),
                    source_track_id: i as u32 + 1,
                    duration_ms: DURATIONS[i],
                    bpm_x100: BPMS_X100[i],
                    title: CatalogText::try_from_str(title).expect("demo title fits"),
                    artist: CatalogText::try_from_str(ARTISTS[i]).expect("demo artist fits"),
                    key: CatalogText::try_from_str(KEYS[i]).expect("demo key fits"),
                    ..CatalogTrack::EMPTY
                })
                .expect("demo catalog capacity");
        }
        let catalog = builder.finish(1);
        let mut tree = PlaylistBuilder::new(InlinePlaylistStore::<8, 16>::new());
        for (id, parent, sort, folder, name) in [
            (1, 0, 1, true, "Sets"),
            (2, 1, 1, false, "Warm-up"),
            (3, 1, 2, false, "Peak time"),
            (4, 0, 2, false, "Recovery checks"),
        ] {
            tree.push_node(id, parent, sort, folder, name);
        }
        for (playlist, entry, track) in [
            (2, 0, 2),
            (2, 1, 1),
            (3, 0, 1),
            (3, 1, 2),
            (4, 0, 3),
            (4, 1, 4),
        ] {
            tree.push_entry(playlist, entry, track);
        }
        let playlists = tree.finish(&catalog);
        self.library.publish(&catalog, Some(&playlists));
        // A reconnect keeps the track-list selection, as before.
        self.library.select(self.selected as u16);
        self.catalog = Some(catalog);
        self.playlists = Some(playlists);
    }

    /// The catalog publication a Web page lists from (`library_revision`).
    pub fn catalog_publication(&self) -> Option<ruzina_media_catalog::CatalogPublicationId> {
        self.web_catalog()
            .map(|catalog| ruzina_media_catalog::CatalogPublicationId {
                lease: catalog.lease(),
                revision: catalog.revision(),
            })
    }
    /// The published catalog while its media lease holds (Web Library).
    pub fn web_catalog(&self) -> Option<&CatalogSnapshot<4>> {
        self.catalog
            .as_ref()
            .filter(|catalog| self.media.validate(catalog.lease()))
    }
    /// The load generation of the deck's current track, 0 when empty.
    pub fn load_generation(&self, i: usize) -> u32 {
        self.loaded[i].map_or(0, |(_, id)| id.load_generation)
    }
    pub fn artist(&self, i: usize) -> &'static str {
        self.loaded[i].map_or("", |(selection, _)| ARTISTS[selection])
    }
    /// Web Remote load (M2.5 `/api/load`): the track must be in the catalog
    /// revision the page listed, then LOAD LOCK, then the normal load path.
    pub fn web_load(
        &mut self,
        target: DeckId,
        track: u32,
        revision: u32,
    ) -> ruzina_web::server::LoadOutcome {
        use ruzina_web::{server::LoadOutcome, status};
        self.note_activity();
        let catalog = self
            .catalog
            .as_ref()
            .filter(|_| self.media.lease().is_some());
        let candidate = match status::resolve_load(catalog, track, revision) {
            Ok(candidate) => candidate,
            Err(status::LoadRefusal::UnknownTrack) => return LoadOutcome::NotFound,
            Err(_) => return LoadOutcome::LibraryChanged,
        };
        if self.decks.load_admission(target).is_err() {
            return LoadOutcome::Locked;
        }
        self.selected = usize::from(candidate.index);
        if self.load(target) {
            LoadOutcome::Loaded
        } else {
            LoadOutcome::Busy
        }
    }
    /// Web Remote seek on a loaded deck; the server already clamped it.
    pub fn web_seek(&mut self, target: DeckId, position_ms: u32) -> bool {
        self.note_activity();
        if !self.presentation(index(target)).transport.loaded {
            return false;
        }
        self.decks.set_position_ms(target, position_ms);
        true
    }

    /// Rows of the current Library level. Without media the demo titles stay
    /// listed (disabled), as before.
    pub fn library_rows(&self) -> Vec<(String, RowKind)> {
        let Some(catalog) = &self.catalog else {
            return TITLES
                .iter()
                .map(|t| ((*t).into(), RowKind::Track))
                .collect();
        };
        (0..self.library.count())
            .filter_map(|i| {
                Some(
                    match self.library.row_at(i, catalog, self.playlists.as_ref())? {
                        LibraryRow::Track(candidate) => {
                            (TITLES[usize::from(candidate.index)].into(), RowKind::Track)
                        }
                        LibraryRow::Node(node) if node.is_folder => {
                            (format!("FOLDER · {}", node.name.as_str()), RowKind::Folder)
                        }
                        LibraryRow::Node(node) => {
                            let tracks = self
                                .playlists
                                .as_ref()
                                .map_or(0, |p| p.tracks(node.id).len());
                            (
                                format!("PLAYLIST · {} ({tracks})", node.name.as_str()),
                                RowKind::Playlist,
                            )
                        }
                    },
                )
            })
            .collect()
    }
    /// The demo track shown in each row of the current Library level, for
    /// its columns and thumbnail (M2.5 `dj_track_t`); `None` on node rows.
    pub fn library_row_tracks(&self) -> Vec<Option<usize>> {
        let Some(catalog) = &self.catalog else {
            return (0..TITLES.len()).map(Some).collect();
        };
        (0..self.library.count())
            .filter_map(
                |i| match self.library.row_at(i, catalog, self.playlists.as_ref())? {
                    LibraryRow::Track(candidate) => Some(Some(usize::from(candidate.index))),
                    LibraryRow::Node(_) => Some(None),
                },
            )
            .collect()
    }
    /// Row columns of a demo track: artist, key, rounded BPM and length.
    pub fn track_columns(&self, track: usize) -> (&'static str, &'static str, u32, u32) {
        (
            ARTISTS[track],
            KEYS[track],
            ruzina_ui_model::labels::library_bpm(BPMS_X100[track]),
            DURATIONS[track],
        )
    }
    pub fn row_artwork(&self, track: usize) -> &[u16] {
        &self.row_artwork[track]
    }

    /// "DEMO LIBRARY" for the track list, otherwise the playlist path.
    pub fn library_path(&self) -> String {
        let level = self.library.level();
        let Some(playlists) = &self.playlists else {
            return "DEMO LIBRARY".into();
        };
        let mut id = match level {
            LibraryLevel::AllTracks => return "DEMO LIBRARY".into(),
            LibraryLevel::Folder(id) | LibraryLevel::Playlist(id) => id,
        };
        let mut names = Vec::new();
        while let Some(node) = playlists.node(id) {
            names.push(node.name.as_str().to_owned());
            id = node.parent_id;
        }
        names.push("PLAYLISTS".into());
        names.reverse();
        names.join(" / ")
    }

    pub fn library_level(&self) -> LibraryLevel {
        self.library.level()
    }

    /// LOAD LOCK refuses a track load to this deck now (M2.5 `D1 LOCKED`).
    /// On folder and playlist rows LOAD only opens, so nothing is locked.
    pub fn load_locked(&self, i: usize) -> bool {
        let on_node = self.catalog.is_some() && self.library.on_node_row();
        !on_node && self.decks.load_admission(deck(i)).is_err()
    }
    pub fn load_lock(&self) -> bool {
        self.decks.load_lock()
    }
    /// M2.5 elapsed/remaining time, from the settings record.
    pub fn time_remaining(&self) -> bool {
        self.settings.settings().time_display == ruzina_settings::TimeDisplay::Remaining
    }
    /// Commits the time display; a failed write keeps the shown one.
    pub fn set_time_remaining(&mut self, remaining: bool) -> bool {
        self.note_activity();
        let time_display = if remaining {
            ruzina_settings::TimeDisplay::Remaining
        } else {
            ruzina_settings::TimeDisplay::Elapsed
        };
        let next = ruzina_settings::ProductSettings {
            time_display,
            ..self.settings.settings()
        };
        let saved = self.settings.apply(next).is_ok();
        if !saved {
            self.message = "Settings not saved · time display unchanged".into();
        }
        saved
    }
    /// Settings LOAD LOCK switch (a Ruzina addition; stable M2.5 keeps it
    /// on). It is committed to the settings record first and reaches the
    /// deck only once stored, so a failed write changes nothing.
    pub fn set_load_lock(&mut self, on: bool) -> bool {
        self.note_activity();
        let next = ruzina_settings::ProductSettings {
            load_lock: on,
            ..self.settings.settings()
        };
        match self.settings.apply(next) {
            Ok(_) => {
                self.decks.set_load_lock(on);
                self.message = if on {
                    "LOAD LOCK on · no load while a deck plays"
                } else {
                    "LOAD LOCK off · a playing deck can be replaced"
                }
                .into();
                true
            }
            Err(_) => {
                self.message = "Settings not saved · LOAD LOCK unchanged".into();
                false
            }
        }
    }
    /// Power-cycles the settings owner: the record is read back from the
    /// partition and applied, as at boot.
    pub fn reload_settings(&mut self) {
        let flash = core::mem::replace(
            &mut self.settings,
            SettingsOwner::load(RamFlash::new())
                .expect("RAM settings partition")
                .0,
        )
        .into_flash();
        self.settings = SettingsOwner::load(flash)
            .expect("RAM settings partition")
            .0;
        self.apply_settings();
    }
    fn apply_settings(&mut self) {
        self.decks.set_load_lock(self.settings.settings().load_lock);
    }
    pub fn library_selected(&self) -> Option<usize> {
        self.library.selected_index().map(usize::from)
    }

    pub fn library_select(&mut self, row: usize) {
        self.note_activity();
        if let Ok(row) = u16::try_from(row)
            && self.library.select(row)
            && self.library.level() == LibraryLevel::AllTracks
        {
            self.selected = usize::from(row);
        }
    }

    /// PLAYLISTS / BACK (M2.5).
    pub fn library_back(&mut self) -> bool {
        self.note_activity();
        match &self.catalog {
            Some(catalog) => self.library.back(catalog, self.playlists.as_ref()),
            None => false,
        }
    }

    /// LOAD on the selected row: a folder or playlist opens without load
    /// admission; a track enters the normal load path.
    pub fn library_load(&mut self, target: DeckId) -> bool {
        self.note_activity();
        let Some(catalog) = &self.catalog else {
            return self.load(target);
        };
        match self.library.activate(catalog, self.playlists.as_ref()) {
            LibraryActivation::Opened => {
                self.message = format!("Opened {}", self.library_path());
                false
            }
            LibraryActivation::Load(candidate) => {
                self.selected = usize::from(candidate.index);
                self.load(target)
            }
            LibraryActivation::Empty => {
                self.message = "Nothing to load here".into();
                false
            }
            LibraryActivation::Stale => {
                self.message = "Library changed · choose again".into();
                false
            }
        }
    }
    fn update_idle(&mut self) {
        let playing = (0..2).any(|i| self.presentation(i).transport.playing);
        self.idle.tick(self.idle_clock_ms, playing, false);
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
