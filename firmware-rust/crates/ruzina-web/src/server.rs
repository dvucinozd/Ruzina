//! One HTTP request to one response over a product [`WebBackend`]
//! (M2.5 `web_server.c`). The order is M2.5's: parse the head, route, then
//! `api_request_allowed` (Host allow-list; mutations also need the control
//! marker and a same-origin `Origin`), then the handler. Management bodies
//! have bounded single-flight leases; rejected bodies are never drained.
//! Implemented here: the Web Remote page
//! ([`crate::assets`]), `GET /api/status`, the paged `GET /api/library`,
//! `POST /api/control`, `POST /api/load` and management backend dispatch.
//! Recording routes remain 501; absent management adapters fail explicitly.

use crate::{
    HttpStatus, assets, guard,
    http::{self, Request},
    json::{self, DeckStatus, LibraryRow, MixerStatus, StatusSnapshot},
    ordering::{LatestValueGate, Ordering},
    parse,
    remote::{self, RemoteCommand},
    response::{self, Response},
    router::{self, Route, RouteMiss},
    status::StatusSequencer,
};
use ruzina_controller_core::{ControlEvent, DeckId};
use ruzina_media_catalog::CatalogPublicationId;

/// M2.5 catch-all target (`WEB_API_CANONICAL_HOSTNAME`).
pub const CAPTIVE_PORTAL: &str = "http://ruzina.local/index.html";
/// Room kept in front of a generated body for its head.
const HEAD_RESERVE: usize = 256;

/// What a status response reports; the server stamps it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusParts<'a> {
    pub catalog: Option<CatalogPublicationId>,
    pub load_generations: [u32; 2],
    pub decks: [DeckStatus<'a>; 2],
    pub mixer: MixerStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlOutcome {
    Applied,
    /// M2.5 `503 Control queue busy`.
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekOutcome {
    Applied,
    /// No loaded, healthy track (M2.5 `Deck not seekable`).
    NotSeekable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadOutcome {
    Loaded,
    /// The page listed the track from another catalog revision.
    LibraryChanged,
    NotFound,
    /// RP-04 LOAD LOCK refused the deck.
    Locked,
    /// Another load is in flight.
    Busy,
}

/// The product side: everything enters the same paths a controller and the
/// touch UI use (RP-04 admission, RP-06 load, RP-03 CUE ownership).
pub trait WebBackend {
    fn management(&mut self, _route: Route, _out: &mut [u8]) -> Result<usize, HttpStatus> {
        Err(HttpStatus::NotImplemented)
    }
    fn upload_begin(
        &mut self,
        _kind: crate::upload::UploadKind,
        _length: usize,
    ) -> Result<(), HttpStatus> {
        Err(HttpStatus::NotImplemented)
    }
    fn upload_write(&mut self, _bytes: &[u8]) -> Result<(), HttpStatus> {
        Err(HttpStatus::NotImplemented)
    }
    fn upload_finish(&mut self) -> Result<(), HttpStatus> {
        Err(HttpStatus::NotImplemented)
    }
    fn upload_cancel(&mut self) {}
    fn status(&mut self) -> StatusParts<'_>;
    /// Events in order; a tap is press then release.
    fn control(&mut self, events: &[ControlEvent]) -> ControlOutcome;
    /// `position_ms` is already clamped where the length is known.
    fn seek(&mut self, deck: DeckId, position_ms: u32) -> SeekOutcome;
    /// `track` is the export track id listed from catalog `revision`
    /// (see [`crate::status::resolve_load`]).
    fn load(&mut self, deck: DeckId, track: u32, revision: u32) -> LoadOutcome;
    /// Known length of the loaded track, for seek clamping.
    fn duration_ms(&self, deck: DeckId) -> Option<u32>;
    /// The listed catalog's revision and row count, or `None` while there is
    /// none (M2.5 `503 Library snapshot unavailable`).
    fn library(&self) -> Option<(u32, usize)>;
    /// Row `index` of that same catalog.
    fn library_row(&self, index: usize) -> Option<LibraryRow<'_>>;
}

pub struct WebServer {
    ap_ipv4: &'static str,
    sequencer: StatusSequencer,
    ordering: LatestValueGate,
    active_upload: Option<crate::upload::Active>,
    next_upload: u64,
}

impl WebServer {
    /// `boot` should differ on every start, so pages notice a reboot.
    pub const fn new(boot: u32, ap_ipv4: &'static str) -> Self {
        Self {
            ap_ipv4,
            sequencer: StatusSequencer::new(boot),
            ordering: LatestValueGate::new(),
            active_upload: None,
            next_upload: 0,
        }
    }

    /// Admit a body before reading it. None means the ordinary handler owns it.
    pub fn begin_body<B: WebBackend>(
        &mut self,
        head: &[u8],
        backend: &mut B,
    ) -> Result<Option<crate::upload::UploadLease>, HttpStatus> {
        let request = http::parse_request(head).map_err(|e| e.status())?;
        let Some(kind) = crate::upload::kind(&request)? else {
            return Ok(None);
        };
        guard::admit(&request.guard_head(), self.ap_ipv4).map_err(|e| e.status())?;
        if self.active_upload.is_some() {
            return Err(HttpStatus::ServiceUnavailable);
        }
        backend.upload_begin(kind, request.content_length)?;
        self.next_upload = self.next_upload.wrapping_add(1);
        let lease = crate::upload::UploadLease(self.next_upload);
        self.active_upload = Some(crate::upload::Active {
            lease,
            remaining: request.content_length,
        });
        Ok(Some(lease))
    }

    pub fn body_chunk<B: WebBackend>(
        &mut self,
        lease: crate::upload::UploadLease,
        bytes: &[u8],
        backend: &mut B,
    ) -> Result<bool, HttpStatus> {
        let active = self
            .active_upload
            .as_mut()
            .filter(|a| a.lease == lease)
            .ok_or(HttpStatus::Conflict)?;
        if bytes.len() > 4096 || bytes.len() > active.remaining {
            self.cancel_body(lease, backend);
            return Err(HttpStatus::BadRequest);
        }
        if let Err(error) = backend.upload_write(bytes) {
            self.cancel_body(lease, backend);
            return Err(error);
        }
        let active = self.active_upload.as_mut().expect("current upload");
        active.remaining -= bytes.len();
        Ok(active.remaining == 0)
    }

    pub fn finish_body<B: WebBackend>(
        &mut self,
        lease: crate::upload::UploadLease,
        backend: &mut B,
    ) -> Result<(), HttpStatus> {
        let active = self
            .active_upload
            .as_ref()
            .filter(|a| a.lease == lease)
            .ok_or(HttpStatus::Conflict)?;
        if active.remaining != 0 {
            self.cancel_body(lease, backend);
            return Err(HttpStatus::BadRequest);
        }
        self.active_upload = None;
        let result = backend.upload_finish();
        if result.is_err() {
            backend.upload_cancel();
        }
        result
    }

    pub fn cancel_body<B: WebBackend>(
        &mut self,
        lease: crate::upload::UploadLease,
        backend: &mut B,
    ) {
        if self
            .active_upload
            .as_ref()
            .is_some_and(|a| a.lease == lease)
        {
            self.active_upload = None;
            backend.upload_cancel();
        }
    }

    /// Handles one request (its head and any body read so far) and writes
    /// the complete response to `out`, returning its length. An `out` too
    /// small for even an error response yields 0: close without answering.
    pub fn handle<B: WebBackend>(
        &mut self,
        request: &[u8],
        backend: &mut B,
        out: &mut [u8],
    ) -> usize {
        match self.respond(request, backend, out) {
            Ok(len) => len,
            Err(response::Overflow) => {
                let error = Response::text(HttpStatus::InternalServerError, "Response too large");
                response::write_response(&error, out).unwrap_or(0)
            }
        }
    }

    fn respond<B: WebBackend>(
        &mut self,
        bytes: &[u8],
        backend: &mut B,
        out: &mut [u8],
    ) -> Result<usize, response::Overflow> {
        let text = |status: HttpStatus, body: &'static str, out: &mut [u8]| {
            response::write_response(&Response::text(status, body), out)
        };
        let request = match http::parse_request(bytes) {
            Ok(request) => request,
            Err(error) => return text(error.status(), "Bad request", out),
        };
        let route = match router::route(request.method, request.path) {
            Ok(route) => route,
            Err(RouteMiss::CaptivePortal) => {
                return response::write_response(&Response::redirect(CAPTIVE_PORTAL), out);
            }
            Err(RouteMiss::MethodNotAllowed) => {
                return text(HttpStatus::MethodNotAllowed, "Method not allowed", out);
            }
            Err(RouteMiss::NotFound) => return text(HttpStatus::NotFound, "Not found", out),
        };
        if request.path.starts_with("/api/")
            && let Err(refusal) = guard::admit(&request.guard_head(), self.ap_ipv4)
        {
            let body = match refusal {
                guard::Refusal::InvalidHost => "Invalid host",
                guard::Refusal::MissingControlMarker => "Missing X-DDJ-Control",
                guard::Refusal::CrossOrigin => "Cross-origin request refused",
            };
            return text(refusal.status(), body, out);
        }
        match route {
            Route::OtaConfigSet | Route::ControllerProfileUpload | Route::OtaUpload => {
                let status = match self.begin_body(bytes, backend) {
                    Ok(Some(lease)) => {
                        let body = &bytes[request.head_len..];
                        let result = if body.len() != request.content_length {
                            Err(HttpStatus::BadRequest)
                        } else {
                            body.chunks(4096)
                                .try_for_each(|chunk| {
                                    self.body_chunk(lease, chunk, backend).map(|_| ())
                                })
                                .and_then(|_| self.finish_body(lease, backend))
                        };
                        self.cancel_body(lease, backend);
                        result.err().unwrap_or(HttpStatus::Ok)
                    }
                    Err(status) => status,
                    Ok(None) => HttpStatus::NotImplemented,
                };
                text(status, status.reason(), out)
            }
            Route::OtaConfigGet
            | Route::ControllerProfiles
            | Route::Firmware
            | Route::Resources
            | Route::DiagnosticLog => {
                if request.content_length != 0 || !request.query.is_empty() {
                    return text(HttpStatus::BadRequest, "Bad request", out);
                }
                let Some(body) = out.get_mut(HEAD_RESERVE..) else {
                    return Err(response::Overflow);
                };
                match backend.management(route, body) {
                    Ok(len) if len <= body.len() => {
                        out.copy_within(HEAD_RESERVE..HEAD_RESERVE + len, 0);
                        // Move the body after an exactly sized response head.
                        let head = response::write_head(
                            &Response::json(&[]),
                            len,
                            &mut [0u8; HEAD_RESERVE],
                        )?;
                        if head + len > out.len() {
                            return Err(response::Overflow);
                        }
                        out.copy_within(0..len, head);
                        response::write_head(&Response::json(&[]), len, out)?;
                        Ok(head + len)
                    }
                    Ok(_) => Err(response::Overflow),
                    Err(status) => text(status, status.reason(), out),
                }
            }
            Route::Index
            | Route::Style
            | Route::App
            | Route::Status
            | Route::Library
            | Route::Control
            | Route::Load
                if !parse::body_length_allowed(parse::BodyKind::Empty, request.content_length) =>
            {
                text(HttpStatus::PayloadTooLarge, "No body expected", out)
            }
            Route::Index => asset(assets::INDEX, out),
            Route::Style => asset(assets::STYLE, out),
            Route::App => asset(assets::APP, out),
            Route::Status => self.status(backend, out),
            Route::Library => library(&request, backend, out),
            Route::Control => {
                let (status, body) = self.control(&request, backend);
                text(status, body, out)
            }
            Route::Load => {
                let (status, body) = load(&request, backend);
                text(status, body, out)
            }
            _ => text(HttpStatus::NotImplemented, "Not implemented yet", out),
        }
    }

    fn status<B: WebBackend>(
        &mut self,
        backend: &mut B,
        out: &mut [u8],
    ) -> Result<usize, response::Overflow> {
        let parts = backend.status();
        let snapshot = StatusSnapshot {
            stamp: self.sequencer.stamp(parts.catalog, parts.load_generations),
            decks: parts.decks,
            mixer: parts.mixer,
        };
        json_response(out, |body| json::write_status(&snapshot, body))
    }

    fn control<B: WebBackend>(
        &mut self,
        request: &Request<'_>,
        backend: &mut B,
    ) -> (HttpStatus, &'static str) {
        let command = match remote::parse_remote(request.query) {
            Ok(command) => command,
            Err(remote::RemoteError::UnknownAction) => {
                return (HttpStatus::BadRequest, "Unknown action");
            }
            Err(remote::RemoteError::Deck) => {
                return (HttpStatus::BadRequest, "Deck must be 1 or 2");
            }
            Err(remote::RemoteError::Value) => {
                return (HttpStatus::BadRequest, "Control value must be 0..16383");
            }
            Err(remote::RemoteError::Track) => {
                return (HttpStatus::BadRequest, "Invalid track identity or deck");
            }
            Err(remote::RemoteError::SecretInQuery) => {
                return (
                    HttpStatus::BadRequest,
                    "Credentials do not belong in a query",
                );
            }
        };
        match command {
            RemoteCommand::Controls { events, ordering } => {
                // A page that numbers its values gets latest-value ordering.
                let order = (
                    ordering,
                    parse::query_value(request.query, "client").and_then(parse::parse_u32),
                    parse::query_value(request.query, "seq").and_then(parse::parse_u32),
                );
                if let (Some(control), Some(client), Some(sequence)) = order
                    && self.ordering.classify(client, control, sequence) == Ordering::Superseded
                {
                    return (HttpStatus::Ok, "OK");
                }
                let outcome = match events {
                    [Some(press), Some(release)] => backend.control(&[press, release]),
                    [Some(event), None] => backend.control(&[event]),
                    _ => return (HttpStatus::BadRequest, "Unknown action"),
                };
                match outcome {
                    ControlOutcome::Applied => {
                        if let (Some(control), Some(client), Some(sequence)) = order {
                            self.ordering.commit_applied(client, control, sequence);
                        }
                        (HttpStatus::Ok, "OK")
                    }
                    ControlOutcome::Busy => (HttpStatus::ServiceUnavailable, "Control queue busy"),
                }
            }
            RemoteCommand::Seek { deck, position_ms } => {
                let position_ms = remote::clamp_seek_ms(position_ms, backend.duration_ms(deck));
                match backend.seek(deck, position_ms) {
                    SeekOutcome::Applied => (HttpStatus::Ok, "OK"),
                    SeekOutcome::NotSeekable => (HttpStatus::BadRequest, "Deck not seekable"),
                }
            }
            RemoteCommand::Load {
                deck,
                track,
                revision,
            } => load_outcome(backend.load(deck, track, revision)),
        }
    }
}

/// A generated JSON body, written after room for its head and moved up.
fn json_response(
    out: &mut [u8],
    write: impl FnOnce(&mut [u8]) -> Result<usize, json::Overflow>,
) -> Result<usize, response::Overflow> {
    let body = out.get_mut(HEAD_RESERVE..).ok_or(response::Overflow)?;
    let body_len = write(body).map_err(|_| response::Overflow)?;
    let mut head = [0u8; HEAD_RESERVE];
    let head_len = response::write_head(&Response::json(b""), body_len, &mut head)?;
    out.copy_within(HEAD_RESERVE..HEAD_RESERVE + body_len, head_len);
    out[..head_len].copy_from_slice(&head[..head_len]);
    Ok(head_len + body_len)
}

fn asset(asset: assets::Asset, out: &mut [u8]) -> Result<usize, response::Overflow> {
    let response = Response {
        status: HttpStatus::Ok,
        content_type: asset.content_type,
        body: asset.body,
        location: None,
    };
    response::write_response(&response, out)
}

/// `GET /api/library?offset=N`: one page of one catalog revision.
fn library<B: WebBackend>(
    request: &Request<'_>,
    backend: &mut B,
    out: &mut [u8],
) -> Result<usize, response::Overflow> {
    let offset = match parse::query_value(request.query, "offset") {
        None if !request.query.contains("offset") => 0,
        value => match value.and_then(parse::parse_u32) {
            Some(offset) => offset as usize,
            None => {
                let refusal = Response::text(HttpStatus::BadRequest, "Offset must be a number");
                return response::write_response(&refusal, out);
            }
        },
    };
    let Some((revision, total)) = backend.library() else {
        let refusal = Response::text(
            HttpStatus::ServiceUnavailable,
            "Library snapshot unavailable",
        );
        return response::write_response(&refusal, out);
    };
    let rows = (offset..total).map_while(|index| backend.library_row(index));
    json_response(out, |body| {
        json::write_library_page(revision, total, offset, rows, body)
    })
}

/// `POST /api/load?track_key=&generation=&deck=` (M2.5 names).
fn load<B: WebBackend>(request: &Request<'_>, backend: &mut B) -> (HttpStatus, &'static str) {
    let value = |key| parse::query_value(request.query, key).and_then(parse::parse_u32);
    let deck = match value("deck") {
        Some(1) => Some(DeckId::One),
        Some(2) => Some(DeckId::Two),
        _ => None,
    };
    match (
        value("track_key").filter(|key| *key != 0),
        value("generation"),
        deck,
    ) {
        (Some(track), Some(revision), Some(deck)) => {
            load_outcome(backend.load(deck, track, revision))
        }
        _ => (HttpStatus::BadRequest, "Invalid track identity or deck"),
    }
}

const fn load_outcome(outcome: LoadOutcome) -> (HttpStatus, &'static str) {
    match outcome {
        LoadOutcome::Loaded => (HttpStatus::Ok, "OK"),
        LoadOutcome::LibraryChanged => (HttpStatus::Conflict, "Library generation changed"),
        LoadOutcome::NotFound => (HttpStatus::NotFound, "Track not found"),
        LoadOutcome::Locked => (HttpStatus::Conflict, "Deck is playing (load lock)"),
        LoadOutcome::Busy => (HttpStatus::Conflict, "Library changed or load busy"),
    }
}
