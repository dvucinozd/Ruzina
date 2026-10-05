//! Host fixture only: synchronous std file operations in immediately-ready
//! futures. Never use this implementation in an Embassy task.
use pajoniiir_media_fs::*;
use pajoniiir_media_session::MediaLease;
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
};
pub struct HostFs {
    pub path: PathBuf,
    pub lease: MediaLease,
}
pub struct Handle {
    file: File,
    lease: MediaLease,
    len: u64,
    pos: u64,
}
impl LeaseBound for HostFs {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl LeaseBound for Handle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl FileHandle for Handle {
    fn len(&self) -> u64 {
        self.len
    }
    fn position(&self) -> u64 {
        self.pos
    }
    fn set_position(&mut self, p: u64) {
        self.pos = p;
    }
}
impl DirectoryHandle for Handle {}
fn unsupported<T>() -> io::Result<T> {
    Err(io::ErrorKind::Unsupported.into())
}
impl AsyncFileSystem for HostFs {
    type Error = io::Error;
    type File = Handle;
    type Directory = Handle;
    fn kind(&self) -> FileSystemKind {
        FileSystemKind::Fat32
    }
    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities::READ_ONLY
    }
    async fn stat(&mut self, _: &str) -> io::Result<FileStat> {
        unsupported()
    }
    async fn open_read(&mut self, path: &str) -> io::Result<Handle> {
        if path != "/TRACK.WAV" {
            return Err(io::ErrorKind::NotFound.into());
        }
        let file = File::open(&self.path)?;
        let len = file.metadata()?.len();
        Ok(Handle {
            file,
            lease: self.lease,
            len,
            pos: 0,
        })
    }
    async fn close_file(&mut self, file: Handle) -> io::Result<()> {
        drop(file);
        Ok(())
    }
    async fn read(&mut self, file: &mut Handle, out: &mut [u8]) -> io::Result<usize> {
        let n = file.file.read(out)?;
        file.pos += n as u64;
        Ok(n)
    }
    async fn seek_absolute(&mut self, _: &mut Handle, _: u64) -> io::Result<()> {
        unsupported()
    }
    async fn open_directory(&mut self, _: &str) -> io::Result<Handle> {
        unsupported()
    }
    async fn next_entry(
        &mut self,
        _: &mut Handle,
        _: &mut [u8; FS_NAME_MAX],
    ) -> io::Result<Option<DirEntry>> {
        unsupported()
    }
    async fn close_directory(&mut self, _: Handle) -> io::Result<()> {
        unsupported()
    }
}
// Host-only blocking driver; a wake before park preserves its notification.
pub fn drive<F: core::future::Future>(f: F) -> F::Output {
    struct WakeThread(std::thread::Thread);
    impl std::task::Wake for WakeThread {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(std::sync::Arc::new(WakeThread(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut f = core::pin::pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            core::task::Poll::Ready(value) => return value,
            core::task::Poll::Pending => std::thread::park(),
        }
    }
}
