//! Single-file analysis bridge to the existing owned-buffer filesystem broker.
//! Cancellation never cancels a submitted operation: `recover` drains it and
//! closes the recovered handle before another job can start. No async Drop.
use crate::*;

// Match the existing fixed-capacity broker path; no heap allocation on firmware.
#[allow(clippy::large_enum_variant)]
pub enum Request<H> {
    Open(FileSystemRequestTicket, BoundedPath),
    Read(FileSystemRequestTicket, H, &'static mut [u8], usize),
    Close(FileSystemRequestTicket, H),
}
pub enum Completion<H, E, U = ()> {
    Unexpected(FileSystemRequestTicket, U),
    Open(FileSystemRequestTicket, Result<H, E>),
    Read(
        FileSystemRequestTicket,
        H,
        &'static mut [u8],
        Result<usize, E>,
    ),
    Close(FileSystemRequestTicket, Result<(), E>),
}
impl<H, E, U> Completion<H, E, U> {
    pub fn ticket(&self) -> FileSystemRequestTicket {
        match self {
            Self::Open(t, _) | Self::Read(t, ..) | Self::Close(t, _) | Self::Unexpected(t, _) => *t,
        }
    }
}
/// An exclusive endpoint. Implementations must preserve owned messages across
/// cancelled receives and route only this client's completions to this endpoint.
#[allow(async_fn_in_trait)]
pub trait Transport {
    type Handle: FileHandle;
    type Error;
    type Unexpected;
    fn valid(&self, original: MediaLease) -> bool;
    fn kind(&self) -> FileSystemKind;
    #[allow(clippy::result_large_err)] // Queue refusal must return all owned resources.
    fn try_send(&mut self, request: Request<Self::Handle>) -> Result<(), Request<Self::Handle>>;
    async fn receive(&mut self) -> Completion<Self::Handle, Self::Error, Self::Unexpected>;
}
#[derive(Debug, PartialEq)]
pub enum Error<E> {
    Backend(E),
    Busy,
    Stale,
    QueueFull,
    Unexpected,
    Contract,
    Unsupported,
    Exhausted,
}
/// Non-cloneable logical file. The actual broker handle stays in the client.
pub struct File {
    lease: MediaLease,
    id: u32,
    len: u64,
    position: u64,
}
impl LeaseBound for File {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl FileHandle for File {
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
pub enum NoDirectory {}
impl LeaseBound for NoDirectory {
    fn lease(&self) -> MediaLease {
        match *self {}
    }
}
impl DirectoryHandle for NoDirectory {}
#[derive(Clone, Copy, PartialEq)]
enum Operation {
    Open,
    Read,
    Close,
}

/// Own one endpoint for the whole analysis-owner lifetime, including repeated
/// jobs and recovery. Keep this object outside cancellable job futures.
pub struct Client<T: Transport> {
    transport: T,
    lease: MediaLease,
    buffer: Option<&'static mut [u8]>,
    handle: Option<T::Handle>,
    pending: Option<(FileSystemRequestTicket, Operation)>,
    unexpected: Option<Completion<T::Handle, T::Error, T::Unexpected>>,
    sequence: u32,
    file_id: u32,
    poisoned: bool,
}
impl<T: Transport> Client<T> {
    pub fn new(transport: T, lease: MediaLease, buffer: &'static mut [u8]) -> Self {
        Self {
            transport,
            lease,
            buffer: Some(buffer),
            handle: None,
            pending: None,
            unexpected: None,
            sequence: 0,
            file_id: 0,
            poisoned: false,
        }
    }
    pub fn is_idle(&self) -> bool {
        self.pending.is_none()
            && self.handle.is_none()
            && self.unexpected.is_none()
            && self.buffer.is_some()
            && !self.poisoned
    }
    /// Change media only after all operations, handles and quarantine are drained.
    /// Ticket and logical-file generations remain monotonic across replacements.
    pub fn bind(&mut self, lease: MediaLease) -> Result<(), Error<T::Error>> {
        if !self.is_idle() {
            return Err(Error::Busy);
        }
        if !self.transport.valid(lease) {
            return Err(Error::Stale);
        }
        self.lease = lease;
        Ok(())
    }
    /// Retire this client after recovery. The endpoint is never returned/reissued.
    pub fn retire_buffer(&mut self) -> Option<&'static mut [u8]> {
        if self.is_idle() {
            self.buffer.take()
        } else {
            None
        }
    }
    pub fn pending_ticket(&self) -> Option<FileSystemRequestTicket> {
        self.pending.map(|p| p.0)
    }
    /// Quarantined resources remain owned until explicitly taken for broker-owner
    /// investigation/cleanup. Taking them does not clear the original pending ticket.
    pub fn take_unexpected(&mut self) -> Option<Completion<T::Handle, T::Error, T::Unexpected>> {
        self.unexpected.take()
    }
    fn ticket(&self) -> Result<FileSystemRequestTicket, Error<T::Error>> {
        let sequence = self.sequence.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(FileSystemRequestTicket {
            client: FileSystemClientId::Analysis,
            sequence,
            lease: self.lease,
        })
    }
    fn submit(
        &mut self,
        request: Request<T::Handle>,
        ticket: FileSystemRequestTicket,
        op: Operation,
    ) -> Result<(), Error<T::Error>> {
        match self.transport.try_send(request) {
            Ok(()) => {
                self.sequence = ticket.sequence();
                self.pending = Some((ticket, op));
                Ok(())
            }
            Err(request) => {
                match request {
                    Request::Open(..) => {}
                    Request::Read(_, h, b, _) => {
                        self.handle = Some(h);
                        self.buffer = Some(b);
                    }
                    Request::Close(_, h) => self.handle = Some(h),
                }
                Err(Error::QueueFull)
            }
        }
    }
    async fn complete(&mut self) -> Result<usize, Error<T::Error>> {
        if self.unexpected.is_some() {
            return Err(Error::Unexpected);
        }
        let (ticket, op) = self.pending.ok_or(Error::Contract)?;
        let completion = self.transport.receive().await;
        let matches_kind = matches!(
            (&completion, op),
            (Completion::Open(..), Operation::Open)
                | (Completion::Read(..), Operation::Read)
                | (Completion::Close(..), Operation::Close)
        );
        if completion.ticket() != ticket || !matches_kind {
            self.unexpected = Some(completion);
            return Err(Error::Unexpected);
        }
        self.pending = None;
        match completion {
            Completion::Unexpected(..) => unreachable!(),
            Completion::Open(_, result) => {
                self.handle = Some(result.map_err(Error::Backend)?);
                Ok(0)
            }
            Completion::Read(_, h, b, result) => {
                let capacity = b.len();
                self.handle = Some(h);
                self.buffer = Some(b);
                let count = result.map_err(Error::Backend)?;
                if count > capacity {
                    return Err(Error::Contract);
                }
                Ok(count)
            }
            Completion::Close(_, result) => {
                result.map_err(Error::Backend)?;
                Ok(0)
            }
        }
    }
    /// Explicit cancellation/recovery. May itself be cancelled and resumed.
    /// Errors preserve recovered resources; call again to finish cleanup. Stale
    /// media must still drain owned completions, even though delivery is forbidden.
    pub async fn recover(&mut self) -> Result<(), Error<T::Error>> {
        self.poisoned = true;
        if self.unexpected.is_some() {
            return Err(Error::Unexpected);
        }
        if self.pending.is_some() {
            self.complete().await?;
        }
        if self.handle.is_some() {
            let ticket = self.ticket()?;
            let h = self.handle.take().unwrap();
            self.submit(Request::Close(ticket, h), ticket, Operation::Close)?;
            self.complete().await?;
        }
        if self.buffer.is_none() {
            return Err(Error::Contract);
        }
        self.poisoned = false;
        Ok(())
    }
    fn valid_file(&self, file: &File) -> bool {
        file.lease == self.lease
            && file.id == self.file_id
            && self.handle.as_ref().is_some_and(|h| {
                h.lease() == self.lease && h.position() == file.position && h.len() == file.len
            })
    }
}
impl<T: Transport> LeaseBound for Client<T> {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl<T: Transport> AsyncFileSystem for Client<T> {
    type Error = Error<T::Error>;
    type File = File;
    type Directory = NoDirectory;
    fn kind(&self) -> FileSystemKind {
        self.transport.kind()
    }
    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities {
            directories: false,
            ..FileSystemCapabilities::READ_ONLY
        }
    }
    async fn open_read(&mut self, path: &str) -> Result<File, Self::Error> {
        if !self.is_idle() {
            return Err(Error::Busy);
        }
        if !self.transport.valid(self.lease) {
            return Err(Error::Stale);
        }
        if self.sequence >= u32::MAX - 1 {
            return Err(Error::Exhausted);
        }
        let path = BoundedPath::new(path).map_err(|_| Error::Contract)?;
        let id = self.file_id.checked_add(1).ok_or(Error::Exhausted)?;
        let ticket = self.ticket()?;
        self.poisoned = true;
        self.file_id = id;
        self.submit(Request::Open(ticket, path), ticket, Operation::Open)?;
        self.complete().await?;
        if !self.transport.valid(self.lease) {
            return Err(Error::Stale);
        }
        let h = self.handle.as_ref().ok_or(Error::Contract)?;
        if h.lease() != self.lease || h.position() != 0 {
            return Err(Error::Contract);
        }
        self.poisoned = false;
        Ok(File {
            lease: self.lease,
            id,
            len: h.len(),
            position: 0,
        })
    }
    async fn read(&mut self, file: &mut File, output: &mut [u8]) -> Result<usize, Self::Error> {
        if self.poisoned || self.pending.is_some() {
            return Err(Error::Busy);
        }
        if !self.valid_file(file) {
            return Err(Error::Contract);
        }
        if !self.transport.valid(self.lease) {
            return Err(Error::Stale);
        }
        if output.is_empty() {
            return Ok(0);
        }
        if self.sequence >= u32::MAX - 1 {
            return Err(Error::Exhausted);
        }
        let size = output
            .len()
            .min(self.buffer.as_ref().map_or(0, |b| b.len()));
        if size == 0 {
            return Err(Error::Contract);
        }
        let ticket = self.ticket()?;
        self.poisoned = true;
        let h = self.handle.take().unwrap();
        let b = self.buffer.take().unwrap();
        self.submit(Request::Read(ticket, h, b, size), ticket, Operation::Read)?;
        let count = self.complete().await?;
        if count > size {
            return Err(Error::Contract);
        }
        if !self.transport.valid(self.lease) {
            return Err(Error::Stale);
        }
        let h = self.handle.as_ref().unwrap();
        if h.lease() != self.lease
            || h.len() != file.len
            || file.position.checked_add(count as u64) != Some(h.position())
            || h.position() > h.len()
        {
            return Err(Error::Contract);
        }
        output[..count].copy_from_slice(&self.buffer.as_ref().unwrap()[..count]);
        file.position = h.position();
        self.poisoned = false;
        Ok(count)
    }
    async fn close_file(&mut self, file: File) -> Result<(), Self::Error> {
        if file.lease != self.lease || file.id != self.file_id {
            return Err(Error::Contract);
        }
        // A cancelled read may have advanced the broker handle. Closing this
        // logical file deliberately uses recovered ownership, not its old cursor.
        self.recover().await?;
        if !self.transport.valid(self.lease) {
            return Err(Error::Stale);
        }
        Ok(())
    }
    async fn stat(&mut self, _: &str) -> Result<FileStat, Self::Error> {
        Err(Error::Unsupported)
    }
    async fn seek_absolute(&mut self, _: &mut File, _: u64) -> Result<(), Self::Error> {
        Err(Error::Unsupported)
    }
    async fn open_directory(&mut self, _: &str) -> Result<NoDirectory, Self::Error> {
        Err(Error::Unsupported)
    }
    async fn next_entry(
        &mut self,
        d: &mut NoDirectory,
        _: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Self::Error> {
        match *d {}
    }
    async fn close_directory(&mut self, d: NoDirectory) -> Result<(), Self::Error> {
        match d {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    struct Endpoint {
        full: bool,
        completion: Option<Completion<File, ()>>,
    }
    impl Transport for Endpoint {
        type Handle = File;
        type Error = ();
        type Unexpected = ();
        fn valid(&self, _: MediaLease) -> bool {
            true
        }
        fn kind(&self) -> FileSystemKind {
            FileSystemKind::Fat32
        }
        fn try_send(&mut self, r: Request<File>) -> Result<(), Request<File>> {
            if self.full {
                return Err(r);
            }
            self.completion = Some(match r {
                Request::Open(t, _) => Completion::Open(
                    t,
                    Ok(File {
                        lease: t.lease(),
                        id: 0,
                        len: 1,
                        position: 0,
                    }),
                ),
                Request::Close(t, _) => Completion::Close(t, Ok(())),
                Request::Read(..) => panic!("read must preserve the last cleanup ticket"),
            });
            Ok(())
        }
        async fn receive(&mut self) -> Completion<File, ()> {
            self.completion.take().unwrap()
        }
    }
    fn run<F: Future>(f: F) -> F::Output {
        match pin!(f)
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(r) => r,
            Poll::Pending => panic!("unexpected pending"),
        }
    }
    #[test]
    fn queue_refusal_does_not_consume_the_final_cleanup_ticket() {
        let mut session = pajoniiir_media_session::MediaSession::new();
        session.on_connect(pajoniiir_media_session::MediaSourceId::new(1).unwrap());
        let buffer = std::boxed::Box::leak(std::boxed::Box::new([]));
        let mut client = Client::new(
            Endpoint {
                full: false,
                completion: None,
            },
            session.lease().unwrap(),
            buffer,
        );
        client.sequence = u32::MAX - 2;
        let mut file = run(client.open_read("/TRACK.WAV")).unwrap();
        assert!(matches!(
            run(client.read(&mut file, &mut [0; 1])),
            Err(Error::Exhausted)
        ));
        assert_eq!(client.sequence, u32::MAX - 1);
        client.transport.full = true;
        assert!(matches!(
            run(client.close_file(file)),
            Err(Error::QueueFull)
        ));
        assert_eq!(client.sequence, u32::MAX - 1);
        assert!(client.handle.is_some());
        client.transport.full = false;
        run(client.recover()).unwrap();
        assert!(client.is_idle());
        assert_eq!(client.sequence, u32::MAX);
        assert!(matches!(
            run(client.open_read("/TRACK.WAV")),
            Err(Error::Exhausted)
        ));
    }
}
