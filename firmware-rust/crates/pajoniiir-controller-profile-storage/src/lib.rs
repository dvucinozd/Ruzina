#![no_std]
#![forbid(unsafe_code)]

pub use pajoniiir_controller_profile::{
    MAX_PROFILE_SIZE, ProfileActivationState, ProfileActivationTicket, ProfileRegistry, ProfileSlot,
};
use pajoniiir_controller_profile::{MAX_CONTROLLER_PROFILES, MAX_PROFILE_ID_BYTES, ProfileId};
use pajoniiir_media_fs::{AsyncFileSystem, EntryKind, FS_NAME_MAX, FileHandle};

pub const CONTROLLER_PROFILE_ROOT: &str = "/controllers";
pub const CONTROLLER_PROFILE_FILENAME: &str = "profile.s3bin";
const PROFILE_PATH_CAPACITY: usize = CONTROLLER_PROFILE_ROOT.len()
    + 1
    + MAX_PROFILE_ID_BYTES
    + 1
    + CONTROLLER_PROFILE_FILENAME.len();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProfileScanStats {
    pub entries_seen: u16,
    pub non_directories: u16,
    pub invalid_ids: u16,
    pub unreadable_profiles: u16,
    pub oversized_profiles: u16,
    pub short_reads: u16,
    pub invalid_profiles: u16,
    pub capacity_reached: bool,
}

pub struct ProfileScanReport {
    pub registry: ProfileRegistry,
    pub stats: ProfileScanStats,
}

/// Fixed-capacity scratch owned by the controller-profile storage task.
///
/// Keeping the 16 KiB profile blob and maximum filename buffer outside the
/// Embassy future avoids inflating the task state while preserving a strict
/// no-allocation scan path.
pub struct ProfileScanBuffers {
    name_storage: [u8; FS_NAME_MAX],
    profile_storage: [u8; MAX_PROFILE_SIZE],
}

impl ProfileScanBuffers {
    pub const fn new() -> Self {
        Self {
            name_storage: [0u8; FS_NAME_MAX],
            profile_storage: [0u8; MAX_PROFILE_SIZE],
        }
    }

    pub async fn scan<F>(
        &mut self,
        filesystem: &mut F,
    ) -> Result<ProfileScanReport, ProfileScanError<F::Error>>
    where
        F: AsyncFileSystem,
    {
        scan_controller_profiles(
            filesystem,
            &mut self.name_storage,
            &mut self.profile_storage,
        )
        .await
    }

    /// Reload one previously scanned profile into the same bounded scratch.
    ///
    /// The caller must revalidate the returned bytes against its activation
    /// ticket before publishing them. Storage I/O intentionally stays outside
    /// the registry lock so a disconnect or rescan can invalidate the ticket.
    pub async fn load_profile<'a, F>(
        &'a mut self,
        filesystem: &mut F,
        slot: ProfileSlot,
    ) -> Result<&'a [u8], ProfileLoadError<F::Error>>
    where
        F: AsyncFileSystem,
    {
        let path = ProfilePath::new(slot.id.as_str()).ok_or(ProfileLoadError::InvalidPath)?;
        let mut file = filesystem
            .open_read(path.as_str())
            .await
            .map_err(ProfileLoadError::Open)?;
        let file_len = match usize::try_from(file.len()) {
            Ok(len) if len <= MAX_PROFILE_SIZE => len,
            _ => {
                let _ = filesystem.close_file(file).await;
                return Err(ProfileLoadError::TooLarge);
            }
        };

        let mut offset = 0usize;
        while offset < file_len {
            match filesystem
                .read(&mut file, &mut self.profile_storage[offset..file_len])
                .await
            {
                Ok(0) => {
                    let _ = filesystem.close_file(file).await;
                    return Err(ProfileLoadError::ShortRead);
                }
                Ok(read) if read <= file_len - offset => offset += read,
                Ok(_) => {
                    let _ = filesystem.close_file(file).await;
                    return Err(ProfileLoadError::ReadOverflow);
                }
                Err(error) => {
                    let _ = filesystem.close_file(file).await;
                    return Err(ProfileLoadError::Read(error));
                }
            }
        }

        filesystem
            .close_file(file)
            .await
            .map_err(ProfileLoadError::Close)?;
        Ok(&self.profile_storage[..file_len])
    }
}

impl Default for ProfileScanBuffers {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum ProfileScanError<E> {
    OpenRoot(E),
    ReadRoot(E),
    CloseRoot(E),
}

#[derive(Debug)]
pub enum ProfileLoadError<E> {
    InvalidPath,
    Open(E),
    TooLarge,
    ShortRead,
    ReadOverflow,
    Read(E),
    Close(E),
}

struct ProfilePath {
    bytes: [u8; PROFILE_PATH_CAPACITY],
    len: usize,
}

impl ProfilePath {
    fn new(id: &str) -> Option<Self> {
        let expected = CONTROLLER_PROFILE_ROOT
            .len()
            .checked_add(1)?
            .checked_add(id.len())?
            .checked_add(1)?
            .checked_add(CONTROLLER_PROFILE_FILENAME.len())?;
        if expected > PROFILE_PATH_CAPACITY {
            return None;
        }

        let mut bytes = [0u8; PROFILE_PATH_CAPACITY];
        let mut cursor = 0usize;
        for part in [
            CONTROLLER_PROFILE_ROOT.as_bytes(),
            b"/",
            id.as_bytes(),
            b"/",
            CONTROLLER_PROFILE_FILENAME.as_bytes(),
        ] {
            let end = cursor.checked_add(part.len())?;
            bytes.get_mut(cursor..end)?.copy_from_slice(part);
            cursor = end;
        }

        Some(Self { bytes, len: cursor })
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len])
            .expect("profile path is assembled from validated UTF-8")
    }
}

/// Scan the released controller-profile directory layout without allocation.
///
/// Per-profile failures are counted and skipped so one corrupt/missing profile
/// cannot hide other valid controllers. Root-directory traversal failures are
/// fatal because the registry snapshot would otherwise be incomplete.
pub async fn scan_controller_profiles<F>(
    filesystem: &mut F,
    name_storage: &mut [u8; FS_NAME_MAX],
    profile_storage: &mut [u8; MAX_PROFILE_SIZE],
) -> Result<ProfileScanReport, ProfileScanError<F::Error>>
where
    F: AsyncFileSystem,
{
    let mut registry = ProfileRegistry::new();
    let mut stats = ProfileScanStats::default();
    let mut directory = filesystem
        .open_directory(CONTROLLER_PROFILE_ROOT)
        .await
        .map_err(ProfileScanError::OpenRoot)?;

    loop {
        let entry = match filesystem.next_entry(&mut directory, name_storage).await {
            Ok(entry) => entry,
            Err(error) => {
                let _ = filesystem.close_directory(directory).await;
                return Err(ProfileScanError::ReadRoot(error));
            }
        };
        let Some(entry) = entry else {
            break;
        };

        stats.entries_seen = stats.entries_seen.saturating_add(1);
        if entry.kind != EntryKind::Directory {
            stats.non_directories = stats.non_directories.saturating_add(1);
            continue;
        }

        let Some(name) = entry.name(name_storage) else {
            stats.invalid_ids = stats.invalid_ids.saturating_add(1);
            continue;
        };
        let Ok(id_text) = core::str::from_utf8(name) else {
            stats.invalid_ids = stats.invalid_ids.saturating_add(1);
            continue;
        };
        let Ok(profile_id) = ProfileId::parse(id_text) else {
            stats.invalid_ids = stats.invalid_ids.saturating_add(1);
            continue;
        };
        let Some(path) = ProfilePath::new(profile_id.as_str()) else {
            stats.invalid_ids = stats.invalid_ids.saturating_add(1);
            continue;
        };

        let mut file = match filesystem.open_read(path.as_str()).await {
            Ok(file) => file,
            Err(_) => {
                stats.unreadable_profiles = stats.unreadable_profiles.saturating_add(1);
                continue;
            }
        };

        let file_len = match usize::try_from(file.len()) {
            Ok(len) if len <= MAX_PROFILE_SIZE => len,
            _ => {
                stats.oversized_profiles = stats.oversized_profiles.saturating_add(1);
                let _ = filesystem.close_file(file).await;
                continue;
            }
        };

        let mut offset = 0usize;
        let mut read_failed = false;
        while offset < file_len {
            match filesystem
                .read(&mut file, &mut profile_storage[offset..file_len])
                .await
            {
                Ok(0) => {
                    stats.short_reads = stats.short_reads.saturating_add(1);
                    read_failed = true;
                    break;
                }
                Ok(read) if read <= file_len - offset => offset += read,
                Ok(_) | Err(_) => {
                    stats.unreadable_profiles = stats.unreadable_profiles.saturating_add(1);
                    read_failed = true;
                    break;
                }
            }
        }

        if filesystem.close_file(file).await.is_err() {
            stats.unreadable_profiles = stats.unreadable_profiles.saturating_add(1);
            continue;
        }
        if read_failed {
            continue;
        }

        if registry
            .push_blob(profile_id.as_str(), &profile_storage[..file_len])
            .is_err()
        {
            if registry.len() >= MAX_CONTROLLER_PROFILES {
                stats.capacity_reached = true;
                break;
            }
            stats.invalid_profiles = stats.invalid_profiles.saturating_add(1);
        }
    }

    filesystem
        .close_directory(directory)
        .await
        .map_err(ProfileScanError::CloseRoot)?;

    Ok(ProfileScanReport { registry, stats })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll},
    };
    use pajoniiir_controller_profile::{HEADER_SIZE, INPUT_ENTRY_SIZE, OUTPUT_ENTRY_SIZE, crc32};
    use pajoniiir_media_fs::{
        DirEntry, DirectoryHandle, FileStat, FileSystemCapabilities, FileSystemKind, LeaseBound,
    };
    use pajoniiir_media_session::{MediaGeneration, MediaLease, MediaSourceId};
    use std::{
        string::{String, ToString},
        sync::Arc,
        task::{Wake, Waker},
        vec::Vec,
    };

    #[derive(Default)]
    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut future = pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    fn lease() -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(7).unwrap(),
        }
    }

    fn profile_blob(vid: u16, pid: u16) -> Vec<u8> {
        let mut bytes = std::vec![0u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(b"S3CP");
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_le_bytes());
        bytes[16..18].copy_from_slice(&vid.to_le_bytes());
        bytes[18..20].copy_from_slice(&pid.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[29] = 2;

        let input = HEADER_SIZE;
        bytes[input] = 0x90;
        bytes[input + 1] = 0x0b;
        bytes[input + 2] = 0;
        bytes[input + 3] = 0xff;
        bytes[input + 4] = 1;
        bytes[input + 5] = 0x10;

        let output = HEADER_SIZE + INPUT_ENTRY_SIZE;
        bytes[output] = 1;
        bytes[output + 2] = 0;
        bytes[output + 3] = 0x90;
        bytes[output + 4] = 0x0b;
        bytes[output + 6] = 0x7f;
        bytes[output + 7] = 0x7f;

        let checksum = crc32(&bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[derive(Clone, Debug)]
    struct TestProfile {
        id: String,
        bytes: Vec<u8>,
        directory: bool,
        readable: bool,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Missing,
        Read,
    }

    #[derive(Clone, Copy, Debug)]
    struct TestFile {
        slot: usize,
        len: u64,
        position: u64,
        lease: MediaLease,
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

    #[derive(Clone, Copy, Debug)]
    struct TestDirectory {
        cursor: usize,
        lease: MediaLease,
    }

    impl LeaseBound for TestDirectory {
        fn lease(&self) -> MediaLease {
            self.lease
        }
    }

    impl DirectoryHandle for TestDirectory {}

    struct TestFs {
        profiles: Vec<TestProfile>,
        lease: MediaLease,
    }

    impl LeaseBound for TestFs {
        fn lease(&self) -> MediaLease {
            self.lease
        }
    }

    impl AsyncFileSystem for TestFs {
        type Error = TestError;
        type File = TestFile;
        type Directory = TestDirectory;

        fn kind(&self) -> FileSystemKind {
            FileSystemKind::Fat32
        }

        fn capabilities(&self) -> FileSystemCapabilities {
            FileSystemCapabilities::READ_ONLY
        }

        async fn stat(&mut self, _path: &str) -> Result<FileStat, Self::Error> {
            Err(TestError::Missing)
        }

        async fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
            let prefix = "/controllers/";
            let suffix = "/profile.s3bin";
            let id = path
                .strip_prefix(prefix)
                .and_then(|path| path.strip_suffix(suffix))
                .ok_or(TestError::Missing)?;
            let slot = self
                .profiles
                .iter()
                .position(|profile| profile.id == id)
                .ok_or(TestError::Missing)?;
            let profile = &self.profiles[slot];
            if !profile.readable {
                return Err(TestError::Read);
            }
            Ok(TestFile {
                slot,
                len: profile.bytes.len() as u64,
                position: 0,
                lease: self.lease,
            })
        }

        async fn close_file(&mut self, _file: Self::File) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn read(
            &mut self,
            file: &mut Self::File,
            output: &mut [u8],
        ) -> Result<usize, Self::Error> {
            let bytes = &self.profiles[file.slot].bytes;
            let start = file.position as usize;
            if start >= bytes.len() {
                return Ok(0);
            }
            let count = output.len().min(bytes.len() - start).min(17);
            output[..count].copy_from_slice(&bytes[start..start + count]);
            file.position += count as u64;
            Ok(count)
        }

        async fn seek_absolute(
            &mut self,
            file: &mut Self::File,
            position: u64,
        ) -> Result<(), Self::Error> {
            file.position = position;
            Ok(())
        }

        async fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
            if path != CONTROLLER_PROFILE_ROOT {
                return Err(TestError::Missing);
            }
            Ok(TestDirectory {
                cursor: 0,
                lease: self.lease,
            })
        }

        async fn next_entry(
            &mut self,
            directory: &mut Self::Directory,
            name_storage: &mut [u8; FS_NAME_MAX],
        ) -> Result<Option<DirEntry>, Self::Error> {
            let Some(profile) = self.profiles.get(directory.cursor) else {
                return Ok(None);
            };
            directory.cursor += 1;
            let name = profile.id.as_bytes();
            name_storage[..name.len()].copy_from_slice(name);
            Ok(Some(DirEntry {
                kind: if profile.directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                len: 0,
                modified_unix_seconds: None,
                name_len: name.len() as u16,
            }))
        }

        async fn close_directory(
            &mut self,
            _directory: Self::Directory,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn fs(profiles: Vec<TestProfile>) -> TestFs {
        TestFs {
            profiles,
            lease: lease(),
        }
    }

    #[test]
    fn scan_populates_exact_vid_pid_registry_and_skips_bad_entries() {
        let valid = profile_blob(0x2b73, 0x0045);
        let mut invalid = profile_blob(0x06f8, 0xe020);
        invalid[0] = b'X';
        let mut filesystem = fs(std::vec![
            TestProfile {
                id: "pioneer_ddj_flx4".to_string(),
                bytes: valid,
                directory: true,
                readable: true,
            },
            TestProfile {
                id: "../escape".to_string(),
                bytes: profile_blob(1, 2),
                directory: true,
                readable: true,
            },
            TestProfile {
                id: "broken".to_string(),
                bytes: invalid,
                directory: true,
                readable: true,
            },
            TestProfile {
                id: "README.txt".to_string(),
                bytes: Vec::new(),
                directory: false,
                readable: true,
            },
            TestProfile {
                id: "missing".to_string(),
                bytes: Vec::new(),
                directory: true,
                readable: false,
            },
        ]);
        let mut names = [0u8; FS_NAME_MAX];
        let mut blob = [0u8; MAX_PROFILE_SIZE];

        let report = block_on_ready(scan_controller_profiles(
            &mut filesystem,
            &mut names,
            &mut blob,
        ))
        .unwrap();

        assert_eq!(report.registry.len(), 1);
        assert_eq!(report.registry.match_index(0x2b73, 0x0045), Some(0));
        assert_eq!(report.stats.entries_seen, 5);
        assert_eq!(report.stats.invalid_ids, 1);
        assert_eq!(report.stats.invalid_profiles, 1);
        assert_eq!(report.stats.non_directories, 1);
        assert_eq!(report.stats.unreadable_profiles, 1);
    }

    #[test]
    fn scan_reuses_one_blob_buffer_for_released_profile_limit() {
        let mut profiles = Vec::new();
        for index in 0..MAX_CONTROLLER_PROFILES {
            profiles.push(TestProfile {
                id: std::format!("controller_{index}"),
                bytes: profile_blob(0x1000 + index as u16, 0x2000 + index as u16),
                directory: true,
                readable: true,
            });
        }
        profiles.push(TestProfile {
            id: "controller_extra".to_string(),
            bytes: profile_blob(0xaaaa, 0xbbbb),
            directory: true,
            readable: true,
        });

        let mut filesystem = fs(profiles);
        let mut names = [0u8; FS_NAME_MAX];
        let mut blob = [0u8; MAX_PROFILE_SIZE];
        let report = block_on_ready(scan_controller_profiles(
            &mut filesystem,
            &mut names,
            &mut blob,
        ))
        .unwrap();

        assert_eq!(report.registry.len(), MAX_CONTROLLER_PROFILES);
        assert!(report.stats.capacity_reached);
        assert_eq!(report.registry.match_index(0xaaaa, 0xbbbb), None);
    }

    #[test]
    fn reload_profile_uses_scanned_slot_and_bounded_scratch() {
        let bytes = profile_blob(0x2b73, 0x0045);
        let slot = ProfileSlot::from_blob("pioneer_ddj_flx4", &bytes).unwrap();
        let mut filesystem = fs(std::vec![TestProfile {
            id: "pioneer_ddj_flx4".to_string(),
            bytes: bytes.clone(),
            directory: true,
            readable: true,
        }]);
        let mut buffers = ProfileScanBuffers::new();

        let loaded = block_on_ready(buffers.load_profile(&mut filesystem, slot)).unwrap();

        assert_eq!(loaded, bytes.as_slice());
        assert_eq!(
            pajoniiir_controller_profile::ProfileMeta::parse(loaded).unwrap(),
            slot.meta
        );
    }

}
