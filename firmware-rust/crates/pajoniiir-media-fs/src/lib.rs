#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_media_session::MediaLease;

/// Maximum UTF-8 bytes required to losslessly expose an exFAT filename.
///
/// exFAT allows 255 UTF-16 code units; a valid Unicode scalar may occupy up to
/// four UTF-8 bytes. Callers own this buffer so directory scans never allocate.
pub const FS_NAME_MAX: usize = 255 * 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSystemKind {
    Fat32,
    ExFat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSystemCapabilities {
    pub read: bool,
    pub write: bool,
    pub directories: bool,
    pub long_names: bool,
    pub flush: bool,
    /// True only when seek cost is independent of the target byte offset.
    pub random_seek: bool,
    /// True when a complete directory can be visited in one backend scan.
    pub streaming_directory_visit: bool,
}

impl FileSystemCapabilities {
    pub const READ_ONLY: Self = Self {
        read: true,
        write: false,
        directories: true,
        long_names: true,
        flush: false,
        random_seek: false,
        streaming_directory_visit: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStat {
    pub kind: EntryKind,
    pub len: u64,
    pub modified_unix_seconds: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirEntry {
    pub kind: EntryKind,
    pub len: u64,
    pub modified_unix_seconds: Option<i64>,
    pub name_len: u16,
}

impl DirEntry {
    pub fn name<'a>(&self, storage: &'a [u8]) -> Option<&'a [u8]> {
        let len = self.name_len as usize;
        if len <= storage.len() {
            Some(&storage[..len])
        } else {
            None
        }
    }
}

pub trait LeaseBound {
    fn lease(&self) -> MediaLease;
}

pub trait FileHandle: LeaseBound {
    fn len(&self) -> u64;
    fn position(&self) -> u64;
    fn set_position(&mut self, position: u64);

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn remaining(&self) -> u64 {
        self.len().saturating_sub(self.position())
    }
}

pub trait DirectoryHandle: LeaseBound {}

/// Allocation-free visitor used for large library scans.
///
/// Implementations receive a name slice backed by caller-owned `name_storage`
/// and must not retain it after `visit` returns.
pub trait DirectoryVisitor {
    /// Return `true` to continue scanning, `false` to stop successfully.
    fn visit(&mut self, entry: DirEntry, name: &[u8]) -> bool;
}

#[derive(Debug, Eq, PartialEq)]
pub enum IoContractError<E> {
    Inner(E),
    UnexpectedEof,
    WriteZero,
    InvalidBackendCount,
    BeyondEnd,
}

pub trait FileSystem: LeaseBound {
    type Error;
    type File: FileHandle;
    type Directory: DirectoryHandle;

    fn kind(&self) -> FileSystemKind;
    fn capabilities(&self) -> FileSystemCapabilities;

    fn stat(&mut self, path: &str) -> Result<FileStat, Self::Error>;

    fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error>;

    fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error>;

    fn read(&mut self, file: &mut Self::File, output: &mut [u8]) -> Result<usize, Self::Error>;

    fn seek_absolute(&mut self, file: &mut Self::File, position: u64) -> Result<(), Self::Error>;

    fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error>;

    fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Self::Error>;

    fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error>;

    /// Visit a directory without requiring a retained iterator or allocation.
    ///
    /// The default implementation is expressed through `next_entry` for
    /// compatibility. Backends with native streaming directory scans should
    /// override this method so large libraries remain O(n).
    fn visit_directory<V: DirectoryVisitor>(
        &mut self,
        path: &str,
        name_storage: &mut [u8; FS_NAME_MAX],
        visitor: &mut V,
    ) -> Result<(), Self::Error> {
        let mut directory = self.open_directory(path)?;
        loop {
            let Some(entry) = self.next_entry(&mut directory, name_storage)? else {
                break;
            };
            let name = entry.name(name_storage).unwrap_or(&[]);
            if !visitor.visit(entry, name) {
                break;
            }
        }
        self.close_directory(directory)
    }

    fn read_exact(
        &mut self,
        file: &mut Self::File,
        mut output: &mut [u8],
    ) -> Result<(), IoContractError<Self::Error>> {
        while !output.is_empty() {
            let read = self.read(file, output).map_err(IoContractError::Inner)?;
            if read == 0 {
                return Err(IoContractError::UnexpectedEof);
            }
            if read > output.len() {
                return Err(IoContractError::InvalidBackendCount);
            }
            output = &mut output[read..];
        }
        Ok(())
    }

    fn seek_checked(
        &mut self,
        file: &mut Self::File,
        position: u64,
    ) -> Result<(), IoContractError<Self::Error>> {
        if position > file.len() {
            return Err(IoContractError::BeyondEnd);
        }
        self.seek_absolute(file, position)
            .map_err(IoContractError::Inner)?;
        file.set_position(position);
        Ok(())
    }
}

pub trait WritableFileSystem: FileSystem {
    fn open_write(&mut self, path: &str, truncate: bool) -> Result<Self::File, Self::Error>;

    fn write(&mut self, file: &mut Self::File, input: &[u8]) -> Result<usize, Self::Error>;

    fn flush_file(&mut self, file: &mut Self::File) -> Result<(), Self::Error>;

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Self::Error>;
    fn remove(&mut self, path: &str) -> Result<(), Self::Error>;
    fn sync(&mut self) -> Result<(), Self::Error>;

    fn write_all(
        &mut self,
        file: &mut Self::File,
        mut input: &[u8],
    ) -> Result<(), IoContractError<Self::Error>> {
        while !input.is_empty() {
            let written = self.write(file, input).map_err(IoContractError::Inner)?;
            if written == 0 {
                return Err(IoContractError::WriteZero);
            }
            if written > input.len() {
                return Err(IoContractError::InvalidBackendCount);
            }
            input = &input[written..];
        }
        Ok(())
    }
}

/// Native asynchronous filesystem contract for executor-owned storage backends.
///
/// This is intentionally a separate trait instead of a blanket adapter around
/// FileSystem. Synchronous FAT/exFAT implementations may block while doing
/// device I/O and must not be disguised as async operations inside Embassy
/// tasks. Target backends should implement this contract over a genuinely
/// asynchronous transport such as the USB0 owned-buffer queue.
#[allow(async_fn_in_trait)]
pub trait AsyncFileSystem: LeaseBound {
    type Error;
    type File: FileHandle;
    type Directory: DirectoryHandle;

    fn kind(&self) -> FileSystemKind;
    fn capabilities(&self) -> FileSystemCapabilities;

    async fn stat(&mut self, path: &str) -> Result<FileStat, Self::Error>;

    async fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error>;

    async fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error>;

    async fn read(
        &mut self,
        file: &mut Self::File,
        output: &mut [u8],
    ) -> Result<usize, Self::Error>;

    async fn seek_absolute(
        &mut self,
        file: &mut Self::File,
        position: u64,
    ) -> Result<(), Self::Error>;

    async fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error>;

    async fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Self::Error>;

    async fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error>;

    async fn read_exact(
        &mut self,
        file: &mut Self::File,
        mut output: &mut [u8],
    ) -> Result<(), IoContractError<Self::Error>> {
        while !output.is_empty() {
            let read = self
                .read(file, output)
                .await
                .map_err(IoContractError::Inner)?;
            if read == 0 {
                return Err(IoContractError::UnexpectedEof);
            }
            if read > output.len() {
                return Err(IoContractError::InvalidBackendCount);
            }
            output = &mut output[read..];
        }
        Ok(())
    }

    async fn seek_checked(
        &mut self,
        file: &mut Self::File,
        position: u64,
    ) -> Result<(), IoContractError<Self::Error>> {
        if position > file.len() {
            return Err(IoContractError::BeyondEnd);
        }
        self.seek_absolute(file, position)
            .await
            .map_err(IoContractError::Inner)?;
        file.set_position(position);
        Ok(())
    }
}

/// Writable extension of AsyncFileSystem.
///
/// Implementations must not return success from flush_file or sync until the
/// underlying asynchronous transport has completed the corresponding
/// durability boundary.
#[allow(async_fn_in_trait)]
pub trait AsyncWritableFileSystem: AsyncFileSystem {
    async fn open_write(&mut self, path: &str, truncate: bool) -> Result<Self::File, Self::Error>;

    async fn write(&mut self, file: &mut Self::File, input: &[u8]) -> Result<usize, Self::Error>;

    async fn flush_file(&mut self, file: &mut Self::File) -> Result<(), Self::Error>;

    async fn rename(&mut self, from: &str, to: &str) -> Result<(), Self::Error>;
    async fn remove(&mut self, path: &str) -> Result<(), Self::Error>;
    async fn sync(&mut self) -> Result<(), Self::Error>;

    async fn write_all(
        &mut self,
        file: &mut Self::File,
        mut input: &[u8],
    ) -> Result<(), IoContractError<Self::Error>> {
        while !input.is_empty() {
            let written = self
                .write(file, input)
                .await
                .map_err(IoContractError::Inner)?;
            if written == 0 {
                return Err(IoContractError::WriteZero);
            }
            if written > input.len() {
                return Err(IoContractError::InvalidBackendCount);
            }
            input = &input[written..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_media_session::{MediaGeneration, MediaSourceId};

    fn lease() -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        InvalidHandle,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestFile {
        lease: MediaLease,
        slot: u8,
        len: u64,
        position: u64,
    }

    impl LeaseBound for TestFile {
        fn lease(&self) -> MediaLease {
            self.lease
        }
    }

    impl FileHandle for TestFile {
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
    struct TestDirectory {
        lease: MediaLease,
        slot: u8,
    }

    impl LeaseBound for TestDirectory {
        fn lease(&self) -> MediaLease {
            self.lease
        }
    }

    impl DirectoryHandle for TestDirectory {}

    struct TestFs {
        lease: MediaLease,
        bytes: [u8; 8],
        open_slots: u8,
        max_chunk: usize,
        lie_about_count: bool,
    }

    impl TestFs {
        fn new() -> Self {
            Self {
                lease: lease(),
                bytes: *b"abcdefgh",
                open_slots: 0,
                max_chunk: 8,
                lie_about_count: false,
            }
        }
    }

    impl LeaseBound for TestFs {
        fn lease(&self) -> MediaLease {
            self.lease
        }
    }

    impl FileSystem for TestFs {
        type Error = TestError;
        type File = TestFile;
        type Directory = TestDirectory;

        fn kind(&self) -> FileSystemKind {
            FileSystemKind::Fat32
        }

        fn capabilities(&self) -> FileSystemCapabilities {
            FileSystemCapabilities::READ_ONLY
        }

        fn stat(&mut self, _path: &str) -> Result<FileStat, Self::Error> {
            Ok(FileStat {
                kind: EntryKind::File,
                len: self.bytes.len() as u64,
                modified_unix_seconds: None,
            })
        }

        fn open_read(&mut self, _path: &str) -> Result<Self::File, Self::Error> {
            let slot = self.open_slots;
            self.open_slots = self.open_slots.wrapping_add(1);
            Ok(TestFile {
                lease: self.lease,
                slot,
                len: self.bytes.len() as u64,
                position: 0,
            })
        }

        fn close_file(&mut self, _file: Self::File) -> Result<(), Self::Error> {
            Ok(())
        }

        fn read(&mut self, file: &mut Self::File, output: &mut [u8]) -> Result<usize, Self::Error> {
            if file.lease != self.lease {
                return Err(TestError::InvalidHandle);
            }
            if self.lie_about_count {
                return Ok(output.len() + 1);
            }
            let position = file.position as usize;
            let remaining = self.bytes.len().saturating_sub(position);
            let count = remaining.min(output.len()).min(self.max_chunk);
            output[..count].copy_from_slice(&self.bytes[position..position + count]);
            file.position += count as u64;
            Ok(count)
        }

        fn seek_absolute(
            &mut self,
            file: &mut Self::File,
            position: u64,
        ) -> Result<(), Self::Error> {
            if file.lease != self.lease {
                return Err(TestError::InvalidHandle);
            }
            file.position = position;
            Ok(())
        }

        fn open_directory(&mut self, _path: &str) -> Result<Self::Directory, Self::Error> {
            Ok(TestDirectory {
                lease: self.lease,
                slot: 0,
            })
        }

        fn next_entry(
            &mut self,
            _directory: &mut Self::Directory,
            _name_storage: &mut [u8; FS_NAME_MAX],
        ) -> Result<Option<DirEntry>, Self::Error> {
            Ok(None)
        }

        fn close_directory(&mut self, _directory: Self::Directory) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn detached_handles_allow_two_decks_to_open_files_concurrently() {
        let mut fs = TestFs::new();
        let first = fs.open_read("deck-a.wav").unwrap();
        let second = fs.open_read("deck-b.wav").unwrap();

        assert_ne!(first.slot, second.slot);
        assert_eq!(first.lease(), fs.lease());
        assert_eq!(second.lease(), fs.lease());
    }

    #[test]
    fn read_exact_handles_partial_backend_reads() {
        let mut fs = TestFs::new();
        fs.max_chunk = 2;
        let mut file = fs.open_read("track.wav").unwrap();
        let mut output = [0u8; 7];

        assert_eq!(fs.read_exact(&mut file, &mut output), Ok(()));
        assert_eq!(&output, b"abcdefg");
        assert_eq!(file.position(), 7);
        assert_eq!(file.remaining(), 1);
    }

    #[test]
    fn read_exact_rejects_eof_and_impossible_backend_count() {
        let mut fs = TestFs::new();
        let mut file = TestFile {
            lease: fs.lease,
            slot: 0,
            len: 3,
            position: 0,
        };
        assert_eq!(
            fs.read_exact(&mut file, &mut [0u8; 9]),
            Err(IoContractError::UnexpectedEof)
        );

        fs.lie_about_count = true;
        let mut liar = fs.open_read("track.wav").unwrap();
        assert_eq!(
            fs.read_exact(&mut liar, &mut [0u8; 4]),
            Err(IoContractError::InvalidBackendCount)
        );
    }

    #[test]
    fn checked_seek_rejects_positions_past_eof() {
        let mut fs = TestFs::new();
        let mut file = fs.open_read("track.wav").unwrap();

        assert_eq!(fs.seek_checked(&mut file, 8), Ok(()));
        assert_eq!(file.position(), 8);
        assert_eq!(
            fs.seek_checked(&mut file, 9),
            Err(IoContractError::BeyondEnd)
        );
    }

    #[test]
    fn stale_generation_is_explicit_on_detached_handles() {
        let mut fs = TestFs::new();
        let mut file = fs.open_read("track.wav").unwrap();
        let stale = file.lease;

        fs.lease = MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(2).unwrap(),
        };

        assert_eq!(file.lease(), stale);
        assert_eq!(
            fs.read(&mut file, &mut [0u8; 1]),
            Err(TestError::InvalidHandle)
        );
    }

    #[test]
    fn dir_entry_never_exposes_name_beyond_caller_storage() {
        let entry = DirEntry {
            kind: EntryKind::File,
            len: 123,
            modified_unix_seconds: None,
            name_len: 5,
        };
        assert_eq!(entry.name(b"track.wav"), Some(b"track".as_slice()));

        let invalid = DirEntry {
            name_len: 10,
            ..entry
        };
        assert_eq!(invalid.name(b"short"), None);
    }

    struct CountVisitor {
        count: usize,
    }

    impl DirectoryVisitor for CountVisitor {
        fn visit(&mut self, _entry: DirEntry, _name: &[u8]) -> bool {
            self.count += 1;
            true
        }
    }

    #[test]
    fn default_directory_visit_is_allocation_free_and_compatible() {
        let mut fs = TestFs::new();
        let mut storage = [0u8; FS_NAME_MAX];
        let mut visitor = CountVisitor { count: 0 };

        assert_eq!(fs.visit_directory("/", &mut storage, &mut visitor), Ok(()));
        assert_eq!(visitor.count, 0);
    }

    #[test]
    fn utf8_name_storage_covers_worst_case_exfat_name_width() {
        assert_eq!(FS_NAME_MAX, 1_020);
    }

    struct AsyncTestFs(TestFs);

    impl AsyncTestFs {
        fn new() -> Self {
            Self(TestFs::new())
        }
    }

    impl LeaseBound for AsyncTestFs {
        fn lease(&self) -> MediaLease {
            self.0.lease()
        }
    }

    impl AsyncFileSystem for AsyncTestFs {
        type Error = TestError;
        type File = TestFile;
        type Directory = TestDirectory;

        fn kind(&self) -> FileSystemKind {
            FileSystem::kind(&self.0)
        }

        fn capabilities(&self) -> FileSystemCapabilities {
            FileSystem::capabilities(&self.0)
        }

        async fn stat(&mut self, path: &str) -> Result<FileStat, Self::Error> {
            FileSystem::stat(&mut self.0, path)
        }

        async fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
            FileSystem::open_read(&mut self.0, path)
        }

        async fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error> {
            FileSystem::close_file(&mut self.0, file)
        }

        async fn read(
            &mut self,
            file: &mut Self::File,
            output: &mut [u8],
        ) -> Result<usize, Self::Error> {
            FileSystem::read(&mut self.0, file, output)
        }

        async fn seek_absolute(
            &mut self,
            file: &mut Self::File,
            position: u64,
        ) -> Result<(), Self::Error> {
            FileSystem::seek_absolute(&mut self.0, file, position)
        }

        async fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
            FileSystem::open_directory(&mut self.0, path)
        }

        async fn next_entry(
            &mut self,
            directory: &mut Self::Directory,
            name_storage: &mut [u8; FS_NAME_MAX],
        ) -> Result<Option<DirEntry>, Self::Error> {
            FileSystem::next_entry(&mut self.0, directory, name_storage)
        }

        async fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error> {
            FileSystem::close_directory(&mut self.0, directory)
        }
    }

    fn block_on_ready<F: core::future::Future>(future: F) -> F::Output {
        let waker = core::task::Waker::noop();
        let mut context = core::task::Context::from_waker(waker);
        let mut future = core::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            core::task::Poll::Ready(output) => output,
            core::task::Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    #[test]
    fn async_contract_preserves_partial_read_and_seek_rules() {
        let mut fs = AsyncTestFs::new();
        fs.0.max_chunk = 2;
        let mut file = block_on_ready(AsyncFileSystem::open_read(&mut fs, "track.wav")).unwrap();
        let mut output = [0u8; 7];

        assert_eq!(
            block_on_ready(AsyncFileSystem::read_exact(&mut fs, &mut file, &mut output)),
            Ok(())
        );
        assert_eq!(&output, b"abcdefg");
        assert_eq!(file.position(), 7);

        assert_eq!(
            block_on_ready(AsyncFileSystem::seek_checked(&mut fs, &mut file, 8)),
            Ok(())
        );
        assert_eq!(file.position(), 8);
        assert_eq!(
            block_on_ready(AsyncFileSystem::seek_checked(&mut fs, &mut file, 9)),
            Err(IoContractError::BeyondEnd)
        );
    }

    #[test]
    fn async_contract_rejects_stale_generation_and_impossible_counts() {
        let mut fs = AsyncTestFs::new();
        let mut file = block_on_ready(AsyncFileSystem::open_read(&mut fs, "track.wav")).unwrap();

        fs.0.lease = MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(2).unwrap(),
        };
        assert_eq!(
            block_on_ready(AsyncFileSystem::read(&mut fs, &mut file, &mut [0u8; 1])),
            Err(TestError::InvalidHandle)
        );

        fs.0.lease = file.lease();
        fs.0.lie_about_count = true;
        assert_eq!(
            block_on_ready(AsyncFileSystem::read_exact(
                &mut fs,
                &mut file,
                &mut [0u8; 4]
            )),
            Err(IoContractError::InvalidBackendCount)
        );
    }

    #[test]
    fn capabilities_keep_write_support_explicit() {
        assert_eq!(
            FileSystemCapabilities::READ_ONLY,
            FileSystemCapabilities {
                read: true,
                write: false,
                directories: true,
                long_names: true,
                flush: false,
                random_seek: false,
                streaming_directory_visit: false,
            }
        );
    }
}
