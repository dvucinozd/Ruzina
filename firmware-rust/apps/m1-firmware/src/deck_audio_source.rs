use alloc::vec::Vec;
use allocator_api2::boxed::Box;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
};
use embassy_time::{Duration, Timer};
use ruzina_audio_dsp::PcmFrame;
use ruzina_audio_runtime::{
    decode::{
        AudioCodec, DecodeAction, DecoderStorage, ENCODED_BUFFER_BYTES, Mp3CheckpointStorage,
        SourceDecoder, SourceDescriptor, SourceDuration, StreamChange,
    },
    pcm_stream::{PcmStreamBlock, StreamActivation, next_stream_generation},
    wav::{
        CHUNK_HEADER_BYTES, PCM_FORMAT_BYTES, RIFF_HEADER_BYTES, WavPcm16Decoder,
        WavPcm16SourceDescriptor, parse_chunk_header, parse_pcm16_format, parse_riff_header,
    },
};
use ruzina_controller_session::DeckId;
use ruzina_media_catalog::CatalogLoadCandidate;
use ruzina_media_fs::{BoundedPath, FileHandle};
use static_cell::StaticCell;

use crate::{
    deck_effect_owners::report_scanned_duration,
    product_runtime::ScannedDuration,
    usb0_library::{current_media_lease, wait_for_media_lease_change},
    usb0_msc::{
        Usb0FilesystemClient, Usb0FsCompletion, Usb0FsFileHandle, Usb0FsRequest, Usb0FsSubmitResult,
    },
};

const SOURCE_QUEUE_DEPTH: usize = 1;
// Decoders publish at most 256 frames per block. A queue can fill the bounded
// lookahead in one owner turn, including high-rate/keylock playback. Capacity
// remains fixed; real throughput/jitter still requires hardware measurement.
const PCM_QUEUE_DEPTH: usize = 16;
// Filesystem transfers remain 512 bytes; compressed decoders retain only their
// fixed encoded window and one bounded output staging frame in PSRAM.
const SOURCE_READ_BUFFER_BYTES: usize = 512;
const _: () = assert!(
    PCM_QUEUE_DEPTH * (SOURCE_READ_BUFFER_BYTES / 4)
        >= ruzina_audio_runtime::source::SOURCE_LOOKAHEAD_FRAMES as usize
);

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
static DECK_A_STREAM_GENERATION: StreamActivation = StreamActivation::new();
static DECK_B_STREAM_GENERATION: StreamActivation = StreamActivation::new();

#[derive(Clone, Copy, Debug)]
pub(crate) struct PreparedAudioToken {
    pub(crate) descriptor: SourceDescriptor,
    pub(crate) generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceCommand {
    Prepare {
        request_id: u32,
        candidate: CatalogLoadCandidate,
    },
    Control {
        request_id: u32,
        change: StreamChange,
        expected_generation: u32,
    },
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeckAudioSourceCompletion {
    Prepared {
        request_id: u32,
        descriptor: SourceDescriptor,
        generation: u32,
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
pub(crate) struct AudioSeekResult {
    pub(crate) generation: u32,
    pub(crate) frame: u64,
    pub(crate) sample_rate_hz: u32,
}

struct PreparedAudioSource {
    candidate: CatalogLoadCandidate,
    file: Usb0FsFileHandle,
    descriptor: SourceDescriptor,
    stream: SourceDecoder,
    primed: Option<PcmStreamBlock>,
    eof_queued: bool,
    /// The walked length was handed to the product once (RP-08).
    duration_reported: bool,
}

pub(crate) async fn prepare_audio_source(
    deck: DeckId,
    request_id: u32,
    candidate: CatalogLoadCandidate,
) -> Result<PreparedAudioToken, ()> {
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
            generation,
        } if completed == request_id => Ok(PreparedAudioToken {
            descriptor,
            generation,
        }),
        DeckAudioSourceCompletion::Failed {
            request_id: completed,
        } if completed == request_id => Err(()),
        DeckAudioSourceCompletion::Prepared { .. } | DeckAudioSourceCompletion::Failed { .. } => {
            esp_hal::system::software_reset()
        }
    }
}

pub(crate) async fn seek_audio_source(
    deck: DeckId,
    request_id: u32,
    position_ms: u32,
) -> Result<AudioSeekResult, ()> {
    change_audio_source(
        deck,
        request_id,
        StreamChange::SeekMilliseconds(position_ms),
    )
    .await
}

pub(crate) async fn change_audio_source(
    deck: DeckId,
    request_id: u32,
    change: StreamChange,
) -> Result<AudioSeekResult, ()> {
    let expected_generation = active_stream_generation(deck);
    if expected_generation == 0 {
        return Err(());
    }
    command_channel(deck)
        .send(DeckAudioSourceCommand::Control {
            request_id,
            change,
            expected_generation,
        })
        .await;

    match control_completion_channel(deck).receive().await {
        DeckAudioSourceControlCompletion::Seeked {
            request_id: completed,
            generation,
            frame,
            sample_rate_hz,
        } if completed == request_id => Ok(AudioSeekResult {
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

pub(crate) fn current_stream_generation(deck: DeckId) -> u32 {
    stream_generation_atomic(deck).current()
}

pub(crate) fn active_stream_generation(deck: DeckId) -> u32 {
    stream_generation_atomic(deck).active()
}
pub(crate) fn activate_prepared_source(deck: DeckId, generation: u32) -> bool {
    stream_generation_atomic(deck).activate(generation)
}

pub(crate) async fn run_deck_a_audio_source_owner() -> ! {
    let buffer = DECK_A_READ_BUFFER.init([0u8; SOURCE_READ_BUFFER_BYTES]);
    run_owner(DeckId::One, Usb0FilesystemClient::deck_a(), &mut buffer[..]).await
}

pub(crate) async fn run_deck_b_audio_source_owner() -> ! {
    let buffer = DECK_B_READ_BUFFER.init([0u8; SOURCE_READ_BUFFER_BYTES]);
    run_owner(DeckId::Two, Usb0FilesystemClient::deck_b(), &mut buffer[..]).await
}

async fn run_owner(
    deck: DeckId,
    mut client: Usb0FilesystemClient,
    mut buffer: &'static mut [u8],
) -> ! {
    let mut storage = match allocate_decoder() {
        Ok(storage) => storage,
        Err(_) => allocation_failed(deck).await,
    };
    let mut checkpoints = [None, None, None];
    for slot in &mut checkpoints {
        match allocate_checkpoint() {
            Ok(checkpoint) => *slot = Some(checkpoint),
            Err(_) => allocation_failed(deck).await,
        }
    }
    let mut checkpoints = checkpoints.map(|slot| slot.expect("provisioned checkpoint"));
    let mut encoded = Vec::new();
    if encoded.try_reserve_exact(ENCODED_BUFFER_BYTES).is_err() {
        allocation_failed(deck).await;
    }
    encoded.resize(ENCODED_BUFFER_BYTES, 0u8);
    let mut prepared: Option<PreparedAudioSource> = None;
    let mut pending_block: Option<PcmStreamBlock> = None;
    let mut generation = 0u32;

    loop {
        let command = if let Ok(command) = command_channel(deck).try_receive() {
            Some(command)
        } else if prepared.is_none() {
            match select(
                command_channel(deck).receive(),
                wait_for_media_lease_change(),
            )
            .await
            {
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

                    stream_generation_atomic(deck).invalidate();

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
                    let (next_buffer, result) = prepare_source(
                        deck,
                        &mut client,
                        candidate,
                        buffer,
                        generation,
                        &mut storage,
                        &mut checkpoints,
                        &mut encoded,
                    )
                    .await;
                    buffer = next_buffer;
                    match result {
                        Ok(source) => {
                            let descriptor = source.descriptor;
                            stream_generation_atomic(deck).prepared(source.stream.generation());
                            prepared = Some(source);
                            prepare_completion_channel(deck)
                                .send(DeckAudioSourceCompletion::Prepared {
                                    request_id,
                                    descriptor,
                                    generation,
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
                DeckAudioSourceCommand::Control {
                    request_id,
                    change,
                    expected_generation,
                } => {
                    // A Library Prepare may have overtaken this queued seek.
                    // Never revoke or seek a replacement source on its behalf.
                    if !stream_generation_atomic(deck).accepts_progress(expected_generation) {
                        control_completion_channel(deck)
                            .send(DeckAudioSourceControlCompletion::Failed { request_id })
                            .await;
                        continue;
                    }
                    pending_block = None;
                    drain_pcm_blocks(deck);

                    let was_active = active_stream_generation(deck) != 0;
                    stream_generation_atomic(deck).invalidate();

                    let result = if let Some(mut source) = prepared.take() {
                        if current_media_lease() != Some(source.candidate.publication.lease) {
                            Err(())
                        } else {
                            generation = next_stream_generation(generation);
                            let (next_buffer, seek_result) = seek_source(
                                deck,
                                &mut client,
                                &mut source,
                                generation,
                                change,
                                buffer,
                                &mut storage,
                                &mut checkpoints,
                                &mut encoded,
                            )
                            .await;
                            buffer = next_buffer;
                            match seek_result {
                                Ok(result) => {
                                    stream_generation_atomic(deck).prepared(result.generation);
                                    if was_active {
                                        stream_generation_atomic(deck).activate(result.generation);
                                    }
                                    prepared = Some(source);
                                    Ok(result)
                                }
                                Err(()) => {
                                    // The generation is already revoked. Release
                                    // only this deck's handle and let it reload.
                                    let _ = close_file(
                                        &mut client,
                                        source.candidate.publication.lease,
                                        source.file,
                                    )
                                    .await;
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
                    stream_generation_atomic(deck).invalidate();
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

        if prepared
            .as_ref()
            .is_some_and(|source| current_media_lease() != Some(source.candidate.publication.lease))
        {
            stream_generation_atomic(deck).invalidate();
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
                    let scanned = match prepared.as_mut() {
                        Some(source) => {
                            let (next_buffer, scanned) =
                                scan_frame_index(deck, &mut client, source, buffer, &mut storage)
                                    .await;
                            buffer = next_buffer;
                            scanned
                        }
                        None => false,
                    };
                    if !scanned {
                        Timer::after(Duration::from_millis(1)).await;
                    }
                }
            }
            continue;
        }

        let Some(source) = prepared.take() else {
            continue;
        };

        if source.eof_queued {
            prepared = Some(source);
            Timer::after(Duration::from_millis(5)).await;
            continue;
        }

        let (next_buffer, source, produced) = produce_pcm_block(
            &mut client,
            source,
            buffer,
            &mut storage,
            &mut checkpoints,
            &mut encoded,
        )
        .await;
        buffer = next_buffer;
        prepared = Some(source);
        pending_block = produced;
    }
}

// Isolate provisioning from the large async poll frame. Cloning immutable flash
// templates avoids materializing the full FLAC/context aggregate on its stack.
// The remaining MP3 clone scratch is checked in the retained P4 assembly audit.
#[inline(never)]
fn allocate_decoder() -> Result<Box<DecoderStorage>, allocator_api2::alloc::AllocError> {
    static INITIAL: DecoderStorage = DecoderStorage::new();
    let slot = Box::<DecoderStorage>::try_new_uninit()?;
    Ok(Box::write(slot, INITIAL.clone()))
}

#[inline(never)]
fn allocate_checkpoint() -> Result<Box<Mp3CheckpointStorage>, allocator_api2::alloc::AllocError> {
    static INITIAL: Mp3CheckpointStorage = Mp3CheckpointStorage::new();
    let slot = Box::<Mp3CheckpointStorage>::try_new_uninit()?;
    Ok(Box::write(slot, INITIAL.clone()))
}

fn checkpoint_views(slots: &mut [Box<Mp3CheckpointStorage>; 3]) -> [&mut Mp3CheckpointStorage; 3] {
    let [a, b, c] = slots;
    [a.as_mut(), b.as_mut(), c.as_mut()]
}

async fn prepare_source(
    deck: DeckId,
    client: &mut Usb0FilesystemClient,
    candidate: CatalogLoadCandidate,
    buffer: &'static mut [u8],
    generation: u32,
    storage: &mut DecoderStorage,
    checkpoints: &mut [Box<Mp3CheckpointStorage>; 3],
    encoded: &mut [u8],
) -> (&'static mut [u8], Result<PreparedAudioSource, ()>) {
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

    if &buffer[..4] != b"RIFF" {
        let codec = if &buffer[..4] == b"fLaC" {
            AudioCodec::Flac
        } else if &buffer[..3] == b"ID3" || buffer[0] == 0xff && buffer[1] & 0xe0 == 0xe0 {
            AudioCodec::Mp3
        } else {
            let _ = close_file(client, lease, file).await;
            return (buffer, Err(()));
        };
        let Some(mut stream) = SourceDecoder::compressed(codec, file_len, generation, storage)
        else {
            let _ = close_file(client, lease, file).await;
            return (buffer, Err(()));
        };
        let (buffer, file, primed) = prime(
            deck,
            client,
            lease,
            file,
            &mut stream,
            buffer,
            storage,
            checkpoints,
            encoded,
        )
        .await;
        return match primed {
            Some(block) if block.len != 0 && !block.faulted => {
                let descriptor = stream.descriptor().expect("decoded source descriptor");
                (
                    buffer,
                    Ok(PreparedAudioSource {
                        candidate,
                        file,
                        descriptor,
                        stream,
                        primed: Some(block),
                        eof_queued: false,
                        duration_reported: false,
                    }),
                )
            }
            _ => {
                let _ = close_file(client, lease, file).await;
                (buffer, Err(()))
            }
        };
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
        let (next_file, next_buffer, bytes) = read_at(client, lease, file, buffer, position).await;
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
            let descriptor =
                match WavPcm16SourceDescriptor::new(format, chunk_data, u64::from(chunk.len)) {
                    Ok(descriptor) if descriptor.data_bytes >= u64::from(format.block_align) => {
                        descriptor
                    }
                    Ok(_) | Err(_) => break,
                };

            let (next_file, next_buffer, bytes) =
                read_at(client, lease, file, buffer, descriptor.data_offset).await;
            file = next_file;
            buffer = next_buffer;
            let mut probe = WavPcm16Decoder::new(descriptor.format, descriptor.data_bytes)
                .expect("descriptor construction validates WAV data alignment before decode probe");
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
            let mut stream =
                SourceDecoder::wav(descriptor, file_len, generation).expect("validated WAV stream");
            let (buffer, file, primed) = prime(
                deck,
                client,
                lease,
                file,
                &mut stream,
                buffer,
                storage,
                checkpoints,
                encoded,
            )
            .await;
            return match primed {
                Some(block) if block.len != 0 && !block.faulted => (
                    buffer,
                    Ok(PreparedAudioSource {
                        candidate,
                        file,
                        descriptor: descriptor.into(),
                        stream,
                        primed: Some(block),
                        eof_queued: false,
                        duration_reported: false,
                    }),
                ),
                _ => {
                    let _ = close_file(client, lease, file).await;
                    (buffer, Err(()))
                }
            };
        }

        position = next_position;
    }

    let _ = close_file(client, lease, file).await;
    (buffer, Err(()))
}

async fn seek_source(
    deck: DeckId,
    client: &mut Usb0FilesystemClient,
    source: &mut PreparedAudioSource,
    generation: u32,
    change: StreamChange,
    buffer: &'static mut [u8],
    storage: &mut DecoderStorage,
    checkpoints: &mut [Box<Mp3CheckpointStorage>; 3],
    encoded: &mut [u8],
) -> (&'static mut [u8], Result<AudioSeekResult, ()>) {
    if !source.stream.change(
        storage,
        &mut checkpoint_views(checkpoints),
        encoded,
        generation,
        change,
    ) {
        return (buffer, Err(()));
    }
    let frame = source.stream.media_frame();
    let (buffer, file, primed) = prime(
        deck,
        client,
        source.candidate.publication.lease,
        source.file,
        &mut source.stream,
        buffer,
        storage,
        checkpoints,
        encoded,
    )
    .await;
    source.file = file;
    match primed {
        Some(block) if !block.faulted => {
            source.primed = Some(block);
            source.eof_queued = false;
            (
                buffer,
                Ok(AudioSeekResult {
                    generation,
                    frame,
                    sample_rate_hz: source.descriptor.sample_rate_hz(),
                }),
            )
        }
        _ => (buffer, Err(())),
    }
}

async fn prime(
    deck: DeckId,
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
    mut file: Usb0FsFileHandle,
    stream: &mut SourceDecoder,
    mut buffer: &'static mut [u8],
    storage: &mut DecoderStorage,
    checkpoints: &mut [Box<Mp3CheckpointStorage>; 3],
    encoded: &mut [u8],
) -> (&'static mut [u8], Usb0FsFileHandle, Option<PcmStreamBlock>) {
    loop {
        if current_media_lease() != Some(lease) || !command_channel(deck).is_empty() {
            return (buffer, file, None);
        }
        match stream.step(storage, &mut checkpoint_views(checkpoints), encoded) {
            DecodeAction::Read { offset, maximum } => {
                let (next_file, next_buffer, bytes) =
                    read_at(client, lease, file, buffer, offset).await;
                file = next_file;
                buffer = next_buffer;
                if bytes > maximum || !stream.provide(encoded, &buffer[..bytes]) {
                    return (buffer, file, None);
                }
            }
            DecodeAction::Pending => embassy_futures::yield_now().await,
            DecodeAction::Block(block) => return (buffer, file, Some(block)),
        }
    }
}

/// One bounded read of the background MP3 frame walk, taken only while the
/// PCM queue is full so it never delays a block (RP-08). Returns false when
/// the source has nothing left to walk; a complete untagged walk then hands
/// its length to the product once. A failed read is retried later.
async fn scan_frame_index(
    deck: DeckId,
    client: &mut Usb0FilesystemClient,
    source: &mut PreparedAudioSource,
    buffer: &'static mut [u8],
    storage: &mut DecoderStorage,
) -> (&'static mut [u8], bool) {
    let Some((offset, maximum)) = source.stream.index_read(storage, buffer.len()) else {
        if !source.duration_reported
            && let SourceDuration::Scanned(_) = source.stream.duration(storage)
        {
            source.duration_reported = true;
            if let Some(duration_ms) = source
                .stream
                .duration(storage)
                .millis(source.descriptor.sample_rate_hz())
            {
                report_scanned_duration(
                    deck,
                    ScannedDuration {
                        track: source.candidate.track.track_id,
                        lease: source.candidate.publication.lease,
                        duration_ms,
                    },
                );
            }
        }
        return (buffer, false);
    };
    let (file, buffer, bytes) = read_at(
        client,
        source.candidate.publication.lease,
        source.file,
        buffer,
        offset,
    )
    .await;
    source.file = file;
    if bytes != 0 {
        let _ = source
            .stream
            .provide_index(storage, offset, &buffer[..bytes.min(maximum)]);
    }
    (buffer, true)
}

async fn produce_pcm_block(
    client: &mut Usb0FilesystemClient,
    mut source: PreparedAudioSource,
    buffer: &'static mut [u8],
    storage: &mut DecoderStorage,
    checkpoints: &mut [Box<Mp3CheckpointStorage>; 3],
    encoded: &mut [u8],
) -> (
    &'static mut [u8],
    PreparedAudioSource,
    Option<PcmStreamBlock>,
) {
    if let Some(block) = source.primed.take() {
        source.eof_queued = block.eof || block.faulted;
        return (buffer, source, Some(block));
    }
    let block = match source
        .stream
        .step(storage, &mut checkpoint_views(checkpoints), encoded)
    {
        DecodeAction::Read { offset, maximum } => {
            let (file, buffer, bytes) = read_at(
                client,
                source.candidate.publication.lease,
                source.file,
                buffer,
                offset,
            )
            .await;
            source.file = file;
            let block = if bytes > maximum || !source.stream.provide(encoded, &buffer[..bytes]) {
                source.eof_queued = true;
                Some(source.stream.fault_block())
            } else {
                None
            };
            return (buffer, source, block);
        }
        DecodeAction::Pending => {
            embassy_futures::yield_now().await;
            None
        }
        DecodeAction::Block(block) => Some(block),
    };
    source.eof_queued = block.as_ref().is_some_and(|b| b.eof || b.faulted);
    (buffer, source, block)
}

async fn allocation_failed(deck: DeckId) -> ! {
    loop {
        match command_channel(deck).receive().await {
            DeckAudioSourceCommand::Prepare { request_id, .. } => {
                prepare_completion_channel(deck)
                    .send(DeckAudioSourceCompletion::Failed { request_id })
                    .await
            }
            DeckAudioSourceCommand::Control { request_id, .. } => {
                control_completion_channel(deck)
                    .send(DeckAudioSourceControlCompletion::Failed { request_id })
                    .await
            }
            DeckAudioSourceCommand::Release => stream_generation_atomic(deck).invalidate(),
        }
    }
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
    read_next(client, lease, file, buffer).await
}

async fn read_next(
    client: &mut Usb0FilesystemClient,
    lease: ruzina_media_session::MediaLease,
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

fn stream_generation_atomic(deck: DeckId) -> &'static StreamActivation {
    match deck {
        DeckId::One => &DECK_A_STREAM_GENERATION,
        DeckId::Two => &DECK_B_STREAM_GENERATION,
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
