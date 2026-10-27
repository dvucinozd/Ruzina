//! The simulator as a Web Remote host: the product `WebBackend` over the
//! session (the same control, seek and load paths touch uses) and a small
//! bounded nonblocking adapter that serves `127.0.0.1`
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
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

/// The loopback address the simulator listens on and treats as its AP.
pub const WEB_HOST: &str = "127.0.0.1";
/// Request bytes read per connection (head plus any refused body).
const REQUEST_MAX: usize = ruzina_web::http::HEAD_MAX_BYTES;
/// Room for the largest page asset and its head.
const RESPONSE_MAX: usize = assets::LARGEST_BYTES + 512;
const DEADLINE: Duration = Duration::from_secs(2);
const CONNECTIONS: usize = 8;

fn deck_status(session: &Session, i: usize) -> DeckStatus<'_> {
    let t = session.presentation(i).transport;
    let pitch_raw = session.decks.deck(deck(i)).pitch_raw;
    if !t.loaded {
        return DeckStatus {
            pitch_raw,
            ..DeckStatus::EMPTY
        };
    }
    DeckStatus {
        title: session.title(i),
        artist: session.artist(i),
        bpm: (t.bpm_milli + 500) / 1000,
        pitch_centipercent: t.pitch_centi_percent.clamp(-10_000, 10_000) as i16,
        pitch_raw,
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
    fn management(
        &mut self,
        route: ruzina_web::router::Route,
        out: &mut [u8],
    ) -> Result<usize, ruzina_web::HttpStatus> {
        self.management_get(route, out)
    }
    fn upload_begin(
        &mut self,
        kind: ruzina_web::upload::UploadKind,
        len: usize,
    ) -> Result<(), ruzina_web::HttpStatus> {
        self.management_begin(kind, len)
    }
    fn upload_write(&mut self, bytes: &[u8]) -> Result<(), ruzina_web::HttpStatus> {
        self.management_write(bytes)
    }
    fn upload_finish(&mut self) -> Result<(), ruzina_web::HttpStatus> {
        self.management_finish()
    }
    fn upload_cancel(&mut self) {
        self.management_cancel()
    }
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
    connections: [Option<Connection<TcpStream>>; CONNECTIONS],
    cursor: usize,
}

impl WebPort {
    /// `port` 0 picks a free port (tests).
    pub fn bind(port: u16, boot: u32) -> io::Result<Self> {
        let listener = TcpListener::bind((WEB_HOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            server: WebServer::new(boot, WEB_HOST),
            connections: std::array::from_fn(|_| None),
            cursor: 0,
        })
    }

    pub fn port(&self) -> io::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// One bounded tick; returns the number of fully sent responses.
    pub fn poll(&mut self, session: &mut Session) -> usize {
        self.poll_at(Instant::now(), session)
    }

    fn poll_at(&mut self, now: Instant, session: &mut Session) -> usize {
        let mut served = 0;
        for _ in 0..2 {
            let Ok((stream, _)) = self.listener.accept() else {
                break;
            };
            if stream.set_nonblocking(true).is_ok()
                && let Some(slot) = self.connections.iter_mut().find(|slot| slot.is_none())
            {
                *slot = Some(Connection::new(stream, now));
            }
        }
        for step in 0..CONNECTIONS {
            let slot = &mut self.connections[(self.cursor + step) % CONNECTIONS];
            if let Some(connection) = slot {
                match connection.tick(now, &mut self.server, session) {
                    Progress::Pending => {}
                    Progress::Sent => {
                        served += 1;
                        *slot = None;
                    }
                    Progress::Closed => {
                        *slot = None;
                    }
                }
            }
        }
        self.cursor = (self.cursor + 1) % CONNECTIONS;
        served
    }
}

enum Phase {
    Reading {
        bytes: Vec<u8>,
    },
    Body {
        lease: ruzina_web::upload::UploadLease,
    },
    Writing {
        bytes: Vec<u8>,
        offset: usize,
    },
}
enum Progress {
    Pending,
    Sent,
    Closed,
}
struct Connection<S> {
    stream: S,
    phase: Phase,
    started: Instant,
}
impl<S: Read + Write> Connection<S> {
    fn new(stream: S, now: Instant) -> Self {
        Self {
            stream,
            phase: Phase::Reading {
                bytes: Vec::with_capacity(REQUEST_MAX),
            },
            started: now,
        }
    }
    fn tick(&mut self, now: Instant, server: &mut WebServer, session: &mut Session) -> Progress {
        let deadline = if matches!(self.phase, Phase::Body { .. }) {
            Duration::from_secs(30)
        } else {
            DEADLINE
        };
        if now.saturating_duration_since(self.started) >= deadline {
            if let Phase::Body { lease } = self.phase {
                server.cancel_body(lease, session);
            }
            return Progress::Closed;
        }
        match &mut self.phase {
            Phase::Reading { bytes } => {
                let mut chunk = [0; 1024];
                let maximum = chunk.len().min(REQUEST_MAX - bytes.len());
                match self.stream.read(&mut chunk[..maximum]) {
                    Ok(0) => return Progress::Closed,
                    Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Progress::Pending,
                    Err(_) => return Progress::Closed,
                }
                if matches!(
                    ruzina_web::http::parse_request(bytes),
                    Err(ruzina_web::http::HttpError::Incomplete)
                ) {
                    return Progress::Pending;
                }
                if let Ok(head) = ruzina_web::http::parse_request(bytes) {
                    match server.begin_body(bytes, session) {
                        Ok(Some(lease)) => {
                            let result = server.body_chunk(lease, &bytes[head.head_len..], session);
                            match result {
                                Ok(false) => {
                                    self.phase = Phase::Body { lease };
                                    self.started = now;
                                    return Progress::Pending;
                                }
                                Ok(true) => {
                                    let status = server
                                        .finish_body(lease, session)
                                        .err()
                                        .unwrap_or(ruzina_web::HttpStatus::Ok);
                                    self.answer(status, now);
                                    return Progress::Pending;
                                }
                                Err(status) => {
                                    self.answer(status, now);
                                    return Progress::Pending;
                                }
                            }
                        }
                        Err(status) => {
                            self.answer(status, now);
                            return Progress::Pending;
                        }
                        Ok(None) => {}
                    }
                }
                let mut response = vec![0; RESPONSE_MAX];
                let len = server.handle(bytes, session, &mut response);
                response.truncate(len);
                self.phase = Phase::Writing {
                    bytes: response,
                    offset: 0,
                };
                self.started = now;
                Progress::Pending
            }
            Phase::Body { lease } => {
                let lease = *lease;
                let mut chunk = [0; 1024];
                let result = match self.stream.read(&mut chunk) {
                    Ok(0) => {
                        server.cancel_body(lease, session);
                        return Progress::Closed;
                    }
                    Ok(n) => server.body_chunk(lease, &chunk[..n], session),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Progress::Pending,
                    Err(_) => {
                        server.cancel_body(lease, session);
                        return Progress::Closed;
                    }
                };
                match result {
                    Ok(false) => Progress::Pending,
                    Ok(true) => {
                        let status = server
                            .finish_body(lease, session)
                            .err()
                            .unwrap_or(ruzina_web::HttpStatus::Ok);
                        self.answer(status, now);
                        Progress::Pending
                    }
                    Err(status) => {
                        self.answer(status, now);
                        Progress::Pending
                    }
                }
            }
            Phase::Writing { bytes, offset } => {
                let end = bytes.len().min(*offset + 4096);
                match self.stream.write(&bytes[*offset..end]) {
                    Ok(0) => Progress::Closed,
                    Ok(written) => {
                        *offset += written;
                        if *offset == bytes.len() {
                            Progress::Sent
                        } else {
                            Progress::Pending
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => Progress::Pending,
                    Err(_) => Progress::Closed,
                }
            }
        }
    }
    fn answer(&mut self, status: ruzina_web::HttpStatus, now: Instant) {
        let mut bytes = vec![0; 512];
        let n = ruzina_web::response::write_response(
            &ruzina_web::response::Response::text(status, status.reason()),
            &mut bytes,
        )
        .unwrap_or(0);
        bytes.truncate(n);
        self.phase = Phase::Writing { bytes, offset: 0 };
        self.started = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Io {
        input: std::collections::VecDeque<u8>,
        output: Vec<u8>,
        blocked: bool,
        eof: bool,
        reads: usize,
        writes: usize,
    }
    impl Read for Io {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.eof {
                return Ok(0);
            }
            if let Some(byte) = self.input.pop_front() {
                bytes[0] = byte;
                Ok(1)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
    }
    impl Write for Io {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.blocked {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.output.extend_from_slice(&bytes[..1]);
            Ok(1)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn partial_io_sends_exactly_one_response_without_waiting_for_a_body() {
        let request = b"POST /api/control?action=play_pause&deck=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nX-DDJ-Control: 1\r\nContent-Length: 999999\r\n\r\n";
        let now = Instant::now();
        let mut c = Connection::new(
            Io {
                input: request.iter().copied().collect(),
                ..Io::default()
            },
            now,
        );
        let mut server = WebServer::new(1, WEB_HOST);
        let mut s = Session::new();
        for _ in 0..request.len() {
            assert!(matches!(
                c.tick(now, &mut server, &mut s),
                Progress::Pending
            ));
        }
        assert_eq!(c.stream.reads, request.len());
        c.stream.blocked = true;
        assert!(matches!(
            c.tick(now, &mut server, &mut s),
            Progress::Pending
        ));
        c.stream.blocked = false;
        for _ in 0..RESPONSE_MAX {
            if matches!(c.tick(now, &mut server, &mut s), Progress::Sent) {
                break;
            }
        }
        let response = String::from_utf8(c.stream.output.clone()).unwrap();
        assert!(response.starts_with("HTTP/1.1 413"));
        assert_eq!(response.matches("HTTP/1.1").count(), 1);
        assert!(matches!(
            c.tick(now + DEADLINE, &mut server, &mut s),
            Progress::Closed
        ));
    }
    #[test]
    fn slow_trickle_and_blocked_write_have_absolute_deadlines() {
        let now = Instant::now();
        let mut server = WebServer::new(1, WEB_HOST);
        let mut s = Session::new();
        let mut c = Connection::new(Io::default(), now);
        for step in 0..4 {
            c.stream.input.push_back(b'G');
            assert!(matches!(
                c.tick(now + Duration::from_millis(step * 500), &mut server, &mut s),
                Progress::Pending
            ));
        }
        assert!(matches!(
            c.tick(now + DEADLINE, &mut server, &mut s),
            Progress::Closed
        ));
        c.phase = Phase::Writing {
            bytes: vec![1],
            offset: 0,
        };
        c.stream.blocked = true;
        assert!(matches!(
            c.tick(now + DEADLINE, &mut server, &mut s),
            Progress::Closed
        ));
    }

    #[test]
    fn eof_closes_incomplete_head_without_a_response() {
        let now = Instant::now();
        let mut c = Connection::new(
            Io {
                eof: true,
                ..Io::default()
            },
            now,
        );
        assert!(matches!(
            c.tick(now, &mut WebServer::new(1, WEB_HOST), &mut Session::new()),
            Progress::Closed
        ));
        assert!(c.stream.output.is_empty());
    }

    #[test]
    fn partial_management_body_timeout_and_eof_cancel_without_commit() {
        for timeout in [false, true] {
            let now = Instant::now();
            let mut server = WebServer::new(1, WEB_HOST);
            let mut s = Session::new();
            let head=b"POST /api/ota/config HTTP/1.1\r\nHost: 127.0.0.1\r\nX-DDJ-Control: 1\r\nContent-Type: application/json\r\nContent-Length: 14\r\n\r\n";
            let mut c = Connection::new(
                Io {
                    input: head.iter().chain(b"{").copied().collect(),
                    ..Io::default()
                },
                now,
            );
            for _ in 0..head.len() + 1 {
                assert!(matches!(
                    c.tick(now, &mut server, &mut s),
                    Progress::Pending
                ));
            }
            assert!(matches!(c.phase, Phase::Body { .. }));
            c.stream.eof = !timeout;
            let at = if timeout {
                now + Duration::from_secs(30)
            } else {
                now
            };
            assert!(matches!(c.tick(at, &mut server, &mut s), Progress::Closed));
            assert!(!s.settings.network().is_configured());
            assert!(
                server.begin_body(head, &mut s).unwrap().is_some(),
                "cancel releases single-flight"
            );
        }
    }

    #[test]
    fn accepts_and_live_connections_are_bounded_and_expire() {
        let mut port = WebPort::bind(0, 1).unwrap();
        let peers: Vec<_> = (0..10)
            .map(|_| TcpStream::connect((WEB_HOST, port.port().unwrap())).unwrap())
            .collect();
        let now = Instant::now();
        let mut s = Session::new();
        port.poll_at(now, &mut s);
        assert_eq!(port.connections.iter().flatten().count(), 2);
        for _ in 0..4 {
            port.poll_at(now, &mut s);
        }
        assert_eq!(port.connections.iter().flatten().count(), CONNECTIONS);
        port.poll_at(now + DEADLINE, &mut s);
        assert_eq!(port.connections.iter().flatten().count(), 0);
        drop(peers);
    }
}
