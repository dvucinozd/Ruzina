use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::select;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use ruzina_audio_runtime::decode::SourceDescriptor;
use ruzina_controller_session::DeckId;
use ruzina_hot_cues::HotCueBank;
use ruzina_media_catalog::CatalogLoadCandidate;
use ruzina_media_fs::{BoundedPath, EntryKind, FileHandle};
use ruzina_rekordbox::anlz_analysis::{
    ANLZ_READ_BYTES, AnalysisFileKind, AnlzImporter, extended_path,
};
use ruzina_track_analysis::owned::AnalysisIdentity;
use static_cell::StaticCell;

#[cfg(feature = "wav-audio-source-bringup")]
use crate::deck_audio_source::{
    activate_prepared_source, prepare_audio_source, request_source_release,
};

use crate::{
    deck_effect_owners::{HotCueLoadResult, load_persisted_hot_cues},
    product_runtime::{LibraryEffect, ProductAnalysisLoad, ProductRuntime, load_gate},
    usb0_library::{
        USB0_LIBRARY_COMMANDS, USB0_LIBRARY_COMPLETIONS, Usb0LibraryCommand, Usb0LibraryCompletion,
        catalog_publication_matches, current_media_lease, wait_for_media_lease_change,
    },
    usb0_msc::{
        Usb0FilesystemClient, Usb0FsCompletion, Usb0FsFileHandle, Usb0FsRequest, Usb0FsSubmitResult,
    },
};

const LIBRARY_DECK_QUEUE_DEPTH: usize = 1;
const ANLZ_SCAN_BUFFER_BYTES: usize = ANLZ_READ_BYTES;

static LIBRARY_DECK_COMMANDS: Channel<
    CriticalSectionRawMutex,
    LibraryEffect,
    LIBRARY_DECK_QUEUE_DEPTH,
> = Channel::new();
static LIBRARY_DECK_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    LibraryDeckCompletion,
    LIBRARY_DECK_QUEUE_DEPTH,
> = Channel::new();
static LIBRARY_DECK_PROGRESS: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static LIBRARY_DECK_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static ANLZ_SCAN_BUFFER: StaticCell<[u8; ANLZ_SCAN_BUFFER_BYTES]> = StaticCell::new();

#[derive(Debug)]
enum LibraryDeckCompletion {
    Applied {
        effect: LibraryEffect,
    },
    LoadReady {
        effect: LibraryEffect,
        deck: DeckId,
        candidate: CatalogLoadCandidate,
        source: HotCueBank,
        persisted: Option<HotCueBank>,
        source_cues_available: bool,
        audio_source: Option<SourceDescriptor>,
        audio_generation: u32,
        analysis: ProductAnalysisLoad,
    },
    Failed {
        effect: LibraryEffect,
        source_revoked: Option<DeckId>,
        /// PLAY/scratch withdrew the permit before the old session was released.
        superseded: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LibraryDeckDispatchReport {
    pub(crate) completed: u8,
    pub(crate) failed: u8,
    pub(crate) invalidated: u8,
    pub(crate) blocked: bool,
}

impl LibraryDeckDispatchReport {
    pub(crate) const fn made_progress(self) -> bool {
        self.completed != 0 || self.failed != 0 || self.invalidated != 0
    }
}

pub(crate) async fn wait_for_library_deck_progress() {
    let _ = select(LIBRARY_DECK_PROGRESS.wait(), wait_for_media_lease_change()).await;
}

pub(crate) fn dispatch_library_deck(product: &mut ProductRuntime) -> LibraryDeckDispatchReport {
    let mut report = LibraryDeckDispatchReport::default();
    report.invalidated = product.invalidate_loaded_media(current_media_lease());

    if let Ok(completion) = LIBRARY_DECK_COMPLETIONS.try_receive() {
        LIBRARY_DECK_IN_FLIGHT.store(false, Ordering::Release);
        let expected = product.next_library_effect();
        let effect = match completion {
            LibraryDeckCompletion::Applied { effect }
            | LibraryDeckCompletion::LoadReady { effect, .. }
            | LibraryDeckCompletion::Failed { effect, .. } => effect,
        };

        if expected != Some(effect) {
            esp_hal::system::software_reset();
        } else {
            match completion {
                LibraryDeckCompletion::Applied { .. } => {
                    report.completed = report.completed.saturating_add(1);
                }
                LibraryDeckCompletion::LoadReady {
                    deck,
                    candidate,
                    source,
                    persisted,
                    source_cues_available,
                    audio_source,
                    audio_generation,
                    analysis,
                    ..
                } => {
                    let committed = catalog_publication_matches(candidate.publication)
                        && product.commit_library_load(
                            deck,
                            candidate,
                            source,
                            persisted,
                            source_cues_available,
                            audio_source,
                            analysis,
                        );
                    if committed
                        && activate_optional_audio_source(deck, audio_source, audio_generation)
                    {
                        report.completed = report.completed.saturating_add(1);
                    } else {
                        if committed {
                            product.invalidate_deck_source(deck);
                        }
                        if !release_prepared_audio_source(deck, audio_source) {
                            esp_hal::system::software_reset();
                        }
                        product.note_library_failure();
                        report.failed = report.failed.saturating_add(1);
                    }
                }
                LibraryDeckCompletion::Failed {
                    source_revoked,
                    superseded,
                    ..
                } => {
                    product.fail_library_load(source_revoked, superseded);
                    report.failed = report.failed.saturating_add(1);
                }
            }
            // Commit, activation or failure has resolved this load; transport
            // claims on the deck are accepted again.
            if let LibraryEffect::Load { deck, permit } = effect {
                load_gate(deck).finish(permit);
            }
            let completed = product.complete_library_effect();
            debug_assert!(completed);
        }
    }

    if LIBRARY_DECK_IN_FLIGHT.load(Ordering::Acquire) {
        report.blocked = product.pending_library_effects() != 0;
        return report;
    }

    let Some(effect) = product.next_library_effect() else {
        return report;
    };
    match LIBRARY_DECK_COMMANDS.try_send(effect) {
        Ok(()) => LIBRARY_DECK_IN_FLIGHT.store(true, Ordering::Release),
        Err(TrySendError::Full(_)) => report.blocked = true,
    }
    report
}

pub(crate) async fn run_library_deck_owner() -> ! {
    let buffer = ANLZ_SCAN_BUFFER.init([0u8; ANLZ_SCAN_BUFFER_BYTES]);
    let mut buffer: &'static mut [u8] = &mut buffer[..];
    // This lane is intentionally separate from DeckA/DeckB. The deck lanes are
    // reserved for persistent audio-source owners so Library preflight/ANLZ
    // reads cannot collide with long-lived playback file handles.
    let mut library_load = Usb0FilesystemClient::library_load();
    let mut request_id = 0u32;

    loop {
        let effect = LIBRARY_DECK_COMMANDS.receive().await;
        request_id = request_id.wrapping_add(1);
        if request_id == 0 {
            request_id = 1;
        }

        USB0_LIBRARY_COMMANDS
            .send(Usb0LibraryCommand { request_id, effect })
            .await;
        let library = USB0_LIBRARY_COMPLETIONS.receive().await;

        let completion = match library {
            Usb0LibraryCompletion::Applied {
                request_id: completed,
            } if completed == request_id => LibraryDeckCompletion::Applied { effect },
            Usb0LibraryCompletion::LoadResolved {
                request_id: completed,
                deck,
                candidate,
            } if completed == request_id
                && matches!(
                    effect,
                    LibraryEffect::Load { deck: requested, .. } if requested == deck
                ) =>
            {
                let LibraryEffect::Load { permit, .. } = effect else {
                    esp_hal::system::software_reset()
                };
                let failed = LibraryDeckCompletion::Failed {
                    effect,
                    source_revoked: None,
                    superseded: false,
                };
                if !catalog_publication_matches(candidate.publication) {
                    failed
                } else {
                    let client = &mut library_load;
                    if !validate_audio_source(client, candidate).await {
                        failed
                    } else {
                        let (next_buffer, source, source_cues_available, analysis) =
                            load_source_analysis(client, candidate, buffer, request_id).await;
                        buffer = next_buffer;

                        if !catalog_publication_matches(candidate.publication) {
                            failed
                        } else {
                            match load_persisted_hot_cues(request_id, candidate.track.track_id)
                                .await
                            {
                                HotCueLoadResult::Ready(persisted)
                                    if catalog_publication_matches(candidate.publication) =>
                                {
                                    // Final admission: Prepare releases the deck's
                                    // old session, so PLAY/scratch since the
                                    // control-time verdict must win here.
                                    if load_gate(deck).begin_replacement(permit).is_err() {
                                        LibraryDeckCompletion::Failed {
                                            effect,
                                            source_revoked: None,
                                            superseded: true,
                                        }
                                    } else {
                                        match prepare_optional_audio_source(
                                            deck, request_id, candidate,
                                        )
                                        .await
                                        {
                                            Ok((audio_source, audio_generation)) => {
                                                LibraryDeckCompletion::LoadReady {
                                                    effect,
                                                    deck,
                                                    candidate,
                                                    source,
                                                    persisted,
                                                    source_cues_available,
                                                    audio_source,
                                                    audio_generation,
                                                    analysis,
                                                }
                                            }
                                            Err(()) => LibraryDeckCompletion::Failed {
                                                effect,
                                                source_revoked: Some(deck),
                                                superseded: false,
                                            },
                                        }
                                    }
                                }
                                HotCueLoadResult::Ready(_) | HotCueLoadResult::Failed => failed,
                            }
                        }
                    }
                }
            }
            Usb0LibraryCompletion::Failed {
                request_id: completed,
                ..
            } if completed == request_id => LibraryDeckCompletion::Failed {
                effect,
                source_revoked: None,
                superseded: false,
            },
            Usb0LibraryCompletion::Applied { .. }
            | Usb0LibraryCompletion::LoadResolved { .. }
            | Usb0LibraryCompletion::Failed { .. } => esp_hal::system::software_reset(),
        };

        LIBRARY_DECK_COMPLETIONS.send(completion).await;
        LIBRARY_DECK_PROGRESS.signal(());
    }
}

async fn prepare_optional_audio_source(
    deck: DeckId,
    request_id: u32,
    candidate: CatalogLoadCandidate,
) -> Result<(Option<SourceDescriptor>, u32), ()> {
    #[cfg(feature = "wav-audio-source-bringup")]
    {
        prepare_audio_source(deck, request_id, candidate)
            .await
            .map(|prepared| (Some(prepared.descriptor), prepared.generation))
    }

    #[cfg(not(feature = "wav-audio-source-bringup"))]
    {
        let _ = (deck, request_id, candidate);
        Ok((None, 0))
    }
}

fn activate_optional_audio_source(
    deck: DeckId,
    source: Option<SourceDescriptor>,
    generation: u32,
) -> bool {
    #[cfg(feature = "wav-audio-source-bringup")]
    {
        source.is_none() || activate_prepared_source(deck, generation)
    }
    #[cfg(not(feature = "wav-audio-source-bringup"))]
    {
        let _ = (deck, source, generation);
        true
    }
}

fn release_prepared_audio_source(deck: DeckId, audio_source: Option<SourceDescriptor>) -> bool {
    #[cfg(feature = "wav-audio-source-bringup")]
    {
        audio_source.is_none() || request_source_release(deck)
    }

    #[cfg(not(feature = "wav-audio-source-bringup"))]
    {
        let _ = (deck, audio_source);
        true
    }
}

async fn validate_audio_source(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
) -> bool {
    if candidate.track.audio_path.is_empty() {
        return false;
    }
    let Ok(path) = BoundedPath::new(candidate.track.audio_path.as_str()) else {
        return false;
    };
    let lease = candidate.publication.lease;
    let mut request = Usb0FsRequest::Stat {
        ticket: client.issue_ticket(lease),
        path,
    };
    loop {
        match client.submit(request).await {
            Usb0FsSubmitResult::Completed(Usb0FsCompletion::Stat { result, .. }) => {
                return result.is_ok_and(|stat| stat.kind == EntryKind::File && stat.len != 0);
            }
            Usb0FsSubmitResult::QueueFull(returned) => {
                request = returned;
                Timer::after(Duration::from_millis(1)).await;
            }
            Usb0FsSubmitResult::PendingCompletion(_) | Usb0FsSubmitResult::WrongClient(_) => {
                return false;
            }
            Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}

async fn load_source_analysis(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    buffer: &'static mut [u8],
    generation: u32,
) -> (&'static mut [u8], HotCueBank, bool, ProductAnalysisLoad) {
    let mut source = HotCueBank::empty(candidate.track.track_id).as_source();
    let mut buffer = buffer;
    let mut importer = AnlzImporter::new();
    let mut publication = None;
    if let Ok(path) = BoundedPath::new(candidate.track.analysis_path.as_str()) {
        let (next_buffer, dat) = scan_analysis_file(
            client,
            candidate,
            path,
            buffer,
            &mut importer,
            AnalysisFileKind::Dat,
        )
        .await;
        buffer = next_buffer;
        if dat == AnalysisScanResult::Complete {
            let mut ext_storage = [0u8; 256];
            let mut valid = true;
            if let Some(path) =
                extended_path(candidate.track.analysis_path.as_str(), &mut ext_storage)
                    .and_then(|path| BoundedPath::new(path).ok())
            {
                let (next_buffer, ext) = scan_analysis_file(
                    client,
                    candidate,
                    path,
                    buffer,
                    &mut importer,
                    AnalysisFileKind::Ext,
                )
                .await;
                buffer = next_buffer;
                valid = ext != AnalysisScanResult::Invalid;
            }
            if valid && catalog_publication_matches(candidate.publication) {
                let identity = AnalysisIdentity {
                    track: candidate.track.track_id,
                    lease: candidate.publication.lease,
                    catalog_revision: candidate.publication.revision,
                    load_generation: generation,
                };
                publication = importer.finish(identity, candidate.track.bpm_x100).ok();
            }
        }
    }
    if let Some(data) = publication.as_ref() {
        for cue in data.view().cues() {
            match cue.end_ms {
                Some(end) => {
                    source.set_loop(cue.slot, cue.start_ms, end);
                }
                None => {
                    source.set_single(cue.slot, cue.start_ms);
                }
            }
        }
    }
    let available = publication.is_some();
    (
        buffer,
        source.as_source(),
        available,
        ProductAnalysisLoad {
            generation,
            publication,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnalysisScanResult {
    Missing,
    Complete,
    Invalid,
}

async fn scan_analysis_file(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    path: BoundedPath,
    buffer: &'static mut [u8],
    importer: &mut AnlzImporter,
    kind: AnalysisFileKind,
) -> (&'static mut [u8], AnalysisScanResult) {
    let lease = candidate.publication.lease;
    let Some(mut file) = open_file(client, lease, path).await else {
        return (buffer, AnalysisScanResult::Missing);
    };
    let mut valid = importer.begin_file(kind, file.len()).is_ok();
    let mut buffer = buffer;
    while valid {
        let Some(plan) = importer.read_plan() else {
            break;
        };
        if !catalog_publication_matches(candidate.publication) {
            valid = false;
            break;
        }
        let (next_file, next_buffer, bytes) =
            read_at(client, lease, file, buffer, u64::from(plan.offset)).await;
        file = next_file;
        buffer = next_buffer;
        valid = bytes >= plan.len && importer.ingest(plan.offset, &buffer[..plan.len]).is_ok();
    }
    valid &= importer.file_complete();
    valid &= close_file(client, lease, file).await;
    (
        buffer,
        if valid {
            AnalysisScanResult::Complete
        } else {
            AnalysisScanResult::Invalid
        },
    )
}

async fn open_file(
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
    path: BoundedPath,
) -> Option<Usb0FsFileHandle> {
    let mut request = Usb0FsRequest::OpenRead {
        ticket: client.issue_ticket(lease),
        path,
    };
    loop {
        match client.submit(request).await {
            Usb0FsSubmitResult::Completed(Usb0FsCompletion::OpenRead { result, .. }) => {
                return result.ok();
            }
            Usb0FsSubmitResult::QueueFull(returned) => {
                request = returned;
                Timer::after(Duration::from_millis(1)).await;
            }
            Usb0FsSubmitResult::PendingCompletion(_)
            | Usb0FsSubmitResult::WrongClient(_)
            | Usb0FsSubmitResult::Completed(_)
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => return None,
        }
    }
}

enum SeekOutcome {
    Ready(Usb0FsFileHandle),
    Failed(Usb0FsFileHandle),
}

async fn read_at(
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
    position: u64,
) -> (Usb0FsFileHandle, &'static mut [u8], usize) {
    let file = match seek_file(client, lease, file, position).await {
        SeekOutcome::Ready(file) => file,
        SeekOutcome::Failed(file) => return (file, buffer, 0),
    };
    let mut request = Usb0FsRequest::Read {
        ticket: client.issue_ticket(lease),
        file,
        buffer,
    };
    loop {
        match client.submit(request).await {
            Usb0FsSubmitResult::Completed(Usb0FsCompletion::Read {
                file,
                buffer,
                result,
                ..
            }) => return (file, buffer, result.unwrap_or(0)),
            Usb0FsSubmitResult::QueueFull(returned) => {
                request = returned;
                Timer::after(Duration::from_millis(1)).await;
            }
            Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Read { file, buffer, .. })
            | Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Read { file, buffer, .. }) => {
                return (file, buffer, 0);
            }
            Usb0FsSubmitResult::Completed(_)
            | Usb0FsSubmitResult::PendingCompletion(_)
            | Usb0FsSubmitResult::WrongClient(_)
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => esp_hal::system::software_reset(),
        }
    }
}

async fn seek_file(
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
    file: Usb0FsFileHandle,
    position: u64,
) -> SeekOutcome {
    let mut request = Usb0FsRequest::Seek {
        ticket: client.issue_ticket(lease),
        file,
        position,
    };
    loop {
        match client.submit(request).await {
            Usb0FsSubmitResult::Completed(Usb0FsCompletion::Seek { file, result, .. }) => {
                return if result.is_ok() {
                    SeekOutcome::Ready(file)
                } else {
                    SeekOutcome::Failed(file)
                };
            }
            Usb0FsSubmitResult::QueueFull(returned) => {
                request = returned;
                Timer::after(Duration::from_millis(1)).await;
            }
            Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Seek { file, .. })
            | Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Seek { file, .. }) => {
                return SeekOutcome::Failed(file);
            }
            Usb0FsSubmitResult::Completed(_)
            | Usb0FsSubmitResult::PendingCompletion(_)
            | Usb0FsSubmitResult::WrongClient(_)
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => esp_hal::system::software_reset(),
        }
    }
}

async fn close_file(
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
    file: Usb0FsFileHandle,
) -> bool {
    let mut request = Usb0FsRequest::CloseFile {
        ticket: client.issue_ticket(lease),
        file,
    };
    loop {
        match client.submit(request).await {
            Usb0FsSubmitResult::Completed(Usb0FsCompletion::CloseFile { result, .. }) => {
                return result.is_ok();
            }
            Usb0FsSubmitResult::QueueFull(returned) => {
                request = returned;
                Timer::after(Duration::from_millis(1)).await;
            }
            Usb0FsSubmitResult::PendingCompletion(_) | Usb0FsSubmitResult::WrongClient(_) => {
                return false;
            }
            Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}
