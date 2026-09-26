#![no_std]
#![forbid(unsafe_code)]

use core::cell::RefCell;
use core::fmt;
use core::ops::ControlFlow;

use embedded_sdmmc::{Block, BlockCount, BlockDevice as SdmmcBlockDevice, BlockIdx};
use pajoniiir_media_block::{
    BlockDevice, BlockGeometry, BlockRange, PartitionDevice, PartitionDeviceError,
    WritableBlockDevice,
};
use pajoniiir_media_partition::{PartitionCandidate, VolumeKind};

pub const FAT_BLOCK_SIZE: u32 = 512;

#[derive(Debug, Eq, PartialEq)]
pub enum Fat32BlockError<E> {
    UnsupportedBlockSize(u32),
    TooManyBlocks(u64),
    BorrowConflict,
    AddressOverflow,
    Backend(E),
}

impl<E: fmt::Display> fmt::Display for Fat32BlockError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBlockSize(size) => {
                write!(formatter, "unsupported FAT block size: {size}")
            }
            Self::TooManyBlocks(count) => {
                write!(formatter, "FAT block count exceeds u32 range: {count}")
            }
            Self::BorrowConflict => formatter.write_str("FAT block adapter borrow conflict"),
            Self::AddressOverflow => formatter.write_str("FAT block address overflow"),
            Self::Backend(error) => write!(formatter, "FAT block backend error: {error}"),
        }
    }
}

impl<E> core::error::Error for Fat32BlockError<E> where E: core::error::Error + 'static {}

const SYNTHETIC_MBR_PARTITION_OFFSET: usize = 446;
const SYNTHETIC_MBR_SIGNATURE_OFFSET: usize = 510;
const SYNTHETIC_MBR_PARTITION_LBA: u32 = 1;
const SYNTHETIC_MBR_FAT32_LBA_TYPE: u8 = 0x0c;

#[derive(Debug, Eq, PartialEq)]
pub enum Fat32MbrShimError<E> {
    Transfer(pajoniiir_media_block::TransferError),
    CapacityTooLarge(u64),
    SyntheticMbrWrite,
    Inner(E),
}

impl<E: fmt::Display> fmt::Display for Fat32MbrShimError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transfer(error) => write!(formatter, "synthetic MBR transfer error: {error:?}"),
            Self::CapacityTooLarge(blocks) => {
                write!(
                    formatter,
                    "FAT partition exceeds synthetic MBR capacity: {blocks} blocks"
                )
            }
            Self::SyntheticMbrWrite => formatter.write_str("synthetic MBR sector is read-only"),
            Self::Inner(error) => write!(formatter, "synthetic MBR backend error: {error}"),
        }
    }
}

impl<E> core::error::Error for Fat32MbrShimError<E> where E: core::error::Error + 'static {}

/// Presents a bounded FAT partition as a one-partition MBR disk.
///
/// embedded-sdmmc 0.10 only opens volumes through an MBR. Pajoniiir performs
/// partition discovery itself (superfloppy, MBR and GPT), so this adapter adds
/// one synthetic sector at LBA 0 and maps the real partition to synthetic LBA 1.
pub struct Fat32MbrShim<D> {
    inner: D,
}

impl<D> Fat32MbrShim<D> {
    pub const fn new(inner: D) -> Self {
        Self { inner }
    }

    pub const fn inner(&self) -> &D {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: BlockDevice> BlockDevice for Fat32MbrShim<D> {
    type Error = Fat32MbrShimError<D::Error>;

    fn geometry(&self) -> BlockGeometry {
        let inner = self.inner.geometry();
        BlockGeometry {
            block_size: inner.block_size,
            block_count: inner.block_count.saturating_add(1),
        }
    }

    fn read_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        output: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.geometry()
            .validate_transfer(first_block, block_count, output.len())
            .map_err(Fat32MbrShimError::Transfer)?;

        if first_block == 0 {
            let inner_blocks = self.inner.geometry().block_count;
            let partition_blocks = u32::try_from(inner_blocks)
                .map_err(|_| Fat32MbrShimError::CapacityTooLarge(inner_blocks))?;
            write_synthetic_mbr(&mut output[..FAT_BLOCK_SIZE as usize], partition_blocks);

            let remaining = block_count - 1;
            if remaining != 0 {
                self.inner
                    .read_blocks(0, remaining, &mut output[FAT_BLOCK_SIZE as usize..])
                    .map_err(Fat32MbrShimError::Inner)?;
            }
            return Ok(());
        }

        self.inner
            .read_blocks(first_block - 1, block_count, output)
            .map_err(Fat32MbrShimError::Inner)
    }
}

impl<D: WritableBlockDevice> WritableBlockDevice for Fat32MbrShim<D> {
    fn write_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        input: &[u8],
    ) -> Result<(), Self::Error> {
        self.geometry()
            .validate_transfer(first_block, block_count, input.len())
            .map_err(Fat32MbrShimError::Transfer)?;
        if first_block == 0 {
            return Err(Fat32MbrShimError::SyntheticMbrWrite);
        }

        self.inner
            .write_blocks(first_block - 1, block_count, input)
            .map_err(Fat32MbrShimError::Inner)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(Fat32MbrShimError::Inner)
    }
}

fn write_synthetic_mbr(output: &mut [u8], partition_blocks: u32) {
    output.fill(0);
    output[SYNTHETIC_MBR_PARTITION_OFFSET + 4] = SYNTHETIC_MBR_FAT32_LBA_TYPE;
    output[SYNTHETIC_MBR_PARTITION_OFFSET + 8..SYNTHETIC_MBR_PARTITION_OFFSET + 12]
        .copy_from_slice(&SYNTHETIC_MBR_PARTITION_LBA.to_le_bytes());
    output[SYNTHETIC_MBR_PARTITION_OFFSET + 12..SYNTHETIC_MBR_PARTITION_OFFSET + 16]
        .copy_from_slice(&partition_blocks.to_le_bytes());
    output[SYNTHETIC_MBR_SIGNATURE_OFFSET..SYNTHETIC_MBR_SIGNATURE_OFFSET + 2]
        .copy_from_slice(&0xaa55u16.to_le_bytes());
}

pub struct Fat32BlockAdapter<D> {
    inner: RefCell<D>,
}

impl<D> Fat32BlockAdapter<D> {
    pub const fn new(inner: D) -> Self {
        Self {
            inner: RefCell::new(inner),
        }
    }

    pub fn into_inner(self) -> D {
        self.inner.into_inner()
    }

    pub fn with_inner<R>(&self, f: impl FnOnce(&D) -> R) -> Result<R, Fat32BlockError<()>> {
        let inner = self
            .inner
            .try_borrow()
            .map_err(|_| Fat32BlockError::BorrowConflict)?;
        Ok(f(&inner))
    }
}

impl<D> Fat32BlockAdapter<D>
where
    D: WritableBlockDevice,
{
    pub fn validate_geometry(&self) -> Result<BlockGeometry, Fat32BlockError<D::Error>> {
        let inner = self
            .inner
            .try_borrow()
            .map_err(|_| Fat32BlockError::BorrowConflict)?;
        let geometry = inner.geometry();
        if geometry.block_size != FAT_BLOCK_SIZE {
            return Err(Fat32BlockError::UnsupportedBlockSize(geometry.block_size));
        }
        if geometry.block_count > u32::MAX as u64 {
            return Err(Fat32BlockError::TooManyBlocks(geometry.block_count));
        }
        Ok(geometry)
    }

    pub fn flush_inner(&self) -> Result<(), Fat32BlockError<D::Error>> {
        let mut inner = self
            .inner
            .try_borrow_mut()
            .map_err(|_| Fat32BlockError::BorrowConflict)?;
        inner.flush().map_err(Fat32BlockError::Backend)
    }
}

impl<D> SdmmcBlockDevice for Fat32BlockAdapter<D>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
{
    type Error = Fat32BlockError<D::Error>;

    fn read(&self, blocks: &mut [Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let geometry = self.validate_geometry()?;
        let start = start_block_idx.0 as u64;
        let count = u32::try_from(blocks.len()).map_err(|_| Fat32BlockError::AddressOverflow)?;

        geometry
            .validate_transfer(
                start,
                count,
                blocks
                    .len()
                    .checked_mul(FAT_BLOCK_SIZE as usize)
                    .ok_or(Fat32BlockError::AddressOverflow)?,
            )
            .map_err(|_| Fat32BlockError::AddressOverflow)?;

        let mut inner = self
            .inner
            .try_borrow_mut()
            .map_err(|_| Fat32BlockError::BorrowConflict)?;

        for (offset, block) in blocks.iter_mut().enumerate() {
            let block_index = start
                .checked_add(offset as u64)
                .ok_or(Fat32BlockError::AddressOverflow)?;
            inner
                .read_blocks(block_index, 1, &mut block.contents)
                .map_err(Fat32BlockError::Backend)?;
        }
        Ok(())
    }

    fn write(&self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let geometry = self.validate_geometry()?;
        let start = start_block_idx.0 as u64;
        let count = u32::try_from(blocks.len()).map_err(|_| Fat32BlockError::AddressOverflow)?;

        geometry
            .validate_transfer(
                start,
                count,
                blocks
                    .len()
                    .checked_mul(FAT_BLOCK_SIZE as usize)
                    .ok_or(Fat32BlockError::AddressOverflow)?,
            )
            .map_err(|_| Fat32BlockError::AddressOverflow)?;

        let mut inner = self
            .inner
            .try_borrow_mut()
            .map_err(|_| Fat32BlockError::BorrowConflict)?;

        for (offset, block) in blocks.iter().enumerate() {
            let block_index = start
                .checked_add(offset as u64)
                .ok_or(Fat32BlockError::AddressOverflow)?;
            inner
                .write_blocks(block_index, 1, &block.contents)
                .map_err(Fat32BlockError::Backend)?;
        }
        Ok(())
    }

    fn num_blocks(&self) -> Result<BlockCount, Self::Error> {
        let geometry = self.validate_geometry()?;
        Ok(BlockCount(geometry.block_count as u32))
    }
}

const FAT32_ID_OFFSET: u32 = 0x4d31_0000;

#[derive(Debug)]
pub enum Fat32FsError<E>
where
    E: core::error::Error + 'static,
{
    Block(Fat32BlockError<E>),
    Backend(embedded_sdmmc::Error<Fat32BlockError<E>>),
    UnsupportedVolume(VolumeKind),
    CandidateOutOfRange,
    StaleLease,
    InvalidPath,
    PositionTooLarge(u64),
    UnsupportedLongDirectory,
    NameTooLong,
}

impl<E> fmt::Display for Fat32FsError<E>
where
    E: core::error::Error + 'static,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Block(error) => write!(formatter, "FAT32 block adapter error: {error}"),
            Self::Backend(error) => write!(formatter, "FAT32 filesystem error: {error:?}"),
            Self::UnsupportedVolume(kind) => {
                write!(formatter, "partition is not a FAT candidate: {kind:?}")
            }
            Self::CandidateOutOfRange => {
                formatter.write_str("partition candidate lies outside block-device geometry")
            }
            Self::StaleLease => formatter.write_str("stale FAT32 media lease"),
            Self::InvalidPath => formatter.write_str("invalid FAT32 path"),
            Self::PositionTooLarge(position) => {
                write!(
                    formatter,
                    "FAT32 seek position exceeds u32 range: {position}"
                )
            }
            Self::UnsupportedLongDirectory => {
                formatter.write_str("embedded-sdmmc cannot open long-name directories")
            }
            Self::NameTooLong => formatter.write_str("FAT32 filename exceeds neutral name storage"),
        }
    }
}

impl<E> core::error::Error for Fat32FsError<E> where E: core::error::Error + 'static {}

impl<E> From<embedded_sdmmc::Error<Fat32BlockError<E>>> for Fat32FsError<E>
where
    E: core::error::Error + 'static,
{
    fn from(error: embedded_sdmmc::Error<Fat32BlockError<E>>) -> Self {
        Self::Backend(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32FileHandle {
    lease: pajoniiir_media_session::MediaLease,
    raw: embedded_sdmmc::RawFile,
    len: u64,
    position: u64,
}

impl pajoniiir_media_fs::LeaseBound for Fat32FileHandle {
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl pajoniiir_media_fs::FileHandle for Fat32FileHandle {
    fn len(&self) -> u64 {
        self.len
    }

    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32DirectoryHandle {
    lease: pajoniiir_media_session::MediaLease,
    raw: embedded_sdmmc::RawDirectory,
    next_index: usize,
}

impl pajoniiir_media_fs::LeaseBound for Fat32DirectoryHandle {
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl pajoniiir_media_fs::DirectoryHandle for Fat32DirectoryHandle {}

pub struct Fat32FileSystem<D, T, const MAX_DIRS: usize = 8, const MAX_FILES: usize = 8>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
    T: embedded_sdmmc::TimeSource,
{
    lease: pajoniiir_media_session::MediaLease,
    manager: embedded_sdmmc::VolumeManager<Fat32BlockAdapter<D>, T, MAX_DIRS, MAX_FILES, 1>,
    volume: embedded_sdmmc::RawVolume,
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize> Fat32FileSystem<D, T, MAX_DIRS, MAX_FILES>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
    T: embedded_sdmmc::TimeSource,
{
    pub fn mount(
        device: D,
        time_source: T,
        volume_index: usize,
        lease: pajoniiir_media_session::MediaLease,
    ) -> Result<Self, Fat32FsError<D::Error>> {
        let adapter = Fat32BlockAdapter::new(device);
        adapter.validate_geometry().map_err(Fat32FsError::Block)?;

        let manager = embedded_sdmmc::VolumeManager::<
            Fat32BlockAdapter<D>,
            T,
            MAX_DIRS,
            MAX_FILES,
            1,
        >::new_with_limits(adapter, time_source, FAT32_ID_OFFSET);
        let volume = manager
            .open_raw_volume(embedded_sdmmc::VolumeIdx(volume_index))
            .map_err(Fat32FsError::Backend)?;

        Ok(Self {
            lease,
            manager,
            volume,
        })
    }

    fn ensure_lease(
        &self,
        lease: pajoniiir_media_session::MediaLease,
    ) -> Result<(), Fat32FsError<D::Error>> {
        if lease == self.lease {
            Ok(())
        } else {
            Err(Fat32FsError::StaleLease)
        }
    }

    fn open_directory_raw(
        &self,
        path: &str,
    ) -> Result<embedded_sdmmc::RawDirectory, Fat32FsError<D::Error>> {
        let mut current = self
            .manager
            .open_root_dir(self.volume)
            .map_err(Fat32FsError::Backend)?;
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(current);
        }

        for component in trimmed.split('/') {
            if component.is_empty() {
                continue;
            }

            let next = match self.manager.open_dir(current, component) {
                Ok(directory) => directory,
                Err(error) => {
                    let _ = self.manager.close_dir(current);
                    return Err(match error {
                        embedded_sdmmc::Error::FilenameError(_) => {
                            Fat32FsError::UnsupportedLongDirectory
                        }
                        other => Fat32FsError::Backend(other),
                    });
                }
            };

            if let Err(error) = self.manager.close_dir(current) {
                let _ = self.manager.close_dir(next);
                return Err(Fat32FsError::Backend(error));
            }
            current = next;
        }

        Ok(current)
    }

    fn split_parent_name<'a>(
        &self,
        path: &'a str,
    ) -> Result<(&'a str, &'a str), Fat32FsError<D::Error>> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Err(Fat32FsError::InvalidPath);
        }
        Ok(match trimmed.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => (parent, name),
            None => ("", trimmed),
            _ => return Err(Fat32FsError::InvalidPath),
        })
    }

    fn open_existing_file_raw(
        &self,
        path: &str,
    ) -> Result<embedded_sdmmc::RawFile, Fat32FsError<D::Error>> {
        let (parent_path, name) = self.split_parent_name(path)?;
        let parent = self.open_directory_raw(parent_path)?;

        let opened = match self.manager.open_long_name_file_in_dir(
            parent,
            name,
            embedded_sdmmc::Mode::ReadOnly,
        ) {
            Ok(file) => Ok(file),
            Err(embedded_sdmmc::Error::NotFound) => {
                self.manager
                    .open_file_in_dir(parent, name, embedded_sdmmc::Mode::ReadOnly)
            }
            Err(error) => Err(error),
        };

        let close_parent = self.manager.close_dir(parent);
        match (opened, close_parent) {
            (Ok(file), Ok(())) => Ok(file),
            (Ok(file), Err(error)) => {
                let _ = self.manager.close_file(file);
                Err(Fat32FsError::Backend(error))
            }
            (Err(error), _) => Err(Fat32FsError::Backend(error)),
        }
    }

    fn neutral_entry(
        entry: &embedded_sdmmc::DirEntry,
        name_len: usize,
    ) -> pajoniiir_media_fs::DirEntry {
        pajoniiir_media_fs::DirEntry {
            kind: if entry.attributes.is_directory() {
                pajoniiir_media_fs::EntryKind::Directory
            } else {
                pajoniiir_media_fs::EntryKind::File
            },
            len: entry.size as u64,
            modified_unix_seconds: None,
            name_len: name_len as u16,
        }
    }

    fn is_visible_entry(entry: &embedded_sdmmc::DirEntry) -> bool {
        !entry.attributes.is_volume()
            && entry.name != embedded_sdmmc::ShortFileName::this_dir()
            && entry.name != embedded_sdmmc::ShortFileName::parent_dir()
    }
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize>
    Fat32FileSystem<Fat32MbrShim<PartitionDevice<D>>, T, MAX_DIRS, MAX_FILES>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
    T: embedded_sdmmc::TimeSource,
{
    /// Mount a partition candidate discovered by pajoniiir-media-partition.
    ///
    /// The candidate may originate from superfloppy, MBR or GPT discovery.
    /// Known exFAT candidates are rejected; unknown GPT Basic Data candidates
    /// are allowed to reach the FAT parser and fail closed if they are not FAT.
    pub fn mount_candidate(
        device: D,
        time_source: T,
        candidate: PartitionCandidate,
        lease: pajoniiir_media_session::MediaLease,
    ) -> Result<Self, Fat32FsError<Fat32MbrShimError<PartitionDeviceError<D::Error>>>> {
        if candidate.kind == VolumeKind::ExFat {
            return Err(Fat32FsError::UnsupportedVolume(candidate.kind));
        }

        let geometry = device.geometry();
        if geometry.block_size != FAT_BLOCK_SIZE {
            return Err(Fat32FsError::Block(Fat32BlockError::UnsupportedBlockSize(
                geometry.block_size,
            )));
        }
        let range =
            candidate_block_range(candidate, geometry).ok_or(Fat32FsError::CandidateOutOfRange)?;
        let partition = PartitionDevice::new(device, range);
        let shim = Fat32MbrShim::new(partition);

        Self::mount(shim, time_source, 0, lease)
    }
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize> pajoniiir_media_fs::LeaseBound
    for Fat32FileSystem<D, T, MAX_DIRS, MAX_FILES>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
    T: embedded_sdmmc::TimeSource,
{
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize> pajoniiir_media_fs::FileSystem
    for Fat32FileSystem<D, T, MAX_DIRS, MAX_FILES>
where
    D: WritableBlockDevice,
    D::Error: core::error::Error + 'static,
    T: embedded_sdmmc::TimeSource,
{
    type Error = Fat32FsError<D::Error>;
    type File = Fat32FileHandle;
    type Directory = Fat32DirectoryHandle;

    fn kind(&self) -> pajoniiir_media_fs::FileSystemKind {
        pajoniiir_media_fs::FileSystemKind::Fat32
    }

    fn capabilities(&self) -> pajoniiir_media_fs::FileSystemCapabilities {
        pajoniiir_media_fs::FileSystemCapabilities {
            read: true,
            write: false,
            directories: true,
            // LFN files can be read/listed, but embedded-sdmmc 0.10 opens
            // directories through the 8.3 ToShortFileName path.
            long_names: false,
            flush: false,
            random_seek: true,
            streaming_directory_visit: true,
        }
    }

    fn stat(&mut self, path: &str) -> Result<pajoniiir_media_fs::FileStat, Self::Error> {
        if path.trim_matches('/').is_empty() {
            return Ok(pajoniiir_media_fs::FileStat {
                kind: pajoniiir_media_fs::EntryKind::Directory,
                len: 0,
                modified_unix_seconds: None,
            });
        }

        match self.open_existing_file_raw(path) {
            Ok(file) => {
                let len = self
                    .manager
                    .file_length(file)
                    .map_err(Fat32FsError::Backend)? as u64;
                self.manager
                    .close_file(file)
                    .map_err(Fat32FsError::Backend)?;
                Ok(pajoniiir_media_fs::FileStat {
                    kind: pajoniiir_media_fs::EntryKind::File,
                    len,
                    modified_unix_seconds: None,
                })
            }
            Err(Fat32FsError::Backend(embedded_sdmmc::Error::OpenedDirAsFile))
            | Err(Fat32FsError::Backend(embedded_sdmmc::Error::NotFound))
            | Err(Fat32FsError::Backend(embedded_sdmmc::Error::FilenameError(_))) => {
                let directory = self.open_directory_raw(path)?;
                self.manager
                    .close_dir(directory)
                    .map_err(Fat32FsError::Backend)?;
                Ok(pajoniiir_media_fs::FileStat {
                    kind: pajoniiir_media_fs::EntryKind::Directory,
                    len: 0,
                    modified_unix_seconds: None,
                })
            }
            Err(error) => Err(error),
        }
    }

    fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
        let raw = self.open_existing_file_raw(path)?;
        let len = match self.manager.file_length(raw) {
            Ok(len) => len as u64,
            Err(error) => {
                let _ = self.manager.close_file(raw);
                return Err(Fat32FsError::Backend(error));
            }
        };
        let position = match self.manager.file_offset(raw) {
            Ok(position) => position as u64,
            Err(error) => {
                let _ = self.manager.close_file(raw);
                return Err(Fat32FsError::Backend(error));
            }
        };

        Ok(Fat32FileHandle {
            lease: self.lease,
            raw,
            len,
            position,
        })
    }

    fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)?;
        self.manager
            .close_file(file.raw)
            .map_err(Fat32FsError::Backend)
    }

    fn read(&mut self, file: &mut Self::File, output: &mut [u8]) -> Result<usize, Self::Error> {
        self.ensure_lease(file.lease)?;
        let read = self
            .manager
            .read(file.raw, output)
            .map_err(Fat32FsError::Backend)?;
        file.position = file.position.saturating_add(read as u64);
        Ok(read)
    }

    fn seek_absolute(&mut self, file: &mut Self::File, position: u64) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)?;
        let position_u32 =
            u32::try_from(position).map_err(|_| Fat32FsError::PositionTooLarge(position))?;
        self.manager
            .file_seek_from_start(file.raw, position_u32)
            .map_err(Fat32FsError::Backend)?;
        file.position = position;
        Ok(())
    }

    fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
        let raw = self.open_directory_raw(path)?;
        Ok(Fat32DirectoryHandle {
            lease: self.lease,
            raw,
            next_index: 0,
        })
    }

    fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; pajoniiir_media_fs::FS_NAME_MAX],
    ) -> Result<Option<pajoniiir_media_fs::DirEntry>, Self::Error> {
        self.ensure_lease(directory.lease)?;

        let target = directory.next_index;
        let mut visible_index = 0usize;
        let mut found = None;
        let mut name_error = false;
        let mut lfn_storage = [0u8; pajoniiir_media_fs::FS_NAME_MAX];
        let mut lfn_buffer = embedded_sdmmc::LfnBuffer::new(&mut lfn_storage);

        self.manager
            .iterate_dir_lfn(directory.raw, &mut lfn_buffer, |entry, long_name| {
                if !Self::is_visible_entry(entry) {
                    return ControlFlow::Continue(());
                }
                if visible_index != target {
                    visible_index = visible_index.saturating_add(1);
                    return ControlFlow::Continue(());
                }

                let name_len = match long_name {
                    Some(name) => copy_utf8(name.as_bytes(), name_storage),
                    None => encode_short_name(entry.name, name_storage),
                };
                let Some(name_len) = name_len else {
                    name_error = true;
                    return ControlFlow::Break(());
                };
                found = Some(Self::neutral_entry(entry, name_len));
                ControlFlow::Break(())
            })
            .map_err(Fat32FsError::Backend)?;

        if name_error {
            return Err(Fat32FsError::NameTooLong);
        }
        if found.is_some() {
            directory.next_index = directory.next_index.saturating_add(1);
        }
        Ok(found)
    }

    fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error> {
        self.ensure_lease(directory.lease)?;
        self.manager
            .close_dir(directory.raw)
            .map_err(Fat32FsError::Backend)
    }

    fn visit_directory<V: pajoniiir_media_fs::DirectoryVisitor>(
        &mut self,
        path: &str,
        name_storage: &mut [u8; pajoniiir_media_fs::FS_NAME_MAX],
        visitor: &mut V,
    ) -> Result<(), Self::Error> {
        let directory = self.open_directory_raw(path)?;
        let mut lfn_storage = [0u8; pajoniiir_media_fs::FS_NAME_MAX];
        let mut lfn_buffer = embedded_sdmmc::LfnBuffer::new(&mut lfn_storage);
        let mut name_error = false;

        let iterate_result =
            self.manager
                .iterate_dir_lfn(directory, &mut lfn_buffer, |entry, long_name| {
                    if !Self::is_visible_entry(entry) {
                        return ControlFlow::Continue(());
                    }

                    match long_name {
                        Some(name) => {
                            let bytes = name.as_bytes();
                            let Some(name_len) = copy_utf8(bytes, name_storage) else {
                                name_error = true;
                                return ControlFlow::Break(());
                            };
                            let item = Self::neutral_entry(entry, name_len);
                            if visitor.visit(item, &name_storage[..name_len]) {
                                ControlFlow::Continue(())
                            } else {
                                ControlFlow::Break(())
                            }
                        }
                        None => {
                            let Some(name_len) = encode_short_name(entry.name, name_storage) else {
                                name_error = true;
                                return ControlFlow::Break(());
                            };
                            let item = Self::neutral_entry(entry, name_len);
                            if visitor.visit(item, &name_storage[..name_len]) {
                                ControlFlow::Continue(())
                            } else {
                                ControlFlow::Break(())
                            }
                        }
                    }
                });

        let close_result = self.manager.close_dir(directory);
        iterate_result.map_err(Fat32FsError::Backend)?;
        close_result.map_err(Fat32FsError::Backend)?;
        if name_error {
            Err(Fat32FsError::NameTooLong)
        } else {
            Ok(())
        }
    }
}

fn candidate_block_range(
    candidate: PartitionCandidate,
    geometry: BlockGeometry,
) -> Option<BlockRange> {
    match candidate.sector_count {
        Some(block_count) => BlockRange::new(candidate.first_lba, block_count, geometry),
        None => BlockRange::from_start(candidate.first_lba, geometry),
    }
}

fn copy_utf8(input: &[u8], output: &mut [u8]) -> Option<usize> {
    if input.len() > output.len() {
        return None;
    }
    output[..input.len()].copy_from_slice(input);
    Some(input.len())
}

fn encode_short_name(name: embedded_sdmmc::ShortFileName, output: &mut [u8]) -> Option<usize> {
    let mut written = 0usize;
    written = encode_latin1(name.base_name(), output, written)?;
    let extension = name.extension();
    if !extension.is_empty() {
        if written >= output.len() {
            return None;
        }
        output[written] = b'.';
        written += 1;
        written = encode_latin1(extension, output, written)?;
    }
    Some(written)
}

fn encode_latin1(input: &[u8], output: &mut [u8], mut written: usize) -> Option<usize> {
    for &byte in input {
        if byte < 0x80 {
            if written >= output.len() {
                return None;
            }
            output[written] = byte;
            written += 1;
        } else {
            if written + 1 >= output.len() {
                return None;
            }
            output[written] = 0xc0 | (byte >> 6);
            output[written + 1] = 0x80 | (byte & 0x3f);
            written += 2;
        }
    }
    Some(written)
}


const FAT32_BOOT_MINIMUM: usize = 512;
const FAT32_BOOT_SIGNATURE_OFFSET: usize = 510;
const FAT32_FSINFO_MINIMUM: usize = 512;
const FAT32_FSINFO_UNKNOWN: u32 = 0xffff_ffff;
const FAT32_FSINFO_LEAD_SIGNATURE: u32 = 0x4161_5252;
const FAT32_FSINFO_STRUCTURE_SIGNATURE: u32 = 0x6141_7272;
const FAT32_FSINFO_TRAIL_SIGNATURE: u32 = 0xaa55_0000;

/// Validated FAT32 BPB geometry needed by the async target mount path.
///
/// This parser is intentionally independent of embedded-sdmmc so USB media
/// with 512/1024/2048/4096-byte logical sectors can be validated without
/// blocking the Embassy executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32BootInfo {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub fat_count: u8,
    pub sectors_per_fat: u32,
    pub total_sectors: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub backup_boot_sector: u16,
    pub fat_start_sector: u32,
    pub data_start_sector: u64,
    pub cluster_count: u32,
    pub max_cluster: u32,
    pub volume_id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fat32BootSectorError {
    BufferTooSmall { minimum: usize, actual: usize },
    InvalidSignature { found: u16 },
    UnsupportedSectorSize(u16),
    DeviceSectorSizeMismatch { device: u32, bpb: u16 },
    InvalidSectorsPerCluster(u8),
    ClusterSizeTooLarge(u32),
    InvalidReservedSectors(u16),
    InvalidFatCount(u8),
    NotFat32Layout {
        root_entries: u16,
        sectors_per_fat_16: u16,
    },
    UnsupportedVersion(u16),
    MissingSectorsPerFat,
    MissingTotalSectors,
    MetadataBeyondVolume,
    VolumeBeyondPartition {
        volume_sectors: u32,
        partition_sectors: u64,
    },
    FatTooSmall,
    InvalidRootCluster {
        root_cluster: u32,
        max_cluster: u32,
    },
    InvalidFsInfoSector {
        fs_info_sector: u16,
        reserved_sectors: u16,
    },
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32FsInfo {
    pub free_cluster_count: Option<u32>,
    pub next_free_cluster: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fat32FsInfoError {
    BufferTooSmall { minimum: usize, actual: usize },
    InvalidLeadSignature { found: u32 },
    InvalidStructureSignature { found: u32 },
    InvalidTrailSignature { found: u32 },
    FreeCountOutOfRange {
        free_count: u32,
        cluster_count: u32,
    },
    NextFreeOutOfRange {
        next_free: u32,
        max_cluster: u32,
    },
}

fn read_le_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn read_le_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

/// Validate the FAT32 boot sector against the already-bounded partition.
///
/// FAT logical-sector size must match the USB MSC logical block size. This is
/// the invariant that makes BPB/FSInfo sector numbers directly usable as LBAs
/// by the async USB0 partition adapter.
pub fn parse_fat32_boot_sector(
    block: &[u8],
    geometry: BlockGeometry,
) -> Result<Fat32BootInfo, Fat32BootSectorError> {
    if block.len() < FAT32_BOOT_MINIMUM {
        return Err(Fat32BootSectorError::BufferTooSmall {
            minimum: FAT32_BOOT_MINIMUM,
            actual: block.len(),
        });
    }

    let signature = read_le_u16(block, FAT32_BOOT_SIGNATURE_OFFSET);
    if signature != 0xaa55 {
        return Err(Fat32BootSectorError::InvalidSignature { found: signature });
    }

    let bytes_per_sector = read_le_u16(block, 11);
    if !matches!(bytes_per_sector, 512 | 1_024 | 2_048 | 4_096) {
        return Err(Fat32BootSectorError::UnsupportedSectorSize(
            bytes_per_sector,
        ));
    }
    if u32::from(bytes_per_sector) != geometry.block_size {
        return Err(Fat32BootSectorError::DeviceSectorSizeMismatch {
            device: geometry.block_size,
            bpb: bytes_per_sector,
        });
    }
    let sector_size = usize::from(bytes_per_sector);
    if block.len() < sector_size {
        return Err(Fat32BootSectorError::BufferTooSmall {
            minimum: sector_size,
            actual: block.len(),
        });
    }

    let sectors_per_cluster = block[13];
    if sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || sectors_per_cluster > 128
    {
        return Err(Fat32BootSectorError::InvalidSectorsPerCluster(
            sectors_per_cluster,
        ));
    }
    let cluster_size = u32::from(bytes_per_sector)
        .checked_mul(u32::from(sectors_per_cluster))
        .ok_or(Fat32BootSectorError::ArithmeticOverflow)?;
    if cluster_size > 32 * 1_024 {
        return Err(Fat32BootSectorError::ClusterSizeTooLarge(cluster_size));
    }

    let reserved_sectors = read_le_u16(block, 14);
    if reserved_sectors == 0 {
        return Err(Fat32BootSectorError::InvalidReservedSectors(
            reserved_sectors,
        ));
    }
    let fat_count = block[16];
    if !matches!(fat_count, 1 | 2) {
        return Err(Fat32BootSectorError::InvalidFatCount(fat_count));
    }

    let root_entries = read_le_u16(block, 17);
    let sectors_per_fat_16 = read_le_u16(block, 22);
    if root_entries != 0 || sectors_per_fat_16 != 0 {
        return Err(Fat32BootSectorError::NotFat32Layout {
            root_entries,
            sectors_per_fat_16,
        });
    }

    let version = read_le_u16(block, 42);
    if version != 0 {
        return Err(Fat32BootSectorError::UnsupportedVersion(version));
    }

    let sectors_per_fat = read_le_u32(block, 36);
    if sectors_per_fat == 0 {
        return Err(Fat32BootSectorError::MissingSectorsPerFat);
    }

    let total_sectors_16 = read_le_u16(block, 19);
    let total_sectors = if total_sectors_16 != 0 {
        u32::from(total_sectors_16)
    } else {
        read_le_u32(block, 32)
    };
    if total_sectors == 0 {
        return Err(Fat32BootSectorError::MissingTotalSectors);
    }
    if u64::from(total_sectors) > geometry.block_count {
        return Err(Fat32BootSectorError::VolumeBeyondPartition {
            volume_sectors: total_sectors,
            partition_sectors: geometry.block_count,
        });
    }

    let fat_sectors = u64::from(fat_count)
        .checked_mul(u64::from(sectors_per_fat))
        .ok_or(Fat32BootSectorError::ArithmeticOverflow)?;
    let metadata_sectors = u64::from(reserved_sectors)
        .checked_add(fat_sectors)
        .ok_or(Fat32BootSectorError::ArithmeticOverflow)?;
    if metadata_sectors >= u64::from(total_sectors) {
        return Err(Fat32BootSectorError::MetadataBeyondVolume);
    }

    let data_sectors = u64::from(total_sectors) - metadata_sectors;
    let cluster_count_u64 = data_sectors / u64::from(sectors_per_cluster);
    let cluster_count =
        u32::try_from(cluster_count_u64).map_err(|_| Fat32BootSectorError::ArithmeticOverflow)?;
    if cluster_count == 0 {
        return Err(Fat32BootSectorError::MetadataBeyondVolume);
    }
    let max_cluster = cluster_count
        .checked_add(1)
        .ok_or(Fat32BootSectorError::ArithmeticOverflow)?;

    let fat_entries = u64::from(sectors_per_fat)
        .checked_mul(u64::from(bytes_per_sector))
        .ok_or(Fat32BootSectorError::ArithmeticOverflow)?
        / 4;
    if fat_entries <= u64::from(max_cluster) {
        return Err(Fat32BootSectorError::FatTooSmall);
    }

    let root_cluster = read_le_u32(block, 44);
    if !(2..=max_cluster).contains(&root_cluster) {
        return Err(Fat32BootSectorError::InvalidRootCluster {
            root_cluster,
            max_cluster,
        });
    }

    let fs_info_sector = read_le_u16(block, 48);
    if fs_info_sector == 0 || fs_info_sector >= reserved_sectors {
        return Err(Fat32BootSectorError::InvalidFsInfoSector {
            fs_info_sector,
            reserved_sectors,
        });
    }

    Ok(Fat32BootInfo {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        fat_count,
        sectors_per_fat,
        total_sectors,
        root_cluster,
        fs_info_sector,
        backup_boot_sector: read_le_u16(block, 50),
        fat_start_sector: u32::from(reserved_sectors),
        data_start_sector: metadata_sectors,
        cluster_count,
        max_cluster,
        volume_id: read_le_u32(block, 67),
    })
}

/// Validate the FAT32 FSInfo sector selected by the BPB.
pub fn parse_fat32_fs_info(
    block: &[u8],
    boot: &Fat32BootInfo,
) -> Result<Fat32FsInfo, Fat32FsInfoError> {
    if block.len() < FAT32_FSINFO_MINIMUM {
        return Err(Fat32FsInfoError::BufferTooSmall {
            minimum: FAT32_FSINFO_MINIMUM,
            actual: block.len(),
        });
    }

    let lead = read_le_u32(block, 0);
    if lead != FAT32_FSINFO_LEAD_SIGNATURE {
        return Err(Fat32FsInfoError::InvalidLeadSignature { found: lead });
    }
    let structure = read_le_u32(block, 484);
    if structure != FAT32_FSINFO_STRUCTURE_SIGNATURE {
        return Err(Fat32FsInfoError::InvalidStructureSignature {
            found: structure,
        });
    }
    let trail = read_le_u32(block, 508);
    if trail != FAT32_FSINFO_TRAIL_SIGNATURE {
        return Err(Fat32FsInfoError::InvalidTrailSignature { found: trail });
    }

    let free_count_raw = read_le_u32(block, 488);
    let free_cluster_count = if free_count_raw == FAT32_FSINFO_UNKNOWN {
        None
    } else {
        if free_count_raw > boot.cluster_count {
            return Err(Fat32FsInfoError::FreeCountOutOfRange {
                free_count: free_count_raw,
                cluster_count: boot.cluster_count,
            });
        }
        Some(free_count_raw)
    };

    let next_free_raw = read_le_u32(block, 492);
    let next_free_cluster = if next_free_raw == FAT32_FSINFO_UNKNOWN {
        None
    } else {
        if !(2..=boot.max_cluster).contains(&next_free_raw) {
            return Err(Fat32FsInfoError::NextFreeOutOfRange {
                next_free: next_free_raw,
                max_cluster: boot.max_cluster,
            });
        }
        Some(next_free_raw)
    };

    Ok(Fat32FsInfo {
        free_cluster_count,
        next_free_cluster,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_media_block::BlockDevice;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Invalid,
    }

    impl core::fmt::Display for TestError {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter.write_str("invalid test block transfer")
        }
    }

    impl core::error::Error for TestError {}

    struct MemoryDevice {
        geometry: BlockGeometry,
        bytes: [u8; 2_048],
        flushes: u32,
    }

    impl MemoryDevice {
        fn new(block_size: u32, block_count: u64) -> Self {
            Self {
                geometry: BlockGeometry {
                    block_size,
                    block_count,
                },
                bytes: [0; 2_048],
                flushes: 0,
            }
        }
    }

    impl BlockDevice for MemoryDevice {
        type Error = TestError;

        fn geometry(&self) -> BlockGeometry {
            self.geometry
        }

        fn read_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            output: &mut [u8],
        ) -> Result<(), Self::Error> {
            self.geometry
                .validate_transfer(first_block, block_count, output.len())
                .map_err(|_| TestError::Invalid)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Invalid)?;
            let end = start.checked_add(output.len()).ok_or(TestError::Invalid)?;
            let source = self.bytes.get(start..end).ok_or(TestError::Invalid)?;
            output.copy_from_slice(source);
            Ok(())
        }
    }

    impl WritableBlockDevice for MemoryDevice {
        fn write_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            self.geometry
                .validate_transfer(first_block, block_count, input.len())
                .map_err(|_| TestError::Invalid)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Invalid)?;
            let end = start.checked_add(input.len()).ok_or(TestError::Invalid)?;
            let destination = self.bytes.get_mut(start..end).ok_or(TestError::Invalid)?;
            destination.copy_from_slice(input);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn synthetic_mbr_maps_partition_to_local_lba_one() {
        let mut inner = MemoryDevice::new(512, 4);
        inner.bytes[..512].fill(0x5a);
        let mut shim = Fat32MbrShim::new(inner);
        let mut mbr = [0u8; 512];

        shim.read_blocks(0, 1, &mut mbr).unwrap();

        assert_eq!(&mbr[510..512], &[0x55, 0xaa]);
        assert_eq!(mbr[446 + 4], SYNTHETIC_MBR_FAT32_LBA_TYPE);
        assert_eq!(u32::from_le_bytes(mbr[454..458].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(mbr[458..462].try_into().unwrap()), 4);

        let mut first_partition_block = [0u8; 512];
        shim.read_blocks(1, 1, &mut first_partition_block).unwrap();
        assert!(first_partition_block.iter().all(|byte| *byte == 0x5a));
    }

    #[test]
    fn synthetic_mbr_is_read_only_but_partition_writes_translate() {
        let mut shim = Fat32MbrShim::new(MemoryDevice::new(512, 4));
        let block = [0x33u8; 512];

        assert_eq!(
            shim.write_blocks(0, 1, &block),
            Err(Fat32MbrShimError::SyntheticMbrWrite)
        );
        shim.write_blocks(1, 1, &block).unwrap();
        assert_eq!(&shim.inner().bytes[..512], &block);
    }

    #[test]
    fn candidate_range_keeps_large_gpt_lba_outside_fat_local_address_space() {
        let geometry = BlockGeometry {
            block_size: 512,
            block_count: u32::MAX as u64 + 16_384,
        };
        let first_lba = u32::MAX as u64 + 4_096;
        let candidate = PartitionCandidate {
            first_lba,
            sector_count: core::num::NonZeroU64::new(8_192),
            kind: VolumeKind::Unknown,
        };

        let range = candidate_block_range(candidate, geometry).unwrap();

        assert_eq!(range.first_block(), first_lba);
        assert_eq!(range.block_count().get(), 8_192);
        assert_eq!(range.translate(0, 1), Some(first_lba));
    }

    #[test]
    fn short_name_encoder_preserves_83_names() {
        let name = embedded_sdmmc::ShortFileName::create_from_str("TRACK01.WAV").unwrap();
        let mut output = [0u8; 32];

        let len = encode_short_name(name, &mut output).unwrap();

        assert_eq!(&output[..len], b"TRACK01.WAV");
    }

    #[test]
    fn latin1_encoder_emits_utf8_and_checks_capacity() {
        let mut output = [0u8; 4];
        let len = encode_latin1(&[b'A', 0xe9], &mut output, 0).unwrap();
        assert_eq!(&output[..len], &[b'A', 0xc3, 0xa9]);

        let mut short = [0u8; 2];
        assert_eq!(encode_latin1(&[0xe9, 0xe9], &mut short, 0), None);
    }

    #[test]
    fn bridge_exposes_exact_512_byte_geometry() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(512, 4));
        assert_eq!(adapter.num_blocks(), Ok(BlockCount(4)));
    }

    #[test]
    fn bridge_rejects_non_512_sector_geometry() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(4_096, 1));
        assert_eq!(
            adapter.num_blocks(),
            Err(Fat32BlockError::UnsupportedBlockSize(4_096))
        );
    }

    #[test]
    fn bridge_rejects_capacity_beyond_embedded_sdmmc_limit() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(512, u32::MAX as u64 + 1));
        assert_eq!(
            adapter.num_blocks(),
            Err(Fat32BlockError::TooManyBlocks(u32::MAX as u64 + 1))
        );
    }

    #[test]
    fn bridge_reads_and_writes_through_pajoniiir_block_contract() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(512, 4));
        let mut write_block = Block::new();
        write_block.contents.fill(0x5a);

        adapter.write(&[write_block], BlockIdx(2)).unwrap();

        let mut read_block = Block::new();
        adapter
            .read(core::slice::from_mut(&mut read_block), BlockIdx(2))
            .unwrap();
        assert!(read_block.contents.iter().all(|byte| *byte == 0x5a));
    }

    #[test]
    fn explicit_flush_reaches_underlying_transport() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(512, 4));
        adapter.flush_inner().unwrap();

        assert_eq!(adapter.with_inner(|inner| inner.flushes).unwrap(), 1);
    }

    #[test]
    fn zero_block_transfer_is_rejected_by_contract() {
        let adapter = Fat32BlockAdapter::new(MemoryDevice::new(512, 4));
        let mut blocks: [Block; 0] = [];
        assert_eq!(
            adapter.read(&mut blocks, BlockIdx(0)),
            Err(Fat32BlockError::AddressOverflow)
        );
    }
}


#[cfg(test)]
mod async_mount_validation_tests {
    use super::*;

    fn put_u16(block: &mut [u8], offset: usize, value: u16) {
        block[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(block: &mut [u8], offset: usize, value: u32) {
        block[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn build_boot_sector(block: &mut [u8], sector_size: u16, sectors_per_fat: u32) {
        block.fill(0);
        put_u16(block, 11, sector_size);
        block[13] = 1;
        put_u16(block, 14, 32);
        block[16] = 2;
        put_u16(block, 17, 0);
        put_u16(block, 19, 0);
        put_u16(block, 22, 0);
        put_u32(block, 32, 131_072);
        put_u32(block, 36, sectors_per_fat);
        put_u16(block, 42, 0);
        put_u32(block, 44, 2);
        put_u16(block, 48, 1);
        put_u16(block, 50, 6);
        put_u32(block, 67, 0x1234_5678);
        put_u16(block, 510, 0xaa55);
    }

    fn build_fs_info(block: &mut [u8], next_free: u32) {
        block.fill(0);
        put_u32(block, 0, FAT32_FSINFO_LEAD_SIGNATURE);
        put_u32(block, 484, FAT32_FSINFO_STRUCTURE_SIGNATURE);
        put_u32(block, 488, FAT32_FSINFO_UNKNOWN);
        put_u32(block, 492, next_free);
        put_u32(block, 508, FAT32_FSINFO_TRAIL_SIGNATURE);
    }

    #[test]
    fn async_mount_parser_accepts_valid_512_byte_fat32() {
        let mut boot = [0u8; 512];
        build_boot_sector(&mut boot, 512, 1_024);
        let info = parse_fat32_boot_sector(
            &boot,
            BlockGeometry {
                block_size: 512,
                block_count: 131_072,
            },
        )
        .unwrap();

        assert_eq!(info.root_cluster, 2);
        assert_eq!(info.fs_info_sector, 1);
        assert_eq!(info.volume_id, 0x1234_5678);

        let mut fs_info = [0u8; 512];
        build_fs_info(&mut fs_info, 2);
        let parsed = parse_fat32_fs_info(&fs_info, &info).unwrap();
        assert_eq!(parsed.free_cluster_count, None);
        assert_eq!(parsed.next_free_cluster, Some(2));
    }

    #[test]
    fn async_mount_parser_accepts_4096_byte_logical_sector() {
        let mut boot = [0u8; 4_096];
        build_boot_sector(&mut boot, 4_096, 128);
        let info = parse_fat32_boot_sector(
            &boot,
            BlockGeometry {
                block_size: 4_096,
                block_count: 131_072,
            },
        )
        .unwrap();

        assert_eq!(info.bytes_per_sector, 4_096);
        assert_eq!(info.data_start_sector, 288);
    }

    #[test]
    fn async_mount_parser_rejects_usb_bpb_sector_size_mismatch() {
        let mut boot = [0u8; 512];
        build_boot_sector(&mut boot, 512, 1_024);

        assert!(matches!(
            parse_fat32_boot_sector(
                &boot,
                BlockGeometry {
                    block_size: 1_024,
                    block_count: 131_072,
                },
            ),
            Err(Fat32BootSectorError::DeviceSectorSizeMismatch { .. })
        ));
    }

    #[test]
    fn async_mount_parser_rejects_invalid_fsinfo_next_free() {
        let mut boot = [0u8; 512];
        build_boot_sector(&mut boot, 512, 1_024);
        let info = parse_fat32_boot_sector(
            &boot,
            BlockGeometry {
                block_size: 512,
                block_count: 131_072,
            },
        )
        .unwrap();

        let mut fs_info = [0u8; 512];
        build_fs_info(&mut fs_info, info.max_cluster + 1);
        assert!(matches!(
            parse_fat32_fs_info(&fs_info, &info),
            Err(Fat32FsInfoError::NextFreeOutOfRange { .. })
        ));
    }

    #[test]
    fn async_mount_parser_rejects_invalid_fsinfo_signature() {
        let mut boot = [0u8; 512];
        build_boot_sector(&mut boot, 512, 1_024);
        let info = parse_fat32_boot_sector(
            &boot,
            BlockGeometry {
                block_size: 512,
                block_count: 131_072,
            },
        )
        .unwrap();

        let mut fs_info = [0u8; 512];
        build_fs_info(&mut fs_info, 2);
        put_u32(&mut fs_info, 484, 0);

        assert!(matches!(
            parse_fat32_fs_info(&fs_info, &info),
            Err(Fat32FsInfoError::InvalidStructureSignature { found: 0 })
        ));
    }
}
