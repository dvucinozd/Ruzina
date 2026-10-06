use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::select;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use pajoniiir_audio_runtime::wav::WavPcm16SourceDescriptor;
use pajoniiir_controller_session::DeckId;
use pajoniiir_hot_cues::HotCueBank;
use pajoniiir_media_catalog::CatalogLoadCandidate;
use pajoniiir_media_fs::{BoundedPath, EntryKind, FileHandle};
use pajoniiir_rekordbox::anlz_hot_cues::{
    ANLZ_PCPT_BYTES, ANLZ_PCOB_HEADER_BYTES, ANLZ_SECTION_HEADER_BYTES, ANLZ_TAG_PCOB,
    AnlzSectionHeader, RekordboxCueKind, RekordboxHotCueBank, parse_pcob_header, parse_pcpt,
    parse_section_header,
};
use static_cell::StaticCell;

#[cfg(feature = "wav-audio-source-bringup")]
use crate::deck_audio_source::{prepare_wav_source, request_source_release};

use crate::{
    deck_effect_owners::{HotCueLoadResult, load_persisted_hot_cues},
    product_runtime::{LibraryEffect, ProductRuntime},
    usb0_library::{
        USB0_LIBRARY_COMMANDS, USB0_LIBRARY_COMPLETIONS, Usb0LibraryCommand,
        Usb0LibraryCompletion, catalog_publication_matches, current_media_lease,
        wait_for_media_lease_change,
    },
    usb0_msc::{
        Usb0FsCompletion, Usb0FsFileHandle, Usb0FsRequest, Usb0FsSubmitResult,
        Usb0FilesystemClient,
    },
};

const LIBRARY_DECK_QUEUE_DEPTH: usize = 1;
const ANLZ_SCAN_BUFFER_BYTES: usize = 64;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        wav_source: Option<WavPcm16SourceDescriptor>,
    },
    Failed {
        effect: LibraryEffect,
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
    let _ = select(
        LIBRARY_DECK_PROGRESS.wait(),
        wait_for_media_lease_change(),
    )
    .await;
}

pub(crate) fn dispatch_library_deck(
    product: &mut ProductRuntime,
) -> LibraryDeckDispatchReport {
    let mut report = LibraryDeckDispatchReport::default();
    report.invalidated = product.invalidate_loaded_media(current_media_lease());

    if let Ok(completion) = LIBRARY_DECK_COMPLETIONS.try_receive() {
        LIBRARY_DECK_IN_FLIGHT.store(false, Ordering::Release);
        let expected = product.next_library_effect();
        let effect = match completion {
            LibraryDeckCompletion::Applied { effect }
            | LibraryDeckCompletion::LoadReady { effect, .. }
            | LibraryDeckCompletion::Failed { effect } => effect,
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
                    wav_source,
                    ..
                } => {
                    if catalog_publication_matches(candidate.publication)
                        && product.commit_library_load(
                            deck,
                            candidate,
                            source,
                            persisted,
                            source_cues_available,
                            wav_source,
                        )
                    {
                        report.completed = report.completed.saturating_add(1);
                    } else {
                        if !release_prepared_wav_source(deck, wav_source) {
                            esp_hal::system::software_reset();
                        }
                        product.note_library_failure();
                        report.failed = report.failed.saturating_add(1);
                    }
                }
                LibraryDeckCompletion::Failed { .. } => {
                    product.note_library_failure();
                    report.failed = report.failed.saturating_add(1);
                }
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
            } if completed == request_id && effect == (LibraryEffect::Load { deck }) => {
                if !catalog_publication_matches(candidate.publication) {
                    LibraryDeckCompletion::Failed { effect }
                } else {
                    let client = &mut library_load;
                    if !validate_audio_source(client, candidate).await {
                        LibraryDeckCompletion::Failed { effect }
                    } else {
                    let (next_buffer, source, source_cues_available) =
                        load_source_hot_cues(client, candidate, buffer).await;
                    buffer = next_buffer;

                    if !catalog_publication_matches(candidate.publication) {
                        LibraryDeckCompletion::Failed { effect }
                    } else {
                        match load_persisted_hot_cues(request_id, candidate.track.track_id).await {
                            HotCueLoadResult::Ready(persisted)
                                if catalog_publication_matches(candidate.publication) =>
                            {
                                match prepare_optional_wav_source(deck, request_id, candidate).await {
                                    Ok(wav_source) => LibraryDeckCompletion::LoadReady {
                                        effect,
                                        deck,
                                        candidate,
                                        source,
                                        persisted,
                                        source_cues_available,
                                        wav_source,
                                    },
                                    Err(()) => LibraryDeckCompletion::Failed { effect },
                                }
                            }
                            HotCueLoadResult::Ready(_) | HotCueLoadResult::Failed => {
                                LibraryDeckCompletion::Failed { effect }
                            }
                        }
                    }
                    }
                }
            }
            Usb0LibraryCompletion::Failed {
                request_id: completed,
                ..
            } if completed == request_id => LibraryDeckCompletion::Failed { effect },
            Usb0LibraryCompletion::Applied { .. }
            | Usb0LibraryCompletion::LoadResolved { .. }
            | Usb0LibraryCompletion::Failed { .. } => esp_hal::system::software_reset(),
        };

        LIBRARY_DECK_COMPLETIONS.send(completion).await;
        LIBRARY_DECK_PROGRESS.signal(());
    }
}


async fn prepare_optional_wav_source(
    deck: DeckId,
    request_id: u32,
    candidate: CatalogLoadCandidate,
) -> Result<Option<WavPcm16SourceDescriptor>, ()> {
    #[cfg(feature = "wav-audio-source-bringup")]
    {
        prepare_wav_source(deck, request_id, candidate).await.map(Some)
    }

    #[cfg(not(feature = "wav-audio-source-bringup"))]
    {
        let _ = (deck, request_id, candidate);
        Ok(None)
    }
}

fn release_prepared_wav_source(
    deck: DeckId,
    wav_source: Option<WavPcm16SourceDescriptor>,
) -> bool {
    #[cfg(feature = "wav-audio-source-bringup")]
    {
        wav_source.is_none() || request_source_release(deck)
    }

    #[cfg(not(feature = "wav-audio-source-bringup"))]
    {
        let _ = (deck, wav_source);
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
            Usb0FsSubmitResult::PendingCompletion(_)
            | Usb0FsSubmitResult::WrongClient(_) => return false,
            Usb0FsSubmitResult::Completed(_)
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}

async fn load_source_hot_cues(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    buffer: &'static mut [u8],
) -> (&'static mut [u8], HotCueBank, bool) {
    let mut source = HotCueBank::empty(candidate.track.track_id);
    if candidate.track.analysis_path.is_empty() {
        return (buffer, source.as_source(), false);
    }

    let Ok(path) = BoundedPath::new(candidate.track.analysis_path.as_str()) else {
        return (buffer, source.as_source(), false);
    };
    let lease = candidate.publication.lease;
    let Some(mut file) = open_file(client, lease, path).await else {
        return (buffer, source.as_source(), false);
    };
    let file_len = match u32::try_from(file.len()) {
        Ok(len) if len >= ANLZ_SECTION_HEADER_BYTES as u32 => len,
        _ => {
            let _ = close_file(client, lease, file).await;
            return (buffer, source.as_source(), false);
        }
    };

    let mut buffer = buffer;
    let mut cues = RekordboxHotCueBank::new();
    let mut position = 0u32;
    let mut valid = true;

    while position + ANLZ_SECTION_HEADER_BYTES as u32 <= file_len {
        let (next_file, next_buffer, bytes) =
            read_at(client, lease, file, buffer, u64::from(position)).await;
        file = next_file;
        buffer = next_buffer;
        if bytes < ANLZ_SECTION_HEADER_BYTES {
            valid = false;
            break;
        }
        let section = match parse_section_header(
            position,
            file_len,
            &buffer[..ANLZ_SECTION_HEADER_BYTES],
        ) {
            Ok(section) => section,
            Err(_) => {
                valid = false;
                break;
            }
        };

        if section.tag == ANLZ_TAG_PCOB {
            let (next_file, next_buffer, bytes) =
                read_at(client, lease, file, buffer, u64::from(position)).await;
            file = next_file;
            buffer = next_buffer;
            if bytes < ANLZ_PCOB_HEADER_BYTES {
                valid = false;
                break;
            }
            let plan = match parse_pcob_header(
                position,
                section,
                &buffer[..ANLZ_PCOB_HEADER_BYTES],
            ) {
                Ok(plan) => plan,
                Err(_) => {
                    valid = false;
                    break;
                }
            };

            for record in 0..u32::from(plan.declared_count) {
                let Some(record_position) = plan
                    .records_position
                    .checked_add(record * ANLZ_PCPT_BYTES as u32)
                else {
                    valid = false;
                    break;
                };
                let (next_file, next_buffer, bytes) =
                    read_at(client, lease, file, buffer, u64::from(record_position)).await;
                file = next_file;
                buffer = next_buffer;
                if bytes < ANLZ_PCPT_BYTES {
                    valid = false;
                    break;
                }
                match parse_pcpt(plan.list_type, &buffer[..ANLZ_PCPT_BYTES]) {
                    Ok(Some(cue)) => {
                        if cues.insert(cue).is_err() {
                            valid = false;
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                break;
            }
        }
        position = section.next_position;
    }

    if position != file_len {
        valid = false;
    }
    valid &= close_file(client, lease, file).await;

    if valid {
        for index in 0..8u8 {
            if let Some(cue) = cues.slot(index) {
                let accepted = match cue.kind {
                    RekordboxCueKind::Single => source.set_single(index, cue.start_ms),
                    RekordboxCueKind::Loop => source.set_loop(index, cue.start_ms, cue.end_ms),
                };
                if !accepted {
                    valid = false;
                    break;
                }
            }
        }
    }

    if valid {
        (buffer, source.as_source(), true)
    } else {
        (buffer, HotCueBank::empty(candidate.track.track_id).as_source(), false)
    }
}

async fn open_file(
    client: &mut Usb0FilesystemClient,
    lease: pajoniiir_media_session::MediaLease,
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
    lease: pajoniiir_media_session::MediaLease,
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
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}

async fn seek_file(
    client: &mut Usb0FilesystemClient,
    lease: pajoniiir_media_session::MediaLease,
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
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}

async fn close_file(
    client: &mut Usb0FilesystemClient,
    lease: pajoniiir_media_session::MediaLease,
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
            Usb0FsSubmitResult::PendingCompletion(_)
            | Usb0FsSubmitResult::WrongClient(_) => return false,
            Usb0FsSubmitResult::Completed(_)
            | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
                esp_hal::system::software_reset()
            }
        }
    }
}

