use core::cell::RefCell;

use embassy_sync::blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex};
use pajoniiir_media_fat32::Fat32AsyncFileHandle;
use pajoniiir_media_fs::{
    BoundedPath, FileHandle, FileSystemKind,
};
use pajoniiir_media_session::MediaLease;
use pajoniiir_rekordbox::{
    PDB_TABLE_ALBUMS, PDB_TABLE_ARTISTS, PDB_TABLE_KEYS, PDB_TABLE_TRACKS, PdbError, PdbHeader,
    PdbTableDirectory, REKORDBOX_EXPORT_PDB_PATH, parse_pdb_header, parse_pdb_table_directory,
};
use static_cell::StaticCell;

use crate::usb0_msc::{
    USB0_LIBRARY_EVENTS, Usb0FsCompletion, Usb0FsOperationError, Usb0FsRequest, Usb0FsSubmitResult,
    Usb0FilesystemClient, Usb0LibraryEvent, Usb0MscBinding,
};

const USB0_LIBRARY_PDB_PAGE_BYTES: usize = 4_096;

static USB0_LIBRARY_PDB_PAGE: StaticCell<[u8; USB0_LIBRARY_PDB_PAGE_BYTES]> = StaticCell::new();

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
    PageTooLarge {
        required: u32,
        available: u32,
    },
    ShortPageRead {
        required: u32,
        actual: u32,
    },
    MissingTracksTable,
    Pdb(PdbError),
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

enum PageReadResult {
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

async fn read_page(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Fat32AsyncFileHandle,
    buffer: &'static mut [u8],
) -> PageReadResult {
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
            Ok(bytes) => PageReadResult::Complete {
                file,
                buffer,
                bytes,
            },
            Err(error) => PageReadResult::Failed {
                file,
                buffer,
                error: map_filesystem_error(error),
            },
        },
        Usb0FsSubmitResult::QueueFull(Usb0FsRequest::Read { file, buffer, .. }) => {
            PageReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerBusy,
            }
        }
        Usb0FsSubmitResult::PendingCompletion(Usb0FsRequest::Read { file, buffer, .. }) => {
            PageReadResult::Failed {
                file,
                buffer,
                error: Usb0LibraryError::BrokerPending,
            }
        }
        Usb0FsSubmitResult::WrongClient(Usb0FsRequest::Read { file, buffer, .. }) => {
            PageReadResult::Failed {
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

    if header.page_size as usize > buffer.len() {
        return Err(Usb0LibraryError::PageTooLarge {
            required: header.page_size,
            available: buffer.len() as u32,
        });
    }
    if available < header.page_size as usize {
        return Err(Usb0LibraryError::ShortPageRead {
            required: header.page_size,
            actual: available as u32,
        });
    }

    let tables = parse_pdb_table_directory(&buffer[..header.page_size as usize], header)
        .map_err(Usb0LibraryError::Pdb)?;
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
    })
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

    let (file, buffer, read_result) = match read_page(client, lease, file, buffer).await {
        PageReadResult::Complete {
            file,
            buffer,
            bytes,
        } => (file, buffer, Ok(bytes)),
        PageReadResult::Failed {
            file,
            buffer,
            error,
        } => (file, buffer, Err(error)),
    };

    let parse_result = match read_result {
        Ok(bytes) => parse_first_page(lease, file_len, buffer, bytes),
        Err(error) => Err(error),
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
    let mut page_buffer = &mut USB0_LIBRARY_PDB_PAGE.init([0; USB0_LIBRARY_PDB_PAGE_BYTES])[..];

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
    }

    #[test]
    fn first_page_probe_rejects_page_larger_than_owned_buffer() {
        let mut page = [0u8; 512];
        put_le32(&mut page, 4, 4_096);
        put_le32(&mut page, 8, 1);

        assert_eq!(
            parse_first_page(lease(), 8_192, &page, page.len()),
            Err(Usb0LibraryError::PageTooLarge {
                required: 4_096,
                available: 512,
            })
        );
    }
}
