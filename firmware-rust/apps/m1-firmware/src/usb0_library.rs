use core::cell::RefCell;

use embassy_sync::blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex};
use pajoniiir_media_fat32::Fat32AsyncFileHandle;
use pajoniiir_media_fs::{
    BoundedPath, FileHandle, FileSystemKind,
};
use pajoniiir_media_session::MediaLease;
use pajoniiir_rekordbox::{
    PDB_TABLE_ALBUMS, PDB_TABLE_ARTISTS, PDB_TABLE_KEYS, PDB_TABLE_TRACKS, PdbError, PdbHeader,
    PdbPageSlotWalker, PdbTableDirectory, REKORDBOX_EXPORT_PDB_PATH, parse_pdb_header,
    parse_pdb_page_header, parse_pdb_table_directory,
};
use static_cell::StaticCell;

use crate::usb0_msc::{
    USB0_LIBRARY_EVENTS, Usb0FsCompletion, Usb0FsOperationError, Usb0FsRequest, Usb0FsSubmitResult,
    Usb0FilesystemClient, Usb0LibraryEvent, Usb0MscBinding,
};

const USB0_LIBRARY_PDB_IO_BYTES: usize = 8_192;

static USB0_LIBRARY_PDB_IO: StaticCell<[u8; USB0_LIBRARY_PDB_IO_BYTES]> = StaticCell::new();

static USB0_LIBRARY_STATE: CriticalSectionMutex<RefCell<Usb0LibraryState>> =
    CriticalSectionMutex::new(RefCell::new(Usb0LibraryState::Idle));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryError {
    FilesystemUnavailable,
    UnsupportedFilesystem(FileSystemKind),
    PathTooLong,
    BrokerBusy,
    BrokerPending,
    BrokerWrongClient,
    FilesystemNoMedia,
    FilesystemStaleLease,
    FilesystemExFatUnavailable,
    FilesystemFat32,
    MissingTracksTable,
    PageOutOfRange(u32),
    PageChainLoop,
    RowCountOverflow,
    Pdb(PdbError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryTableProbe {
    pub(crate) pages: u32,
    pub(crate) declared_rows: u32,
    pub(crate) present_rows: u32,
    pub(crate) truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryPdbProbe {
    pub(crate) lease: MediaLease,
    pub(crate) file_len: u64,
    pub(crate) header: PdbHeader,
    pub(crate) tracks_first_page: u32,
    pub(crate) artists_first_page: Option<u32>,
    pub(crate) albums_first_page: Option<u32>,
    pub(crate) keys_first_page: Option<u32>,
    pub(crate) tracks_layout: Option<Usb0LibraryTableProbe>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryState {
    Idle,
    Probing {
        binding: Usb0MscBinding,
    },
    Ready {
        binding: Usb0MscBinding,
        pdb: Usb0LibraryPdbProbe,
    },
    Failed {
        binding: Usb0MscBinding,
        error: Usb0LibraryError,
    },
    Detached {
        binding: Usb0MscBinding,
    },
}

fn set_library_state(state: Usb0LibraryState) {
    USB0_LIBRARY_STATE.lock(|cell| {
        *cell.borrow_mut() = state;
    });
}

pub(crate) fn library_state() -> Usb0LibraryState {
    USB0_LIBRARY_STATE.lock(|cell| *cell.borrow())
}

fn map_filesystem_error(error: Usb0FsOperationError) -> Usb0LibraryError {
    match error {
        Usb0FsOperationError::NoMedia => Usb0LibraryError::FilesystemNoMedia,
        Usb0FsOperationError::StaleLease => Usb0LibraryError::FilesystemStaleLease,
        Usb0FsOperationError::ExFatUnavailable => Usb0LibraryError::FilesystemExFatUnavailable,
        Usb0FsOperationError::Fat32(_) => Usb0LibraryError::FilesystemFat32,
    }
}

fn broker_invariant_reset() -> ! {
    esp_hal::system::software_reset()
}

async fn open_export_pdb(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
) -> Result<Fat32AsyncFileHandle, Usb0LibraryError> {
    let path =
        BoundedPath::new(REKORDBOX_EXPORT_PDB_PATH).map_err(|_| Usb0LibraryError::PathTooLong)?;
    let ticket = client.issue_ticket(lease);

    match client.submit(Usb0FsRequest::OpenRead { ticket, path }).await {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::OpenRead { result, .. }) => {
            result.map_err(map_filesystem_error)
        }
        Usb0FsSubmitResult::QueueFull(_) => Err(Usb0LibraryError::BrokerBusy),
        Usb0FsSubmitResult::PendingCompletion(_) => Err(Usb0LibraryError::BrokerPending),
        Usb0FsSubmitResult::WrongClient(_) => Err(Usb0LibraryError::BrokerWrongClient),
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

enum PrefixReadResult {
    Complete {
        file: Fat32AsyncFileHandle,
        buffer: &'static mut [u8],
        bytes: usize,
    },
    Failed {
        file: Fat32AsyncFileHandle,
        buffer: &'static mut [u8],
        error: Usb0LibraryError,
    },
}

enum SeekResult {
    Complete {
        file: Fat32AsyncFileHandle,
    },
    Failed {
        file: Fat32AsyncFileHandle,
        error: Usb0LibraryError,
    },
}

async fn seek_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Fat32AsyncFileHandle,
    position: u64,
) -> SeekResult {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::Seek {
            ticket,
            file,
            position,
        })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::Seek {
            file, result, ..
        }) => match result {
            Ok(()) => SeekResult::Complete { file },
            Err(error) => SeekResult::Failed {
                file,
                error: map_filesystem_error(error),
            },
        },
        Usb0FsSubmitResult::QueueFull(Usb0FsRequest::Seek { file, .. }) => SeekResult::Failed {
            file,
            error: Usb0LibraryError::BrokerBusy,
        },
        Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Seek { file, .. }) => {
            SeekResult::Failed {
                file,
                error: Usb0LibraryError::BrokerPending,
            }
        }
        Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Seek { file, .. }) => SeekResult::Failed {
            file,
            error: Usb0LibraryError::BrokerWrongClient,
        },
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::QueueFull(_)
        | Usb0FsSubmitResult::PendingCompletion(_)
        | Usb0FsSubmitResult::WrongClient(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

async fn read_prefix(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Fat32AsyncFileHandle,
    buffer: &'static mut [u8],
) -> PrefixReadResult {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::Read {
            ticket,
            file,
            buffer,
        })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::Read {
            file,
            buffer,
            result,
            ..
        }) => match result {
            Ok(bytes) => PrefixReadResult::Complete {
                file,
                buffer,
                bytes,
            },
            Err(error) => PrefixReadResult::Failed {
                file,
                buffer,
                error: map_filesystem_error(error),
            },
        },
        Usb0FsSubmitResult::QueueFull(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerBusy,
            }
        }
        Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerPending,
            }
        }
        Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Read { file, buffer, .. }) => {
            PrefixReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerWrongClient,
            }
        }
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::QueueFull(_)
        | Usb0FsSubmitResult::PendingCompletion(_)
        | Usb0FsSubmitResult::WrongClient(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

async fn read_at(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Fat32AsyncFileHandle,
    buffer: &'static mut [u8],
    position: u64,
) -> PrefixReadResult {
    let file = match seek_file(client, lease, file, position).await {
        SeekResult::Complete { file } => file,
        SeekResult::Failed { file, error } => {
            return PrefixReadResult::Failed {
                file,
                buffer,
                error,
            };
        }
    };
    read_prefix(client, lease, file, buffer).await
}

async fn close_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Fat32AsyncFileHandle,
) -> Result<(), Usb0LibraryError> {
    let ticket = client.issue_ticket(lease);
    match client
        .submit(Usb0FsRequest::CloseFile { ticket, file })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::CloseFile { result, .. }) => {
            result.map_err(map_filesystem_error)
        }
        Usb0FsSubmitResult::QueueFull(_) => Err(Usb0LibraryError::BrokerBusy),
        Usb0FsSubmitResult::PendingCompletion(_) => Err(Usb0LibraryError::BrokerPending),
        Usb0FsSubmitResult::WrongClient(_) => Err(Usb0LibraryError::BrokerWrongClient),
        Usb0FsSubmitResult::Completed(_)
        | Usb0FsSubmitResult::UnexpectedCompletion { .. } => broker_invariant_reset(),
    }
}

fn first_page(tables: &PdbTableDirectory, kind: u32) -> Option<u32> {
    tables
        .find(kind)
        .and_then(|entry| (entry.first_page != u32::MAX).then_some(entry.first_page))
}

fn parse_first_page(
    lease: MediaLease,
    file_len: u64,
    buffer: &[u8],
    bytes: usize,
) -> Result<Usb0LibraryPdbProbe, Usb0LibraryError> {
    let available = bytes.min(buffer.len());
    let header = parse_pdb_header(&buffer[..available], file_len).map_err(Usb0LibraryError::Pdb)?;
    let tables =
        parse_pdb_table_directory(&buffer[..available], header).map_err(Usb0LibraryError::Pdb)?;
    let tracks_first_page =
        first_page(&tables, PDB_TABLE_TRACKS).ok_or(Usb0LibraryError::MissingTracksTable)?;

    Ok(Usb0LibraryPdbProbe {
        lease,
        file_len,
        header,
        tracks_first_page,
        artists_first_page: first_page(&tables, PDB_TABLE_ARTISTS),
        albums_first_page: first_page(&tables, PDB_TABLE_ALBUMS),
        keys_first_page: first_page(&tables, PDB_TABLE_KEYS),
        tracks_layout: None,
    })
}

async fn scan_table_layout(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    mut file: Fat32AsyncFileHandle,
    mut buffer: &'static mut [u8],
    header: PdbHeader,
    first_page: u32,
) -> (
    Fat32AsyncFileHandle,
    &'static mut [u8],
    Result<Usb0LibraryTableProbe, Usb0LibraryError>,
) {
    let mut page = first_page;
    let mut pages = 0u32;
    let mut declared_rows = 0u32;
    let mut present_rows = 0u32;
    let mut truncated = false;

    loop {
        if page >= header.total_pages {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::PageOutOfRange(page)),
            );
        }
        if pages >= header.total_pages {
            return (file, buffer, Err(Usb0LibraryError::PageChainLoop));
        }

        let page_start = u64::from(page) * u64::from(header.page_size);
        let header_read = read_at(client, lease, file, buffer, page_start).await;
        let (next_file, next_buffer, bytes) = match header_read {
            PrefixReadResult::Complete {
                file,
                buffer,
                bytes,
            } => (file, buffer, bytes),
            PrefixReadResult::Failed {
                file,
                buffer,
                error,
            } => return (file, buffer, Err(error)),
        };
        file = next_file;
        buffer = next_buffer;

        let page_header = match parse_pdb_page_header(&buffer[..bytes], header.page_size) {
            Ok(page_header) => page_header,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };
        let mut walker = match PdbPageSlotWalker::new(header.page_size, page_header.declared_rows) {
            Ok(walker) => walker,
            Err(error) => return (file, buffer, Err(Usb0LibraryError::Pdb(error))),
        };

        while let Some(plan) = walker.next_batch_plan() {
            let absolute = page_start + u64::from(plan.offset);
            let slot_read = read_at(client, lease, file, buffer, absolute).await;
            let (next_file, next_buffer, bytes) = match slot_read {
                PrefixReadResult::Complete {
                    file,
                    buffer,
                    bytes,
                } => (file, buffer, bytes),
                PrefixReadResult::Failed {
                    file,
                    buffer,
                    error,
                } => return (file, buffer, Err(error)),
            };
            file = next_file;
            buffer = next_buffer;

            if let Err(error) = walker.ingest_batch(plan, &buffer[..bytes]) {
                return (file, buffer, Err(Usb0LibraryError::Pdb(error)));
            }
        }

        let summary = walker.summary();
        declared_rows = match declared_rows.checked_add(summary.declared_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        present_rows = match present_rows.checked_add(summary.present_rows) {
            Some(total) => total,
            None => return (file, buffer, Err(Usb0LibraryError::RowCountOverflow)),
        };
        truncated |= summary.truncated;
        pages += 1;

        if page_header.next_page == u32::MAX {
            break;
        }
        if page_header.next_page >= header.total_pages {
            return (
                file,
                buffer,
                Err(Usb0LibraryError::PageOutOfRange(page_header.next_page)),
            );
        }
        page = page_header.next_page;
    }

    (
        file,
        buffer,
        Ok(Usb0LibraryTableProbe {
            pages,
            declared_rows,
            present_rows,
            truncated,
        }),
    )
}

async fn probe_export_pdb(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    buffer: &'static mut [u8],
) -> (
    &'static mut [u8],
    Result<Usb0LibraryPdbProbe, Usb0LibraryError>,
) {
    let file = match open_export_pdb(client, lease).await {
        Ok(file) => file,
        Err(error) => return (buffer, Err(error)),
    };
    let file_len = file.len();

    let (file, buffer, read_result) = match read_prefix(client, lease, file, buffer).await {
        PrefixReadResult::Complete {
            file,
            buffer,
            bytes,
        } => (file, buffer, Ok(bytes)),
        PrefixReadResult::Failed {
            file,
            buffer,
            error,
        } => (file, buffer, Err(error)),
    };

    let parse_result = match read_result {
        Ok(bytes) => parse_first_page(lease, file_len, buffer, bytes),
        Err(error) => Err(error),
    };

    let (file, buffer, parse_result) = match parse_result {
        Ok(mut probe) => {
            let (file, buffer, layout) = scan_table_layout(
                client,
                lease,
                file,
                buffer,
                probe.header,
                probe.tracks_first_page,
            )
            .await;
            let result = layout.map(|layout| {
                probe.tracks_layout = Some(layout);
                probe
            });
            (file, buffer, result)
        }
        Err(error) => (file, buffer, Err(error)),
    };

    let close_result = close_file(client, lease, file).await;
    let result = match parse_result {
        Err(error) => Err(error),
        Ok(probe) => close_result.map(|()| probe),
    };

    (buffer, result)
}

pub(crate) async fn run_usb0_library_worker() -> ! {
    let mut client = Usb0FilesystemClient::library();
    let mut page_buffer = &mut USB0_LIBRARY_PDB_IO.init([0; USB0_LIBRARY_PDB_IO_BYTES])[..];

    loop {
        match USB0_LIBRARY_EVENTS.wait().await {
            Usb0LibraryEvent::Ready { binding, kind } => {
                if kind != FileSystemKind::Fat32 {
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::UnsupportedFilesystem(kind),
                    });
                    continue;
                }

                set_library_state(Usb0LibraryState::Probing { binding });
                let (restored, result) =
                    probe_export_pdb(&mut client, binding.lease, page_buffer).await;
                page_buffer = restored;

                match result {
                    Ok(pdb) => {
                        set_library_state(Usb0LibraryState::Ready { binding, pdb });
                    }
                    Err(error) => {
                        set_library_state(Usb0LibraryState::Failed { binding, error });
                    }
                }
            }
            Usb0LibraryEvent::FilesystemFailed { binding } => {
                set_library_state(Usb0LibraryState::Failed {
                    binding,
                    error: Usb0LibraryError::FilesystemUnavailable,
                });
            }
            Usb0LibraryEvent::Detached { binding } => {
                set_library_state(Usb0LibraryState::Detached { binding });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease() -> MediaLease {
        use pajoniiir_media_session::{MediaGeneration, MediaSourceId};

        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        }
    }

    fn put_le32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn first_page_probe_extracts_rekordbox_table_roots() {
        let mut page = [0u8; 512];
        put_le32(&mut page, 4, 512);
        put_le32(&mut page, 8, 4);

        for (index, (kind, first_page)) in [
            (PDB_TABLE_TRACKS, 1u32),
            (PDB_TABLE_ARTISTS, 2),
            (PDB_TABLE_ALBUMS, 3),
            (PDB_TABLE_KEYS, 4),
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 0x1c + index * 16;
            put_le32(&mut page, offset, kind);
            put_le32(&mut page, offset + 8, first_page);
            put_le32(&mut page, offset + 12, first_page);
        }

        let probe = parse_first_page(lease(), 512 * 6, &page, page.len()).unwrap();
        assert_eq!(probe.header.page_size, 512);
        assert_eq!(probe.tracks_first_page, 1);
        assert_eq!(probe.artists_first_page, Some(2));
        assert_eq!(probe.albums_first_page, Some(3));
        assert_eq!(probe.keys_first_page, Some(4));
        assert_eq!(probe.tracks_layout, None);
    }

    #[test]
    fn first_page_probe_accepts_large_pdb_page_from_bounded_prefix() {
        let mut prefix = [0u8; 64];
        put_le32(&mut prefix, 4, 65_536);
        put_le32(&mut prefix, 8, 1);
        put_le32(&mut prefix, 0x1c, PDB_TABLE_TRACKS);
        put_le32(&mut prefix, 0x24, 1);
        put_le32(&mut prefix, 0x28, 1);

        let probe =
            parse_first_page(lease(), 65_536 * 2, &prefix, prefix.len()).unwrap();

        assert_eq!(probe.header.page_size, 65_536);
        assert_eq!(probe.tracks_first_page, 1);
    }
}
