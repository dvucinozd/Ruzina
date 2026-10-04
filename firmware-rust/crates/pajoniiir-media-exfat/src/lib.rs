#![no_std]
#![forbid(unsafe_code)]

mod async_file_system;
mod async_fs;
mod entry_set;

pub use async_file_system::{
    ExFatAsyncDirectoryHandle, ExFatAsyncFileHandle, ExFatAsyncFsError,
};
pub use async_fs::{
    ExFatAsyncBootInfo, ExFatAsyncBootSectorError, ExFatAsyncIoError, ExFatAsyncMountError,
    ExFatAsyncVolume, parse_exfat_boot_sector,
};
pub use entry_set::{ExFatEntrySetError, ExFatFileEntrySetParser, ExFatParsedFileEntry};

pub type BackendFsError<E> = exfat_embedded::Error<ExFatBlockError<E>>;

#[derive(Debug, Eq, PartialEq)]
pub enum ExFatFsError<E> {
    Backend(BackendFsError<E>),
    StaleLease,
    UnsupportedRandomSeek,
    NameTooLong,
    UnsupportedCrossDirectoryRename,
}

impl<E> From<BackendFsError<E>> for ExFatFsError<E> {
    fn from(error: BackendFsError<E>) -> Self {
        Self::Backend(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatFileHandle {
    lease: pajoniiir_media_session::MediaLease,
    inner: exfat_embedded::File,
}

impl pajoniiir_media_fs::LeaseBound for ExFatFileHandle {
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl pajoniiir_media_fs::FileHandle for ExFatFileHandle {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn position(&self) -> u64 {
        self.inner.position()
    }

    fn set_position(&mut self, _position: u64) {
        // The backend handle intentionally has no public random-seek setter.
        // seek_absolute() only accepts the current position, so this is a
        // deliberate no-op until an O(1) backend seek API is available.
    }
}

// `exfat_embedded::Directory` is intentionally stored inline: this crate is
// no_std/no_alloc and the directory handle must remain detached + Copy. The
// size asymmetry is therefore a deliberate memory tradeoff, not an accidental
// candidate for heap indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExFatDirectoryKind {
    Root,
    Nested(exfat_embedded::Directory),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatDirectoryHandle {
    lease: pajoniiir_media_session::MediaLease,
    kind: ExFatDirectoryKind,
    next_index: usize,
}

impl pajoniiir_media_fs::LeaseBound for ExFatDirectoryHandle {
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl pajoniiir_media_fs::DirectoryHandle for ExFatDirectoryHandle {}

pub struct ExFatFileSystem<'a, D> {
    lease: pajoniiir_media_session::MediaLease,
    backend: exfat_embedded::FileSystem<ExFatBlockAdapter<D>>,
    scratch_storage: &'a mut [u8],
    workspace: &'a mut exfat_embedded::Workspace,
}

impl<'a, D> ExFatFileSystem<'a, D>
where
    D: WritableBlockDevice,
{
    pub fn mount(
        device: D,
        lease: pajoniiir_media_session::MediaLease,
        scratch_storage: &'a mut [u8],
        workspace: &'a mut exfat_embedded::Workspace,
    ) -> Result<Self, ExFatFsError<D::Error>> {
        let adapter = ExFatBlockAdapter::new(device);
        let sector_size = adapter
            .validate_geometry()
            .map_err(|error| ExFatFsError::Backend(exfat_embedded::Error::Device(error)))?
            .block_size as usize;
        if scratch_storage.len() < sector_size {
            return Err(ExFatFsError::Backend(
                exfat_embedded::Error::InvalidSectorSize,
            ));
        }

        let backend = {
            let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
            exfat_embedded::FileSystem::mount(adapter, &mut scratch)?
        };

        Ok(Self {
            lease,
            backend,
            scratch_storage,
            workspace,
        })
    }

    pub fn into_device(self) -> D {
        self.backend.into_device().into_inner()
    }

    pub const fn lease_value(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }

    fn ensure_lease(
        &self,
        lease: pajoniiir_media_session::MediaLease,
    ) -> Result<(), ExFatFsError<D::Error>> {
        if lease == self.lease {
            Ok(())
        } else {
            Err(ExFatFsError::StaleLease)
        }
    }

    fn directory_for_path(
        &mut self,
        path: &str,
    ) -> Result<ExFatDirectoryKind, ExFatFsError<D::Error>> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(ExFatDirectoryKind::Root);
        }

        let Self {
            backend,
            scratch_storage,
            workspace,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let entry = backend.lookup_with_workspace(trimmed, &mut scratch, workspace)?;
        entry
            .directory()
            .map(ExFatDirectoryKind::Nested)
            .ok_or(ExFatFsError::Backend(exfat_embedded::Error::NotDirectory))
    }

    fn backend_directory_entry(
        &mut self,
        directory: ExFatDirectoryKind,
        wanted_index: usize,
    ) -> Result<Option<exfat_embedded::DirectoryEntry>, ExFatFsError<D::Error>> {
        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let mut index = 0usize;
        let mut found = None;
        let mut visitor = |entry: &exfat_embedded::DirectoryEntry| {
            if index == wanted_index {
                found = Some(entry.clone());
                false
            } else {
                index += 1;
                true
            }
        };

        match directory {
            ExFatDirectoryKind::Root => backend.read_root(&mut scratch, &mut visitor)?,
            ExFatDirectoryKind::Nested(directory) => {
                backend.read_directory(directory, &mut scratch, &mut visitor)?
            }
        }
        Ok(found)
    }

    fn entry_to_stat(entry: &exfat_embedded::DirectoryEntry) -> pajoniiir_media_fs::FileStat {
        pajoniiir_media_fs::FileStat {
            kind: if entry.is_directory {
                pajoniiir_media_fs::EntryKind::Directory
            } else {
                pajoniiir_media_fs::EntryKind::File
            },
            len: entry.valid_length,
            modified_unix_seconds: None,
        }
    }
}

impl<'a, D> pajoniiir_media_fs::LeaseBound for ExFatFileSystem<'a, D>
where
    D: WritableBlockDevice,
{
    fn lease(&self) -> pajoniiir_media_session::MediaLease {
        self.lease
    }
}

impl<'a, D> pajoniiir_media_fs::FileSystem for ExFatFileSystem<'a, D>
where
    D: WritableBlockDevice,
{
    type Error = ExFatFsError<D::Error>;
    type File = ExFatFileHandle;
    type Directory = ExFatDirectoryHandle;

    fn kind(&self) -> pajoniiir_media_fs::FileSystemKind {
        pajoniiir_media_fs::FileSystemKind::ExFat
    }

    fn capabilities(&self) -> pajoniiir_media_fs::FileSystemCapabilities {
        pajoniiir_media_fs::FileSystemCapabilities {
            read: true,
            write: true,
            directories: true,
            long_names: true,
            flush: true,
            random_seek: false,
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

        let Self {
            backend,
            scratch_storage,
            workspace,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let entry = backend.lookup_with_workspace(path, &mut scratch, workspace)?;
        Ok(Self::entry_to_stat(&entry))
    }

    fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
        let Self {
            lease,
            backend,
            scratch_storage,
            workspace,
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let inner = backend.open_with_workspace(path, &mut scratch, workspace)?;
        Ok(ExFatFileHandle {
            lease: *lease,
            inner,
        })
    }

    fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)
    }

    fn read(&mut self, file: &mut Self::File, output: &mut [u8]) -> Result<usize, Self::Error> {
        self.ensure_lease(file.lease)?;
        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        backend
            .read(&mut file.inner, output, &mut scratch)
            .map_err(ExFatFsError::Backend)
    }

    fn seek_absolute(&mut self, file: &mut Self::File, position: u64) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)?;
        if position == file.inner.position() {
            Ok(())
        } else {
            Err(ExFatFsError::UnsupportedRandomSeek)
        }
    }

    fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
        let kind = self.directory_for_path(path)?;
        Ok(ExFatDirectoryHandle {
            lease: self.lease,
            kind,
            next_index: 0,
        })
    }

    fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; pajoniiir_media_fs::FS_NAME_MAX],
    ) -> Result<Option<pajoniiir_media_fs::DirEntry>, Self::Error> {
        self.ensure_lease(directory.lease)?;
        let Some(entry) = self.backend_directory_entry(directory.kind, directory.next_index)?
        else {
            return Ok(None);
        };
        let name_len =
            encode_utf16_name(entry.name_utf16(), name_storage).ok_or(ExFatFsError::NameTooLong)?;
        directory.next_index = directory.next_index.saturating_add(1);

        Ok(Some(pajoniiir_media_fs::DirEntry {
            kind: if entry.is_directory {
                pajoniiir_media_fs::EntryKind::Directory
            } else {
                pajoniiir_media_fs::EntryKind::File
            },
            len: entry.valid_length,
            modified_unix_seconds: None,
            name_len: name_len as u16,
        }))
    }

    fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error> {
        self.ensure_lease(directory.lease)
    }

    fn visit_directory<V: pajoniiir_media_fs::DirectoryVisitor>(
        &mut self,
        path: &str,
        name_storage: &mut [u8; pajoniiir_media_fs::FS_NAME_MAX],
        visitor: &mut V,
    ) -> Result<(), Self::Error> {
        let directory = self.directory_for_path(path)?;
        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let mut name_error = false;
        let mut backend_visitor = |entry: &exfat_embedded::DirectoryEntry| {
            let Some(name_len) = encode_utf16_name(entry.name_utf16(), name_storage) else {
                name_error = true;
                return false;
            };
            let item = pajoniiir_media_fs::DirEntry {
                kind: if entry.is_directory {
                    pajoniiir_media_fs::EntryKind::Directory
                } else {
                    pajoniiir_media_fs::EntryKind::File
                },
                len: entry.valid_length,
                modified_unix_seconds: None,
                name_len: name_len as u16,
            };
            visitor.visit(item, &name_storage[..name_len])
        };

        match directory {
            ExFatDirectoryKind::Root => backend.read_root(&mut scratch, &mut backend_visitor)?,
            ExFatDirectoryKind::Nested(directory) => {
                backend.read_directory(directory, &mut scratch, &mut backend_visitor)?
            }
        }
        if name_error {
            Err(ExFatFsError::NameTooLong)
        } else {
            Ok(())
        }
    }
}

impl<'a, D> pajoniiir_media_fs::WritableFileSystem for ExFatFileSystem<'a, D>
where
    D: WritableBlockDevice,
{
    fn open_write(&mut self, path: &str, truncate: bool) -> Result<Self::File, Self::Error> {
        let Self {
            lease,
            backend,
            scratch_storage,
            workspace,
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);

        let inner = if truncate {
            backend.create_with_workspace(path, &mut scratch, workspace)?
        } else {
            match backend.open_with_workspace(path, &mut scratch, workspace) {
                Ok(file) => file,
                Err(exfat_embedded::Error::PathNotFound) => {
                    backend.create_with_workspace(path, &mut scratch, workspace)?
                }
                Err(error) => return Err(ExFatFsError::Backend(error)),
            }
        };

        Ok(ExFatFileHandle {
            lease: *lease,
            inner,
        })
    }

    fn write(&mut self, file: &mut Self::File, input: &[u8]) -> Result<usize, Self::Error> {
        self.ensure_lease(file.lease)?;
        if input.is_empty() {
            return Ok(0);
        }

        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        let position = file.inner.position();
        let len = file.inner.len();

        if position >= len {
            return backend
                .append(&mut file.inner, input, &mut scratch)
                .map_err(ExFatFsError::Backend);
        }

        let within_len = usize::try_from((len - position).min(input.len() as u64))
            .map_err(|_| ExFatFsError::Backend(exfat_embedded::Error::Corrupt))?;
        let mut written = backend
            .write(&mut file.inner, &input[..within_len], &mut scratch)
            .map_err(ExFatFsError::Backend)?;

        if written == within_len && written < input.len() {
            written += backend
                .append(&mut file.inner, &input[written..], &mut scratch)
                .map_err(ExFatFsError::Backend)?;
        }
        Ok(written)
    }

    fn flush_file(&mut self, file: &mut Self::File) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)?;
        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        backend.flush(&mut scratch).map_err(ExFatFsError::Backend)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Self::Error> {
        let (from_parent, _) = split_parent_name(from);
        let (to_parent, to_name) = split_parent_name(to);
        if from_parent != to_parent || to_name.is_empty() {
            return Err(ExFatFsError::UnsupportedCrossDirectoryRename);
        }

        let Self {
            backend,
            scratch_storage,
            workspace,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        backend
            .rename_with_workspace(from, to_name, &mut scratch, workspace)
            .map_err(ExFatFsError::Backend)
    }

    fn remove(&mut self, path: &str) -> Result<(), Self::Error> {
        let Self {
            backend,
            scratch_storage,
            workspace,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        backend
            .remove_with_workspace(path, &mut scratch, workspace)
            .map_err(ExFatFsError::Backend)
    }

    fn sync(&mut self) -> Result<(), Self::Error> {
        let Self {
            backend,
            scratch_storage,
            ..
        } = self;
        let mut scratch = exfat_embedded::Scratch::new(scratch_storage);
        backend.flush(&mut scratch).map_err(ExFatFsError::Backend)
    }
}

fn split_parent_name(path: &str) -> (&str, &str) {
    let trimmed = path.trim_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", trimmed),
    }
}

fn encode_utf16_name(input: &[u16], output: &mut [u8]) -> Option<usize> {
    let mut written = 0usize;
    for decoded in core::char::decode_utf16(input.iter().copied()) {
        let character = decoded.unwrap_or(core::char::REPLACEMENT_CHARACTER);
        let mut encoded = [0u8; 4];
        let utf8 = character.encode_utf8(&mut encoded).as_bytes();
        let end = written.checked_add(utf8.len())?;
        if end > output.len() {
            return None;
        }
        output[written..end].copy_from_slice(utf8);
        written = end;
    }
    Some(written)
}

#[cfg(test)]
extern crate std;

use exfat_embedded::BlockDevice as ExFatBlockDevice;
use pajoniiir_media_block::{BlockGeometry, WritableBlockDevice};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExFatBlockError<E> {
    InvalidGeometry(BlockGeometry),
    InvalidTransfer,
    Backend(E),
}

pub struct ExFatBlockAdapter<D> {
    inner: D,
}

impl<D> ExFatBlockAdapter<D> {
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

impl<D> ExFatBlockAdapter<D>
where
    D: WritableBlockDevice,
{
    pub fn validate_geometry(&self) -> Result<BlockGeometry, ExFatBlockError<D::Error>> {
        let geometry = self.inner.geometry();
        if geometry.block_count == 0 || !matches!(geometry.block_size, 512 | 1_024 | 2_048 | 4_096)
        {
            return Err(ExFatBlockError::InvalidGeometry(geometry));
        }
        Ok(geometry)
    }
}

impl<D> ExFatBlockDevice for ExFatBlockAdapter<D>
where
    D: WritableBlockDevice,
{
    type Error = ExFatBlockError<D::Error>;

    fn sector_size(&self) -> usize {
        self.inner.geometry().block_size as usize
    }

    fn sector_count(&self) -> u64 {
        self.inner.geometry().block_count
    }

    fn read_sector(&mut self, lba: u64, output: &mut [u8]) -> Result<(), Self::Error> {
        let geometry = self.validate_geometry()?;
        geometry
            .validate_transfer(lba, 1, output.len())
            .map_err(|_| ExFatBlockError::InvalidTransfer)?;
        self.inner
            .read_blocks(lba, 1, output)
            .map_err(ExFatBlockError::Backend)
    }

    fn write_sector(&mut self, lba: u64, input: &[u8]) -> Result<(), Self::Error> {
        let geometry = self.validate_geometry()?;
        geometry
            .validate_transfer(lba, 1, input.len())
            .map_err(|_| ExFatBlockError::InvalidTransfer)?;
        self.inner
            .write_blocks(lba, 1, input)
            .map_err(ExFatBlockError::Backend)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.validate_geometry()?;
        self.inner.flush().map_err(ExFatBlockError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_media_block::{BlockDevice, TransferError};
    use pajoniiir_media_fs::{
        DirectoryVisitor, FS_NAME_MAX, FileHandle, FileSystem as _, WritableFileSystem as _,
    };
    use pajoniiir_media_session::{MediaGeneration, MediaLease, MediaSourceId};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Transfer(TransferError),
    }

    struct MemoryDevice<const N: usize> {
        geometry: BlockGeometry,
        bytes: [u8; N],
        reads: u32,
        writes: u32,
        flushes: u32,
    }

    impl<const N: usize> MemoryDevice<N> {
        fn new(block_size: u32, block_count: u64) -> Self {
            Self {
                geometry: BlockGeometry {
                    block_size,
                    block_count,
                },
                bytes: [0; N],
                reads: 0,
                writes: 0,
                flushes: 0,
            }
        }
    }

    impl<const N: usize> BlockDevice for MemoryDevice<N> {
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
                .map_err(TestError::Transfer)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let end = start
                .checked_add(output.len())
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let source = self
                .bytes
                .get(start..end)
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            output.copy_from_slice(source);
            self.reads += 1;
            Ok(())
        }
    }

    impl<const N: usize> WritableBlockDevice for MemoryDevice<N> {
        fn write_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            self.geometry
                .validate_transfer(first_block, block_count, input.len())
                .map_err(TestError::Transfer)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let end = start
                .checked_add(input.len())
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let destination = self
                .bytes
                .get_mut(start..end)
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            destination.copy_from_slice(input);
            self.writes += 1;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn bridge_accepts_all_exfat_sector_sizes() {
        for size in [512, 1_024, 2_048, 4_096] {
            let adapter = ExFatBlockAdapter::new(MemoryDevice::<4096>::new(size, 1));
            assert_eq!(
                adapter.validate_geometry(),
                Ok(BlockGeometry {
                    block_size: size,
                    block_count: 1,
                })
            );
            assert_eq!(ExFatBlockDevice::sector_size(&adapter), size as usize);
            assert_eq!(ExFatBlockDevice::sector_count(&adapter), 1);
        }
    }

    #[test]
    fn bridge_rejects_invalid_sector_size_and_empty_device() {
        let invalid = ExFatBlockAdapter::new(MemoryDevice::<4096>::new(768, 1));
        assert_eq!(
            invalid.validate_geometry(),
            Err(ExFatBlockError::InvalidGeometry(BlockGeometry {
                block_size: 768,
                block_count: 1,
            }))
        );

        let empty = ExFatBlockAdapter::new(MemoryDevice::<4096>::new(512, 0));
        assert_eq!(
            empty.validate_geometry(),
            Err(ExFatBlockError::InvalidGeometry(BlockGeometry {
                block_size: 512,
                block_count: 0,
            }))
        );
    }

    #[test]
    fn single_sector_read_write_and_flush_reach_transport() {
        let mut adapter = ExFatBlockAdapter::new(MemoryDevice::<2048>::new(512, 4));
        let write = [0x5au8; 512];
        ExFatBlockDevice::write_sector(&mut adapter, 2, &write).unwrap();

        let mut read = [0u8; 512];
        ExFatBlockDevice::read_sector(&mut adapter, 2, &mut read).unwrap();
        ExFatBlockDevice::flush(&mut adapter).unwrap();

        assert_eq!(read, write);
        assert_eq!(adapter.inner().reads, 1);
        assert_eq!(adapter.inner().writes, 1);
        assert_eq!(adapter.inner().flushes, 1);
    }

    #[test]
    fn bridge_rejects_wrong_buffer_size_and_out_of_range_lba() {
        let mut adapter = ExFatBlockAdapter::new(MemoryDevice::<2048>::new(512, 4));

        assert_eq!(
            ExFatBlockDevice::read_sector(&mut adapter, 0, &mut [0u8; 511]),
            Err(ExFatBlockError::InvalidTransfer)
        );
        assert_eq!(
            ExFatBlockDevice::write_sector(&mut adapter, 4, &[0u8; 512]),
            Err(ExFatBlockError::InvalidTransfer)
        );
        assert_eq!(adapter.inner().reads, 0);
        assert_eq!(adapter.inner().writes, 0);
    }

    struct HeapDevice {
        geometry: BlockGeometry,
        bytes: std::vec::Vec<u8>,
    }

    impl HeapDevice {
        fn new(block_size: u32, block_count: u64) -> Self {
            let byte_len = usize::try_from(block_size as u64 * block_count).unwrap();
            Self {
                geometry: BlockGeometry {
                    block_size,
                    block_count,
                },
                bytes: std::vec![0; byte_len],
            }
        }
    }

    impl BlockDevice for HeapDevice {
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
                .map_err(TestError::Transfer)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let end = start
                .checked_add(output.len())
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            output.copy_from_slice(
                self.bytes
                    .get(start..end)
                    .ok_or(TestError::Transfer(TransferError::OutOfRange))?,
            );
            Ok(())
        }
    }

    impl WritableBlockDevice for HeapDevice {
        fn write_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            self.geometry
                .validate_transfer(first_block, block_count, input.len())
                .map_err(TestError::Transfer)?;
            let start = usize::try_from(first_block)
                .ok()
                .and_then(|value| value.checked_mul(self.geometry.block_size as usize))
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            let end = start
                .checked_add(input.len())
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?;
            self.bytes
                .get_mut(start..end)
                .ok_or(TestError::Transfer(TransferError::OutOfRange))?
                .copy_from_slice(input);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn format_mount_append_reopen_and_read_round_trip() {
        use exfat_embedded::{FileSystem, Scratch, format_exfat};

        const BLOCK_SIZE: u32 = 512;
        const BLOCK_COUNT: u64 = 16_384;

        let device = HeapDevice::new(BLOCK_SIZE, BLOCK_COUNT);
        let mut adapter = ExFatBlockAdapter::new(device);
        let mut scratch_bytes = [0u8; BLOCK_SIZE as usize];
        let mut scratch = Scratch::new(&mut scratch_bytes);

        format_exfat(&mut adapter, &mut scratch).unwrap();

        let mut filesystem = FileSystem::mount(adapter, &mut scratch).unwrap();
        let mut created = filesystem.create("TRACK.BIN", &mut scratch).unwrap();
        let payload = b"Pajoniiir-M1 exFAT round-trip";
        assert_eq!(
            filesystem.append(&mut created, payload, &mut scratch),
            Ok(payload.len())
        );
        filesystem.flush(&mut scratch).unwrap();

        let mut reopened = filesystem.open("TRACK.BIN", &mut scratch).unwrap();
        assert_eq!(reopened.len(), payload.len() as u64);
        assert_eq!(reopened.position(), 0);

        let mut output = [0u8; 30];
        let read = filesystem
            .read(&mut reopened, &mut output, &mut scratch)
            .unwrap();
        assert_eq!(read, payload.len());
        assert_eq!(&output[..read], payload);
    }

    fn test_lease(source: u32) -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(source).unwrap(),
        }
    }

    fn mounted_neutral_fs<'a>(
        scratch_bytes: &'a mut [u8; 512],
        workspace: &'a mut exfat_embedded::Workspace,
    ) -> ExFatFileSystem<'a, HeapDevice> {
        let device = HeapDevice::new(512, 16_384);
        let mut adapter = ExFatBlockAdapter::new(device);
        let mut scratch = exfat_embedded::Scratch::new(scratch_bytes);
        exfat_embedded::format_exfat(&mut adapter, &mut scratch).unwrap();
        let device = adapter.into_inner();
        ExFatFileSystem::mount(device, test_lease(1), scratch_bytes, workspace).unwrap()
    }

    #[test]
    fn neutral_adapter_reads_writes_and_rejects_fake_random_seek() {
        let mut scratch_bytes = [0u8; 512];
        let mut workspace = exfat_embedded::Workspace::new();
        let mut fs = mounted_neutral_fs(&mut scratch_bytes, &mut workspace);

        assert_eq!(fs.kind(), pajoniiir_media_fs::FileSystemKind::ExFat);
        assert!(fs.capabilities().streaming_directory_visit);
        assert!(!fs.capabilities().random_seek);

        let mut file = fs.open_write("track.bin", true).unwrap();
        fs.write_all(&mut file, b"abcdef").unwrap();
        fs.flush_file(&mut file).unwrap();
        assert_eq!(file.position(), 6);
        fs.close_file(file).unwrap();

        let mut read = fs.open_read("track.bin").unwrap();
        let mut output = [0u8; 6];
        fs.read_exact(&mut read, &mut output).unwrap();
        assert_eq!(&output, b"abcdef");
        assert_eq!(
            fs.seek_absolute(&mut read, 0),
            Err(ExFatFsError::UnsupportedRandomSeek)
        );
    }

    struct NameVisitor {
        count: usize,
        saw_unicode: bool,
    }

    impl DirectoryVisitor for NameVisitor {
        fn visit(&mut self, _entry: pajoniiir_media_fs::DirEntry, name: &[u8]) -> bool {
            self.count += 1;
            if name == "Žuta mačka 🎵.flac".as_bytes() {
                self.saw_unicode = true;
            }
            true
        }
    }

    #[test]
    fn neutral_adapter_streams_unicode_directory_names_in_one_scan() {
        let mut scratch_bytes = [0u8; 512];
        let mut workspace = exfat_embedded::Workspace::new();
        let mut fs = mounted_neutral_fs(&mut scratch_bytes, &mut workspace);

        let mut file = fs.open_write("Žuta mačka 🎵.flac", true).unwrap();
        fs.write_all(&mut file, b"FLAC").unwrap();

        let mut storage = [0u8; FS_NAME_MAX];
        let mut visitor = NameVisitor {
            count: 0,
            saw_unicode: false,
        };
        fs.visit_directory("/", &mut storage, &mut visitor).unwrap();

        assert_eq!(visitor.count, 1);
        assert!(visitor.saw_unicode);
    }

    #[test]
    fn neutral_adapter_rejects_stale_media_handles() {
        let mut scratch_bytes = [0u8; 512];
        let mut workspace = exfat_embedded::Workspace::new();
        let mut fs = mounted_neutral_fs(&mut scratch_bytes, &mut workspace);
        let mut file = fs.open_write("stale.bin", true).unwrap();

        file.lease = test_lease(2);
        assert_eq!(
            fs.read(&mut file, &mut [0u8; 1]),
            Err(ExFatFsError::StaleLease)
        );
        assert_eq!(fs.write(&mut file, b"x"), Err(ExFatFsError::StaleLease));
    }

    #[test]
    fn utf16_name_encoder_is_allocation_free_and_lossless_for_unicode() {
        let input = [
            'Ž' as u16,
            'u' as u16,
            't' as u16,
            'a' as u16,
            0xd83c,
            0xdfb5,
        ];
        let mut output = [0u8; FS_NAME_MAX];
        let len = encode_utf16_name(&input, &mut output).unwrap();
        assert_eq!(&output[..len], "Žuta🎵".as_bytes());
    }

    #[test]
    fn four_kib_sector_transfer_remains_single_backend_block() {
        let mut adapter = ExFatBlockAdapter::new(MemoryDevice::<8192>::new(4_096, 2));
        let write = [0xa5u8; 4_096];

        ExFatBlockDevice::write_sector(&mut adapter, 1, &write).unwrap();
        let mut read = [0u8; 4_096];
        ExFatBlockDevice::read_sector(&mut adapter, 1, &mut read).unwrap();

        assert_eq!(read, write);
        assert_eq!(adapter.inner().writes, 1);
        assert_eq!(adapter.inner().reads, 1);
    }
}
