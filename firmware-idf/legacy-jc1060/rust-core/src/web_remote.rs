//! The Web Remote on the deck's own Wi-Fi: `ruzina-web` over the engine.
//!
//! The C side (`main/wifi_probe.c`) accepts one connection at a time, reads a request and
//! hands its bytes to [`ruzina_web_handle`], which answers with `ruzina-web`'s page and
//! API. Controls take the touch path to the USB task: [`deck_view::WEB`] is a second touch
//! queue, so they pass the same admission as the deck screen's buttons.
//!
//! The page lists the catalog from the Web Remote's own copy
//! ([`deck_view::take_web_library`]) and shows each deck's title and duration
//! ([`deck_view::take_web_track`]). A load goes to the USB task through
//! [`deck_view::WEB_LOADS`], into the same admission as a LOAD, without moving the
//! screen's selection. Controls are PLAY and CUE; others and seeks are refused.
use crate::deck_view::{self, TouchControl, WebLibrary, WebTrack};
use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::Ordering;
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_media_catalog::CatalogPublicationId;
use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
use ruzina_web::json::{DeckStateText, DeckStatus, LibraryRow, MixerStatus};
use ruzina_web::server::{
    ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend, WebServer,
};

/// The access point's address; the guard admits requests for it.
const AP_IPV4: &str = "192.168.4.1";

/// What the web task keeps between requests: the catalog and each deck's track.
#[derive(Default)]
struct WebState {
    library: Option<Box<WebLibrary>>,
    tracks: [Option<Box<WebTrack>>; 2],
}

impl WebState {
    /// Takes whatever the load task published since the last request.
    fn refresh(&mut self) {
        if let Some(library) = deck_view::take_web_library() {
            self.library = Some(library);
        }
        for (deck, track) in self.tracks.iter_mut().enumerate() {
            if let Some(new) = deck_view::take_web_track(deck) {
                *track = Some(new);
            }
        }
    }

    /// Deck `deck`'s track, while the engine has it loaded.
    fn track(&self, deck: usize) -> Option<&WebTrack> {
        let loaded = deck_view::LIVE[deck].load().has(deck_view::LOADED);
        self.tracks[deck]
            .as_deref()
            .filter(|track| loaded && track.index != u32::MAX)
    }
}

/// The page asks for the catalog by revision; the lease is not shown, so any will do.
const LEASE: MediaLease = MediaLease {
    generation: MediaGeneration::initial(),
    source: match MediaSourceId::new(1) {
        Some(source) => source,
        None => unreachable!(),
    },
};

struct Backend<'a> {
    state: &'a WebState,
}

fn deck_index(deck: DeckId) -> usize {
    match deck {
        DeckId::One => 0,
        DeckId::Two => 1,
    }
}

/// The touch request an event of the first step stands for, if it is one.
fn touch(event: &ControlEvent) -> Option<(usize, Option<TouchControl>)> {
    let deck = deck_index(event.deck?);
    match (event.control, event.value) {
        // PLAY acts on the press; its release has nothing to do.
        (SemanticControl::Play, ControlValue::Pressed(down)) => {
            Some((deck, down.then_some(TouchControl::Play)))
        }
        (SemanticControl::Cue, ControlValue::Pressed(down)) => {
            Some((deck, Some(TouchControl::Cue(down))))
        }
        _ => None,
    }
}

impl WebBackend for Backend<'_> {
    fn status(&mut self) -> StatusParts<'_> {
        let state = self.state;
        let deck = |i: usize| {
            let live = deck_view::LIVE[i].load();
            if !live.has(deck_view::LOADED) {
                return DeckStatus::EMPTY;
            }
            let playing = live.has(deck_view::PLAYING);
            let pitch = live.pitch_centipercent;
            let track = state.track(i);
            DeckStatus {
                title: track.map_or("", |track| track.title.as_str()),
                artist: track.map_or("", |track| track.artist.as_str()),
                duration_ms: track.map_or(0, |track| track.duration_ms),
                bpm: (i64::from(live.bpm_x100) * (10_000 + i64::from(pitch)) / 1_000_000) as u32,
                pitch_centipercent: pitch.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                position_ms: live.position_ms,
                playing,
                state: if playing {
                    DeckStateText::Playing
                } else {
                    DeckStateText::Ready
                },
                ..DeckStatus::EMPTY
            }
        };
        StatusParts {
            catalog: state.library.as_ref().map(|library| CatalogPublicationId {
                lease: LEASE,
                revision: library.revision,
            }),
            load_generations: [0, 0],
            decks: [deck(0), deck(1)],
            mixer: MixerStatus {
                volume: [0; 2],
                crossfader: 8192,
                master_volume: 0,
                headphone_mix: 0,
                eq: [[8192; 3]; 2],
                filter: [8192; 2],
                smart_cfx: false,
                smart_fader: false,
                pfl: [false; 2],
            },
        }
    }

    fn control(&mut self, events: &[ControlEvent]) -> ControlOutcome {
        // All or nothing: a command with a control this step does not carry is refused.
        let mut requests = [None; 2];
        for (slot, event) in requests.iter_mut().zip(events) {
            let Some(request) = touch(event) else {
                return ControlOutcome::Busy;
            };
            *slot = Some(request);
        }
        if events.len() > requests.len() {
            return ControlOutcome::Busy;
        }
        // Room for the whole command first: a CUE press queued without its release would
        // leave the deck holding a preview. Only this task pushes, so the room stays.
        let needed = requests
            .iter()
            .flatten()
            .filter(|(_, c)| c.is_some())
            .count() as u32;
        if deck_view::WEB.free() < needed {
            return ControlOutcome::Busy;
        }
        for (deck, control) in requests.into_iter().flatten() {
            if let Some(control) = control {
                deck_view::WEB.push(deck, control);
            }
        }
        ControlOutcome::Applied
    }

    fn seek(&mut self, _deck: DeckId, _position_ms: u32) -> SeekOutcome {
        SeekOutcome::NotSeekable
    }

    /// `track` is the row's `track_key`: the catalog index plus one.
    fn load(&mut self, deck: DeckId, track: u32, revision: u32) -> LoadOutcome {
        let Some(library) = self.state.library.as_deref() else {
            return LoadOutcome::NotFound;
        };
        if library.revision != revision {
            return LoadOutcome::LibraryChanged;
        }
        if track == 0 || track as usize > library.rows.len() {
            return LoadOutcome::NotFound;
        }
        let i = deck_index(deck);
        // The load lock, as the engine applies it: never onto a playing deck.
        if deck_view::LIVE[i].load().has(deck_view::PLAYING) {
            return LoadOutcome::Locked;
        }
        // One load per deck in flight; the USB task takes it on its next turn.
        match deck_view::WEB_LOADS[i].compare_exchange(
            0,
            track,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => LoadOutcome::Loaded,
            Err(_) => LoadOutcome::Busy,
        }
    }

    fn duration_ms(&self, deck: DeckId) -> Option<u32> {
        self.state
            .track(deck_index(deck))
            .map(|track| track.duration_ms)
    }

    fn library(&self) -> Option<(u32, usize)> {
        self.state
            .library
            .as_deref()
            .map(|library| (library.revision, library.rows.len()))
    }

    fn library_row(&self, index: usize) -> Option<LibraryRow<'_>> {
        let row = self.state.library.as_deref()?.rows.get(index)?;
        Some(LibraryRow {
            track_key: index as u32 + 1,
            title: if row.title.is_empty() {
                row.artist.as_str()
            } else {
                row.title.as_str()
            },
            artist: row.artist.as_str(),
            bpm: (row.bpm_x100 + 50) / 100,
            duration_ms: row.duration_ms,
        })
    }
}

struct Server(UnsafeCell<Option<(WebServer, WebState)>>);

// SAFETY: only the web task calls `ruzina_web_handle`, one request at a time.
unsafe impl Sync for Server {}

static SERVER: Server = Server(UnsafeCell::new(None));

/// Answers one HTTP request: `request` holds its head and any body, and the complete
/// response is written to `out`. Returns the response's length; 0 means close without
/// answering. `boot` should differ on each start, so the page notices a reboot.
///
/// # Safety
/// `request` must be valid for `request_len` reads and `out` for `out_len` writes. Web
/// task only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_web_handle(
    boot: u32,
    request: *const u8,
    request_len: usize,
    out: *mut u8,
    out_len: usize,
) -> usize {
    if request.is_null() || out.is_null() {
        return 0;
    }
    // SAFETY: the caller hands over both buffers for this call.
    let (request, out) = unsafe {
        (
            core::slice::from_raw_parts(request, request_len),
            core::slice::from_raw_parts_mut(out, out_len),
        )
    };
    // SAFETY: the web task is the only caller, so nothing else holds the server.
    let (server, state) = unsafe { &mut *SERVER.0.get() }
        .get_or_insert_with(|| (WebServer::new(boot, AP_IPV4), WebState::default()));
    state.refresh();
    server.handle(request, &mut Backend { state }, out)
}

/// The largest page asset, for the response buffer the C side allocates.
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_web_largest_asset() -> usize {
    ruzina_web::assets::LARGEST_BYTES
}

#[cfg(test)]
mod tests;
