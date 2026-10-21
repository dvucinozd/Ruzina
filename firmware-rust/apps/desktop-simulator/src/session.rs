//! Host owner: real deck semantics and importer, simulated transport/media.
//! This does not pump audio, access USB, or persist settings/cues to disk.
use pajoniiir_controller_core::{ControlEvent, DeckId};
use pajoniiir_core::MediaTrackId;
use pajoniiir_deck::{DeckEffect, DeckProductState};
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_media_session::{MediaSession, MediaSourceId};
use pajoniiir_rekordbox::anlz_analysis::{AnalysisFileKind, AnlzImporter};
use pajoniiir_track_analysis::{
    TrackAnalysis,
    owned::{AnalysisIdentity, AnalysisPins},
};
use pajoniiir_ui_model::{DeckPresentation, present_deck};

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
    pub decks: DeckProductState,
    pub selected: usize,
    pub message: String,
    media: MediaSession,
    pins: AnalysisPins,
    loaded: [Option<(usize, AnalysisIdentity)>; 2],
    local: [Option<HotCueBank>; 4],
    generation: u32,
    fraction: [u64; 2],
}

pub const fn deck(index: usize) -> DeckId {
    if index == 0 { DeckId::One } else { DeckId::Two }
}
pub const fn index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

impl Session {
    pub fn new() -> Self {
        let mut result = Self {
            decks: DeckProductState::new(),
            selected: 0,
            message: "Choose a demo track in Library".into(),
            media: MediaSession::new(),
            pins: AnalysisPins::new(),
            loaded: [None, None],
            local: [None; 4],
            generation: 0,
            fraction: [0; 2],
        };
        result.reconnect();
        result
    }
    pub fn connected(&self) -> bool {
        self.media.lease().is_some()
    }
    pub fn reconnect(&mut self) {
        self.media.on_connect(MediaSourceId::new(1).unwrap());
        self.message = "Demo media connected · choose a track".into();
    }
    pub fn detach(&mut self) {
        self.media.on_disconnect(None);
        for i in 0..2 {
            self.decks.unload_track(deck(i));
            self.pins.clear(i);
            self.loaded[i] = None;
            self.fraction[i] = 0;
        }
        self.message = "Media removed · decks stopped · reconnect in Library".into();
    }
    pub fn load(&mut self, target: DeckId) -> bool {
        let i = index(target);
        let Some(lease) = self.media.lease() else {
            self.message = "Load unavailable · reconnect media".into();
            return false;
        };
        if self.decks.deck(target).playing {
            self.message = "Load locked · pause the destination deck".into();
            return false;
        }
        let selection = self.selected;
        if selection >= TITLES.len() {
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
        if !self.decks.commit_track_load(
            target,
            id.track,
            if selection == 1 { 12800 } else { 12000 },
            source.as_source(),
            self.local[selection],
        ) {
            return false;
        }
        self.pins.clear(i);
        let available = publication.is_some();
        if let Some(publication) = publication {
            assert!(self.pins.publish(i, id, publication));
        }
        self.loaded[i] = Some((selection, id));
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
        let Some(target) = event.deck else {
            return;
        };
        if self.loaded[index(target)].is_none() {
            self.message = "Load a track before using deck controls".into();
            return;
        }
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
                        self.message = "End of track · press CUE to return".into();
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
                    }
                }
                DeckEffect::PersistHotCues { deck, bank } => {
                    if let Some((selection, _)) = self.loaded[index(deck)] {
                        self.local[selection] = Some(bank);
                    }
                    self.message = "Local cue edit retained for this session".into();
                }
                _ => {} // Deck state already records pause/pitch/loop; no audio DSP in host.
            }
        }
    }
    /// Explicit host-only transport clock. Fractional pitch time is retained;
    /// real product timing continues to come from its PCM owner.
    pub fn tick(&mut self, elapsed_ms: u32) {
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
            } else {
                self.decks.set_position_ms(deck(i), next as u32);
            }
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
