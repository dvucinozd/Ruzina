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
//!
//! A firmware bundle (`POST /api/ota/p4`) is too long for one request buffer: the C side
//! streams its body through [`ruzina_web_stream_open`], [`ruzina_web_stream_feed`] and
//! [`ruzina_web_stream_finish`] into [`crate::deck_ota`].
use crate::deck_view::{self, MixView, TouchControl, WebLibrary, WebTrack};
use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_media_catalog::CatalogPublicationId;
use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
use ruzina_web::HttpStatus;
use ruzina_web::json::{DeckStateText, DeckStatus, LibraryRow, MixerStatus};
use ruzina_web::server::{
    ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend, WebServer,
};
use ruzina_web::settings::{
    ControllerProfile, Jog, Media, PasswordOutcome, SettingChange, SettingsInfo, SettingsView,
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
    /// The firmware upload in progress, if any.
    upload: &'a mut Option<crate::deck_ota::Upload>,
    /// The SETUP page's texts made for one answer.
    built: alloc::string::String,
    controller: alloc::string::String,
    profiles: alloc::vec::Vec<ControllerProfile<'static>>,
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

/// The running firmware's version, build date and build time (the deck screen reads
/// IDF's app description).
#[cfg(not(test))]
fn firmware_texts() -> (&'static str, &'static str, &'static str) {
    crate::slint_ui::firmware_texts()
}
#[cfg(test)]
fn firmware_texts() -> (&'static str, &'static str, &'static str) {
    ("test", "", "")
}

/// Microseconds since boot (IDF's monotonic clock).
#[cfg(not(test))]
fn now_us() -> i64 {
    unsafe extern "C" {
        fn esp_timer_get_time() -> i64;
    }
    // SAFETY: IDF's monotonic clock, callable from any task.
    unsafe { esp_timer_get_time() }
}
#[cfg(test)]
fn now_us() -> i64 {
    0
}

/// A setting from the SETUP page, as the deck's Settings page would set it.
fn edit_setting(settings: &mut ruzina_settings::ProductSettings, change: SettingChange) {
    use ruzina_settings::{JogMode, MasterTrim, Screensaver};
    match change {
        SettingChange::LoadLock(on) => settings.load_lock = on,
        SettingChange::Jog(deck, jog) => {
            settings.jog_modes[deck_index(deck)] = match jog {
                Jog::Vinyl => JogMode::Vinyl,
                Jog::Cdj => JogMode::Cdj,
            }
        }
        SettingChange::Brightness(percent) => {
            settings.backlight_percent = percent.clamp(
                ruzina_settings::BACKLIGHT_MIN_PERCENT,
                crate::deck_settings::BACKLIGHT_MAX_PERCENT,
            )
        }
        SettingChange::TrimDb(db) => {
            settings.master_trim = match db {
                3 => MasterTrim::Minus3Db,
                6 => MasterTrim::Minus6Db,
                _ => MasterTrim::ZeroDb,
            }
        }
        SettingChange::ScreensaverMin(minutes) => {
            settings.screensaver = match minutes {
                0 => Screensaver::Off,
                5 => Screensaver::Minutes5,
                10 => Screensaver::Minutes10,
                _ => Screensaver::Minutes2,
            }
        }
    }
}

impl WebBackend for Backend<'_> {
    fn settings(&mut self) -> Option<SettingsView<'_>> {
        use ruzina_settings::{JogMode, MasterTrim, Screensaver};
        let settings = crate::deck_settings::current();
        let (firmware, date, time) = firmware_texts();
        self.built = alloc::format!("{date} {time}");
        self.controller = match crate::controller_midi::attached() {
            None => alloc::string::String::new(),
            Some((_, _, Some(name))) => name.into(),
            Some((vid, pid, None)) => alloc::format!("{vid:04X}:{pid:04X}"),
        };
        let media = if crate::media_signal::muted() {
            Media::Removed
        } else if self.state.library.is_none() {
            Media::Reading
        } else {
            Media::Ready
        };
        let uptime_s = (now_us() / 1_000_000).clamp(0, i64::from(u32::MAX));
        self.profiles = crate::controller_midi::profiles()
            .map(|(name, vid, pid, tested, active)| ControllerProfile {
                name,
                vid,
                pid,
                tested,
                active,
            })
            .collect();
        let jog = |deck: usize| match settings.jog_modes[deck] {
            JogMode::Cdj => Jog::Cdj,
            JogMode::Vinyl => Jog::Vinyl,
        };
        Some(SettingsView {
            load_lock: settings.load_lock,
            jog: [jog(0), jog(1)],
            brightness_percent: settings.backlight_percent,
            brightness_min: ruzina_settings::BACKLIGHT_MIN_PERCENT,
            brightness_max: crate::deck_settings::BACKLIGHT_MAX_PERCENT,
            brightness_step: crate::deck_settings::BACKLIGHT_STEP_PERCENT,
            trim_db: match settings.master_trim {
                MasterTrim::ZeroDb => 0,
                MasterTrim::Minus3Db => 3,
                MasterTrim::Minus6Db => 6,
            },
            screensaver_min: match settings.screensaver {
                Screensaver::Off => 0,
                Screensaver::Minutes2 => 2,
                Screensaver::Minutes5 => 5,
                Screensaver::Minutes10 => 10,
            },
            info: SettingsInfo {
                network: self.state.name.as_str(),
                firmware,
                built: &self.built,
                controller: &self.controller,
                tracks: deck_view::library_len(),
                media,
                uptime_s: uptime_s as u32,
                slot: crate::deck_ota::running_slot(),
            },
            profiles: &self.profiles,
        })
    }

    fn upload_begin(
        &mut self,
        kind: ruzina_web::upload::UploadKind,
        length: usize,
    ) -> Result<(), HttpStatus> {
        if !matches!(kind, ruzina_web::upload::UploadKind::Ota) {
            return Err(HttpStatus::NotImplemented);
        }
        if let Some(mut stale) = self.upload.take() {
            stale.cancel();
        }
        *self.upload = Some(crate::deck_ota::Upload::new(length).map_err(push_status)?);
        Ok(())
    }

    fn upload_write(&mut self, bytes: &[u8]) -> Result<(), HttpStatus> {
        self.upload
            .as_mut()
            .ok_or(HttpStatus::Conflict)?
            .feed(bytes)
            .map_err(push_status)
    }

    fn upload_finish(&mut self) -> Result<(), HttpStatus> {
        let mut upload = self.upload.take().ok_or(HttpStatus::Conflict)?;
        upload.finish().map_err(push_status)
    }

    fn upload_cancel(&mut self) {
        if let Some(mut upload) = self.upload.take() {
            upload.cancel();
        }
    }

    /// Through the same store as the deck's Settings page; the screen's brightness and
    /// that page follow on the screen's next frame, and the save waits as theirs does.
    fn change_setting(&mut self, change: SettingChange) -> bool {
        let now_ms = (now_us() / 1_000) as u32;
        crate::deck_settings::change(now_ms, |settings| edit_setting(settings, change));
        true
    }

    /// Handed to the app main task, which saves it and restarts the network with it. Not
    /// while a deck plays: the save writes flash, which stalls the audio.
    fn change_network_password(&mut self, password: &str) -> PasswordOutcome {
        if !crate::deck_network::valid(password) {
            return PasswordOutcome::Invalid;
        }
        if crate::media_signal::playing()
            .into_iter()
            .any(|playing| playing)
            || !crate::deck_network::put(Some(password))
        {
            return PasswordOutcome::Busy;
        }
        PasswordOutcome::Accepted
    }

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
                tempo_x100: (i64::from(live.bpm_x100) * (10_000 + i64::from(pitch)) / 10_000)
                    .clamp(0, i64::from(u32::MAX)) as u32,
                level: deck_view::meter_segments(deck_view::WEB_PEAKS.take(i)) as u8,
                hot_cues: view.hot_cues[i],
                track_key: track.map_or(0, |track| track.index + 1),
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

    fn wave(&self, deck: DeckId) -> Option<(u32, &[u8])> {
        self.state
            .track(deck_index(deck))
            .filter(|track| !track.wave.is_empty())
            .map(|track| (track.index + 1, track.wave.as_slice()))
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
        hot_cues: mix.engine().hot_cue_masks(),
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

/// What the web task keeps: the server, its state between requests, and a streamed
/// firmware upload's lease and progress.
struct Web {
    server: WebServer,
    state: WebState,
    lease: Option<ruzina_web::upload::UploadLease>,
    upload: Option<crate::deck_ota::Upload>,
}

struct Server(UnsafeCell<Option<Web>>);

// SAFETY: only the web task calls `ruzina_web_handle` and the stream functions, one
// request at a time.
unsafe impl Sync for Server {}

static SERVER: Server = Server(UnsafeCell::new(None));

/// The web task's server, made on its first request.
///
/// # Safety
/// Web task only, and no other reference to it may be live.
unsafe fn web(boot: u32) -> &'static mut Web {
    // SAFETY: the caller's.
    unsafe { &mut *SERVER.0.get() }.get_or_insert_with(|| Web {
        server: WebServer::new(boot, AP_IPV4),
        state: WebState::default(),
        lease: None,
        upload: None,
    })
}

/// The HTTP status for an upload `ruzina-ota` refused.
fn push_status(error: crate::deck_ota::PushError) -> HttpStatus {
    use crate::deck_ota::PushError;
    match error {
        // A deck plays: stop both first, as for the network's password.
        PushError::Busy | PushError::Inactive => HttpStatus::Conflict,
        PushError::Flash => HttpStatus::InternalServerError,
        PushError::Length | PushError::Invalid => HttpStatus::BadRequest,
    }
}

/// Starts a streamed body from its request head (`head`, `len` bytes): 0 when a firmware
/// upload takes it, else the HTTP status refusing it.
///
/// # Safety
/// `head` must be valid for `len` reads. Web task only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_web_stream_open(boot: u32, head: *const u8, len: usize) -> i32 {
    if head.is_null() {
        return i32::from(HttpStatus::BadRequest.code());
    }
    // SAFETY: the caller hands over the head for this call; the web task is the only one
    // to hold the server.
    let (head, web) = unsafe { (core::slice::from_raw_parts(head, len), web(boot)) };
    if let Some(lease) = web.lease.take() {
        let mut backend = upload_backend(&web.state, &mut web.upload);
        web.server.cancel_body(lease, &mut backend);
    }
    let mut backend = upload_backend(&web.state, &mut web.upload);
    match web.server.begin_body(head, &mut backend) {
        Ok(Some(lease)) => {
            web.lease = Some(lease);
            0
        }
        // Not an upload: no other request carries a body this long.
        Ok(None) => i32::from(HttpStatus::PayloadTooLarge.code()),
        Err(status) => i32::from(status.code()),
    }
}

/// Hands on `len` bytes of the streamed body: 0, else the HTTP status that ended it.
///
/// # Safety
/// `bytes` must be valid for `len` reads. Web task only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_web_stream_feed(bytes: *const u8, len: usize) -> i32 {
    // SAFETY: as `ruzina_web_stream_open`.
    let web = unsafe { web(0) };
    let Some(lease) = web.lease else {
        return i32::from(HttpStatus::Conflict.code());
    };
    if bytes.is_null() {
        return i32::from(HttpStatus::BadRequest.code());
    }
    // SAFETY: the caller hands over the bytes for this call.
    let bytes = unsafe { core::slice::from_raw_parts(bytes, len) };
    let mut backend = upload_backend(&web.state, &mut web.upload);
    match web.server.body_chunk(lease, bytes, &mut backend) {
        Ok(_) => 0,
        Err(status) => {
            web.lease = None;
            i32::from(status.code())
        }
    }
}

/// The streamed body is all in: finishes the upload (or, with `status` not 0, answers the
/// refusal) and writes the answer to `out`. Returns its length.
///
/// # Safety
/// `out` must be valid for `out_len` writes. Web task only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_web_stream_finish(
    status: i32,
    out: *mut u8,
    out_len: usize,
) -> usize {
    if out.is_null() {
        return 0;
    }
    // SAFETY: as `ruzina_web_stream_open`; the caller hands over `out` for this call.
    let (web, out) = unsafe { (web(0), core::slice::from_raw_parts_mut(out, out_len)) };
    let status = match (status, web.lease.take()) {
        (0, Some(lease)) => {
            let mut backend = upload_backend(&web.state, &mut web.upload);
            match web.server.finish_body(lease, &mut backend) {
                Ok(()) => HttpStatus::Ok,
                Err(status) => status,
            }
        }
        (0, None) => HttpStatus::Conflict,
        (code, lease) => {
            if let Some(lease) = lease {
                let mut backend = upload_backend(&web.state, &mut web.upload);
                web.server.cancel_body(lease, &mut backend);
            }
            u16::try_from(code)
                .ok()
                .and_then(HttpStatus::from_code)
                .unwrap_or(HttpStatus::BadRequest)
        }
    };
    let body = if status == HttpStatus::Ok {
        "Installed; restarting"
    } else {
        status.reason()
    };
    ruzina_web::response::write_response(&ruzina_web::response::Response::text(status, body), out)
        .unwrap_or(0)
}

/// The streamed body ended unfinished (the connection dropped or timed out).
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_web_stream_cancel() {
    // SAFETY: web task only, as the other stream functions.
    let web = unsafe { web(0) };
    if let Some(lease) = web.lease.take() {
        let mut backend = upload_backend(&web.state, &mut web.upload);
        web.server.cancel_body(lease, &mut backend);
    }
}

/// A new image is set to boot: the web task restarts once its answer has gone.
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_web_restart_due() -> bool {
    crate::deck_ota::restart_due()
}

/// A backend for the upload calls only.
fn upload_backend<'a>(
    state: &'a WebState,
    upload: &'a mut Option<crate::deck_ota::Upload>,
) -> Backend<'a> {
    Backend {
        state,
        upload,
        built: alloc::string::String::new(),
        controller: alloc::string::String::new(),
        profiles: alloc::vec::Vec::new(),
    }
}

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
    let web = unsafe { web(boot) };
    web.state.refresh();
    let mut backend = upload_backend(&web.state, &mut web.upload);
    web.server.handle(request, &mut backend, out)
}

/// The largest page asset, for the response buffer the C side allocates.
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_web_largest_asset() -> usize {
    ruzina_web::assets::LARGEST_BYTES
}

#[cfg(test)]
mod tests;
