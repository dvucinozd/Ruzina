//! Raw HTTP requests through the whole adapter core (parse, route, guard,
//! handler, response) against a recording fake product backend.
use ruzina_controller_core::{ControlEvent, ControlValue, DeckId, SemanticControl};
use ruzina_web::{
    assets,
    json::{DeckStateText, DeckStatus, LibraryRow, MixerStatus},
    server::{
        CAPTIVE_PORTAL, ControlOutcome, LoadOutcome, SeekOutcome, StatusParts, WebBackend,
        WebServer,
    },
};

const AP: &str = "192.168.4.1";

#[derive(Default)]
struct Fake {
    controls: Vec<ControlEvent>,
    seeks: Vec<(DeckId, u32)>,
    loads: Vec<(DeckId, u32, u32)>,
    busy: bool,
    load_outcome: Option<LoadOutcome>,
    duration: Option<u32>,
    /// (revision, rows) of the listed catalog.
    library: Option<(u32, Vec<(u32, String)>)>,
}

impl WebBackend for Fake {
    fn status(&mut self) -> StatusParts<'_> {
        StatusParts {
            catalog: None,
            load_generations: [2, 0],
            decks: [
                DeckStatus {
                    title: "Helpless",
                    artist: "The Flirts",
                    bpm: 118,
                    pitch_centipercent: 0,
                    pitch_raw: 8192,
                    position_ms: 1000,
                    duration_ms: 465_000,
                    playing: true,
                    sync_enabled: false,
                    sync_master: false,
                    state: DeckStateText::Playing,
                },
                DeckStatus::EMPTY,
            ],
            mixer: MixerStatus::default(),
        }
    }
    fn control(&mut self, events: &[ControlEvent]) -> ControlOutcome {
        if self.busy {
            return ControlOutcome::Busy;
        }
        self.controls.extend_from_slice(events);
        ControlOutcome::Applied
    }
    fn seek(&mut self, deck: DeckId, position_ms: u32) -> SeekOutcome {
        self.seeks.push((deck, position_ms));
        SeekOutcome::Applied
    }
    fn load(&mut self, deck: DeckId, track: u32, revision: u32) -> LoadOutcome {
        self.loads.push((deck, track, revision));
        self.load_outcome.unwrap_or(LoadOutcome::Loaded)
    }
    fn duration_ms(&self, _: DeckId) -> Option<u32> {
        self.duration
    }
    fn library(&self) -> Option<(u32, usize)> {
        self.library
            .as_ref()
            .map(|(revision, rows)| (*revision, rows.len()))
    }
    fn library_row(&self, index: usize) -> Option<LibraryRow<'_>> {
        let (key, title) = self.library.as_ref()?.1.get(index)?;
        Some(LibraryRow {
            track_key: *key,
            title,
            artist: "Artist",
            bpm: 120,
            duration_ms: 10_000,
        })
    }
}

fn exchange(server: &mut WebServer, fake: &mut Fake, request: &str) -> String {
    exchange_in(server, fake, request, 4096)
}

fn exchange_in(server: &mut WebServer, fake: &mut Fake, request: &str, size: usize) -> String {
    let mut out = vec![0u8; size];
    let len = server.handle(request.as_bytes(), fake, &mut out);
    String::from_utf8(out[..len].to_vec()).unwrap()
}

fn post(path_query: &str) -> String {
    format!(
        "POST {path_query} HTTP/1.1\r\nHost: ruzina.local\r\nX-DDJ-Control: 1\r\n\
Origin: http://ruzina.local\r\nContent-Length: 0\r\n\r\n"
    )
}

fn status_line(response: &str) -> &str {
    response.split("\r\n").next().unwrap()
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

#[test]
fn status_is_stamped_json_with_an_exact_length() {
    let mut server = WebServer::new(77, AP);
    let mut fake = Fake::default();
    let request = "GET /api/status HTTP/1.1\r\nHost: 192.168.4.1:80\r\n\r\n";
    let first = exchange(&mut server, &mut fake, request);
    assert_eq!(status_line(&first), "HTTP/1.1 200 OK");
    assert!(first.contains("\r\nContent-Type: application/json\r\n"));
    let json = body(&first);
    assert!(first.contains(&format!("\r\nContent-Length: {}\r\n", json.len())));
    assert!(json.starts_with("{\"stamp\":{\"boot\":77,\"sequence\":1,\"library_revision\":null,"));
    assert!(
        json.contains("\"deck1\":{\"title\":\"Helpless\",\"artist\":\"The Flirts\",\"bpm\":118")
    );
    assert!(json.ends_with("\"pfl2\":false}}"));
    let second = exchange(&mut server, &mut fake, request);
    assert!(
        body(&second).contains("\"sequence\":2"),
        "one stamp per response"
    );
}

#[test]
fn control_taps_and_faders_become_semantic_events() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake::default();
    let ok = exchange(
        &mut server,
        &mut fake,
        &post("/api/control?action=cue&deck=2"),
    );
    assert_eq!(status_line(&ok), "HTTP/1.1 200 OK");
    assert_eq!(body(&ok), "OK");
    let press = |pressed| ControlEvent {
        deck: Some(DeckId::Two),
        control: SemanticControl::Cue,
        value: ControlValue::Pressed(pressed),
    };
    assert_eq!(
        fake.controls,
        [press(true), press(false)],
        "a tap, not a hold"
    );

    // Latest-value ordering: an older numbered fader value is not applied.
    fake.controls.clear();
    for seq in [5, 4, 6] {
        let request = post(&format!(
            "/api/control?action=volume&deck=1&value=8000&client=9&seq={seq}"
        ));
        assert_eq!(
            status_line(&exchange(&mut server, &mut fake, &request)),
            "HTTP/1.1 200 OK"
        );
    }
    assert_eq!(fake.controls.len(), 2, "seq 4 arrived late and was dropped");

    fake.busy = true;
    let busy = exchange(
        &mut server,
        &mut fake,
        &post("/api/control?action=sync&deck=1"),
    );
    assert_eq!(status_line(&busy), "HTTP/1.1 503 Service Unavailable");
    assert_eq!(body(&busy), "Control queue busy");

    for (query, message) in [
        ("action=dance&deck=1", "Unknown action"),
        ("action=cue&deck=3", "Deck must be 1 or 2"),
        (
            "action=volume&deck=1&value=16384",
            "Control value must be 0..16383",
        ),
        (
            "action=cue&deck=1&password=x",
            "Credentials do not belong in a query",
        ),
    ] {
        let refused = exchange(
            &mut server,
            &mut fake,
            &post(&format!("/api/control?{query}")),
        );
        assert_eq!(status_line(&refused), "HTTP/1.1 400 Bad Request");
        assert_eq!(body(&refused), message);
    }
}

#[test]
fn seeks_are_clamped_and_loads_carry_the_listed_revision() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake {
        duration: Some(10_000),
        ..Fake::default()
    };
    exchange(
        &mut server,
        &mut fake,
        &post("/api/control?action=seek&deck=1&value=99000"),
    );
    assert_eq!(fake.seeks, [(DeckId::One, 10_000)]);

    let ok = exchange(
        &mut server,
        &mut fake,
        &post("/api/load?track_key=42&generation=3&deck=2"),
    );
    assert_eq!(body(&ok), "OK");
    assert_eq!(fake.loads, [(DeckId::Two, 42, 3)]);
    for (outcome, line, message) in [
        (
            LoadOutcome::Locked,
            "HTTP/1.1 409 Conflict",
            "Deck is playing (load lock)",
        ),
        (
            LoadOutcome::LibraryChanged,
            "HTTP/1.1 409 Conflict",
            "Library generation changed",
        ),
        (
            LoadOutcome::NotFound,
            "HTTP/1.1 404 Not Found",
            "Track not found",
        ),
    ] {
        fake.load_outcome = Some(outcome);
        let refused = exchange(
            &mut server,
            &mut fake,
            &post("/api/load?track_key=42&generation=3&deck=1"),
        );
        assert_eq!(status_line(&refused), line);
        assert_eq!(body(&refused), message);
    }
    let bad = exchange(
        &mut server,
        &mut fake,
        &post("/api/load?track_key=0&generation=3&deck=1"),
    );
    assert_eq!(body(&bad), "Invalid track identity or deck");
}

#[test]
fn admission_routing_and_framing_refusals() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake::default();
    let cases = [
        // DNS rebinding: a foreign Host never reaches the API.
        (
            "GET /api/status HTTP/1.1\r\nHost: evil.example\r\n\r\n",
            "HTTP/1.1 403 Forbidden",
        ),
        (
            "POST /api/control?action=cue&deck=1 HTTP/1.1\r\nHost: ruzina.local\r\n\r\n",
            "HTTP/1.1 403 Forbidden",
        ),
        (
            "POST /api/control?action=cue&deck=1 HTTP/1.1\r\nHost: ruzina.local\r\n\
X-DDJ-Control: 1\r\nOrigin: http://evil.example\r\n\r\n",
            "HTTP/1.1 403 Forbidden",
        ),
        (
            "POST /api/control?action=cue&deck=1 HTTP/1.1\r\nHost: ruzina.local\r\n\
X-DDJ-Control: 1\r\nContent-Length: 4\r\n\r\nbody",
            "HTTP/1.1 413 Payload Too Large",
        ),
        (
            "GET /api/control HTTP/1.1\r\nHost: ruzina.local\r\n\r\n",
            "HTTP/1.1 405 Method Not Allowed",
        ),
        (
            "POST /api/nothing HTTP/1.1\r\nHost: ruzina.local\r\n\r\n",
            "HTTP/1.1 404 Not Found",
        ),
        (
            "GET /api/status HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "HTTP/1.1 501 Not Implemented",
        ),
        (
            "GET /api/status HTTP/2.0\r\n\r\n",
            "HTTP/1.1 505 HTTP Version Not Supported",
        ),
        (
            "GET /api/firmware HTTP/1.1\r\nHost: ruzina.local\r\n\r\n",
            "HTTP/1.1 501 Not Implemented",
        ),
    ];
    for (request, line) in cases {
        assert_eq!(
            status_line(&exchange(&mut server, &mut fake, request)),
            line,
            "{request}"
        );
    }
    assert!(
        fake.controls.is_empty(),
        "no refused request reached the deck"
    );

    // Any unknown page is the captive portal, whatever the Host.
    let portal = exchange(
        &mut server,
        &mut fake,
        "GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\n\r\n",
    );
    assert_eq!(status_line(&portal), "HTTP/1.1 302 Found");
    assert!(portal.contains(&format!("\r\nLocation: {CAPTIVE_PORTAL}\r\n")));

    // A buffer too small for the status JSON still gets a framed 500.
    let mut small = [0u8; 300];
    let len = server.handle(
        b"GET /api/status HTTP/1.1\r\nHost: ruzina.local\r\n\r\n",
        &mut fake,
        &mut small,
    );
    assert!(
        std::str::from_utf8(&small[..len])
            .unwrap()
            .starts_with("HTTP/1.1 500 ")
    );
}

#[test]
fn library_pages_cover_one_catalog_revision() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake::default();
    let get =
        |query: &str| format!("GET /api/library{query} HTTP/1.1\r\nHost: ruzina.local\r\n\r\n");
    let none = exchange(&mut server, &mut fake, &get(""));
    assert_eq!(status_line(&none), "HTTP/1.1 503 Service Unavailable");
    assert_eq!(body(&none), "Library snapshot unavailable");

    fake.library = Some((
        4,
        (1..=40)
            .map(|key| (key, format!("Track number {key}")))
            .collect(),
    ));
    // Small responses: follow `next` until null and collect every row once.
    let mut keys = Vec::new();
    let mut offset = Some(0u32);
    let mut pages = 0;
    while let Some(at) = offset {
        let page = exchange_in(&mut server, &mut fake, &get(&format!("?offset={at}")), 1500);
        assert_eq!(status_line(&page), "HTTP/1.1 200 OK");
        let json = body(&page);
        assert!(json.starts_with(&format!("{{\"generation\":4,\"total\":40,\"offset\":{at},")));
        for part in json.split("\"track_key\":").skip(1) {
            keys.push(part.split(',').next().unwrap().parse::<u32>().unwrap());
        }
        let next = json
            .rsplit_once("\"next\":")
            .unwrap()
            .1
            .trim_end_matches('}');
        offset = (next != "null").then(|| next.parse().unwrap());
        pages += 1;
    }
    assert!(pages > 1, "paged");
    assert_eq!(keys, (1..=40).collect::<Vec<_>>());

    let bad = exchange(&mut server, &mut fake, &get("?offset=-1"));
    assert_eq!(status_line(&bad), "HTTP/1.1 400 Bad Request");
}

#[test]
fn the_web_remote_page_is_served_from_the_image() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake::default();
    for (path, asset) in [
        ("/", assets::INDEX),
        ("/index.html", assets::INDEX),
        ("/style.css", assets::STYLE),
        ("/app.js", assets::APP),
    ] {
        let response = exchange_in(
            &mut server,
            &mut fake,
            &format!("GET {path} HTTP/1.1\r\nHost: ruzina.local\r\n\r\n"),
            assets::LARGEST_BYTES + 512,
        );
        assert_eq!(status_line(&response), "HTTP/1.1 200 OK", "{path}");
        assert!(response.contains(&format!("\r\nContent-Type: {}\r\n", asset.content_type)));
        assert_eq!(body(&response).as_bytes(), asset.body, "{path}");
    }
    // The page uses only routes this server implements.
    let app = std::str::from_utf8(assets::APP.body).unwrap();
    for route in ["/api/status", "/api/control", "/api/load", "/api/library"] {
        assert!(app.contains(route), "{route}");
    }
    for unimplemented in [
        "/api/ota",
        "/api/firmware",
        "/api/recording",
        "/api/controller-profile",
    ] {
        assert!(!app.contains(unimplemented), "{unimplemented}");
    }
    assert!(app.contains("'X-DDJ-Control': '1'"));
}
#[test]
fn review_busy_value_must_remain_retryable() {
    let mut server = WebServer::new(1, AP);
    let mut fake = Fake {
        busy: true,
        ..Fake::default()
    };
    let request = post("/api/control?action=volume&deck=1&value=1000&client=7&seq=12");
    assert_eq!(
        status_line(&exchange(&mut server, &mut fake, &request)),
        "HTTP/1.1 503 Service Unavailable"
    );
    fake.busy = false;
    let retry = exchange(&mut server, &mut fake, &request);
    assert_eq!(status_line(&retry), "HTTP/1.1 200 OK");
    assert_eq!(
        fake.controls.len(),
        1,
        "The accepted retry must actually deliver its value"
    );
    exchange(&mut server, &mut fake, &request);
    assert_eq!(fake.controls.len(), 1, "duplicate must not apply twice");
    fake.busy = true;
    let newer = post("/api/control?action=volume&deck=1&value=2000&client=7&seq=14");
    exchange(&mut server, &mut fake, &newer);
    fake.busy = false;
    let middle = post("/api/control?action=volume&deck=1&value=1500&client=7&seq=13");
    exchange(&mut server, &mut fake, &middle);
    assert_eq!(fake.controls.len(), 2, "unaccepted 14 must not exclude 13");
    exchange(&mut server, &mut fake, &newer);
    exchange(&mut server, &mut fake, &middle);
    assert_eq!(fake.controls.len(), 3, "accepted 14 excludes late 13");
}
