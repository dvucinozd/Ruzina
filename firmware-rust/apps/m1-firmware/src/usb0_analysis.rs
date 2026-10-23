//! Opt-in, stopped-deck USB0 analysis. Never enabled by a production default.
use crate::{
    analysis_product::{Driver, Report},
    product_runtime::ProductRuntime,
};
use alloc::vec::Vec;
use core::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    signal::Signal,
};
use ruzina_apta_adapter::{
    owner::{Output, Owner, Workspace},
    service::{Authority, Command, Port, Retained, Selection, Service, Status},
    storage::{LocalTempoBuffers, LocalTempoStorage},
    streaming::StreamLimits,
};
use ruzina_media_fs::analysis::Client;

const FRAMES: u64 = 48_000 * 600;
const FRAMES_PER_COLUMN: u32 = 65_536;
const BEATS: usize = 4096;
const LIMITS: StreamLimits = StreamLimits {
    object_bytes: 256 * 1024 * 1024,
    duration_ms: 600_000,
    scan_steps: 1_000_000,
    processing_steps: 2_000_000,
};
static COMMANDS: Channel<CriticalSectionRawMutex, Command<'static>, 2> = Channel::new();
static OUTPUTS: Channel<CriticalSectionRawMutex, Retained<'static>, 1> = Channel::new();
static PROGRESS: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static ALLOWED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
use ruzina_service::analysis::{Monitor, Observation, Reason, Sample};
struct Snapshot {
    diagnostic: Monitor,
    authority: Authority,
    report: Report,
    selected: Option<Selection>,
}
static SNAPSHOT: CriticalSectionMutex<RefCell<Snapshot>> =
    CriticalSectionMutex::new(RefCell::new(Snapshot {
        diagnostic: Monitor::new(),
        authority: Authority::EMPTY,
        report: Report {
            serial: 0,
            selection: None,
            status: None,
            reason: Reason::None,
        },
        selected: None,
    }));

pub(crate) fn io_allowed() -> bool {
    ALLOWED.load(Ordering::Acquire)
}
pub(crate) async fn progress() {
    let _ = embassy_futures::select::select(PROGRESS.wait(), embassy_time::Timer::after_millis(20))
        .await;
}
pub(crate) fn product_turn(product: &mut ProductRuntime) {
    let mut driver = core::mem::replace(&mut product.analysis_driver, Driver::new());
    driver.turn(
        product,
        &mut ProductPort,
        crate::deck_effect_owners::analysis_transport_idle(),
    );
    product.analysis_driver = driver;
}
/// Keep the original USB event future pinned across analysis notifications.
pub(crate) async fn product_wait<F: core::future::Future>(
    product: &mut ProductRuntime,
    future: F,
) -> F::Output {
    let mut future = core::pin::pin!(future);
    loop {
        product_turn(product);
        match embassy_futures::select::select(future.as_mut(), progress()).await {
            embassy_futures::select::Either::First(value) => return value,
            embassy_futures::select::Either::Second(()) => {}
        }
    }
}
struct ProductPort;
impl crate::analysis_product::Port<'static> for ProductPort {
    fn observe(&mut self, sample: Sample) {
        SNAPSHOT.lock(|s| {
            s.borrow_mut()
                .diagnostic
                .observe(sample, embassy_time::Instant::now().as_millis())
        });
    }
    fn ready(&self) -> bool {
        READY.load(Ordering::Acquire)
    }
    fn media(&self) -> ruzina_media_session::MediaSession {
        crate::usb0_msc::analysis_media_snapshot()
    }
    fn allow_io(&mut self, allowed: bool) {
        ALLOWED.store(allowed, Ordering::Release);
    }
    fn authority(&self) -> Authority {
        SNAPSHOT.lock(|s| s.borrow().authority)
    }
    fn report(&self) -> Report {
        SNAPSHOT.lock(|s| s.borrow().report)
    }
    fn send(&mut self, command: Command<'static>) -> Result<(), Command<'static>> {
        COMMANDS
            .try_send(command)
            .map_err(|embassy_sync::channel::TrySendError::Full(command)| command)
    }
    fn output(&mut self) -> Option<Retained<'static>> {
        OUTPUTS.try_receive().ok()
    }
}
struct OwnerPort;
impl Port<'static> for OwnerPort {
    async fn receive(&mut self) -> Command<'static> {
        let command = COMMANDS.receive().await;
        SNAPSHOT.lock(|s| {
            let mut s = s.borrow_mut();
            match &command {
                Command::Analyze(selection, _) => s.selected = Some(*selection),
                Command::Cancel => s.selected = None,
                _ => {}
            }
        });
        command
    }
    fn media(&self) -> ruzina_media_session::MediaSession {
        crate::usb0_msc::analysis_media_snapshot()
    }
    fn authority(&mut self, authority: Authority) {
        SNAPSHOT.lock(|s| s.borrow_mut().authority = authority);
    }
    fn publish(&mut self, output: Retained<'static>) -> Result<(), Output<'static>> {
        OUTPUTS
            .try_send(output)
            .map_err(
                |embassy_sync::channel::TrySendError::Full(output)| match output.reject() {
                    Ok(bank) => bank,
                    Err(_) => unreachable!("unaccepted service output"),
                },
            )
    }
    fn failure(&mut self, reason: ruzina_apta_adapter::service::Failure) {
        report_status(
            Status::Failed,
            crate::analysis_product::failure_reason(reason),
        );
    }
    fn report(&mut self, status: Status) {
        report_status(
            status,
            if status == Status::Rejected {
                Reason::Rejected
            } else {
                Reason::None
            },
        );
    }
}
pub(crate) fn diagnostic_snapshot() -> Observation {
    SNAPSHOT.lock(|s| s.borrow().diagnostic.snapshot())
}
fn report_status(status: Status, reason: Reason) {
    SNAPSHOT.lock(|s| {
        let mut s = s.borrow_mut();
        // Exhaustion fails visibly rather than reusing an old report serial.
        s.report = Report {
            serial: s
                .report
                .serial
                .checked_add(1)
                .expect("analysis status serial exhausted"),
            selection: s.selected,
            status: Some(status),
            reason,
        };
    });
    PROGRESS.signal(());
}

fn array<T: Clone>(len: usize, value: T) -> Result<Vec<T>, ()> {
    let mut values = Vec::new();
    values.try_reserve_exact(len).map_err(|_| ())?;
    values.resize(len, value);
    Ok(values)
}
struct Storage {
    queue: Vec<libapta::waveform::NormalizedSample>,
    columns: Vec<libapta::WaveformColumn>,
    bins: Vec<libapta::analysis::OnsetBin>,
    flux: Vec<f32>,
    bytes: Vec<u8>,
    pcm: Vec<f32>,
    io: Vec<u8>,
    beats: [Vec<ruzina_track_analysis::Beat>; 2],
    waveform: [Vec<ruzina_waveform::WaveformColumn>; 2],
}
impl Storage {
    fn new() -> Result<Self, ()> {
        let plan = LocalTempoStorage::plan(FRAMES, FRAMES_PER_COLUMN, 256, BEATS, 201_000)
            .map_err(|_| ())?;
        let beat = ruzina_track_analysis::Beat {
            time_ms: 0,
            phase: 0,
            bpm_x100: 0,
        };
        let wave = ruzina_waveform::WaveformColumn {
            peak: 0,
            color_rgb565: 0,
        };
        Ok(Self {
            queue: array(256, Default::default())?,
            columns: array(plan.overview_columns, Default::default())?,
            bins: array(libapta::analysis::BIN_CAPACITY, Default::default())?,
            flux: array(libapta::analysis::BIN_CAPACITY, 0.)?,
            bytes: array(4096, 0)?,
            pcm: array(512, 0.)?,
            io: array(512, 0)?,
            beats: [array(BEATS, beat)?, array(BEATS, beat)?],
            waveform: [
                array(plan.overview_columns, wave)?,
                array(plan.overview_columns, wave)?,
            ],
        })
    }
}
#[embassy_executor::task]
pub(crate) async fn owner_task() {
    let Ok(storage) = Storage::new() else {
        report_status(Status::Failed, Reason::Allocation);
        return;
    };
    let lease = loop {
        let media = crate::usb0_msc::analysis_media_snapshot();
        if media.is_available() {
            if let Some(lease) = media.lease() {
                break lease;
            }
        }
        embassy_time::Timer::after_millis(20).await;
    };
    let Some(endpoint) = crate::usb0_msc::Usb0AnalysisTransport::take() else {
        report_status(Status::Failed, Reason::Contract);
        return;
    };
    // Leak once, only after EVERY fallible allocation and endpoint acquisition.
    // These fixed arrays belong to this never-restarted owner, across all jobs.
    let [a, b] = storage.beats;
    let [wa, wb] = storage.waveform;
    let workspace = Workspace {
        local: LocalTempoBuffers {
            queue: storage.queue.leak(),
            columns: storage.columns.leak(),
            bins: storage.bins.leak(),
            flux: storage.flux.leak(),
        },
        bytes: storage.bytes.leak().try_into().expect("fixed byte scratch"),
        pcm: storage.pcm.leak().try_into().expect("fixed PCM scratch"),
    };
    let client = Client::new(endpoint, lease, storage.io.leak());
    let mut service = Service::new(
        Owner::new(client, Default::default(), workspace),
        [
            Output {
                beats: a.leak(),
                waveform: wa.leak(),
            },
            Output {
                beats: b.leak(),
                waveform: wb.leak(),
            },
        ],
    );
    READY.store(true, Ordering::Release);
    PROGRESS.signal(());
    service
        .run(&mut OwnerPort, LIMITS, FRAMES_PER_COLUMN, 0x07e0)
        .await
}
