//! Compile-only native consumer owner. No boot call, task spawn or provider switch.
// SPDX-License-Identifier: Apache-2.0
use crate::usb0_msc::{Usb0AnalysisTransport, Usb0FsOperationError};
use alloc::vec::Vec;
use libapta::{
    analysis::{BIN_CAPACITY, OnsetBin},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{
    owner::{Delivery, Output, Owner, Workspace},
    source::SourceError,
    storage::{LocalTempoBuffers, LocalTempoStorage},
    streaming::StreamLimits,
};
use pajoniiir_media_fs::analysis::{Client, Error};
use pajoniiir_media_session::MediaLease;
use pajoniiir_track_analysis::Beat;
use static_cell::StaticCell;

pub(crate) type AnalysisOwner<'a> = Owner<'a, Usb0AnalysisTransport>;
// Internal SRAM owned broker buffer, following USB0's existing StaticCell pattern.
// It is not in PSRAM and is never reclaimed on cancellation. DMA/cache still
// requires physical qualification of the existing broker/backend path.
static IO: StaticCell<[u8; 4096]> = StaticCell::new();

/// Uses the foundation PSRAM allocator, initialized before tasks in main.
/// Allocate once, then reborrow for all jobs. Capacity is explicit caller policy;
/// no resize/allocation occurs during analysis. Two output banks are separate so
/// an existing Deck Pin can outlive/reject a replacement job.
pub(crate) struct Storage {
    queue: Vec<NormalizedSample>,
    columns: Vec<libapta::WaveformColumn>,
    bins: Vec<OnsetBin>,
    flux: Vec<f32>,
    bytes: Vec<u8>,
    pcm: Vec<f32>,
}
pub(crate) struct Bank {
    beats: Vec<Beat>,
    waveform: Vec<pajoniiir_waveform::WaveformColumn>,
}
fn filled<T: Clone>(count: usize, value: T) -> Result<Vec<T>, ()> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| ())?;
    values.resize(count, value);
    Ok(values)
}
impl Storage {
    pub(crate) fn try_new(plan: LocalTempoStorage) -> Result<Self, ()> {
        Ok(Self {
            queue: filled(plan.queue_frames, NormalizedSample::default())?,
            columns: filled(plan.overview_columns, libapta::WaveformColumn::default())?,
            bins: filled(BIN_CAPACITY, OnsetBin::default())?,
            flux: filled(BIN_CAPACITY, 0.)?,
            bytes: filled(4096, 0)?,
            pcm: filled(512, 0.)?,
        })
    }
    /// Allocate Storage and both Banks first; claim only after all allocations succeed.
    pub(crate) fn claim_service<'a>(
        &'a mut self,
        lease: MediaLease,
        a: &'a mut Bank,
        b: &'a mut Bank,
    ) -> Option<AnalysisService<'a>> {
        Some(Service::new(self.claim(lease)?, [a.output(), b.output()]))
    }
    /// Only the actual once-claimed USB0 Analysis endpoint can construct this
    /// concrete owner. Call after storage allocation succeeds. Keep it alive.
    pub(crate) fn claim(&mut self, lease: MediaLease) -> Option<AnalysisOwner<'_>> {
        let transport = Usb0AnalysisTransport::take()?;
        Some(Owner::new(
            Client::new(transport, lease, IO.init_with(|| [0; 4096])),
            Default::default(),
            Workspace {
                local: LocalTempoBuffers {
                    queue: &mut self.queue,
                    columns: &mut self.columns,
                    bins: &mut self.bins,
                    flux: &mut self.flux,
                },
                bytes: self.bytes.as_mut_slice().try_into().unwrap(),
                pcm: self.pcm.as_mut_slice().try_into().unwrap(),
            },
        ))
    }
}
impl Bank {
    pub(crate) fn try_new(plan: LocalTempoStorage) -> Result<Self, ()> {
        Ok(Self {
            beats: filled(
                plan.beats_per_bank,
                Beat {
                    time_ms: 0,
                    phase: 0,
                    bpm_x100: 0,
                },
            )?,
            waveform: filled(
                plan.overview_columns,
                pajoniiir_waveform::WaveformColumn {
                    peak: 0,
                    color_rgb565: 0,
                },
            )?,
        })
    }
    pub(crate) fn output(&mut self) -> Output<'_> {
        Output {
            beats: &mut self.beats,
            waveform: &mut self.waveform,
        }
    }
}
/// Concrete USB0 job entry, borrowed from a long-lived caller. The catalog owner
/// selects synchronously before this await; no catalog/lifecycle lock spans I/O.
/// Drop this future to replace/cancel, then retain and recover the SAME owner.
/// Delivery must use a fresh media snapshot and existing Pin::load/upgrade.
pub(crate) async fn run_selected<'b>(
    owner: &mut AnalysisOwner<'_>,
    bank: &'b mut Bank,
    limits: StreamLimits,
    frames_per_column: u32,
) -> Result<Delivery<'b>, SourceError<Error<Usb0FsOperationError>>> {
    let media = crate::usb0_msc::analysis_media_snapshot();
    owner
        .run(&media, limits, frames_per_column, bank.output(), 0x07e0)
        .await
}

// Diagnostic channel capacities, not approved production limits.
use core::cell::RefCell;
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::{Channel, TrySendError},
};
use pajoniiir_apta_adapter::service::{Authority, Command, Port, Retained, Service, Status};
pub(crate) type AnalysisService<'a> = Service<'a, Usb0AnalysisTransport>;
static COMMANDS: Channel<CriticalSectionRawMutex, Command<'static>, 2> = Channel::new();
static OUTPUTS: Channel<CriticalSectionRawMutex, Retained<'static>, 2> = Channel::new();
static DELIVERY: CriticalSectionMutex<RefCell<(Authority, Option<Status>)>> =
    CriticalSectionMutex::new(RefCell::new((Authority::EMPTY, None)));

pub(crate) struct OwnerPort;
impl Port<'static> for OwnerPort {
    async fn receive(&mut self) -> Command<'static> {
        COMMANDS.receive().await
    }
    fn media(&self) -> pajoniiir_media_session::MediaSession {
        crate::usb0_msc::analysis_media_snapshot()
    }
    fn authority(&mut self, value: Authority) {
        DELIVERY.lock(|s| s.borrow_mut().0 = value);
    }
    fn publish(&mut self, value: Retained<'static>) -> Result<(), Output<'static>> {
        OUTPUTS
            .try_send(value)
            .map_err(|TrySendError::Full(value)| value.reject().ok().unwrap())
    }
    fn report(&mut self, value: Status) {
        DELIVERY.lock(|s| s.borrow_mut().1 = Some(value));
    }
}
/// Ownership-returning bounded enqueue; caller retries or reports refusal.
#[allow(clippy::result_large_err)]
pub(crate) fn command(value: Command<'static>) -> Result<(), Command<'static>> {
    COMMANDS
        .try_send(value)
        .map_err(|TrySendError::Full(value)| value)
}
pub(crate) fn status() -> Option<Status> {
    DELIVERY.lock(|s| s.borrow().1)
}
/// Actual product owner supplies its live Deck state. The returned bank must be
/// kept with that Deck and used through its short neutral views. Rejected output
/// is still owned and must be retired/recycled explicitly. No production call.
#[allow(clippy::result_large_err)]
pub(crate) fn take_delivery(
    product: &crate::product_runtime::ProductRuntime,
) -> Option<Result<Retained<'static>, Retained<'static>>> {
    let mut output = OUTPUTS.try_receive().ok()?;
    Some(DELIVERY.lock(|s| {
        let media = crate::usb0_msc::analysis_media_snapshot();
        match output.accept(s.borrow().0, &media, product.deck()) {
            Ok(()) => Ok(output),
            Err(_) => Err(output),
        }
    }))
}
/// Caller retains the returned Command on queue refusal. Playing/pending banks
/// remain retained; they cannot be converted to a recycle command.
#[allow(clippy::result_large_err)]
pub(crate) fn retirement(
    output: Retained<'static>,
    product: &crate::product_runtime::ProductRuntime,
) -> Result<Command<'static>, Retained<'static>> {
    output.retire(product.deck()).map(Command::Recycle)
}

// Never spawned by main. All persistent resources live in the supplied service.
#[embassy_executor::task]
pub(crate) async fn compile_owner_task(service: &'static mut AnalysisService<'static>) {
    service
        .run(
            &mut OwnerPort,
            StreamLimits {
                object_bytes: 2_000_000,
                duration_ms: 40_000,
                scan_steps: 20_000,
                processing_steps: 40_000,
            },
            256,
            0x07e0,
        )
        .await;
}
#[unsafe(no_mangle)]
fn analysis_service_future_size(
    service: &mut AnalysisService<'static>,
    port: &mut OwnerPort,
) -> usize {
    core::mem::size_of_val(&service.run(port, StreamLimits::UNLIMITED, 256, 0x07e0))
}
#[used]
#[unsafe(no_mangle)]
static ANALYSIS_SERVICE_LAYOUT: [usize; 7] = [
    core::mem::size_of::<AnalysisService<'static>>(),
    core::mem::size_of::<Command<'static>>(),
    core::mem::size_of::<Retained<'static>>(),
    core::mem::size_of::<Authority>(),
    core::mem::size_of::<Channel<CriticalSectionRawMutex, Command<'static>, 2>>(),
    core::mem::size_of::<Channel<CriticalSectionRawMutex, Retained<'static>, 2>>(),
    core::mem::size_of::<CriticalSectionMutex<RefCell<(Authority, Option<Status>)>>>(),
];

// Compiler-observed owner metadata only; PSRAM backing capacities are additional.
#[used]
#[unsafe(no_mangle)]
static ANALYSIS_OWNER_LAYOUT: [usize; 4] = [
    core::mem::size_of::<AnalysisOwner<'static>>(),
    core::mem::size_of::<Storage>(),
    core::mem::size_of::<Bank>(),
    4096,
];
#[unsafe(no_mangle)]
fn analysis_owner_future_size(owner: &mut AnalysisOwner<'_>, bank: &mut Bank) -> usize {
    core::mem::size_of_val(&run_selected(owner, bank, StreamLimits::UNLIMITED, 256))
}
