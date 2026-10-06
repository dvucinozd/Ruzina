use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
};
use embassy_time::{Duration, Timer};
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::wav::{
    CHUNK_HEADER_BYTES, PCM_FORMAT_BYTES, RIFF_HEADER_BYTES, WavPcm16Decoder,
    WavPcm16SourceDescriptor, parse_chunk_header, parse_pcm16_format, parse_riff_header,
};
use pajoniiir_controller_session::DeckId;
use pajoniiir_media_catalog::CatalogLoadCandidate;
use pajoniiir_media_fs::{BoundedPath, FileHandle};
use static_cell::StaticCell;

use crate::{
    usb0_library::{current_media_lease, wait_for_media_lease_change},
    usb0_msc::{
        Usb0FsCompletion, Usb0FsFileHandle, Usb0FsRequest, Usb0FsSubmitResult,
        Usb0FilesystemClient,
    },
};

const SOURCE_QUEUE_DEPTH: usize = 1;
const SOURCE_READ_BUFFER_BYTES: usize = 4_096;

static DECK_A_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCommand,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_B_COMMANDS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCommand,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_A_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_B_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();

static DECK_A_READ_BUFFER: StaticCell<[u8; SOURCE_READ_BUFFER_BYTES]> = StaticCell::new();
static DECK_B_READ_BUFFER: StaticCell<[u8; SOURCE_READ_BUFFER_BYTES]> = StaticCell::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceCommand {
    Prepare {
        request_id: u32,
        candidate: CatalogLoadCandidate,
    },
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceCompletion {
    Prepared {
        request_id: u32,
        descriptor: WavPcm16SourceDescriptor,
    },
    Failed {
        request_id: u32,
    },
}

struct PreparedWavSource {
    candidate: CatalogLoadCandidate,
    file: Usb0FsFileHandle,
    descriptor: WavPcm16SourceDescriptor,
}

pub(crate) async fn prepare_wav_source(
    deck: DeckId,
    request_id: u32,
    candidate: CatalogLoadCandidate,
) -> Result<WavPcm16SourceDescriptor, ()> {
    command_channel(deck)
        .send(DeckAudioSourceCommand::Prepare {
            request_id,
            candidate,
        })
        .await;

    match completion_channel(deck).receive().await {
        DeckAudioSourceCompletion::Prepared {
            request_id: completed,
            descriptor,
        } if completed == request_id => Ok(descriptor),
        DeckAudioSourceCompletion::Failed {
            request_id: completed,
        } if completed == request_id => Err(()),
        DeckAudioSourceCompletion::Prepared { .. } | DeckAudioSourceCompletion::Failed { .. } => {
            esp_hal::system::software_reset()
        }
    }
}

pub(crate) fn request_source_release(deck: DeckId) -> bool {
    match command_channel(deck).try_send(DeckAudioSourceCommand::Release) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,
    }
}

pub(crate) async fn run_deck_a_audio_source_owner() -> ! {
    let buffer = DECK_A_READ_BUFFER.init([0u8; SOURCE_READ_BUFFER_BYTES]);
    run_owner(
        DeckId::One,
        Usb0FilesystemClient::deck_a(),
        &mut buffer[..],
    )
    .await
}

pub(crate) async fn run_deck_b_audio_source_owner() -> ! {
    let buffer = DECK_B_READ_BUFFER.init([0u8; SOURCE_READ_BUFFER_BYTES]);
    run_owner(
        DeckId::Two,
        Usb0FilesystemClient::deck_b(),
        &mut buffer[..],
    )
    .await
}

async fn run_owner(
    deck: DeckId,
    mut client: Usb0FilesystemClient,
    mut buffer: &'static mut [u8],
) -> ! {
    let mut prepared: Option<PreparedWavSource> = None;

    loop {
        match select(command_channel(deck).receive(), wait_for_media_lease_change()).await {
            Either::First(DeckAudioSourceCommand::Prepare {
                request_id,
                candidate,
            }) => {
                if let Some(previous) = prepared.take()
                    && current_media_lease() == Some(previous.candidate.publication.lease)
                    && !close_file(
                        &mut client,
                        previous.candidate.publication.lease,
                        previous.file,
                    )
                    .await
                {
                    completion_channel(deck)
                        .send(DeckAudioSourceCompletion::Failed { request_id })
                        .await;
                    continue;
                }

                let (next_buffer, result) = prepare_source(&mut client, candidate, buffer).await;
                buffer = next_buffer;
                match result {
                    Ok(source) => {
                        let descriptor = source.descriptor;
                        prepared = Some(source);
                        completion_channel(deck)
                            .send(DeckAudioSourceCompletion::Prepared {
                                request_id,
                                descriptor,
                            })
                            .await;
                    }
                    Err(()) => {
                        completion_channel(deck)
                            .send(DeckAudioSourceCompletion::Failed { request_id })
                            .await;
                    }
                }
            }
            Either::First(DeckAudioSourceCommand::Release) => {
                if let Some(source) = prepared.take()
                    && current_media_lease() == Some(source.candidate.publication.lease)
                {
                    let _ = close_file(
                        &mut client,
                        source.candidate.publication.lease,
                        source.file,
                    )
                    .await;
                }
            }
            Either::Second(()) => {
                if prepared.as_ref().is_some_and(|source| {
                    current_media_lease() != Some(source.candidate.publication.lease)
                }) {
                    // The filesystem generation is already gone. The detached,
                    // lease-bound handle is dropped rather than sent to a new
                    // generation for close.
                    prepared = None;
                }
            }
        }
    }
}

async fn prepare_source(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    buffer: &'static mut [u8],
) -> (&'static mut [u8], Result<PreparedWavSource, ()>) {
    let lease = candidate.publication.lease;
    if current_media_lease() != Some(lease) || candidate.track.audio_path.is_empty() {
        return (buffer, Err(()));
    }

    let Ok(path) = BoundedPath::new(candidate.track.audio_path.as_str()) else {
        return (buffer, Err(()));
    };
    let Some(file) = open_file(client, lease, path).await else {
        return (buffer, Err(()));
    };
    let file_len = file.len();

    let (file, buffer, bytes) = read_at(client, lease, file, buffer, 0).await;
    if bytes < RIFF_HEADER_BYTES {
        let _ = close_file(client, lease, file).await;
        return (buffer, Err(()));
    }

    let riff_size = match parse_riff_header(&buffer[..RIFF_HEADER_BYTES]) {
        Ok(size) => size,
        Err(_) => {
            let _ = close_file(client, lease, file).await;
            return (buffer, Err(()));
        }
    };
    let Some(riff_end) = u64::from(riff_size).checked_add(8) else {
        let _ = close_file(client, lease, file).await;
        return (buffer, Err(()));
    };
    if riff_end > file_len || riff_end < RIFF_HEADER_BYTES as u64 {
        let _ = close_file(client, lease, file).await;
        return (buffer, Err(()));
    }

    let mut file = file;
    let mut buffer = buffer;
    let mut position = RIFF_HEADER_BYTES as u64;
    let mut format = None;

    while position + CHUNK_HEADER_BYTES as u64 <= riff_end {
        let (next_file, next_buffer, bytes) =
            read_at(client, lease, file, buffer, position).await;
        file = next_file;
        buffer = next_buffer;
        if bytes < CHUNK_HEADER_BYTES {
            break;
        }

        let chunk = match parse_chunk_header(&buffer[..CHUNK_HEADER_BYTES]) {
            Ok(chunk) => chunk,
            Err(_) => break,
        };
        let Some(chunk_data) = position.checked_add(CHUNK_HEADER_BYTES as u64) else {
            break;
        };
        let Some(chunk_end) = chunk_data.checked_add(u64::from(chunk.len)) else {
            break;
        };
        let Some(next_position) = chunk_data.checked_add(chunk.padded_len()) else {
            break;
        };
        if chunk_end > riff_end || next_position > riff_end {
            break;
        }

        if chunk.is_format() {
            if chunk.len < PCM_FORMAT_BYTES as u32 {
                break;
            }
            let (next_file, next_buffer, bytes) =
                read_at(client, lease, file, buffer, chunk_data).await;
            file = next_file;
            buffer = next_buffer;
            if bytes < PCM_FORMAT_BYTES {
                break;
            }
            format = parse_pcm16_format(&buffer[..PCM_FORMAT_BYTES]).ok();
            if format.is_none() {
                break;
            }
        } else if chunk.is_data() {
            let Some(format) = format else {
                break;
            };
            let descriptor = match WavPcm16SourceDescriptor::new(
                format,
                chunk_data,
                u64::from(chunk.len),
            ) {
                Ok(descriptor) if descriptor.data_bytes >= u64::from(format.block_align) => {
                    descriptor
                }
                Ok(_) | Err(_) => break,
            };

            let (next_file, next_buffer, bytes) =
                read_at(client, lease, file, buffer, descriptor.data_offset).await;
            file = next_file;
            buffer = next_buffer;
            let mut decoder =
                WavPcm16Decoder::new(descriptor.format, descriptor.data_bytes).expect(
                    "descriptor construction validates WAV data alignment before decode probe",
                );
            let mut first_frame = [PcmFrame::default(); 1];
            let progress = decoder.decode(&buffer[..bytes], &mut first_frame);
            if progress.produced_frames != 1 {
                break;
            }

            file = match seek_file(client, lease, file, descriptor.data_offset).await {
                SeekOutcome::Ready(file) => file,
                SeekOutcome::Failed(file) => {
                    let _ = close_file(client, lease, file).await;
                    return (buffer, Err(()));
                }
            };
            if current_media_lease() != Some(lease) {
                return (buffer, Err(()));
            }
            return (
                buffer,
                Ok(PreparedWavSource {
                    candidate,
                    file,
                    descriptor,
                }),
            );
        }

        position = next_position;
    }

    let _ = close_file(client, lease, file).await;
    (buffer, Err(()))
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

fn command_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, DeckAudioSourceCommand, SOURCE_QUEUE_DEPTH> {
    match deck {
        DeckId::One => &DECK_A_COMMANDS,
        DeckId::Two => &DECK_B_COMMANDS,
    }
}

fn completion_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, DeckAudioSourceCompletion, SOURCE_QUEUE_DEPTH> {
    match deck {
        DeckId::One => &DECK_A_COMPLETIONS,
        DeckId::Two => &DECK_B_COMPLETIONS,
    }
}
