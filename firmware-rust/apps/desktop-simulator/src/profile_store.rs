//! Explicit per-process RAM filesystem for host profile transactions.
//! Immediate futures here model RAM, not asynchronous device I/O.
use ruzina_media_fs::*;
use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
fn lease() -> MediaLease {
    MediaLease {
        generation: MediaGeneration::initial(),
        source: MediaSourceId::new(3).unwrap(),
    }
}
#[derive(Clone)]
pub struct RamProfiles {
    pub files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    pub mutation_budget: Option<usize>,
}
impl RamProfiles {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            dirs: BTreeSet::new(),
            mutation_budget: None,
        }
    }
    fn mutate(&mut self) -> Result<(), ()> {
        if let Some(left) = &mut self.mutation_budget {
            if *left == 0 {
                return Err(());
            }
            *left -= 1;
        }
        Ok(())
    }
}
pub struct File {
    path: String,
    len: u64,
    position: u64,
}
impl LeaseBound for RamProfiles {
    fn lease(&self) -> MediaLease {
        lease()
    }
}
impl LeaseBound for File {
    fn lease(&self) -> MediaLease {
        lease()
    }
}
impl FileHandle for File {
    fn len(&self) -> u64 {
        self.len
    }
    fn position(&self) -> u64 {
        self.position
    }
    fn set_position(&mut self, p: u64) {
        self.position = p;
    }
}
pub struct Directory {
    entries: Vec<String>,
    cursor: usize,
}
impl LeaseBound for Directory {
    fn lease(&self) -> MediaLease {
        lease()
    }
}
impl DirectoryHandle for Directory {}
impl AsyncFileSystem for RamProfiles {
    type Error = ();
    type File = File;
    type Directory = Directory;
    fn kind(&self) -> FileSystemKind {
        FileSystemKind::Fat32
    }
    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities {
            write: true,
            flush: true,
            ..FileSystemCapabilities::READ_ONLY
        }
    }
    async fn stat(&mut self, path: &str) -> Result<FileStat, ()> {
        let (kind, len) = if let Some(bytes) = self.files.get(path) {
            (EntryKind::File, bytes.len() as u64)
        } else if self.dirs.contains(path) {
            (EntryKind::Directory, 0)
        } else {
            return Err(());
        };
        Ok(FileStat {
            kind,
            len,
            modified_unix_seconds: None,
        })
    }
    async fn open_read(&mut self, path: &str) -> Result<File, ()> {
        Ok(File {
            path: path.into(),
            len: self.files.get(path).ok_or(())?.len() as u64,
            position: 0,
        })
    }
    async fn close_file(&mut self, _: File) -> Result<(), ()> {
        Ok(())
    }
    async fn read(&mut self, file: &mut File, out: &mut [u8]) -> Result<usize, ()> {
        let bytes = self.files.get(&file.path).ok_or(())?;
        let start = (file.position as usize).min(bytes.len());
        let n = out.len().min(bytes.len() - start);
        out[..n].copy_from_slice(&bytes[start..start + n]);
        file.position += n as u64;
        Ok(n)
    }
    async fn seek_absolute(&mut self, file: &mut File, p: u64) -> Result<(), ()> {
        file.position = p;
        Ok(())
    }
    async fn open_directory(&mut self, path: &str) -> Result<Directory, ()> {
        if !self.dirs.contains(path) {
            return Err(());
        }
        let prefix = format!("{path}/");
        Ok(Directory {
            entries: self
                .dirs
                .iter()
                .filter_map(|s| s.strip_prefix(&prefix))
                .filter(|s| !s.contains('/'))
                .map(String::from)
                .collect(),
            cursor: 0,
        })
    }
    async fn next_entry(
        &mut self,
        d: &mut Directory,
        names: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, ()> {
        let Some(name) = d.entries.get(d.cursor) else {
            return Ok(None);
        };
        d.cursor += 1;
        names[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Some(DirEntry {
            kind: EntryKind::Directory,
            len: 0,
            modified_unix_seconds: None,
            name_len: name.len() as u16,
        }))
    }
    async fn close_directory(&mut self, _: Directory) -> Result<(), ()> {
        Ok(())
    }
}
impl AsyncWritableFileSystem for RamProfiles {
    async fn open_write(&mut self, path: &str, truncate: bool) -> Result<File, ()> {
        self.mutate()?;
        let bytes = self.files.entry(path.into()).or_default();
        if truncate {
            bytes.clear();
        }
        Ok(File {
            path: path.into(),
            len: bytes.len() as u64,
            position: 0,
        })
    }
    async fn write(&mut self, file: &mut File, input: &[u8]) -> Result<usize, ()> {
        self.mutate()?;
        let bytes = self.files.get_mut(&file.path).ok_or(())?;
        let end = file.position as usize + input.len();
        bytes.resize(end.max(bytes.len()), 0);
        bytes[file.position as usize..end].copy_from_slice(input);
        file.position = end as u64;
        file.len = bytes.len() as u64;
        Ok(input.len())
    }
    async fn flush_file(&mut self, _: &mut File) -> Result<(), ()> {
        self.mutate()
    }
    async fn rename(&mut self, from: &str, to: &str) -> Result<(), ()> {
        self.mutate()?;
        let bytes = self.files.remove(from).ok_or(())?;
        self.files.insert(to.into(), bytes);
        Ok(())
    }
    async fn remove(&mut self, path: &str) -> Result<(), ()> {
        self.mutate()?;
        self.files.remove(path).ok_or(())?;
        Ok(())
    }
    async fn sync(&mut self) -> Result<(), ()> {
        self.mutate()
    }
    async fn create_directory(&mut self, path: &str) -> Result<(), ()> {
        self.mutate()?;
        self.dirs.insert(path.into());
        Ok(())
    }
}
/// All RamProfiles futures must be immediately ready. Never use this with a
/// device backend; a real adapter must await its own owner/executor.
pub fn ready<F: Future>(future: F) -> F::Output {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("RAM future unexpectedly pending"),
    }
}
