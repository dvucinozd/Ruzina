//! The Web Remote on the deck's own Wi-Fi: `ruzina-web` over the engine.
//!
//! The C side (`main/wifi_probe.c`) accepts one connection at a time, reads a request and
//! hands its bytes to [`ruzina_web_handle`], which answers with `ruzina-web`'s page and
//! API. Controls take the touch path to the USB task: [`deck_view::WEB`] is a second touch
//! queue, so they pass the same admission as the deck screen's buttons.
//!
//! This first step carries PLAY and CUE only. Other controls, loads and seeks are
//! refused, and the status has no titles or durations: the screen owns those.
use crate::deck_view::{self, TouchControl};
use core::cell::UnsafeCell;
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_web::json::{DeckStateText, DeckStatus, MixerStatus};
use ruzina_web::server::{
    ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend, WebServer,
};

/// The access point's address; the guard admits requests for it.
const AP_IPV4: &str = "192.168.4.1";

struct Backend;

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

impl WebBackend for Backend {
    fn status(&mut self) -> StatusParts<'_> {
        let deck = |i: usize| {
            let live = deck_view::LIVE[i].load();
            if !live.has(deck_view::LOADED) {
                return DeckStatus::EMPTY;
            }
            let playing = live.has(deck_view::PLAYING);
            let pitch = live.pitch_centipercent;
            DeckStatus {
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
            catalog: None,
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

    fn load(&mut self, _deck: DeckId, _track: u32, _revision: u32) -> LoadOutcome {
        LoadOutcome::NotFound
    }

    fn duration_ms(&self, _deck: DeckId) -> Option<u32> {
        None
    }

    fn library(&self) -> Option<(u32, usize)> {
        None
    }

    fn library_row(&self, _index: usize) -> Option<ruzina_web::json::LibraryRow<'_>> {
        None
    }
}

struct Server(UnsafeCell<Option<WebServer>>);

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
    let server =
        unsafe { &mut *SERVER.0.get() }.get_or_insert_with(|| WebServer::new(boot, AP_IPV4));
    server.handle(request, &mut Backend, out)
}

/// The largest page asset, for the response buffer the C side allocates.
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_web_largest_asset() -> usize {
    ruzina_web::assets::LARGEST_BYTES
}

#[cfg(test)]
mod tests;
