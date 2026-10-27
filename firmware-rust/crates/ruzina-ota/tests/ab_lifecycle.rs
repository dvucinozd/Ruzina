//! A/B install, candidate confirmation and rollback against a fake device
//! that models ESP-IDF otadata: `set_boot` marks the slot NEW, the
//! bootloader turns NEW into PENDING_VERIFY and aborts a PENDING_VERIFY
//! image that resets unconfirmed. Every transfer offset and flash operation
//! is interrupted before any board test.

use ruzina_ota::boot::{
    BootControl, BootHealth, ImageState, Readiness, STARTUP_TIMEOUT_MS, StartupGate, StartupVerdict,
};
use ruzina_ota::bundle::{
    ESP32P4_CHIP_ID, Expected, HEADER_SIZE, MAX_IMAGE_SIZE, ManifestError, RELEASE_PUBLIC_KEY,
    TARGET_P4,
};
use ruzina_ota::channel::{self, Offer};
use ruzina_ota::install::{
    BundleSource, Failure, Fault, Playback, Policy, ReadError, RebootPermit, Request, SlotFlash,
    Stage, install,
};
use sha2::{Digest, Sha256};

const NEWER: &[u8] = include_bytes!("fixtures/newer.ddjota");
const OLDER: &[u8] = include_bytes!("fixtures/older.ddjota");
const OTHER_PROJECT: &[u8] = include_bytes!("fixtures/other_project.ddjota");
const TEST_KEY: &[u8; 65] = include_bytes!("fixtures/test_key.sec1");
const PROJECT: &str = "ruzina-m1-test";
const RUNNING: &str = "M2.5-0-g20f1c3f";
const NEWER_VERSION: &str = "M2.5-4-g1a2b3c4d";
const EXPECTED: Expected<'static> = Expected {
    target: TARGET_P4,
    chip_id: ESP32P4_CHIP_ID,
    project: PROJECT,
    max_image_size: MAX_IMAGE_SIZE,
};

/// HTTP composition over the same A/B device and playback models used by the
/// sync installer tests. Trust is explicit test-only configuration.
struct HttpOta {
    device: Device,
    audio: Audio,
    push: Option<ruzina_ota::push::Push>,
    key: [u8; 65],
}
impl HttpOta {
    fn new() -> Self {
        Self {
            device: Device::new(),
            audio: Audio::default(),
            push: None,
            key: *TEST_KEY,
        }
    }
}
impl ruzina_web::server::WebBackend for HttpOta {
    fn upload_begin(
        &mut self,
        kind: ruzina_web::upload::UploadKind,
        len: usize,
    ) -> Result<(), ruzina_web::HttpStatus> {
        if !matches!(kind, ruzina_web::upload::UploadKind::Ota) {
            return Err(ruzina_web::HttpStatus::NotImplemented);
        }
        if self.push.is_some() {
            return Err(ruzina_web::HttpStatus::ServiceUnavailable);
        }
        self.push =
            Some(ruzina_ota::push::Push::new(len).map_err(|_| ruzina_web::HttpStatus::BadRequest)?);
        Ok(())
    }
    fn upload_write(&mut self, bytes: &[u8]) -> Result<(), ruzina_web::HttpStatus> {
        self.push
            .as_mut()
            .ok_or(ruzina_web::HttpStatus::Conflict)?
            .feed(
                bytes,
                &EXPECTED,
                &self.key,
                &mut self.device,
                &mut self.audio,
            )
            .map_err(push_status)
    }
    fn upload_finish(&mut self) -> Result<(), ruzina_web::HttpStatus> {
        self.push
            .take()
            .ok_or(ruzina_web::HttpStatus::Conflict)?
            .finish(&EXPECTED, &mut self.device, &mut self.audio)
            .map(|_| ())
            .map_err(push_status)
    }
    fn upload_cancel(&mut self) {
        if let Some(mut push) = self.push.take() {
            push.cancel(&mut self.device, &mut self.audio);
        }
    }
    fn status(&mut self) -> ruzina_web::server::StatusParts<'_> {
        panic!("status outside this OTA composition")
    }
    fn control(
        &mut self,
        _: &[ruzina_controller_core::ControlEvent],
    ) -> ruzina_web::server::ControlOutcome {
        panic!("control outside OTA composition")
    }
    fn seek(
        &mut self,
        _: ruzina_controller_core::DeckId,
        _: u32,
    ) -> ruzina_web::server::SeekOutcome {
        panic!("seek outside OTA composition")
    }
    fn load(
        &mut self,
        _: ruzina_controller_core::DeckId,
        _: u32,
        _: u32,
    ) -> ruzina_web::server::LoadOutcome {
        panic!("load outside OTA composition")
    }
    fn duration_ms(&self, _: ruzina_controller_core::DeckId) -> Option<u32> {
        None
    }
    fn library(&self) -> Option<(u32, usize)> {
        None
    }
    fn library_row(&self, _: usize) -> Option<ruzina_web::json::LibraryRow<'_>> {
        None
    }
}
fn push_status(e: ruzina_ota::push::PushError) -> ruzina_web::HttpStatus {
    use ruzina_ota::push::PushError::*;
    use ruzina_web::HttpStatus as H;
    match e {
        Flash => H::InternalServerError,
        Busy => H::ServiceUnavailable,
        Inactive => H::Conflict,
        _ => H::BadRequest,
    }
}
fn http_head(len: usize) -> Vec<u8> {
    format!("POST /api/ota/p4 HTTP/1.1\r\nHost: 192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Type: application/octet-stream\r\nContent-Length: {len}\r\n\r\n").into_bytes()
}
#[test]
fn incremental_http_ota_installs_then_boots_pending_and_rolls_back_unconfirmed() {
    for (blob, version) in [(NEWER, NEWER_VERSION), (OLDER, "M2.4-7-g0badc0d")] {
        let mut backend = HttpOta::new();
        backend.device.descriptor_version = Some(version.into());
        let mut server = ruzina_web::server::WebServer::new(1, "192.168.4.1");
        let lease = server
            .begin_body(&http_head(blob.len()), &mut backend)
            .unwrap()
            .unwrap();
        assert_eq!(
            server.begin_body(&http_head(blob.len()), &mut backend),
            Err(ruzina_web::HttpStatus::ServiceUnavailable)
        );
        for byte in blob {
            server.body_chunk(lease, &[*byte], &mut backend).unwrap();
        }
        assert_eq!(
            backend.device.boot, 0,
            "candidate must not publish before finish"
        );
        server.finish_body(lease, &mut backend).unwrap();
        assert!(backend.audio.stopped);
        backend.device.power_on();
        assert_eq!(backend.device.running().1, Otadata::Pending);
        backend.device.power_on();
        assert_eq!(
            backend.device.running().0,
            0,
            "unconfirmed reset rolls back"
        );
    }
}
#[test]
fn incremental_http_ota_rejects_bad_trust_hash_and_project_and_cancels_partial_flash() {
    for variant in 0..3 {
        let mut blob = NEWER.to_vec();
        let mut backend = HttpOta::new();
        match variant {
            0 => backend.key = RELEASE_PUBLIC_KEY,
            1 => {
                let last = blob.len() - 1;
                blob[last] ^= 1;
            }
            _ => blob = OTHER_PROJECT.to_vec(),
        }
        let mut raw = http_head(blob.len());
        raw.extend_from_slice(&blob);
        let mut server = ruzina_web::server::WebServer::new(1, "192.168.4.1");
        let mut out = [0; 512];
        let n = server.handle(&raw, &mut backend, &mut out);
        assert!(out[..n].starts_with(b"HTTP/1.1 400"));
        assert_eq!(backend.device.boot, 0);
        assert!(!backend.audio.stopped);
        if variant != 1 {
            assert_eq!(backend.device.flash_ops, 0, "reject before slot mutation");
        }
    }
    for cut in [0, 1, 187, 211, 212, 500, NEWER.len() - 1] {
        let mut backend = HttpOta::new();
        let mut server = ruzina_web::server::WebServer::new(1, "192.168.4.1");
        let lease = server
            .begin_body(&http_head(NEWER.len()), &mut backend)
            .unwrap()
            .unwrap();
        for chunk in NEWER[..cut].chunks(512) {
            server.body_chunk(lease, chunk, &mut backend).unwrap();
        }
        server.cancel_body(lease, &mut backend);
        assert!(!backend.audio.stopped);
        assert_eq!(backend.device.boot, 0);
        assert!(backend.device.open.is_none());
        assert!(server.body_chunk(lease, &[0], &mut backend).is_err());
    }
}
#[test]
fn incremental_http_flash_failures_never_publish_a_bad_candidate() {
    for budget in 0..15 {
        let mut backend = HttpOta::new();
        backend.device.budget = Some(budget);
        let mut raw = http_head(NEWER.len());
        raw.extend_from_slice(NEWER);
        let mut server = ruzina_web::server::WebServer::new(1, "192.168.4.1");
        let mut out = [0; 512];
        let n = server.handle(&raw, &mut backend, &mut out);
        if !out[..n].starts_with(b"HTTP/1.1 200") {
            assert_eq!(backend.device.boot, 0);
            assert!(!backend.audio.stopped);
        }
        backend.device.power_on();
        assert!(matches!(
            backend.device.running().1,
            Otadata::Valid | Otadata::Pending
        ));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Otadata {
    Valid,
    New,
    Pending,
    Invalid,
    Aborted,
}

#[derive(Clone, Debug)]
struct Slot {
    image: Vec<u8>,
    state: Otadata,
    version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cut {
    Power,
    Refused,
}

struct Device {
    slots: [Option<Slot>; 2],
    boot: usize,
    running: usize,
    open: Option<Vec<u8>>,
    written: Option<Vec<u8>>,
    slot_size: u32,
    /// Flash operations left before the power fails.
    budget: Option<usize>,
    powered: bool,
    flash_ops: usize,
    descriptor_version: Option<String>,
}

impl Device {
    fn new() -> Self {
        Self {
            slots: [
                Some(Slot {
                    image: vec![0xe9; 64],
                    state: Otadata::Valid,
                    version: RUNNING.into(),
                }),
                None,
            ],
            boot: 0,
            running: 0,
            open: None,
            written: None,
            slot_size: 8192,
            budget: None,
            powered: true,
            flash_ops: 0,
            descriptor_version: None,
        }
    }

    fn update_index(&self) -> usize {
        1 - self.running
    }

    fn op(&mut self) -> Result<(), Cut> {
        if !self.powered {
            return Err(Cut::Power);
        }
        self.flash_ops += 1;
        if let Some(budget) = &mut self.budget {
            if *budget == 0 {
                self.powered = false;
                return Err(Cut::Power);
            }
            *budget -= 1;
        }
        Ok(())
    }

    /// Reset: RAM state is lost, then the bootloader picks a slot.
    fn power_on(&mut self) {
        self.open = None;
        self.written = None;
        self.powered = true;
        self.budget = None;
        let candidate = self.boot;
        let state = self.slots[candidate].as_ref().map(|slot| slot.state);
        match state {
            Some(Otadata::New) => {
                self.slots[candidate].as_mut().unwrap().state = Otadata::Pending;
                self.running = candidate;
            }
            Some(Otadata::Pending) => {
                self.slots[candidate].as_mut().unwrap().state = Otadata::Aborted;
                self.boot = 1 - candidate;
                self.running = self.boot;
            }
            Some(Otadata::Valid) => self.running = candidate,
            _ => {
                self.boot = 1 - candidate;
                self.running = self.boot;
            }
        }
    }

    fn running(&self) -> (usize, Otadata, &str) {
        let slot = self.slots[self.running].as_ref().unwrap();
        (self.running, slot.state, slot.version.as_str())
    }
}

impl SlotFlash for Device {
    type Error = Cut;

    fn update_slot_size(&mut self) -> Option<u32> {
        Some(self.slot_size)
    }

    fn begin(&mut self, _image_size: u32) -> Result<(), Cut> {
        self.op()?;
        let index = self.update_index();
        self.slots[index] = None;
        self.open = Some(Vec::new());
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<(), Cut> {
        self.op()?;
        self.open
            .as_mut()
            .ok_or(Cut::Refused)?
            .extend_from_slice(data);
        Ok(())
    }

    fn abort(&mut self) {
        self.open = None;
        self.written = None;
    }

    fn end(&mut self) -> Result<(), Cut> {
        self.op()?;
        self.written = Some(self.open.take().ok_or(Cut::Refused)?);
        Ok(())
    }

    fn descriptor(&mut self) -> Result<([u8; 32], [u8; 32]), Cut> {
        self.written.as_ref().ok_or(Cut::Refused)?;
        let field = |text: &str| {
            let mut field = [0; 32];
            field[..text.len()].copy_from_slice(text.as_bytes());
            field
        };
        let version = self
            .descriptor_version
            .clone()
            .unwrap_or(NEWER_VERSION.into());
        Ok((field(PROJECT), field(&version)))
    }

    fn set_boot(&mut self) -> Result<(), Cut> {
        self.op()?;
        let image = self.written.take().ok_or(Cut::Refused)?;
        let index = self.update_index();
        let version = self
            .descriptor_version
            .clone()
            .unwrap_or(NEWER_VERSION.into());
        self.slots[index] = Some(Slot {
            image,
            state: Otadata::New,
            version,
        });
        self.boot = index;
        Ok(())
    }
}

impl BootControl for &mut Device {
    type Error = Cut;

    fn running_state(&mut self) -> Result<ImageState, Cut> {
        Ok(match self.running().1 {
            Otadata::Valid => ImageState::Valid,
            Otadata::Pending => ImageState::PendingVerify,
            Otadata::Invalid => ImageState::Invalid,
            Otadata::Aborted | Otadata::New => ImageState::Aborted,
        })
    }

    fn mark_valid(&mut self) -> Result<(), Cut> {
        let running = self.running;
        self.slots[running].as_mut().unwrap().state = Otadata::Valid;
        Ok(())
    }

    /// The fake performs the reboot and returns, standing in for the call
    /// that never returns on hardware.
    fn reject_and_reboot(&mut self) -> Result<(), Cut> {
        let running = self.running;
        self.slots[running].as_mut().unwrap().state = Otadata::Invalid;
        self.boot = 1 - running;
        self.power_on();
        Ok(())
    }
}

#[derive(Default)]
struct Audio {
    stopped: bool,
    refuse: bool,
    stops: u32,
}

impl Playback for Audio {
    fn stop_for_update(&mut self) -> bool {
        if self.refuse {
            return false;
        }
        self.stops += 1;
        self.stopped = true;
        true
    }

    fn resume(&mut self) {
        self.stopped = false;
    }
}

struct Source<'a> {
    data: &'a [u8],
    at: usize,
    chunk: usize,
    /// End of stream once this many bytes were delivered.
    eof_at: usize,
    /// Timeouts forever from this offset on.
    stall_at: Option<usize>,
    /// Timeouts before every successful read (below the retry bound).
    jitter: u8,
    pending_jitter: u8,
}

impl<'a> Source<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            chunk: 1000,
            eof_at: data.len(),
            stall_at: None,
            jitter: 0,
            pending_jitter: 0,
        }
    }
}

impl BundleSource for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ReadError> {
        if self.stall_at.is_some_and(|stall| self.at >= stall) {
            return Err(ReadError::Timeout);
        }
        if self.pending_jitter < self.jitter {
            self.pending_jitter += 1;
            return Err(ReadError::Timeout);
        }
        self.pending_jitter = 0;
        let end = self
            .eof_at
            .min(self.at + self.chunk)
            .min(self.at + buf.len());
        let got = end - self.at;
        buf[..got].copy_from_slice(&self.data[self.at..end]);
        self.at = end;
        Ok(got)
    }
}

fn local(declared_len: usize) -> Request<'static> {
    Request {
        declared_len: declared_len as u32,
        expected: EXPECTED,
        public_key: TEST_KEY,
        policy: Policy::Local,
    }
}

fn offer_for(bundle: &[u8], release: &str) -> Offer {
    let sha: String = Sha256::digest(bundle)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let doc = format!(
        r#"{{"schema_version":1,"release":"{release}","p4":{{"url":"{release}/m1.ddjota","size":{},"sha256":"{sha}"}}}}"#,
        bundle.len()
    );
    channel::parse(doc.as_bytes(), "p4").unwrap()
}

fn run(
    device: &mut Device,
    audio: &mut Audio,
    source: &mut Source<'_>,
    request: &Request<'_>,
) -> Result<RebootPermit, Failure> {
    let mut buf = [0; 512];
    install(source, device, audio, request, &mut buf)
}

fn assert_untouched(device: &mut Device, audio: &Audio, context: &str) {
    assert!(!audio.stopped, "playback resumed: {context}");
    assert_eq!(device.boot, 0, "boot slot unchanged: {context}");
    device.power_on();
    assert_eq!(device.running(), (0, Otadata::Valid, RUNNING), "{context}");
}

const READY: Readiness = Readiness {
    critical_tasks_alive: true,
    critical_allocations_ok: true,
    network_active: true,
};

#[test]
fn verified_candidate_boots_pending_and_is_confirmed_when_ready() {
    let mut device = Device::new();
    let mut audio = Audio::default();
    let permit = run(
        &mut device,
        &mut audio,
        &mut Source::new(NEWER),
        &local(NEWER.len()),
    )
    .unwrap();
    assert_eq!(permit.version(), NEWER_VERSION);
    assert_eq!(RebootPermit::CORE, 0, "M2.2 reboot panic regression");
    assert!(audio.stopped, "playback stays stopped for the reboot");
    assert_eq!(
        device.slots[1].as_ref().unwrap().image,
        NEWER[HEADER_SIZE..],
        "exact image in the inactive slot"
    );
    device.power_on();
    assert_eq!(device.running(), (1, Otadata::Pending, NEWER_VERSION));

    let gate = StartupGate::new(0, true);
    let mut health = BootHealth::new(&mut device).unwrap();
    let waiting = Readiness {
        network_active: false,
        ..READY
    };
    assert_eq!(
        health.settle(&gate, 5_000, waiting),
        Ok(StartupVerdict::Wait)
    );
    assert_eq!(
        health.settle(&gate, 6_000, READY),
        Ok(StartupVerdict::Ready)
    );
    device.power_on();
    assert_eq!(device.running(), (1, Otadata::Valid, NEWER_VERSION));
}

#[test]
fn unconfirmed_or_late_candidates_physically_roll_back() {
    let mut device = Device::new();
    run(
        &mut device,
        &mut Audio::default(),
        &mut Source::new(NEWER),
        &local(NEWER.len()),
    )
    .unwrap();
    device.power_on();
    device.power_on();
    assert_eq!(
        device.running(),
        (0, Otadata::Valid, RUNNING),
        "reset before confirmation"
    );
    assert_eq!(device.slots[1].as_ref().unwrap().state, Otadata::Aborted);

    let mut device = Device::new();
    run(
        &mut device,
        &mut Audio::default(),
        &mut Source::new(NEWER),
        &local(NEWER.len()),
    )
    .unwrap();
    device.power_on();
    let gate = StartupGate::new(0, true);
    let mut health = BootHealth::new(&mut device).unwrap();
    let no_network = Readiness {
        network_active: false,
        ..READY
    };
    assert_eq!(
        health.settle(&gate, STARTUP_TIMEOUT_MS, no_network),
        Ok(StartupVerdict::Timeout)
    );
    assert_eq!(device.running(), (0, Otadata::Valid, RUNNING));
    assert_eq!(device.slots[1].as_ref().unwrap().state, Otadata::Invalid);
    let mut health = BootHealth::new(&mut device).unwrap();
    assert!(!health.pending());
    assert!(
        health.reject().is_err(),
        "the accepted image is never invalidated"
    );
}

#[test]
fn every_transfer_interruption_keeps_the_running_image() {
    let mut offsets: Vec<usize> = (0..=HEADER_SIZE + 30).collect();
    offsets.extend((HEADER_SIZE + 31..NEWER.len()).step_by(211));
    offsets.push(NEWER.len() - 1);
    for eof_at in offsets {
        let mut device = Device::new();
        let mut audio = Audio::default();
        let mut source = Source::new(NEWER);
        source.eof_at = eof_at;
        let failure = run(&mut device, &mut audio, &mut source, &local(NEWER.len())).unwrap_err();
        assert_eq!(failure.fault, Fault::Truncated, "eof at {eof_at}");
        let expected_stage = if eof_at < HEADER_SIZE {
            Stage::ReadSignedHeader
        } else if eof_at < HEADER_SIZE + 24 {
            Stage::ReadImageHeader
        } else {
            Stage::DownloadImage
        };
        assert_eq!(failure.stage, expected_stage, "eof at {eof_at}");
        if !expected_stage.touched_flash() {
            assert_eq!(device.flash_ops, 0, "nothing erased before eof {eof_at}");
            assert_eq!(audio.stops, 0);
        }
        assert_untouched(&mut device, &audio, &format!("eof at {eof_at}"));
    }
}

#[test]
fn every_flash_operation_interruption_keeps_the_running_image() {
    let mut device = Device::new();
    run(
        &mut device,
        &mut Audio::default(),
        &mut Source::new(NEWER),
        &local(NEWER.len()),
    )
    .unwrap();
    let total = device.flash_ops;
    for cut in 0..total {
        let mut device = Device::new();
        device.budget = Some(cut);
        let mut audio = Audio::default();
        let failure = run(
            &mut device,
            &mut audio,
            &mut Source::new(NEWER),
            &local(NEWER.len()),
        )
        .unwrap_err();
        assert_eq!(failure.fault, Fault::Flash, "cut at op {cut}");
        assert_untouched(&mut device, &audio, &format!("cut at op {cut}"));
    }
    // A cut after the boot slot switched boots the verified candidate,
    // which must still confirm or roll back.
    let mut device = Device::new();
    device.budget = Some(total);
    run(
        &mut device,
        &mut Audio::default(),
        &mut Source::new(NEWER),
        &local(NEWER.len()),
    )
    .unwrap();
    device.power_on();
    assert_eq!(device.running().1, Otadata::Pending);
}

#[test]
fn stalls_are_bounded_and_jitter_below_the_bound_completes() {
    let mut device = Device::new();
    let mut audio = Audio::default();
    let mut source = Source::new(NEWER);
    source.stall_at = Some(HEADER_SIZE + 2000);
    let failure = run(&mut device, &mut audio, &mut source, &local(NEWER.len())).unwrap_err();
    assert_eq!(
        failure,
        Failure {
            stage: Stage::DownloadImage,
            fault: Fault::Stalled
        }
    );
    assert_untouched(&mut device, &audio, "stall");

    let mut device = Device::new();
    let mut source = Source::new(NEWER);
    source.jitter = 2;
    source.chunk = 97;
    assert!(
        run(
            &mut device,
            &mut Audio::default(),
            &mut source,
            &local(NEWER.len())
        )
        .is_ok()
    );
}

#[test]
fn identity_signature_and_hash_checks_refuse_before_switching() {
    let check = |bundle: &[u8], request: &Request<'_>, device: &mut Device, audio: &mut Audio| {
        run(device, audio, &mut Source::new(bundle), request).unwrap_err()
    };
    let mut device = Device::new();
    let mut audio = Audio::default();
    let mut bad_sig = NEWER.to_vec();
    bad_sig[84] ^= 1;
    assert_eq!(
        check(&bad_sig, &local(NEWER.len()), &mut device, &mut audio),
        Failure {
            stage: Stage::VerifySignature,
            fault: Fault::Manifest(ManifestError::Signature)
        }
    );
    let release_key = Request {
        public_key: &RELEASE_PUBLIC_KEY,
        ..local(NEWER.len())
    };
    assert_eq!(
        check(NEWER, &release_key, &mut device, &mut audio).stage,
        Stage::VerifySignature
    );
    assert_eq!(
        check(
            OTHER_PROJECT,
            &local(OTHER_PROJECT.len()),
            &mut device,
            &mut audio
        ),
        Failure {
            stage: Stage::ParseSignedHeader,
            fault: Fault::Manifest(ManifestError::Project)
        }
    );
    for declared in [NEWER.len() - 1, NEWER.len() + 1, HEADER_SIZE + 23] {
        assert_eq!(
            check(NEWER, &local(declared), &mut device, &mut audio).fault,
            Fault::Length,
            "declared {declared}"
        );
    }
    assert_eq!(device.flash_ops, 0);
    assert_eq!(
        audio.stops, 0,
        "playback never stopped for a refused bundle"
    );

    let mut not_p4 = NEWER.to_vec();
    not_p4[HEADER_SIZE] = 0;
    let mut corrupt = NEWER.to_vec();
    corrupt[HEADER_SIZE + 3000] ^= 0x40;
    assert_eq!(
        check(&not_p4, &local(NEWER.len()), &mut device, &mut audio),
        Failure {
            stage: Stage::ImageHeader,
            fault: Fault::NotP4Image
        }
    );
    assert_eq!(
        check(&corrupt, &local(NEWER.len()), &mut device, &mut audio),
        Failure {
            stage: Stage::ImageHash,
            fault: Fault::ImageHash
        }
    );
    assert_untouched(&mut device, &audio, "corrupt image");

    let mut device = Device::new();
    device.descriptor_version = Some("M2.5-5-g1a2b3c4d".into());
    let mut audio = Audio::default();
    assert_eq!(
        check(NEWER, &local(NEWER.len()), &mut device, &mut audio),
        Failure {
            stage: Stage::Descriptor,
            fault: Fault::VersionMismatch
        }
    );
    assert_untouched(&mut device, &audio, "descriptor");

    let mut device = Device::new();
    device.slot_size = 4000;
    let mut audio = Audio::default();
    assert_eq!(
        check(NEWER, &local(NEWER.len()), &mut device, &mut audio).fault,
        Fault::SlotTooSmall
    );
    audio.refuse = true;
    device.slot_size = 8192;
    assert_eq!(
        check(NEWER, &local(NEWER.len()), &mut device, &mut audio),
        Failure {
            stage: Stage::StopAudio,
            fault: Fault::Playback
        }
    );
    assert_eq!(device.flash_ops, 0);
}

#[test]
fn pull_binds_offer_release_size_hash_and_newer_only() {
    let pull = |bundle: &[u8], offer: &Offer, device: &mut Device, audio: &mut Audio| {
        let request = Request {
            policy: Policy::Pull {
                offer,
                running_version: RUNNING,
            },
            ..local(bundle.len())
        };
        run(device, audio, &mut Source::new(bundle), &request)
    };
    let mut device = Device::new();
    let mut audio = Audio::default();
    let offer = offer_for(NEWER, NEWER_VERSION);
    assert!(pull(NEWER, &offer, &mut device, &mut audio).is_ok());

    let mut device = Device::new();
    let mut audio = Audio::default();
    let mut wrong_hash = offer;
    wrong_hash.sha256[0] ^= 1;
    assert_eq!(
        pull(NEWER, &wrong_hash, &mut device, &mut audio).unwrap_err(),
        Failure {
            stage: Stage::ChannelHash,
            fault: Fault::ChannelHash
        }
    );
    assert_untouched(&mut device, &audio, "channel hash");

    let mut device = Device::new();
    let mut audio = Audio::default();
    let renamed = offer_for(NEWER, "M2.5-5-g1a2b3c4d");
    assert_eq!(
        pull(NEWER, &renamed, &mut device, &mut audio).unwrap_err(),
        Failure {
            stage: Stage::ReleaseBinding,
            fault: Fault::ReleaseMismatch
        }
    );
    let older = offer_for(OLDER, "M2.4-7-g0badc0d");
    assert_eq!(
        pull(OLDER, &older, &mut device, &mut audio).unwrap_err(),
        Failure {
            stage: Stage::ReleaseBinding,
            fault: Fault::NotNewer
        }
    );
    let mut resized = offer;
    resized.size += 1;
    assert_eq!(
        pull(NEWER, &resized, &mut device, &mut audio)
            .unwrap_err()
            .fault,
        Fault::Length
    );
    assert_eq!((device.flash_ops, audio.stops), (0, 0));

    // A signed older build still installs through the local service path.
    let mut device = Device::new();
    device.descriptor_version = Some("M2.4-7-g0badc0d".into());
    let permit = run(
        &mut device,
        &mut Audio::default(),
        &mut Source::new(OLDER),
        &local(OLDER.len()),
    )
    .unwrap();
    assert_eq!(permit.version(), "M2.4-7-g0badc0d");
}
