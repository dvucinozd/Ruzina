//! Profile upload (RP-02): validation through `Profile::parse`, the
//! upload/backup swap, and recovery after a power cut at every step.

use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll},
};
use ruzina_controller_profile::{
    HEADER_SIZE, INPUT_ENTRY_SIZE, OUTPUT_ENTRY_SIZE, ParseError, ProfileId, crc32,
};
use ruzina_controller_profile_storage::{
    CONTROLLER_PROFILE_ROOT, InstallError, InstallStep, ProfileScanBuffers,
};
use ruzina_media_fs::{
    AsyncFileSystem, AsyncWritableFileSystem, DirEntry, DirectoryHandle, EntryKind, FS_NAME_MAX,
    FileHandle, FileStat, FileSystemCapabilities, FileSystemKind, LeaseBound,
};
use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::task::{Wake, Waker};

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn run<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match pin!(future).as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test future pending"),
    }
}

fn lease() -> MediaLease {
    MediaLease {
        generation: MediaGeneration::initial(),
        source: MediaSourceId::new(3).unwrap(),
    }
}

/// A minimal valid S3CP v2 profile; `note` makes two profiles differ.
fn profile(vid: u16, pid: u16, note: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER_SIZE + INPUT_ENTRY_SIZE + OUTPUT_ENTRY_SIZE];
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
    bytes[input..input + 6].copy_from_slice(&[0x90, note, 0, 0xff, 1, 0x10]);
    let output = HEADER_SIZE + INPUT_ENTRY_SIZE;
    bytes[output] = 1;
    bytes[output + 3] = 0x90;
    bytes[output + 4] = note;
    bytes[output + 6] = 0x7f;
    bytes[output + 7] = 0x7f;
    let checksum = crc32(&bytes[16..]);
    bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemError {
    Missing,
    Cut,
}

#[derive(Clone, Copy, Debug)]
struct MemFile {
    slot: usize,
    len: u64,
    position: u64,
    lease: MediaLease,
}
impl LeaseBound for MemFile {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl FileHandle for MemFile {
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

#[derive(Clone, Debug)]
struct MemDir {
    entries: Vec<String>,
    cursor: usize,
    lease: MediaLease,
}
impl LeaseBound for MemDir {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl DirectoryHandle for MemDir {}

/// Writes land immediately; a cut stops every later operation until revive.
struct MemFs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    handles: Vec<String>,
    /// Mutating operations left before the power fails.
    budget: Option<usize>,
    dead: bool,
    mutations: usize,
    corrupt_writes: bool,
}

impl MemFs {
    fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            dirs: BTreeSet::new(),
            handles: Vec::new(),
            budget: None,
            dead: false,
            mutations: 0,
            corrupt_writes: false,
        }
    }

    fn with_profile(id: &str, bytes: &[u8]) -> Self {
        let mut fs = Self::new();
        fs.dirs.insert(CONTROLLER_PROFILE_ROOT.into());
        fs.dirs.insert(format!("{CONTROLLER_PROFILE_ROOT}/{id}"));
        fs.files.insert(
            format!("{CONTROLLER_PROFILE_ROOT}/{id}/profile.s3bin"),
            bytes.to_vec(),
        );
        fs
    }

    fn live(&self) -> Result<(), MemError> {
        if self.dead {
            Err(MemError::Cut)
        } else {
            Ok(())
        }
    }

    fn mutate(&mut self) -> Result<(), MemError> {
        self.live()?;
        if let Some(budget) = &mut self.budget {
            if *budget == 0 {
                self.dead = true;
                return Err(MemError::Cut);
            }
            *budget -= 1;
        }
        self.mutations += 1;
        Ok(())
    }

    fn revive(&mut self) {
        self.dead = false;
        self.budget = None;
        self.handles.clear();
    }

    fn file(&self, id: &str, name: &str) -> Option<&Vec<u8>> {
        self.files
            .get(&format!("{CONTROLLER_PROFILE_ROOT}/{id}/{name}"))
    }
}

impl LeaseBound for MemFs {
    fn lease(&self) -> MediaLease {
        lease()
    }
}

impl AsyncFileSystem for MemFs {
    type Error = MemError;
    type File = MemFile;
    type Directory = MemDir;

    fn kind(&self) -> FileSystemKind {
        FileSystemKind::Fat32
    }
    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities::READ_ONLY
    }
    async fn stat(&mut self, path: &str) -> Result<FileStat, MemError> {
        self.live()?;
        if let Some(bytes) = self.files.get(path) {
            return Ok(FileStat {
                kind: EntryKind::File,
                len: bytes.len() as u64,
                modified_unix_seconds: None,
            });
        }
        if self.dirs.contains(path) {
            return Ok(FileStat {
                kind: EntryKind::Directory,
                len: 0,
                modified_unix_seconds: None,
            });
        }
        Err(MemError::Missing)
    }
    async fn open_read(&mut self, path: &str) -> Result<MemFile, MemError> {
        self.live()?;
        let len = self.files.get(path).ok_or(MemError::Missing)?.len() as u64;
        self.handles.push(path.into());
        Ok(MemFile {
            slot: self.handles.len() - 1,
            len,
            position: 0,
            lease: lease(),
        })
    }
    async fn close_file(&mut self, _: MemFile) -> Result<(), MemError> {
        self.live()
    }
    async fn read(&mut self, file: &mut MemFile, output: &mut [u8]) -> Result<usize, MemError> {
        self.live()?;
        let bytes = self
            .files
            .get(&self.handles[file.slot])
            .ok_or(MemError::Missing)?;
        let start = (file.position as usize).min(bytes.len());
        let count = output.len().min(bytes.len() - start).min(97);
        output[..count].copy_from_slice(&bytes[start..start + count]);
        file.position += count as u64;
        Ok(count)
    }
    async fn seek_absolute(&mut self, file: &mut MemFile, position: u64) -> Result<(), MemError> {
        file.position = position;
        Ok(())
    }
    async fn open_directory(&mut self, path: &str) -> Result<MemDir, MemError> {
        self.live()?;
        if !self.dirs.contains(path) {
            return Err(MemError::Missing);
        }
        let prefix = format!("{path}/");
        let entries = self
            .dirs
            .iter()
            .filter_map(|dir| dir.strip_prefix(&prefix))
            .filter(|name| !name.contains('/'))
            .map(String::from)
            .collect();
        Ok(MemDir {
            entries,
            cursor: 0,
            lease: lease(),
        })
    }
    async fn next_entry(
        &mut self,
        directory: &mut MemDir,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, MemError> {
        let Some(name) = directory.entries.get(directory.cursor) else {
            return Ok(None);
        };
        directory.cursor += 1;
        name_storage[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Some(DirEntry {
            kind: EntryKind::Directory,
            len: 0,
            modified_unix_seconds: None,
            name_len: name.len() as u16,
        }))
    }
    async fn close_directory(&mut self, _: MemDir) -> Result<(), MemError> {
        Ok(())
    }
}

impl AsyncWritableFileSystem for MemFs {
    async fn open_write(&mut self, path: &str, truncate: bool) -> Result<MemFile, MemError> {
        self.mutate()?;
        let bytes = self.files.entry(path.into()).or_default();
        if truncate {
            bytes.clear();
        }
        let len = bytes.len() as u64;
        self.handles.push(path.into());
        Ok(MemFile {
            slot: self.handles.len() - 1,
            len,
            position: len,
            lease: lease(),
        })
    }
    async fn write(&mut self, file: &mut MemFile, input: &[u8]) -> Result<usize, MemError> {
        self.mutate()?;
        let count = input.len().min(100);
        let corrupt = self.corrupt_writes;
        let bytes = self
            .files
            .get_mut(&self.handles[file.slot])
            .ok_or(MemError::Missing)?;
        bytes.extend_from_slice(&input[..count]);
        if corrupt && !bytes.is_empty() {
            bytes[0] ^= 1;
        }
        file.position += count as u64;
        file.len = bytes.len() as u64;
        Ok(count)
    }
    async fn flush_file(&mut self, _: &mut MemFile) -> Result<(), MemError> {
        self.mutate()
    }
    async fn rename(&mut self, from: &str, to: &str) -> Result<(), MemError> {
        self.mutate()?;
        let bytes = self.files.remove(from).ok_or(MemError::Missing)?;
        self.files.insert(to.into(), bytes);
        Ok(())
    }
    async fn remove(&mut self, path: &str) -> Result<(), MemError> {
        self.mutate()?;
        self.files.remove(path).map(drop).ok_or(MemError::Missing)
    }
    async fn sync(&mut self) -> Result<(), MemError> {
        self.mutate()
    }
    async fn create_directory(&mut self, path: &str) -> Result<(), MemError> {
        self.mutate()?;
        self.dirs.insert(path.into());
        Ok(())
    }
}

const ID: &str = "pioneer_ddj_flx4";

fn installed_ids(fs: &mut MemFs, buffers: &mut ProfileScanBuffers) -> Vec<(String, u16)> {
    let report = run(buffers.scan(fs)).unwrap();
    (0..report.registry.len() as u8)
        .filter_map(|index| report.registry.profile(index))
        .map(|slot| (slot.id.as_str().to_string(), slot.meta.vid))
        .collect()
}

#[test]
fn a_valid_upload_installs_and_the_registry_scan_finds_it() {
    let mut fs = MemFs::new();
    let mut buffers = ProfileScanBuffers::new();
    let data = profile(0x2b73, 0x0045, 0x10);
    let installed = run(buffers.install(&mut fs, ID, &data, false)).unwrap();
    assert_eq!(installed.id, ProfileId::parse(ID).unwrap());
    assert_eq!(
        (installed.vid, installed.pid, installed.size),
        (0x2b73, 0x0045, data.len())
    );
    assert!(!installed.replaced);
    assert_eq!(fs.file(ID, "profile.s3bin"), Some(&data));
    assert_eq!(fs.file(ID, "profile.upload"), None);
    assert_eq!(fs.file(ID, "profile.backup"), None);
    assert_eq!(
        installed_ids(&mut fs, &mut buffers),
        [(ID.to_string(), 0x2b73)]
    );
}

#[test]
fn invalid_uploads_are_refused_before_any_write() {
    let mut buffers = ProfileScanBuffers::new();
    let data = profile(1, 2, 3);
    let mut bad_crc = data.clone();
    bad_crc[40] ^= 0xff;
    for (id, bytes, expected) in [
        ("../escape", data.clone(), InstallError::InvalidId),
        ("", data.clone(), InstallError::InvalidId),
        (ID, data[..31].to_vec(), InstallError::Size),
        (ID, vec![0u8; 16_385], InstallError::Size),
        (ID, bad_crc, InstallError::InvalidProfile(ParseError::Crc)),
    ] {
        let mut fs = MemFs::new();
        assert_eq!(
            run(buffers.install(&mut fs, id, &bytes, true)),
            Err(expected),
            "{id}"
        );
        assert_eq!(fs.mutations, 0, "nothing touched the card for {id}");
    }
}

#[test]
fn an_existing_profile_needs_overwrite_and_is_replaced_whole() {
    let old = profile(0x2b73, 0x0045, 0x10);
    let new = profile(0x2b73, 0x0045, 0x20);
    let mut fs = MemFs::with_profile(ID, &old);
    let mut buffers = ProfileScanBuffers::new();
    assert_eq!(
        run(buffers.install(&mut fs, ID, &new, false)),
        Err(InstallError::Exists)
    );
    assert_eq!(fs.file(ID, "profile.s3bin"), Some(&old));
    let installed = run(buffers.install(&mut fs, ID, &new, true)).unwrap();
    assert!(installed.replaced);
    assert_eq!(fs.file(ID, "profile.s3bin"), Some(&new));
    assert_eq!(fs.file(ID, "profile.backup"), None);
}

#[test]
fn a_write_that_reads_back_wrong_never_replaces_the_profile() {
    let old = profile(0x2b73, 0x0045, 0x10);
    let mut fs = MemFs::with_profile(ID, &old);
    fs.corrupt_writes = true;
    let mut buffers = ProfileScanBuffers::new();
    assert_eq!(
        run(buffers.install(&mut fs, ID, &profile(0x2b73, 0x0045, 0x20), true)),
        Err(InstallError::Io {
            step: InstallStep::VerifyUpload,
            error: None
        })
    );
    assert_eq!(fs.file(ID, "profile.s3bin"), Some(&old));
    assert_eq!(fs.file(ID, "profile.upload"), None);
}

/// Cut the power at every mutating step; after recovery the profile is the
/// old one or the new one, and no partial file remains.
fn every_cut(old: Option<&[u8]>, new: &[u8]) {
    let fresh = || match old {
        Some(old) => MemFs::with_profile(ID, old),
        None => MemFs::new(),
    };
    let mut probe = fresh();
    let mut buffers = ProfileScanBuffers::new();
    run(buffers.install(&mut probe, ID, new, true)).unwrap();
    let total = probe.mutations;
    assert!(total > 8);
    for cut in 0..total {
        let mut fs = fresh();
        fs.budget = Some(cut);
        assert!(
            run(buffers.install(&mut fs, ID, new, true)).is_err(),
            "cut {cut}"
        );
        fs.revive();
        run(buffers.recover_profile(&mut fs, ID)).unwrap();
        let target = fs.file(ID, "profile.s3bin").cloned();
        match old {
            Some(old) => assert!(
                target.as_deref() == Some(old) || target.as_deref() == Some(new),
                "cut {cut}: profile is neither old nor new"
            ),
            None => assert!(
                target.is_none() || target.as_deref() == Some(new),
                "cut {cut}: a partial profile survived"
            ),
        }
        assert_eq!(fs.file(ID, "profile.upload"), None, "cut {cut}");
        assert_eq!(fs.file(ID, "profile.backup"), None, "cut {cut}");
        // The interrupted install can simply be retried.
        run(buffers.install(&mut fs, ID, new, true)).unwrap();
        assert_eq!(fs.file(ID, "profile.s3bin").map(Vec::as_slice), Some(new));
    }
}

#[test]
fn a_power_cut_at_any_step_leaves_the_old_or_the_new_profile() {
    every_cut(
        Some(&profile(0x2b73, 0x0045, 0x10)),
        &profile(0x2b73, 0x0045, 0x20),
    );
    every_cut(None, &profile(0x2b73, 0x0045, 0x20));
}

#[test]
fn recovery_restores_a_valid_backup_over_a_broken_profile() {
    let old = profile(0x2b73, 0x0045, 0x10);
    let mut fs = MemFs::with_profile(ID, b"broken profile bytes, not S3CP");
    fs.files.insert(
        format!("{CONTROLLER_PROFILE_ROOT}/{ID}/profile.backup"),
        old.clone(),
    );
    let mut buffers = ProfileScanBuffers::new();
    run(buffers.recover_profile(&mut fs, ID)).unwrap();
    assert_eq!(fs.file(ID, "profile.s3bin"), Some(&old));
    assert_eq!(fs.file(ID, "profile.backup"), None);

    let mut fs = MemFs::new();
    fs.dirs.insert(CONTROLLER_PROFILE_ROOT.into());
    fs.files.insert(
        format!("{CONTROLLER_PROFILE_ROOT}/{ID}/profile.backup"),
        old.clone(),
    );
    run(buffers.recover_profile(&mut fs, ID)).unwrap();
    assert_eq!(
        fs.file(ID, "profile.s3bin"),
        Some(&old),
        "a missing target takes the backup"
    );
}
