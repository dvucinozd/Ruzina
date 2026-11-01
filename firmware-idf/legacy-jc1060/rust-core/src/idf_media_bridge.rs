//! Concrete task-only IDF transport. C owns the SD host, DMA bounce allocation
//! and static FreeRTOS task handles; Rust owns requests, buffers and completion
//! gating. No Rust object layout crosses the C ABI.

use crate::media_bridge::{
    Binding, ReadClient, ReadPort, ReadQueue, ReadRequest, ReadResult, Ticket,
};
use core::cell::UnsafeCell;
use core::future::Future;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use ruzina_media_block::BlockGeometry;
use ruzina_media_io::OwnedBlockDevice;
use ruzina_media_session::{MediaHandle, MediaSourceId};

type Buffer = [u8; 512];
struct State {
    queue: ReadQueue<Buffer>,
    binding: Binding,
    active: Option<ReadRequest<Buffer>>,
    waker: Option<Waker>,
}
struct Shared(UnsafeCell<Option<State>>);
// SAFETY: all accesses use the same C portMUX critical section. Only task
// context may call these exports. The static storage outlives every request.
unsafe impl Sync for Shared {}
static SHARED: Shared = Shared(UnsafeCell::new(None));

unsafe extern "C" {
    fn ruzina_bridge_lock();
    fn ruzina_bridge_unlock();
    fn ruzina_bridge_notify_worker();
    fn ruzina_bridge_notify_client();
    fn ruzina_bridge_wait_client();
    fn ruzina_bridge_delay();
    fn ruzina_bridge_report(pass: u32, event: u32, value: u32);
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: paired with the same task's enter in with_state.
        unsafe { ruzina_bridge_unlock() }
    }
}
fn with_state<R>(f: impl FnOnce(&mut Option<State>) -> R) -> R {
    // SAFETY: task-only API; no reference escapes the closure or crosses C.
    unsafe { ruzina_bridge_lock() };
    let _guard = Guard;
    // SAFETY: the portMUX excludes both cores. State cannot escape this closure.
    f(unsafe { &mut *SHARED.0.get() })
}

struct Port;
impl ReadPort for Port {
    type Buffer = Buffer;
    fn is_current(&self, binding: Binding) -> bool {
        with_state(|state| state.as_ref().is_some_and(|s| s.queue.is_current(binding)))
    }
    fn submit(
        &mut self,
        binding: Binding,
        first: u64,
        count: u32,
        buffer: Buffer,
    ) -> Result<Ticket, (crate::media_bridge::Error, Buffer)> {
        // Keep the bounded owned result in this task's stack frame; no heap or
        // second large Result value is returned through the lock helper.
        let mut submitted = None;
        with_state(|state| {
            submitted = Some(match state.as_mut() {
                Some(s) => s.queue.submit(binding, first, count, buffer),
                None => Err((crate::media_bridge::Error::Stale, buffer)),
            });
        });
        let result = submitted.expect("locked submission always assigns result");
        if result.is_ok() {
            // SAFETY: no queue lock is held; notification is a retained hint.
            unsafe { ruzina_bridge_notify_worker() };
        }
        result
    }
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Buffer>> {
        // Our static task waker has no allocation/locks/drop callback work.
        let waker = cx.waker().clone();
        with_state(|state| {
            let s = state.as_mut().expect("transport remains initialized");
            if let Some(result) = s.queue.take_completion(ticket) {
                Poll::Ready(result)
            } else {
                // Same lock as completion: no readiness/waker-registration gap.
                s.waker = Some(waker);
                Poll::Pending
            }
        })
    }
}

fn wake_client() {
    // SAFETY: fixed client task persists for the life of this diagnostic.
    unsafe { ruzina_bridge_notify_client() }
}
unsafe fn clone_waker(_: *const ()) -> RawWaker {
    raw_waker()
}
unsafe fn wake(_: *const ()) {
    wake_client();
}
unsafe fn wake_by_ref(_: *const ()) {
    wake_client();
}
unsafe fn drop_waker(_: *const ()) {}
static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_waker, wake, wake_by_ref, drop_waker);
fn raw_waker() -> RawWaker {
    RawWaker::new(core::ptr::null(), &VTABLE)
}

/// Runs only on the dedicated diagnostic client task. Pending sleeps that task,
/// not a product async executor. Task notifications persist across poll/wait.
fn block_on<F: Future>(future: F) -> F::Output {
    // SAFETY: static vtable owns no pointer/refcount and only notifies the static
    // C client task. clone/wake/drop remain valid until application reset.
    let waker = unsafe { Waker::from_raw(raw_waker()) };
    let mut cx = Context::from_waker(&waker);
    let mut future = core::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                // SAFETY: client task only; bounded wait also permits repolling.
                unsafe { ruzina_bridge_wait_client() };
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_bridge_init(sectors: u32) -> i32 {
    with_state(|state| {
        if state.is_some() {
            return -1;
        }
        let mut queue = ReadQueue::new();
        let Ok(binding) = queue.activate(
            MediaSourceId::new(0x5344_0001).unwrap(),
            MediaHandle::new(1).unwrap(),
            BlockGeometry {
                block_size: 512,
                block_count: sectors.into(),
            },
        ) else {
            return -2;
        };
        *state = Some(State {
            queue,
            binding,
            active: None,
            waker: None,
        });
        0
    })
}

/// # Safety
/// `first` and `count` are valid, aligned, disjoint u32 outputs on the single C
/// worker stack for this call; no ISR, other worker or reentrant caller exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_bridge_take(first: *mut u32, count: *mut u32) -> i32 {
    if first.is_null()
        || count.is_null()
        || first == count
        || !(first as usize).is_multiple_of(4)
        || !(count as usize).is_multiple_of(4)
    {
        return -1;
    }
    with_state(|state| {
        let Some(s) = state.as_mut() else {
            return -1;
        };
        if s.active.is_some() {
            return -1;
        }
        let Some(request) = s.queue.take_request() else {
            return 0;
        };
        // SAFETY: caller's task-stack outputs satisfy the contract above. Queue
        // admission limits geometry to u32 sectors and buffer to one sector.
        unsafe {
            *first = request.first_block() as u32;
            *count = request.block_count();
        }
        s.active = Some(request);
        1
    })
}

/// # Safety
/// On success `data` points to `bytes` initialized DMA-complete bytes owned by
/// the C worker, aligned to4 and disjoint from all Rust static/task storage.
/// The caller retains allocation/free ownership; Rust copies synchronously and
/// never retains the pointer. On IO failure data may be null and bytes is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_bridge_complete(
    status: i32,
    data: *const u8,
    bytes: u32,
) -> i32 {
    if status == 0
        && (data.is_null()
            || !(data as usize).is_multiple_of(4)
            || bytes != 512
            || (data as usize).checked_add(bytes as usize).is_none())
    {
        return -1;
    }
    let (result, waker) = with_state(|state| {
        let Some(s) = state.as_mut() else {
            return (-1, None);
        };
        let Some(mut request) = s.active.take() else {
            return (-1, None);
        };
        let io_result = if status == 0 {
            // SAFETY: C owns a DMA-complete buffer satisfying the contract above.
            let input = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
            request.buffer_mut().copy_from_slice(input);
            Ok(bytes as usize)
        } else {
            s.queue.invalidate();
            Err(status)
        };
        let result = match s.queue.complete(request, io_result) {
            Ok(()) => 0,
            Err(request) => {
                s.active = Some(request);
                -1
            }
        };
        (result, s.waker.take())
    });
    if let Some(waker) = waker {
        waker.wake();
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_bridge_invalidate() -> i32 {
    let (safe, waker) = with_state(|state| {
        let Some(s) = state.as_mut() else {
            return (true, None);
        };
        s.queue.invalidate();
        (
            s.queue.can_deinitialize() && s.active.is_none(),
            s.waker.take(),
        )
    });
    if let Some(waker) = waker {
        waker.wake();
    }
    i32::from(safe)
}

#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_bridge_run() -> i32 {
    let Some(binding) = with_state(|state| state.as_ref().map(|s| s.binding)) else {
        return -1;
    };
    let mut client = ReadClient::new(Port, binding);
    let mut buffer = [0u8; 512];
    let mut expected = None;
    for pass in 1..=u32::MAX {
        let mut hash = 2166136261u32;
        for sector in 0..8 {
            buffer = match block_on(client.read_blocks_owned(sector, 1, buffer)) {
                Ok(buffer) => buffer,
                Err((_error, _returned)) => {
                    // SAFETY: report only POD scalars, no formatting ABI.
                    unsafe { ruzina_bridge_report(pass, 2, sector as u32) };
                    return -2;
                }
            };
            for &byte in &buffer {
                hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
            }
        }
        if expected.is_some_and(|expected| expected != hash) {
            unsafe { ruzina_bridge_report(pass, 3, hash) };
            return -3;
        }
        expected = Some(hash);
        // SAFETY: POD telemetry and task-only cooperative delay.
        unsafe {
            ruzina_bridge_report(pass, 1, hash);
            ruzina_bridge_delay();
        }
    }
    0
}

#[cfg(test)]
mod tests;

#[cfg(feature = "idf-exfat-fixture")]
unsafe extern "C" {
    fn ruzina_bridge_fs_report(pass: u32, event: u32, value: u32);
    fn ruzina_bridge_fixture_yield();
}

#[cfg(feature = "idf-exfat-fixture")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_exfat_run() -> i32 {
    let Some(binding) = with_state(|state| state.as_ref().map(|s| s.binding)) else {
        return -1;
    };
    let current = || Port.is_current(binding);
    let mut client = ReadClient::new(Port, binding);
    let mut volume = match block_on(crate::exfat_fixture::mount(
        &mut client,
        binding.lease,
        &current,
    )) {
        Ok(volume) => volume,
        Err(error) => {
            // SAFETY: diagnostic telemetry uses three u32 POD values only.
            unsafe {
                ruzina_bridge_fs_report(0, 4, error as u32);
            }
            return -4;
        }
    };
    // SAFETY: queue geometry bounds all partition LBAs to u32. No borrowed
    // Rust object or filesystem layout is passed to the C logging function.
    unsafe {
        ruzina_bridge_fs_report(0, 1, volume.device().range().first_block() as u32);
    }
    for pass in 1..=u32::MAX {
        unsafe {
            ruzina_bridge_fs_report(pass, 2, crate::exfat_fixture::SIZE);
        }
        let result = block_on(crate::exfat_fixture::verify(&mut volume, &current, || {
            // SAFETY: dedicated task only; deliberately yield one tick/16KiB.
            unsafe {
                ruzina_bridge_fixture_yield();
            }
        }));
        if let Err(error) = result {
            unsafe {
                ruzina_bridge_fs_report(pass, 4, error as u32);
            }
            return -5;
        }
        if !current() {
            return -6;
        }
        unsafe {
            ruzina_bridge_fs_report(pass, 3, crate::exfat_fixture::SIZE);
            ruzina_bridge_delay();
        }
    }
    0
}

#[cfg(feature = "idf-media-load")]
unsafe extern "C" {
    fn ruzina_bridge_load_report(pass: u32, event: u32, value: u32);
    /// Lends UTF-8 bytes for the duration of the call; C copies a bounded
    /// prefix for the status display and keeps no pointer.
    fn ruzina_bridge_load_title(
        index: u32,
        artist: *const u8,
        artist_len: u32,
        title: *const u8,
        title_len: u32,
    );
    /// The selected track's catalog fields and analysis summary, with its
    /// preview peaks (0..255, one per PWAV column). Bytes are lent for the
    /// call only; zero counts and no peaks mean no analysis was loaded.
    fn ruzina_bridge_load_detail(
        index: u32,
        bpm_x100: u32,
        duration_ms: u32,
        key: *const u8,
        key_len: u32,
        beats: u32,
        cues: u32,
        memory_cues: u32,
        peaks: *const u8,
        peaks_len: u32,
    );
}

/// The track the display selected (`u32::MAX` none) and whether its detail is
/// still owed. Written by the C touch task, read by the load client task.
#[cfg(feature = "idf-media-load")]
static LOAD_SELECTED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
#[cfg(feature = "idf-media-load")]
static LOAD_DETAIL_OWED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Select a catalog track for the detail view. The load task preflights it
/// out of turn at its next track boundary and reports the detail once.
#[cfg(feature = "idf-media-load")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_load_select(index: u32) {
    use core::sync::atomic::Ordering;
    LOAD_SELECTED.store(index, Ordering::Relaxed);
    LOAD_DETAIL_OWED.store(true, Ordering::Release);
}

/// Preview peaks reported per selected track; PWAV previews hold 400 columns.
#[cfg(feature = "idf-media-load")]
const DETAIL_PEAKS: usize = 400;

/// Load-path telemetry events; values are scalars only.
#[cfg(feature = "idf-media-load")]
mod load_event {
    /// Mounted; value is the partition's first LBA.
    pub const MOUNTED: u32 = 1;
    /// Catalog published; value is the track count.
    pub const CATALOG: u32 = 2;
    /// Playlist nodes published with that catalog.
    pub const PLAYLISTS: u32 = 3;
    /// One track's preflight; value is `index << 8 | outcome`.
    pub const TRACK: u32 = 4;
    /// Pass complete; value is the number of tracks loaded with analysis.
    pub const PASS: u32 = 5;
    /// Failure; value is the stage (1 mount, 2 allocation, 3 import, 4 stale).
    pub const FAILED: u32 = 6;
}

/// Mount, import `export.pdb` and preflight every catalog track, repeatedly.
/// Track outcomes: 0 analysis loaded, 1 no analysis, 2 source error, 3 stale.
/// Read-only: the SD owner denies every write command.
#[cfg(feature = "idf-media-load")]
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_load_run() -> i32 {
    use crate::media_load::{Error, import_catalog, preflight, volume_identity};
    use alloc::boxed::Box;
    use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId};
    use ruzina_media_load::pdb::{PdbNameStage, VecCatalogStage};

    // SAFETY: diagnostic telemetry passes three u32 POD values only.
    let report = |pass, event, value| unsafe { ruzina_bridge_load_report(pass, event, value) };
    let Some(binding) = with_state(|state| state.as_ref().map(|s| s.binding)) else {
        return -1;
    };
    let current = || Port.is_current(binding);
    let mut client = ReadClient::new(Port, binding);
    let mut volume = match block_on(crate::exfat_fixture::mount(
        &mut client,
        binding.lease,
        &current,
    )) {
        Ok(volume) => volume,
        Err(error) => {
            report(0, load_event::FAILED, 1 << 8 | error as u32);
            return -4;
        }
    };
    report(
        0,
        load_event::MOUNTED,
        volume.device().range().first_block() as u32,
    );
    let identity = volume_identity(&volume);
    // Capacity is reserved once, in PSRAM, before any page is read.
    let (Ok(mut names), Ok(mut stage)) = (PdbNameStage::try_new(), VecCatalogStage::try_new())
    else {
        report(0, load_event::FAILED, 2 << 8);
        return -7;
    };
    // With deck playback the first pass hands the task to `serve_decks`.
    #[cfg_attr(feature = "idf-deck-play", allow(clippy::never_loop))]
    for pass in 1..=u32::MAX {
        stage.prepare_generation(binding.lease);
        names.clear();
        // The import and preflight futures hold owned read buffers across
        // awaits (tens of KiB). They live in the PSRAM heap, as the native
        // tasks keep theirs in static memory, not on the 48 KiB task stack.
        let probe = match block_on(Box::pin(import_catalog(
            &mut volume,
            identity,
            &current,
            &mut names,
            &mut stage,
        ))) {
            Ok(probe) => probe,
            Err(_) => {
                stage.invalidate(binding.lease);
                report(pass, load_event::FAILED, 3 << 8);
                return -8;
            }
        };
        let Some(snapshot) = stage.current() else {
            report(pass, load_event::FAILED, 3 << 8);
            return -8;
        };
        let (publication, tracks) = (
            CatalogPublicationId {
                lease: snapshot.lease(),
                revision: snapshot.revision(),
            },
            snapshot.len(),
        );
        report(pass, load_event::CATALOG, tracks as u32);
        if pass == 1 {
            for (index, track) in (0..tracks).filter_map(|i| snapshot.get(i).map(|t| (i, t))) {
                let (artist, title) = (track.artist.as_str(), track.title.as_str());
                // SAFETY: both slices outlive the call; C copies and keeps no pointer.
                unsafe {
                    ruzina_bridge_load_title(
                        index as u32,
                        artist.as_ptr(),
                        artist.len() as u32,
                        title.as_ptr(),
                        title.len() as u32,
                    )
                };
            }
        }
        if let Some(playlists) = probe.playlists {
            report(pass, load_event::PLAYLISTS, playlists.nodes);
        }
        let published = |candidate: CatalogPublicationId| {
            candidate == publication
                && stage.current().is_some_and(|snapshot| {
                    snapshot.lease() == candidate.lease && snapshot.revision() == candidate.revision
                })
        };
        // One track's preflight; `detail` also reports the selected-track view.
        // Returns the outcome, or `None` when the catalog snapshot is gone.
        let run = |volume: &mut crate::exfat_fixture::Volume<_>,
                   index: usize,
                   detail: bool|
         -> Option<u32> {
            let track = stage
                .current()
                .and_then(|snapshot| snapshot.get(index).copied())?;
            let candidate = CatalogLoadCandidate {
                publication,
                index: index as u16,
                track,
            };
            let prepared = block_on(Box::pin(preflight(
                volume, candidate, pass, &current, &published,
            )));
            if detail && prepared.is_ok() {
                let view = prepared
                    .as_ref()
                    .ok()
                    .and_then(|p| p.publication.as_ref())
                    .map(|publication| publication.view());
                let mut peaks = [0u8; DETAIL_PEAKS];
                let (mut beats, mut cues, mut memory, mut columns) = (0, 0, 0, 0);
                if let Some(view) = &view {
                    beats = view.beat_grid().map_or(0, |grid| grid.beats().len());
                    (cues, memory) = (view.cues().len(), view.memory_cues().len());
                    for (peak, column) in peaks.iter_mut().zip(view.preview()) {
                        *peak = (column.peak >> 8) as u8;
                        columns += 1;
                    }
                }
                let key = track.key.as_str();
                // SAFETY: `key` and `peaks` outlive the call; C copies and
                // keeps no pointer. All other arguments are scalars.
                unsafe {
                    ruzina_bridge_load_detail(
                        index as u32,
                        track.bpm_x100,
                        track.duration_ms,
                        key.as_ptr(),
                        key.len() as u32,
                        beats as u32,
                        cues as u32,
                        memory as u32,
                        peaks.as_ptr(),
                        columns,
                    )
                };
            }
            Some(match prepared {
                Ok(prepared) if prepared.publication.is_some() => 0,
                Ok(_) => 1,
                Err(Error::Source) => 2,
                Err(Error::Stale) => 3,
            })
        };
        let mut loaded = 0u32;
        for index in 0..tracks {
            use core::sync::atomic::Ordering;
            // A selection is served out of turn, before the next track.
            if LOAD_DETAIL_OWED.swap(false, Ordering::Acquire) {
                let selected = LOAD_SELECTED.load(Ordering::Relaxed) as usize;
                if selected < tracks && run(&mut volume, selected, true) == Some(3) {
                    report(pass, load_event::TRACK, (selected as u32) << 8 | 3);
                    report(pass, load_event::FAILED, 4 << 8);
                    return -6;
                }
            }
            let Some(outcome) = run(&mut volume, index, false) else {
                report(pass, load_event::FAILED, 4 << 8);
                return -6;
            };
            loaded += u32::from(outcome == 0);
            if pass == 1 || outcome == 3 {
                report(pass, load_event::TRACK, (index as u32) << 8 | outcome);
            }
            if outcome == 3 {
                report(pass, load_event::FAILED, 4 << 8);
                return -6;
            }
            // SAFETY: dedicated task only; yield one tick between tracks.
            unsafe { ruzina_bridge_fixture_yield() };
        }
        report(pass, load_event::PASS, loaded);
        if !current() {
            return -6;
        }
        // With deck playback the first pass is the only one: the task then
        // serves selections and decodes deck 1 instead of re-reading the card.
        #[cfg(feature = "idf-deck-play")]
        return serve_decks(
            pass,
            &mut volume,
            &current,
            |volume, index| run(volume, index, true),
            |index| {
                stage
                    .current()
                    .and_then(|snapshot| snapshot.get(index as usize).map(|t| t.audio_path))
                    .map(CatalogPath)
            },
            tracks,
        );
        // SAFETY: task-only cooperative delay between passes.
        #[cfg(not(feature = "idf-deck-play"))]
        unsafe {
            ruzina_bridge_delay()
        };
    }
    0
}

#[cfg(feature = "idf-deck-play")]
struct CatalogPath(ruzina_media_catalog::CatalogText<{ ruzina_media_catalog::CATALOG_PATH_MAX }>);

#[cfg(feature = "idf-deck-play")]
impl AsRef<str> for CatalogPath {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

/// Deck audio files on the exFAT volume, each operation under the live lease.
#[cfg(feature = "idf-deck-play")]
struct VolumeFiles<'a, D: OwnedBlockDevice<Buffer = Buffer>, C: Fn() -> bool> {
    volume: &'a mut crate::exfat_fixture::Volume<D>,
    current: &'a C,
}

#[cfg(feature = "idf-deck-play")]
impl<D: OwnedBlockDevice<Buffer = Buffer>, C: Fn() -> bool> crate::deck_play::SourceFiles
    for VolumeFiles<'_, D, C>
{
    type File = <crate::exfat_fixture::Volume<D> as ruzina_media_fs::AsyncFileSystem>::File;

    fn open(&mut self, path: &str) -> Option<Self::File> {
        use ruzina_media_fs::AsyncFileSystem;
        if !(self.current)() {
            return None;
        }
        block_on(self.volume.open_read(path)).ok()
    }

    fn len(&self, file: &Self::File) -> u64 {
        ruzina_media_fs::FileHandle::len(file)
    }

    fn read_at(&mut self, file: &mut Self::File, offset: u64, out: &mut [u8]) -> Option<usize> {
        use ruzina_media_fs::AsyncFileSystem;
        if !(self.current)() {
            return None;
        }
        let volume = &mut *self.volume;
        block_on(async {
            volume.seek_absolute(file, offset).await.ok()?;
            volume.read(file, out).await.ok()
        })
    }

    fn close(&mut self, file: Self::File) {
        use ruzina_media_fs::AsyncFileSystem;
        let _ = block_on(self.volume.close_file(file));
    }
}

/// Deck playback telemetry, through the load report.
#[cfg(feature = "idf-deck-play")]
mod play_event {
    /// Deck 1 decodes a track; value is `index << 8 | codec` (1 WAV, 2 MP3, 3 FLAC).
    pub const READY: u32 = 7;
    /// The track's source rate in Hz.
    pub const RATE: u32 = 8;
    /// The track failed to load; value is `index << 8 | stage`
    /// (1 path, 2 open, 3 format, 4 decode).
    pub const FAILED: u32 = 9;
    /// The track ended; value is `index << 8 | faulted`.
    pub const ENDED: u32 = 10;
}

/// After the first pass: serve detail selections and the deck 1 producer
/// until the lease ends. Returns only on a lost lease or allocation failure.
#[cfg(feature = "idf-deck-play")]
fn serve_decks<D: OwnedBlockDevice<Buffer = Buffer>, C: Fn() -> bool, P: AsRef<str>>(
    pass: u32,
    volume: &mut crate::exfat_fixture::Volume<D>,
    current: &C,
    detail: impl Fn(&mut crate::exfat_fixture::Volume<D>, usize) -> Option<u32>,
    path: impl Fn(u32) -> Option<P>,
    tracks: usize,
) -> i32 {
    use crate::deck_play::{COMMANDS, Event, Producer, RING, attach_ring};
    use core::sync::atomic::Ordering;
    // SAFETY: diagnostic telemetry passes three u32 POD values only.
    let report = |event, value| unsafe { ruzina_bridge_load_report(pass, event, value) };
    if !attach_ring() {
        report(load_event::FAILED, 2 << 8 | 1);
        return -7;
    }
    let mut producer = Producer::new();
    loop {
        if LOAD_DETAIL_OWED.swap(false, Ordering::Acquire) {
            let selected = LOAD_SELECTED.load(Ordering::Relaxed) as usize;
            if selected < tracks && detail(volume, selected) == Some(3) {
                report(load_event::FAILED, 4 << 8);
                return -6;
            }
        }
        let mut files = VolumeFiles {
            volume: &mut *volume,
            current,
        };
        let (busy, event) = producer.poll(&mut files, &RING, &COMMANDS, &path);
        match event {
            Some(Event::Ready { index, codec, rate }) => {
                report(play_event::READY, index << 8 | codec);
                report(play_event::RATE, rate);
            }
            Some(Event::Failed { index, stage }) => report(play_event::FAILED, index << 8 | stage),
            Some(Event::Ended { index, faulted }) => {
                report(play_event::ENDED, index << 8 | u32::from(faulted))
            }
            None => {}
        }
        if !current() {
            report(load_event::FAILED, 4 << 8);
            return -6;
        }
        if !busy {
            // SAFETY: dedicated task only; one tick while the ring is full.
            unsafe { ruzina_bridge_fixture_yield() };
        }
    }
}
