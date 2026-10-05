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

// This task exists only to compile the complete concrete coroutine and Embassy
// pool. main never calls/spawns it, and it cannot select the production provider.
#[embassy_executor::task]
pub(crate) async fn compile_owner_task(
    owner: &'static mut AnalysisOwner<'static>,
    bank: &'static mut Bank,
) {
    let _ = run_selected(
        owner,
        bank,
        StreamLimits {
            object_bytes: 2_000_000,
            duration_ms: 40_000,
            scan_steps: 20_000,
            processing_steps: 40_000,
        },
        256,
    )
    .await;
}

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
