use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
};
use embassy_time::{Duration, Timer};
use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::{
    pcm_stream::{
        PCM_STREAM_BLOCK_FRAMES, PcmStreamBlock, frame_from_ms, next_stream_generation,
    },
    wav::{
        CHUNK_HEADER_BYTES, PCM_FORMAT_BYTES, RIFF_HEADER_BYTES, WavPcm16Decoder,
        WavPcm16SourceDescriptor, parse_chunk_header, parse_pcm16_format, parse_riff_header,
    },
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
const PCM_QUEUE_DEPTH: usize = 3;
// 512 bytes produces at most 256 mono PCM16 frames or 128 stereo frames,
// fitting one fixed PcmStreamBlock without hidden input retention.
const SOURCE_READ_BUFFER_BYTES: usize = 512;

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
static DECK_A_PREPARE_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_B_PREPARE_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_A_CONTROL_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceControlCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();
static DECK_B_CONTROL_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    DeckAudioSourceControlCompletion,
    SOURCE_QUEUE_DEPTH,
> = Channel::new();

static DECK_A_PCM: Channel<CriticalSectionRawMutex, PcmStreamBlock, PCM_QUEUE_DEPTH> =
    Channel::new();
static DECK_B_PCM: Channel<CriticalSectionRawMutex, PcmStreamBlock, PCM_QUEUE_DEPTH> =
    Channel::new();

static DECK_A_READ_BUFFER: StaticCell<[u8; SOURCE_READ_BUFFER_BYTES]> = StaticCell::new();
static DECK_B_READ_BUFFER: StaticCell<[u8; SOURCE_READ_BUFFER_BYTES]> = StaticCell::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceCommand {
    Prepare {
        request_id: u32,
        candidate: CatalogLoadCandidate,
    },
    Seek {
        request_id: u32,
        position_ms: u32,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceControlCompletion {
    Seeked {
        request_id: u32,
        generation: u32,
        frame: u64,
        sample_rate_hz: u32,
    },
    Failed {
        request_id: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WavSeekResult {
    pub(crate) generation: u32,
    pub(crate) frame: u64,
    pub(crate) sample_rate_hz: u32,
}

struct PreparedWavSource {
    candidate: CatalogLoadCandidate,
    file: Usb0FsFileHandle,
    descriptor: WavPcm16SourceDescriptor,
    decoder: WavPcm16Decoder,
    generation: u32,
    next_frame: u64,
    eof_queued: bool,
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

    match prepare_completion_channel(deck).receive().await {
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

pub(crate) async fn seek_wav_source(
    deck: DeckId,
    request_id: u32,
    position_ms: u32,
) -> Result<WavSeekResult, ()> {
    command_channel(deck)
        .send(DeckAudioSourceCommand::Seek {
            request_id,
            position_ms,
        })
        .await;

    match control_completion_channel(deck).receive().await {
        DeckAudioSourceControlCompletion::Seeked {
            request_id: completed,
            generation,
            frame,
            sample_rate_hz,
        } if completed == request_id => Ok(WavSeekResult {
            generation,
            frame,
            sample_rate_hz,
        }),
        DeckAudioSourceControlCompletion::Failed {
            request_id: completed,
        } if completed == request_id => Err(()),
        DeckAudioSourceControlCompletion::Seeked { .. }
        | DeckAudioSourceControlCompletion::Failed { .. } => esp_hal::system::software_reset(),
    }
}

pub(crate) fn request_source_release(deck: DeckId) -> bool {
    match command_channel(deck).try_send(DeckAudioSourceCommand::Release) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,
    }
}

pub(crate) fn try_receive_pcm_block(deck: DeckId) -> Option<PcmStreamBlock> {
    pcm_channel(deck).try_receive().ok()
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
    let mut pending_block: Option<PcmStreamBlock> = None;
    let mut generation = 0u32;

    loop {
        let command = if let Ok(command) = command_channel(deck).try_receive() {
            Some(command)
        } else if prepared.is_none() {
            match select(command_channel(deck).receive(), wait_for_media_lease_change()).await {
                Either::First(command) => Some(command),
                Either::Second(()) => None,
            }
        } else {
            None
        };

        if let Some(command) = command {
            match command {
                DeckAudioSourceCommand::Prepare {
                    request_id,
                    candidate,
                } => {
                    pending_block = None;
                    drain_pcm_blocks(deck);

                    if let Some(previous) = prepared.take()
                        && current_media_lease() == Some(previous.candidate.publication.lease)
                        && !close_file(
                            &mut client,
                            previous.candidate.publication.lease,
                            previous.file,
                        )
                        .await
                    {
                        prepare_completion_channel(deck)
                            .send(DeckAudioSourceCompletion::Failed { request_id })
                            .await;
                        continue;
                    }

                    generation = next_stream_generation(generation);
                    let (next_buffer, result) =
                        prepare_source(&mut client, candidate, buffer, generation).await;
                    buffer = next_buffer;
                    match result {
                        Ok(source) => {
                            let descriptor = source.descriptor;
                            prepared = Some(source);
                            prepare_completion_channel(deck)
                                .send(DeckAudioSourceCompletion::Prepared {
                                    request_id,
                                    descriptor,
                                })
                                .await;
                        }
                        Err(()) => {
                            prepare_completion_channel(deck)
                                .send(DeckAudioSourceCompletion::Failed { request_id })
                                .await;
                        }
                    }
                }
                DeckAudioSourceCommand::Seek {
                    request_id,
                    position_ms,
                } => {
                    pending_block = None;
                    drain_pcm_blocks(deck);

                    let result = if let Some(mut source) = prepared.take() {
                        if current_media_lease() != Some(source.candidate.publication.lease) {
                            Err(())
                        } else {
                            generation = next_stream_generation(generation);
                            match seek_source(
                                &mut client,
                                &mut source,
                                generation,
                                position_ms,
                            )
                            .await
                            {
                                Ok(result) => {
                                    prepared = Some(source);
                                    Ok(result)
                                }
                                Err(()) => {
                                    prepared = Some(source);
                                    Err(())
                                }
                            }
                        }
                    } else {
                        Err(())
                    };

                    match result {
                        Ok(result) => {
                            control_completion_channel(deck)
                                .send(DeckAudioSourceControlCompletion::Seeked {
                                    request_id,
                                    generation: result.generation,
                                    frame: result.frame,
                                    sample_rate_hz: result.sample_rate_hz,
                                })
                                .await;
                        }
                        Err(()) => {
                            control_completion_channel(deck)
                                .send(DeckAudioSourceControlCompletion::Failed { request_id })
                                .await;
                        }
                    }
                }
                DeckAudioSourceCommand::Release => {
                    pending_block = None;
                    drain_pcm_blocks(deck);
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
            }
            continue;
        }

        if prepared.as_ref().is_some_and(|source| {
            current_media_lease() != Some(source.candidate.publication.lease)
        }) {
            prepared = None;
            pending_block = None;
            drain_pcm_blocks(deck);
            continue;
        }

        if let Some(block) = pending_block.take() {
            match pcm_channel(deck).try_send(block) {
                Ok(()) => {}
                Err(TrySendError::Full(block)) => {
                    pending_block = Some(block);
                    Timer::after(Duration::from_millis(1)).await;
                }
            }
            continue;
        }

        let Some(source) = prepared.take() else {
            continue;
        };

        if source.eof_queued {
            prepared = Some(source);
            match select(command_channel(deck).receive(), wait_for_media_lease_change()).await {
                Either::First(command) => {
                    // Put the command back into the bounded owner mailbox so the
                    // single command path above preserves completion ordering.
                    if command_channel(deck).try_send(command).is_err() {
                        esp_hal::system::software_reset();
                    }
                }
                Either::Second(()) => {}
            }
            continue;
        }

        let (next_buffer, source, produced) = produce_pcm_block(&mut client, source, buffer).await;
        buffer = next_buffer;
        prepared = Some(source);
        pending_block = produced;
    }
}

async fn prepare_source(
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    buffer: &'static mut [u8],
    generation: u32,
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
            let mut probe =
                WavPcm16Decoder::new(descriptor.format, descriptor.data_bytes).expect(
                    "descriptor construction validates WAV data alignment before decode probe",
                );
            let mut first_frame = [PcmFrame::default(); 1];
            let progress = probe.decode(&buffer[..bytes], &mut first_frame);
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
            let decoder = WavPcm16Decoder::new(descriptor.format, descriptor.data_bytes)
                .expect("validated descriptor must construct streaming decoder");
            return (
                buffer,
                Ok(PreparedWavSource {
                    candidate,
                    file,
                    descriptor,
                    decoder,
                    generation,
                    next_frame: 0,
                    eof_queued: false,
                }),
            );
        }

        position = next_position;
    }

    let _ = close_file(client, lease, file).await;
    (buffer, Err(()))
}

async fn seek_source(
    client: &mut Usb0FilesystemClient,
    source: &mut PreparedWavSource,
    generation: u32,
    position_ms: u32,
) -> Result<WavSeekResult, ()> {
    let format = source.descriptor.format;
    let frame = frame_from_ms(
        position_ms,
        format.sample_rate_hz,
        source.descriptor.frame_count(),
    );
    let byte_offset = frame
        .checked_mul(u64::from(format.block_align))
        .and_then(|bytes| source.descriptor.data_offset.checked_add(bytes))
        .ok_or(())?;
    let remaining_bytes = source
        .descriptor
        .data_bytes
        .saturating_sub(frame.saturating_mul(u64::from(format.block_align)));

    let file = core::mem::replace(
        &mut source.file,
        placeholder_file(source.file),
    );
    source.file = match seek_file(client, source.candidate.publication.lease, file, byte_offset).await {
        SeekOutcome::Ready(file) => file,
        SeekOutcome::Failed(file) => {
            source.file = file;
            return Err(());
        }
    };
    source.decoder = WavPcm16Decoder::new(format, remaining_bytes).map_err(|_| ())?;
    source.generation = generation;
    source.next_frame = frame;
    source.eof_queued = false;

    Ok(WavSeekResult {
        generation,
        frame,
        sample_rate_hz: format.sample_rate_hz,
    })
}

async fn produce_pcm_block(
    client: &mut Usb0FilesystemClient,
    mut source: PreparedWavSource,
    buffer: &'static mut [u8],
) -> (
    &'static mut [u8],
    PreparedWavSource,
    Option<PcmStreamBlock>,
) {
    if source.decoder.is_finished() {
        let mut block = PcmStreamBlock::empty(
            source.generation,
            source.next_frame,
            source.descriptor.format.sample_rate_hz,
        );
        block.mark_eof();
        source.eof_queued = true;
        return (buffer, source, Some(block));
    }

    let lease = source.candidate.publication.lease;
    let (file, buffer, bytes) = read_next(client, lease, source.file, buffer).await;
    source.file = file;

    if bytes == 0 {
        let mut block = PcmStreamBlock::empty(
            source.generation,
            source.next_frame,
            source.descriptor.format.sample_rate_hz,
        );
        block.mark_faulted();
        source.eof_queued = true;
        return (buffer, source, Some(block));
    }

    let first_frame = source.next_frame;
    let mut block = PcmStreamBlock::empty(
        source.generation,
        first_frame,
        source.descriptor.format.sample_rate_hz,
    );
    let progress = source
        .decoder
        .decode(&buffer[..bytes], block.frames_mut());
    if progress.produced_frames > PCM_STREAM_BLOCK_FRAMES
        || !block.set_len(progress.produced_frames)
    {
        block.mark_faulted();
        source.eof_queued = true;
        return (buffer, source, Some(block));
    }

    source.next_frame = source
        .next_frame
        .saturating_add(progress.produced_frames as u64);

    if progress.finished {
        block.mark_eof();
        source.eof_queued = true;
    } else if progress.consumed_bytes != bytes {
        block.mark_faulted();
        source.eof_queued = true;
    }

    if block.len == 0 && !block.eof && !block.faulted {
        return (buffer, source, None);
    }

    (buffer, source, Some(block))
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
    read_next(client, lease, file, buffer).await
}

async fn read_next(
    client: &mut Usb0FilesystemClient,
    lease: pajoniiir_media_session::MediaLease,
    file: Usb0FsFileHandle,
    buffer: &'static mut [u8],
) -> (Usb0FsFileHandle, &'static mut [u8], usize) {
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

fn drain_pcm_blocks(deck: DeckId) {
    while pcm_channel(deck).try_receive().is_ok() {}
}

fn command_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, DeckAudioSourceCommand, SOURCE_QUEUE_DEPTH> {
    match deck {
        DeckId::One => &DECK_A_COMMANDS,
        DeckId::Two => &DECK_B_COMMANDS,
    }
}

fn prepare_completion_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, DeckAudioSourceCompletion, SOURCE_QUEUE_DEPTH> {
    match deck {
        DeckId::One => &DECK_A_PREPARE_COMPLETIONS,
        DeckId::Two => &DECK_B_PREPARE_COMPLETIONS,
    }
}

fn control_completion_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, DeckAudioSourceControlCompletion, SOURCE_QUEUE_DEPTH>
{
    match deck {
        DeckId::One => &DECK_A_CONTROL_COMPLETIONS,
        DeckId::Two => &DECK_B_CONTROL_COMPLETIONS,
    }
}

fn pcm_channel(
    deck: DeckId,
) -> &'static Channel<CriticalSectionRawMutex, PcmStreamBlock, PCM_QUEUE_DEPTH> {
    match deck {
        DeckId::One => &DECK_A_PCM,
        DeckId::Two => &DECK_B_PCM,
    }
}

fn placeholder_file(file: Usb0FsFileHandle) -> Usb0FsFileHandle {
    file
}
