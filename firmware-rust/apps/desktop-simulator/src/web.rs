//! The simulator as a Web Remote host: the product `WebBackend` over the
//! session (the same control, seek and load paths touch uses) and a small
//! blocking adapter that serves `127.0.0.1` one closed connection at a time
//! from the UI timer. The loopback address plays the AP address, so the
//! M2.5 Host allow-list admits a local browser and nothing else.
use crate::session::{Session, deck, index};
use ruzina_controller_core::{ControlEvent, DeckId};
use ruzina_web::{
    assets,
    json::{DeckStateText, DeckStatus, LibraryRow, MixerStatus},
    server::{ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend, WebServer},
};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    time::Duration,
};

/// The loopback address the simulator listens on and treats as its AP.
pub const WEB_HOST: &str = "127.0.0.1";
/// Request bytes read per connection (head plus any refused body).
const REQUEST_MAX: usize = 4096;
/// Room for the largest page asset and its head.
const RESPONSE_MAX: usize = assets::LARGEST_BYTES + 512;
const READ_TIMEOUT: Duration = Duration::from_millis(250);

fn deck_status(session: &Session, i: usize) -> DeckStatus<'_> {
    let t = session.presentation(i).transport;
    if !t.loaded {
        return DeckStatus::EMPTY;
    }
    DeckStatus {
        title: session.title(i),
        artist: session.artist(i),
        bpm: (t.bpm_milli + 500) / 1000,
        pitch_centipercent: t.pitch_centi_percent.clamp(-10_000, 10_000) as i16,
        position_ms: t.position_ms.min(u64::from(u32::MAX)) as u32,
        duration_ms: t.duration_ms.min(u64::from(u32::MAX)) as u32,
        playing: t.playing,
        sync_enabled: t.sync,
        sync_master: session.decks.deck(deck(i)).sync_master,
        state: if t.playing {
            DeckStateText::Playing
        } else {
            DeckStateText::Ready
        },
    }
}

impl WebBackend for Session {
    fn status(&mut self) -> StatusParts<'_> {
        let channel = |i: usize| self.mixer.deck(deck(i));
        StatusParts {
            catalog: self.catalog_publication(),
            load_generations: [self.load_generation(0), self.load_generation(1)],
            decks: [deck_status(self, 0), deck_status(self, 1)],
            mixer: MixerStatus {
                volume: [channel(0).channel_volume, channel(1).channel_volume],
                crossfader: self.mixer.crossfader(),
                master_volume: self.mixer.master_volume(),
                headphone_mix: self.mixer.headphone_mix(),
                eq: [0, 1].map(|i| {
                    let channel = channel(i);
                    [channel.eq_low, channel.eq_mid, channel.eq_high]
                }),
                filter: [channel(0).filter, channel(1).filter],
                smart_cfx: self.mixer.smart_cfx_enabled(),
                smart_fader: self.mixer.smart_fader_enabled(),
                pfl: [channel(0).pfl_enabled, channel(1).pfl_enabled],
            },
        }
    }

    fn control(&mut self, events: &[ControlEvent]) -> ControlOutcome {
        for event in events {
            Session::control(self, *event);
        }
        ControlOutcome::Applied
    }

    fn seek(&mut self, target: DeckId, position_ms: u32) -> SeekOutcome {
        if self.web_seek(target, position_ms) {
            SeekOutcome::Applied
        } else {
            SeekOutcome::NotSeekable
        }
    }

    fn load(&mut self, target: DeckId, track: u32, revision: u32) -> LoadOutcome {
        self.web_load(target, track, revision)
    }

    fn duration_ms(&self, target: DeckId) -> Option<u32> {
        let t = self.presentation(index(target)).transport;
        (t.loaded && t.duration_ms > 0).then(|| t.duration_ms.min(u64::from(u32::MAX)) as u32)
    }

    fn library(&self) -> Option<(u32, usize)> {
        self.web_catalog()
            .map(|catalog| (catalog.revision(), catalog.len()))
    }

    fn library_row(&self, i: usize) -> Option<LibraryRow<'_>> {
        let track = self.web_catalog()?.get(i)?;
        Some(LibraryRow {
            track_key: track.source_track_id,
            title: track.title.as_str(),
            artist: track.artist.as_str(),
            bpm: ruzina_ui_model::labels::library_bpm(track.bpm_x100),
            duration_ms: track.duration_ms,
        })
    }
}

/// A listening Web Remote on the loopback address.
pub struct WebPort {
    listener: TcpListener,
    server: WebServer,
}

impl WebPort {
    /// `port` 0 picks a free port (tests).
    pub fn bind(port: u16, boot: u32) -> io::Result<Self> {
        let listener = TcpListener::bind((WEB_HOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            server: WebServer::new(boot, WEB_HOST),
        })
    }

    pub fn port(&self) -> io::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// Serves every connection already waiting and returns how many. A
    /// client that stalls costs at most the read timeout.
    pub fn poll(&mut self, session: &mut Session) -> usize {
        let mut served = 0;
        while let Ok((stream, _)) = self.listener.accept() {
            if serve(stream, &mut self.server, session).is_ok() {
                served += 1;
            }
        }
        served
    }
}

fn serve(mut stream: TcpStream, server: &mut WebServer, session: &mut Session) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut request = [0u8; REQUEST_MAX];
    let mut len = 0;
    // Read until the head (and the length it declares) is in, or the buffer
    // is full; the server refuses whatever is incomplete or too large.
    while len < request.len() {
        match stream.read(&mut request[len..]) {
            Ok(0) => break,
            Ok(read) => len += read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => break,
            Err(error) => return Err(error),
        }
        if let Ok(head) = ruzina_web::http::parse_request(&request[..len])
            && len >= head.head_len + head.content_length
        {
            break;
        }
    }
    let mut response = vec![0u8; RESPONSE_MAX];
    let written = server.handle(&request[..len], session, &mut response);
    stream.write_all(&response[..written])?;
    stream.shutdown(Shutdown::Both)
}
