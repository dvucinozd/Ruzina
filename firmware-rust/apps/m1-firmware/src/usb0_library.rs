use core::cell::RefCell;

use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    signal::Signal,
};
use ruzina_media_catalog::CatalogBuildError;
use ruzina_media_catalog::CatalogLoadCandidate;
use ruzina_media_catalog::CatalogPublicationId;
#[cfg(not(feature = "p4-psram-storage"))]
use ruzina_media_catalog::playlists::PlaylistImportStats;
use ruzina_media_catalog::playlists::{LibraryActivation, LibraryBrowser};
use ruzina_media_fs::BoundedPath;
use ruzina_media_identity::IdentityError;
#[cfg(not(feature = "p4-psram-storage"))]
use ruzina_media_identity::VolumeIdentity;
use ruzina_media_load::pdb::{
    CatalogError, CatalogFiles, CatalogStage, ImportError, PdbNameStage, PdbProbe,
    StageAllocationError, probe_export_pdb,
};
#[cfg(not(feature = "p4-psram-storage"))]
use ruzina_media_load::pdb::{PdbNameStore, PdbTableProbe};
#[cfg(feature = "p4-psram-storage")]
use ruzina_media_load::pdb::{VecCatalogStage, VecNameMap};
use ruzina_media_session::MediaLease;
#[cfg(not(feature = "p4-psram-storage"))]
use ruzina_rekordbox::{
    PDB_NAME_MAX, PdbNameEntry, PdbNameLookup, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PdbText,
    PdbTrackRow,
};
use ruzina_rekordbox::{PdbCatalogError, PdbError};
use static_cell::StaticCell;

use crate::product_runtime::{LibraryEffect, set_library_cursor_on_node};
use crate::usb0_msc::{
    USB0_LIBRARY_EVENTS, Usb0FilesystemClient, Usb0FsCompletion, Usb0FsFileHandle,
    Usb0FsOperationError, Usb0FsRequest, Usb0FsSubmitResult, Usb0LibraryEvent, Usb0MscBinding,
};

const USB0_LIBRARY_PDB_IO_BYTES: usize = 8_192;

static USB0_LIBRARY_PDB_IO: StaticCell<[u8; USB0_LIBRARY_PDB_IO_BYTES]> = StaticCell::new();

static USB0_LIBRARY_STATE: CriticalSectionMutex<RefCell<Usb0LibraryState>> =
    CriticalSectionMutex::new(RefCell::new(Usb0LibraryState::Idle));

const USB0_LIBRARY_COMMAND_DEPTH: usize = 1;
const USB0_LIBRARY_COMPLETION_DEPTH: usize = 1;

pub(crate) static USB0_LIBRARY_COMMANDS: Channel<
    CriticalSectionRawMutex,
    Usb0LibraryCommand,
    USB0_LIBRARY_COMMAND_DEPTH,
> = Channel::new();

pub(crate) static USB0_LIBRARY_COMPLETIONS: Channel<
    CriticalSectionRawMutex,
    Usb0LibraryCompletion,
    USB0_LIBRARY_COMPLETION_DEPTH,
> = Channel::new();

static USB0_LIBRARY_PUBLICATION: CriticalSectionMutex<RefCell<Option<CatalogPublicationId>>> =
    CriticalSectionMutex::new(RefCell::new(None));
static USB0_LIBRARY_MEDIA_LEASE: CriticalSectionMutex<RefCell<Option<MediaLease>>> =
    CriticalSectionMutex::new(RefCell::new(None));
static USB0_LIBRARY_MEDIA_LEASE_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb0LibraryCommand {
    pub(crate) request_id: u32,
    pub(crate) effect: LibraryEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryCommandError {
    CatalogUnavailable,
    EmptyCatalog,
    StaleSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryCompletion {
    Applied {
        request_id: u32,
    },
    LoadResolved {
        request_id: u32,
        deck: ruzina_controller_session::DeckId,
        candidate: CatalogLoadCandidate,
    },
    Failed {
        request_id: u32,
        error: Usb0LibraryCommandError,
    },
}

pub(crate) fn catalog_publication_matches(publication: CatalogPublicationId) -> bool {
    USB0_LIBRARY_PUBLICATION.lock(|cell| *cell.borrow() == Some(publication))
}

pub(crate) fn current_media_lease() -> Option<MediaLease> {
    USB0_LIBRARY_MEDIA_LEASE.lock(|cell| *cell.borrow())
}

pub(crate) async fn wait_for_media_lease_change() {
    USB0_LIBRARY_MEDIA_LEASE_CHANGED.wait().await;
}

fn set_media_lease(lease: MediaLease) {
    let changed = USB0_LIBRARY_MEDIA_LEASE.lock(|cell| {
        let mut current = cell.borrow_mut();
        if *current == Some(lease) {
            false
        } else {
            *current = Some(lease);
            true
        }
    });
    if changed {
        USB0_LIBRARY_MEDIA_LEASE_CHANGED.signal(());
    }
}

fn clear_media_lease(lease: MediaLease) {
    let changed = USB0_LIBRARY_MEDIA_LEASE.lock(|cell| {
        let mut current = cell.borrow_mut();
        if *current == Some(lease) {
            *current = None;
            true
        } else {
            false
        }
    });
    if changed {
        USB0_LIBRARY_MEDIA_LEASE_CHANGED.signal(());
    }
}

fn set_catalog_publication(publication: Option<CatalogPublicationId>) {
    USB0_LIBRARY_PUBLICATION.lock(|cell| {
        *cell.borrow_mut() = publication;
    });
}

/// Mirrors the cursor for load admission and stale-candidate checks.
fn publish_browser(browser: &LibraryBrowser) {
    set_catalog_publication(browser.publication());
    set_library_cursor_on_node(browser.on_node_row());
}

#[cfg(feature = "p4-psram-storage")]
type Usb0LibraryNameMap = VecNameMap;
#[cfg(feature = "p4-psram-storage")]
type Usb0LibraryCatalogStage = VecCatalogStage;
type Usb0LibraryNameStage = PdbNameStage<Usb0LibraryNameMap>;
pub(crate) type Usb0LibraryPdbProbe = PdbProbe;

#[cfg(feature = "p4-psram-storage")]
fn new_name_stage() -> Result<Usb0LibraryNameStage, StageAllocationError> {
    PdbNameStage::try_new()
}

/// Without PSRAM storage the import still walks the export but keeps no rows.
#[cfg(not(feature = "p4-psram-storage"))]
struct Usb0LibraryNameMap;

#[cfg(not(feature = "p4-psram-storage"))]
impl PdbNameLookup for Usb0LibraryNameMap {
    fn lookup(&self, _id: u32) -> Option<&PdbText<PDB_NAME_MAX>> {
        None
    }
}

#[cfg(not(feature = "p4-psram-storage"))]
impl PdbNameStore for Usb0LibraryNameMap {
    fn clear(&mut self) {}

    fn retain(&mut self, _entry: PdbNameEntry) -> Result<(), CatalogError> {
        Ok(())
    }
}

#[cfg(not(feature = "p4-psram-storage"))]
fn new_name_stage() -> Result<Usb0LibraryNameStage, StageAllocationError> {
    Ok(PdbNameStage {
        artists: Usb0LibraryNameMap,
        albums: Usb0LibraryNameMap,
        keys: Usb0LibraryNameMap,
        artworks: Usb0LibraryNameMap,
    })
}

#[cfg(not(feature = "p4-psram-storage"))]
struct Usb0LibraryCatalogStage;

#[cfg(not(feature = "p4-psram-storage"))]
impl Usb0LibraryCatalogStage {
    fn try_new() -> Result<Self, StageAllocationError> {
        Ok(Self)
    }

    fn prepare_generation(&mut self, _lease: MediaLease) {}

    fn invalidate(&mut self, _lease: MediaLease) {}

    fn sync_selection(&self, browser: &mut LibraryBrowser) {
        browser.clear();
    }

    fn browse_back(&self, _browser: &mut LibraryBrowser) {}

    fn activate(&self, _browser: &mut LibraryBrowser) -> LibraryActivation {
        LibraryActivation::Stale
    }
}

#[cfg(not(feature = "p4-psram-storage"))]
impl CatalogStage<Usb0LibraryNameMap> for Usb0LibraryCatalogStage {
    fn validate_names(&self, _probe: &PdbProbe) -> Result<(), CatalogError> {
        Ok(())
    }

    fn validate_tracks(&self, _probe: PdbTableProbe) -> Result<(), CatalogError> {
        Ok(())
    }

    fn begin(&mut self, _lease: MediaLease) -> Result<(), CatalogError> {
        Ok(())
    }

    fn retain_track(
        &mut self,
        _volume_identity: VolumeIdentity,
        _track: &PdbTrackRow,
        _names: &Usb0LibraryNameStage,
    ) -> Result<(), CatalogError> {
        Ok(())
    }

    fn retain_playlist_node(&mut self, _row: PdbPlaylistTreeRow, _name: &str) -> bool {
        true
    }

    fn retain_playlist_entry(&mut self, _row: PdbPlaylistEntryRow) -> bool {
        true
    }

    fn note_playlist_invalid_row(&mut self) {}

    fn note_playlist_truncated(&mut self) {}

    fn publish(&mut self) -> Result<Option<PlaylistImportStats>, CatalogError> {
        Ok(None)
    }

    fn abort(&mut self) {}
}

/// Points the browser at the published catalog and mirrors the cursor.
fn sync_selection(catalog: &Usb0LibraryCatalogStage, browser: &mut LibraryBrowser) {
    catalog.sync_selection(browser);
    publish_browser(browser);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb0LibraryError {
    FilesystemUnavailable,
    PathTooLong,
    BrokerBusy,
    BrokerPending,
    BrokerWrongClient,
    #[cfg(feature = "apta-usb0-bringup")]
    FilesystemAnalysisPaused,
    FilesystemNoMedia,
    FilesystemStaleLease,
    FilesystemHandleKindMismatch,
    FilesystemFat32,
    FilesystemExFat,
    MissingVolumeIdentity,
    StagingAllocation,
    DuplicateNameId(u32),
    CatalogBankUnavailable,
    CatalogNameCapacityExceeded,
    CatalogTableTruncated,
    CatalogDecode(PdbCatalogError),
    CatalogBuild(CatalogBuildError),
    InvalidTrackPathUtf8,
    Identity(IdentityError),
    MissingTracksTable,
    PageOutOfRange(u32),
    PageChainLoop,
    RowCountOverflow,
    Pdb(PdbError),
}

impl From<CatalogError> for Usb0LibraryError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::DuplicateNameId(id) => Self::DuplicateNameId(id),
            CatalogError::CatalogBankUnavailable => Self::CatalogBankUnavailable,
            CatalogError::CatalogNameCapacityExceeded => Self::CatalogNameCapacityExceeded,
            CatalogError::CatalogTableTruncated => Self::CatalogTableTruncated,
            CatalogError::CatalogDecode(error) => Self::CatalogDecode(error),
            CatalogError::CatalogBuild(error) => Self::CatalogBuild(error),
        }
    }
}

impl From<ImportError<Usb0LibraryError>> for Usb0LibraryError {
    fn from(error: ImportError<Usb0LibraryError>) -> Self {
        match error {
            ImportError::Io(error) => error,
            ImportError::PathTooLong => Self::PathTooLong,
            ImportError::Catalog(error) => error.into(),
            ImportError::InvalidTrackPathUtf8 => Self::InvalidTrackPathUtf8,
            ImportError::Identity(error) => Self::Identity(error),
            ImportError::MissingTracksTable => Self::MissingTracksTable,
            ImportError::PageOutOfRange(page) => Self::PageOutOfRange(page),
            ImportError::PageChainLoop => Self::PageChainLoop,
            ImportError::RowCountOverflow => Self::RowCountOverflow,
            ImportError::Pdb(error) => Self::Pdb(error),
        }
    }
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
        #[cfg(feature = "apta-usb0-bringup")]
        Usb0FsOperationError::AnalysisPaused => Usb0LibraryError::FilesystemAnalysisPaused,
        Usb0FsOperationError::NoMedia => Usb0LibraryError::FilesystemNoMedia,
        Usb0FsOperationError::StaleLease => Usb0LibraryError::FilesystemStaleLease,
        Usb0FsOperationError::HandleKindMismatch => Usb0LibraryError::FilesystemHandleKindMismatch,
        Usb0FsOperationError::Fat32(_) => Usb0LibraryError::FilesystemFat32,
        Usb0FsOperationError::ExFat(_) => Usb0LibraryError::FilesystemExFat,
    }
}

fn broker_invariant_reset() -> ! {
    esp_hal::system::software_reset()
}

async fn open_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    path: BoundedPath,
) -> Result<Usb0FsFileHandle, Usb0LibraryError> {
    let ticket = client.issue_ticket(lease);

    match client
        .submit(Usb0FsRequest::OpenRead { ticket, path })
        .await
    {
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::OpenRead { result, .. }) => {
            result.map_err(map_filesystem_error)
        }
        Usb0FsSubmitResult::QueueFull(_) => Err(Usb0LibraryError::BrokerBusy),
        Usb0FsSubmitResult::PendingCompletion(_) => Err(Usb0LibraryError::BrokerPending),
        Usb0FsSubmitResult::WrongClient(_) => Err(Usb0LibraryError::BrokerWrongClient),
        Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
            broker_invariant_reset()
        }
    }
}

enum PrefixReadResult {
    Complete {
        file: Usb0FsFileHandle,
        buffer: &'static mut [u8],
        bytes: usize,
    },
    Failed {
        file: Usb0FsFileHandle,
        buffer: &'static mut [u8],
        error: Usb0LibraryError,
    },
}

enum SeekResult {
    Complete {
        file: Usb0FsFileHandle,
    },
    Failed {
        file: Usb0FsFileHandle,
        error: Usb0LibraryError,
    },
}

async fn seek_file(
    client: &mut Usb0FilesystemClient,
    lease: MediaLease,
    file: Usb0FsFileHandle,
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
        Usb0FsSubmitResult::Completed(Usb0FsCompletion::Seek { file, result, .. }) => {
            match result {
                Ok(()) => SeekResult::Complete { file },
                Err(error) => SeekResult::Failed {
                    file,
                    error: map_filesystem_error(error),
                },
            }
        }
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
    file: Usb0FsFileHandle,
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
    file: Usb0FsFileHandle,
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
    file: Usb0FsFileHandle,
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
        Usb0FsSubmitResult::Completed(_) | Usb0FsSubmitResult::UnexpectedCompletion { .. } => {
            broker_invariant_reset()
        }
    }
}

/// The Library worker's USB0 filesystem client, as the shared import's files.
struct Usb0LibraryFiles<'a>(&'a mut Usb0FilesystemClient);

impl CatalogFiles for Usb0LibraryFiles<'_> {
    type File = Usb0FsFileHandle;
    type Buffer = &'static mut [u8];
    type Error = Usb0LibraryError;

    async fn open_read(
        &mut self,
        lease: MediaLease,
        path: BoundedPath,
    ) -> Result<Usb0FsFileHandle, Usb0LibraryError> {
        open_file(self.0, lease, path).await
    }

    async fn read_at(
        &mut self,
        lease: MediaLease,
        file: Usb0FsFileHandle,
        buffer: &'static mut [u8],
        position: u64,
    ) -> (
        Usb0FsFileHandle,
        &'static mut [u8],
        Result<usize, Usb0LibraryError>,
    ) {
        match read_at(self.0, lease, file, buffer, position).await {
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
        }
    }

    async fn close_file(
        &mut self,
        lease: MediaLease,
        file: Usb0FsFileHandle,
    ) -> Result<(), Usb0LibraryError> {
        close_file(self.0, lease, file).await
    }
}

fn handle_library_command(
    catalog: Option<&Usb0LibraryCatalogStage>,
    browser: &mut LibraryBrowser,
    command: Usb0LibraryCommand,
) -> Usb0LibraryCompletion {
    let request_id = command.request_id;
    let completion = match command.effect {
        LibraryEffect::Browse { delta, multiplier } => {
            let _ = browser.browse(delta, multiplier);
            Usb0LibraryCompletion::Applied { request_id }
        }
        LibraryEffect::OpenLibrary => Usb0LibraryCompletion::Applied { request_id },
        LibraryEffect::Back => {
            if let Some(catalog) = catalog {
                catalog.browse_back(browser);
            }
            Usb0LibraryCompletion::Applied { request_id }
        }
        // No permit: a stale mirror that now rests on a track loads nothing.
        LibraryEffect::Open => {
            if let Some(catalog) = catalog
                && browser.on_node_row()
            {
                let _ = catalog.activate(browser);
            }
            Usb0LibraryCompletion::Applied { request_id }
        }
        LibraryEffect::Load { deck, .. } => {
            let Some(catalog) = catalog else {
                return Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::CatalogUnavailable,
                };
            };
            match catalog.activate(browser) {
                // The permit is finished by the Library deck owner on Applied.
                LibraryActivation::Opened => Usb0LibraryCompletion::Applied { request_id },
                LibraryActivation::Load(candidate) => Usb0LibraryCompletion::LoadResolved {
                    request_id,
                    deck,
                    candidate,
                },
                LibraryActivation::Empty => Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::EmptyCatalog,
                },
                LibraryActivation::Stale => Usb0LibraryCompletion::Failed {
                    request_id,
                    error: Usb0LibraryCommandError::StaleSelection,
                },
            }
        }
    };
    publish_browser(browser);
    completion
}

pub(crate) async fn run_usb0_library_worker() -> ! {
    let mut client = Usb0FilesystemClient::library();
    let mut page_buffer = &mut USB0_LIBRARY_PDB_IO.init([0; USB0_LIBRARY_PDB_IO_BYTES])[..];
    let mut name_stage: Option<Usb0LibraryNameStage> = None;
    let mut catalog_stage: Option<Usb0LibraryCatalogStage> = None;
    let mut selection = LibraryBrowser::new();

    loop {
        match select(USB0_LIBRARY_EVENTS.wait(), USB0_LIBRARY_COMMANDS.receive()).await {
            Either::First(Usb0LibraryEvent::Ready {
                binding,
                kind: _,
                volume_identity,
            }) => {
                set_media_lease(binding.lease);
                let Some(volume_identity) = volume_identity else {
                    if let Some(catalog) = catalog_stage.as_mut() {
                        catalog.invalidate(binding.lease);
                        sync_selection(catalog, &mut selection);
                    } else {
                        selection.clear();
                        publish_browser(&selection);
                    }
                    if let Some(staging) = name_stage.as_mut() {
                        staging.clear();
                    }
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::MissingVolumeIdentity,
                    });
                    continue;
                };

                if name_stage.is_none() {
                    name_stage = new_name_stage().ok();
                }
                let Some(staging) = name_stage.as_mut() else {
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::StagingAllocation,
                    });
                    continue;
                };
                if catalog_stage.is_none() {
                    catalog_stage = Usb0LibraryCatalogStage::try_new().ok();
                }
                let Some(catalog) = catalog_stage.as_mut() else {
                    staging.clear();
                    set_library_state(Usb0LibraryState::Failed {
                        binding,
                        error: Usb0LibraryError::StagingAllocation,
                    });
                    continue;
                };
                catalog.prepare_generation(binding.lease);
                sync_selection(catalog, &mut selection);
                staging.clear();

                set_library_state(Usb0LibraryState::Probing { binding });
                let (restored, result) = probe_export_pdb(
                    &mut Usb0LibraryFiles(&mut client),
                    binding.lease,
                    volume_identity,
                    page_buffer,
                    staging,
                    catalog,
                )
                .await;
                page_buffer = restored;
                let result = result.map_err(Usb0LibraryError::from);

                match result {
                    Ok(pdb) => {
                        sync_selection(catalog, &mut selection);
                        set_library_state(Usb0LibraryState::Ready { binding, pdb });
                    }
                    Err(error) => {
                        catalog.abort();
                        staging.clear();
                        set_library_state(Usb0LibraryState::Failed { binding, error });
                    }
                }
            }
            Either::First(Usb0LibraryEvent::FilesystemFailed { binding }) => {
                clear_media_lease(binding.lease);
                if let Some(staging) = name_stage.as_mut() {
                    staging.clear();
                }
                if let Some(catalog) = catalog_stage.as_mut() {
                    catalog.invalidate(binding.lease);
                    sync_selection(catalog, &mut selection);
                } else {
                    selection.clear();
                    publish_browser(&selection);
                }
                set_library_state(Usb0LibraryState::Failed {
                    binding,
                    error: Usb0LibraryError::FilesystemUnavailable,
                });
            }
            Either::First(Usb0LibraryEvent::Detached { binding }) => {
                clear_media_lease(binding.lease);
                if let Some(staging) = name_stage.as_mut() {
                    staging.clear();
                }
                if let Some(catalog) = catalog_stage.as_mut() {
                    catalog.invalidate(binding.lease);
                    sync_selection(catalog, &mut selection);
                } else {
                    selection.clear();
                    publish_browser(&selection);
                }
                set_library_state(Usb0LibraryState::Detached { binding });
            }
            Either::Second(command) => {
                let completion =
                    handle_library_command(catalog_stage.as_ref(), &mut selection, command);
                USB0_LIBRARY_COMPLETIONS.send(completion).await;
            }
        }
    }
}
