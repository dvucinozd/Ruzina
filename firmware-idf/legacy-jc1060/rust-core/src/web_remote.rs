//! The Web Remote on the deck's own Wi-Fi: `ruzina-web` over the engine.
//!
//! The C side (`main/wifi_remote.c`) accepts one connection at a time, reads a request and
//! hands its bytes to [`ruzina_web_handle`], which answers with `ruzina-web`'s page and
//! API. PLAY and CUE take the touch path to the USB task: [`deck_view::WEB`] is a second
//! touch queue, so they pass the same admission as the deck screen's buttons. The other
//! controls (the mixer, SYNC, PFL, Tempo, loops) go as the controller sends them, through
//! [`deck_view::WEB_EVENTS`]. The USB task's side of all of it is [`service`].
//!
//! The page lists the catalog from the Web Remote's own copy
//! ([`deck_view::take_web_library`]) and shows each deck's title and duration
//! ([`deck_view::take_web_track`]). A load goes to the USB task through [`WEB_LOAD`], into
//! the same admission as a LOAD, without moving the screen's selection. A seek goes
//! through [`WEB_SEEK`]. For both, the web task waits for the engine's answer, so the page
//! hears what the deck did, not only that the request was queued.
use crate::deck_view::{self, MixView, TouchControl, WebLibrary, WebTrack};
use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_media_catalog::CatalogPublicationId;
use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
use ruzina_web::json::{DeckStateText, DeckStatus, LibraryRow, MixerStatus};
use ruzina_web::server::{
    ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend, WebServer,
};

/// The access point's address; the guard admits requests for it.
const AP_IPV4: &str = "192.168.4.1";

/// One request at a time from the web task to the USB task, and the USB task's answer.
pub(crate) struct Exchange {
    /// The ticket of the request waiting for the USB task; 0 for none.
    asked: AtomicU32,
    args: [AtomicU32; 3],
    /// The last answered request: its ticket, shifted up 8 bits, and its answer.
    answered: AtomicU32,
}

/// How long the web task waits for the USB task to take a request. The USB task serves
/// the page at least every 50 ms while the controller is attached.
const TAKE_MS: u32 = 250;
/// How long it then waits for the answer, which the USB task gives in the same turn.
const ANSWER_MS: u32 = 1_000;
/// One wait's length.
const STEP_MS: u32 = 2;

/// The engine did it.
pub(crate) const DONE: u8 = 0;
/// LOAD LOCK refused the load.
pub(crate) const LOCKED: u8 = 1;
/// Another load onto the deck is in flight.
pub(crate) const BUSY: u8 = 2;
/// The catalog or the deck's track changed since the page saw it, or the media went.
pub(crate) const CHANGED: u8 = 3;
/// The deck has no track to seek in.
pub(crate) const NO_TRACK: u8 = 4;
/// A scratch or a loop holds the deck's place.
pub(crate) const HELD: u8 = 5;

/// Tests: puts a request up on `exchange`, lets `before` act on the mix as the controller
/// would meanwhile, runs the USB task's turn and gives the engine's answer.
#[cfg(test)]
pub(crate) fn engine_answer(
    exchange: &Exchange,
    args: [u32; 3],
    mix: &mut crate::deck_mix::Mix,
    before: impl FnOnce(&mut crate::deck_mix::Mix),
) -> Option<u8> {
    let ticket = exchange.post(args);
    before(mix);
    service(mix);
    exchange.reply(ticket)
}

impl Exchange {
    pub(crate) const fn new() -> Self {
        Self {
            asked: AtomicU32::new(0),
            args: [const { AtomicU32::new(0) }; 3],
            answered: AtomicU32::new(0),
        }
    }

    /// Web task: asks, then waits for the answer. `None` if the USB task did not take the
    /// request in time; it is then withdrawn and never runs.
    fn ask(&self, args: [u32; 3]) -> Option<u8> {
        let ticket = self.post(args);
        let mut waited = 0;
        loop {
            if let Some(answer) = self.reply(ticket) {
                return Some(answer);
            }
            if waited >= TAKE_MS {
                let withdrawn = self
                    .asked
                    .compare_exchange(ticket, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok();
                if withdrawn || waited >= TAKE_MS + ANSWER_MS {
                    return None;
                }
            }
            wait();
            waited += STEP_MS;
        }
    }

    /// Web task: puts a request up for the USB task; returns its ticket.
    fn post(&self, args: [u32; 3]) -> u32 {
        let ticket = TICKET.fetch_add(1, Ordering::Relaxed) % 0x00FF_FFFF + 1;
        for (slot, value) in self.args.iter().zip(args) {
            slot.store(value, Ordering::Relaxed);
        }
        self.asked.store(ticket, Ordering::Release);
        ticket
    }

    /// Web task: the answer to the request with `ticket`, once there is one.
    fn reply(&self, ticket: u32) -> Option<u8> {
        let answered = self.answered.load(Ordering::Acquire);
        (answered >> 8 == ticket).then_some(answered as u8)
    }

    /// USB task: the waiting request's ticket and arguments, if one waits.
    fn take(&self) -> Option<(u32, [u32; 3])> {
        let ticket = self.asked.swap(0, Ordering::Acquire);
        (ticket != 0).then(|| {
            (
                ticket,
                self.args.each_ref().map(|a| a.load(Ordering::Relaxed)),
            )
        })
    }

    /// USB task: the answer to the request with `ticket`.
    fn answer(&self, ticket: u32, answer: u8) {
        self.answered
            .store(ticket << 8 | u32::from(answer), Ordering::Release);
    }
}

/// Web task only: each request's ticket comes from here.
static TICKET: AtomicU32 = AtomicU32::new(0);

/// Each deck's Web Remote load: the catalog index plus one, and the catalog revision the
/// page listed it from.
pub(crate) static WEB_LOAD: [Exchange; 2] = [const { Exchange::new() }; 2];
/// Each deck's Web Remote seek: the position in ms, whether the page sent the deck's load
/// generation, and that generation.
pub(crate) static WEB_SEEK: [Exchange; 2] = [const { Exchange::new() }; 2];
/// Each deck's load generation, which the USB task publishes after every turn for the
/// status.
pub(crate) static LOAD_GENERATIONS: [AtomicU32; 2] = [const { AtomicU32::new(0) }; 2];

/// One short wait of the web task.
#[cfg(not(test))]
fn wait() {
    unsafe extern "C" {
        fn ruzina_web_wait(ms: u32);
    }
    // SAFETY: a plain delay of the calling task, the web task.
    unsafe { ruzina_web_wait(STEP_MS) }
}

#[cfg(test)]
std::thread_local! {
    /// What a test's USB task does while the web task waits.
    static USB_TURN: core::cell::RefCell<Option<Box<dyn FnMut()>>> =
        core::cell::RefCell::new(None);
}

#[cfg(test)]
fn wait() {
    USB_TURN.with(|turn| {
        if let Some(turn) = turn.borrow_mut().as_mut() {
            turn();
        }
    });
}

/// What the web task keeps between requests: the catalog and each deck's track.
#[derive(Default)]
struct WebState {
    /// The deck's name, as `ruzina_web_set_name` set it.
    name: alloc::string::String,
    library: Option<Box<WebLibrary>>,
    tracks: [Option<Box<WebTrack>>; 2],
}

impl WebState {
    /// Takes whatever the load task published since the last request.
    fn refresh(&mut self) {
        let len = NAME_LEN.load(Ordering::Acquire).min(NAME.len());
        let changed = len != self.name.len()
            || self
                .name
                .bytes()
                .zip(&NAME)
                .any(|(have, slot)| have != slot.load(Ordering::Relaxed));
        if changed {
            let bytes: alloc::vec::Vec<u8> = NAME[..len]
                .iter()
                .map(|b| b.load(Ordering::Relaxed))
                .collect();
            self.name = alloc::string::String::from_utf8(bytes).unwrap_or_default();
        }
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
            let view = MixView::load().unwrap_or_default();
            DeckStatus {
                sync_enabled: view.sync[i],
                sync_master: view.sync_master[i],
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
            name: state.name.as_str(),
            catalog: state.library.as_ref().map(|library| CatalogPublicationId {
                lease: LEASE,
                revision: library.revision,
            }),
            load_generations: LOAD_GENERATIONS
                .each_ref()
                .map(|g| g.load(Ordering::Acquire)),
            decks: [deck(0), deck(1)],
            mixer: mixer_status(),
        }
    }

    fn control(&mut self, events: &[ControlEvent]) -> ControlOutcome {
        // PLAY and CUE as the screen's buttons; anything else as the controller sends it.
        // A command is one action, so it goes one way, whole or not at all.
        let mut requests = [None; 2];
        let mut touched = events.len() <= requests.len();
        for (slot, event) in requests.iter_mut().zip(events) {
            match touch(event) {
                Some(request) => *slot = Some(request),
                None => touched = false,
            }
        }
        if !touched {
            if deck_view::WEB_EVENTS.free() < events.len() as u32 {
                return ControlOutcome::Busy;
            }
            for event in events {
                deck_view::WEB_EVENTS.push(*event);
            }
            return ControlOutcome::Applied;
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

    /// A seek onto a loaded deck with a known duration, answered as the engine did it.
    fn seek(&mut self, deck: DeckId, position_ms: u32, generation: Option<u32>) -> SeekOutcome {
        let i = deck_index(deck);
        let Some(duration) = self.state.track(i).map(|track| track.duration_ms) else {
            return SeekOutcome::NotSeekable;
        };
        if duration == 0 {
            return SeekOutcome::NotSeekable;
        }
        let args = [
            position_ms.min(duration - 1),
            u32::from(generation.is_some()),
            generation.unwrap_or(0),
        ];
        match WEB_SEEK[i].ask(args) {
            Some(DONE) => SeekOutcome::Applied,
            Some(NO_TRACK) => SeekOutcome::NotSeekable,
            Some(HELD) => SeekOutcome::Held,
            Some(CHANGED) => SeekOutcome::TrackChanged,
            _ => SeekOutcome::Busy,
        }
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
        // The engine decides, as for a LOAD on the controller: LOAD LOCK, a load in flight
        // from any source, the media. `Loaded` means it took the load; a track that then
        // fails to open leaves the deck empty in the status.
        match WEB_LOAD[deck_index(deck)].ask([track, revision, 0]) {
            Some(DONE) => LoadOutcome::Loaded,
            Some(LOCKED) => LoadOutcome::Locked,
            Some(CHANGED) => LoadOutcome::LibraryChanged,
            _ => LoadOutcome::Busy,
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

/// The mixer as the page shows it; neutral until the USB task published its first view.
fn mixer_status() -> MixerStatus {
    let view = MixView::load().unwrap_or(MixView {
        volume: [0; 2],
        crossfader: 8192,
        eq: [[8192; 3]; 2],
        filter: [8192; 2],
        ..MixView::default()
    });
    MixerStatus {
        volume: view.volume,
        crossfader: view.crossfader,
        master_volume: view.master_volume,
        headphone_mix: view.headphone_mix,
        eq: view.eq,
        filter: view.filter,
        smart_cfx: view.smart_cfx,
        smart_fader: view.smart_fader,
        pfl: view.pfl,
    }
}

/// The USB task's side, once a turn before the mix ticks: the Web Remote's taps, controls,
/// loads and seeks, then the loads in flight and the mix for its status.
pub(crate) fn service(mix: &mut crate::deck_mix::Mix) {
    const DECKS: [DeckId; 2] = [DeckId::One, DeckId::Two];
    while let Some((deck, control)) = deck_view::WEB.pop() {
        crate::screen_idle::remote();
        let fx = mix.engine().fx_view();
        for event in crate::controller_midi::touch_events(deck, control, fx)
            .into_iter()
            .flatten()
        {
            mix.control(&event);
        }
    }
    while let Some(event) = deck_view::WEB_EVENTS.pop() {
        crate::screen_idle::remote();
        // LOOP CLEAR arrives as the FLX4's Shift + 4 BEAT (`ReloopStop`), which brings the
        // last loop back when none plays. From the page it only clears: without a loop
        // playing there is nothing to clear, so it goes no further.
        if let (
            Some(deck),
            ControlValue::DeckExtAction(ruzina_controller_core::DeckExtActionValue {
                action: ruzina_controller_core::DeckExtAction::ReloopStop,
                ..
            }),
        ) = (event.deck, event.value)
            && !mix.engine().loop_active(deck)
        {
            continue;
        }
        mix.web_control(&event);
    }
    use crate::deck_engine::{LoadRefused, SeekRefused};
    for (i, deck) in DECKS.into_iter().enumerate() {
        if let Some((ticket, [track, revision, _])) = WEB_LOAD[i].take() {
            crate::screen_idle::remote();
            let answer = if revision != deck_view::web_revision() {
                CHANGED
            } else {
                match mix.engine().try_load(deck, track - 1) {
                    Ok(()) => DONE,
                    Err(LoadRefused::Locked) => LOCKED,
                    Err(LoadRefused::Busy) => BUSY,
                    Err(LoadRefused::NoMedia) => CHANGED,
                }
            };
            WEB_LOAD[i].answer(ticket, answer);
        }
        if let Some((ticket, [position_ms, sent, generation])) = WEB_SEEK[i].take() {
            crate::screen_idle::remote();
            let answer = if sent != 0 && generation != mix.engine().load_generation(deck) {
                CHANGED
            } else {
                match mix.engine().web_seek(deck, position_ms) {
                    Ok(()) => DONE,
                    Err(SeekRefused::NoTrack) => NO_TRACK,
                    Err(SeekRefused::Held) => HELD,
                }
            };
            WEB_SEEK[i].answer(ticket, answer);
        }
        LOAD_GENERATIONS[i].store(mix.engine().load_generation(deck), Ordering::Release);
    }
    let mixer = *mix.mixer();
    let sync = DECKS.map(|deck| mix.engine().sync_flags(deck));
    let channel = |deck| mixer.deck(deck);
    MixView {
        volume: DECKS.map(|deck| channel(deck).channel_volume),
        crossfader: mixer.crossfader(),
        master_volume: mixer.master_volume(),
        headphone_mix: mixer.headphone_mix(),
        eq: DECKS.map(|deck| {
            let c = channel(deck);
            [c.eq_low, c.eq_mid, c.eq_high]
        }),
        filter: DECKS.map(|deck| channel(deck).filter),
        pfl: DECKS.map(|deck| channel(deck).pfl_enabled),
        smart_cfx: mixer.smart_cfx_enabled(),
        smart_fader: mixer.smart_fader_enabled(),
        sync: sync.map(|(on, _)| on),
        sync_master: sync.map(|(_, master)| master),
    }
    .store();
}

/// Faders and knobs a page can set, which the controller's then takes over softly.
const TAKEN_OVER: [SemanticControl; 9] = [
    SemanticControl::ChannelVolume,
    SemanticControl::Crossfader,
    SemanticControl::Tempo,
    SemanticControl::EqLow,
    SemanticControl::EqMid,
    SemanticControl::EqHigh,
    SemanticControl::Filter,
    SemanticControl::MasterVolume,
    SemanticControl::HeadphoneMix,
];

/// How near the controller's fader must come to the page's value to take it back:
/// 2 % of the travel.
const PICKUP_TOLERANCE: u16 = 328;

/// A fader seen from either side: where the controller's own sits, and the page's value
/// while it holds the fader.
#[derive(Clone, Copy)]
struct Fader {
    deck: Option<DeckId>,
    control: SemanticControl,
    /// The controller's last value, held or not, so the first move after a hold can be
    /// seen to pass the page's value.
    physical: Option<u16>,
    /// The page's value, until the controller's fader picks it up.
    target: Option<u16>,
}

/// Soft takeover after the Web Remote moved a fader. The controller's fader still sits
/// where it was; its events count again once it comes within [`PICKUP_TOLERANCE`] of
/// the page's value, or passes it, so the level never jumps. USB task only (in `Mix`).
pub(crate) struct Takeover {
    faders: [Option<Fader>; 16],
}

/// A fader event's value on the shared 14-bit scale, if it is one this takes over.
fn fader(event: &ControlEvent) -> Option<u16> {
    let ControlValue::Absolute { value, max } = event.value else {
        return None;
    };
    if max == 0 || !TAKEN_OVER.contains(&event.control) {
        return None;
    }
    Some(
        (u32::from(value.min(max)) * u32::from(ruzina_web::remote::REMOTE_ABSOLUTE_MAX)
            / u32::from(max)) as u16,
    )
}

impl Takeover {
    pub(crate) const fn new() -> Self {
        Self { faders: [None; 16] }
    }

    /// This event's fader, made on first sight; `None` only if every slot is taken.
    fn entry(&mut self, event: &ControlEvent) -> Option<&mut Fader> {
        let index = self
            .faders
            .iter()
            .position(|f| f.is_some_and(|f| f.deck == event.deck && f.control == event.control))
            .or_else(|| self.faders.iter().position(Option::is_none))?;
        Some(self.faders[index].get_or_insert(Fader {
            deck: event.deck,
            control: event.control,
            physical: None,
            target: None,
        }))
    }

    /// The page set a fader: the controller's must reach this value again. Where the
    /// controller's fader sits is kept.
    pub(crate) fn claim(&mut self, event: &ControlEvent) {
        if let Some(target) = fader(event)
            && let Some(entry) = self.entry(event)
        {
            entry.target = Some(target);
        }
    }

    /// Whether a controller event counts: anything but a held fader, and a held one once
    /// it reaches or passes the page's value, which ends the hold.
    pub(crate) fn admit(&mut self, event: &ControlEvent) -> bool {
        let Some(value) = fader(event) else {
            return true;
        };
        let Some(entry) = self.entry(event) else {
            return true;
        };
        let last = entry.physical.replace(value);
        let Some(target) = entry.target else {
            return true;
        };
        let near = value.abs_diff(target) <= PICKUP_TOLERANCE;
        let passed = last.is_some_and(|last| (last <= target) != (value <= target));
        if near || passed {
            entry.target = None;
            return true;
        }
        false
    }
}

/// The deck's name for its page: its access point's, set once before the server starts.
static NAME: [core::sync::atomic::AtomicU8; 32] =
    [const { core::sync::atomic::AtomicU8::new(0) }; 32];
static NAME_LEN: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Sets the deck's name for its page, up to 32 bytes. Before the web server starts.
///
/// # Safety
/// `name` must be valid for `len` reads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_web_set_name(name: *const u8, len: usize) {
    if name.is_null() {
        return;
    }
    let len = len.min(NAME.len());
    // SAFETY: the caller lends `len` readable bytes for this call.
    let name = unsafe { core::slice::from_raw_parts(name, len) };
    for (slot, &byte) in NAME.iter().zip(name) {
        slot.store(byte, Ordering::Relaxed);
    }
    NAME_LEN.store(len, Ordering::Release);
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
