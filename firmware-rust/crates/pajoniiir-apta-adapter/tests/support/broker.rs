//! Host owner for the real broker messages; native filesystem engines unchanged.
use super::*;
use pajoniiir_media_fs::analysis::{Completion, Request, Transport};
use std::{cell::Cell, rc::Rc};
#[derive(Default)]
pub struct Control {
    pub pause: Cell<bool>,
    pub yield_receive: Cell<bool>,
    pub stale_on_receive: Cell<bool>,
    pub wrong_ticket: Cell<bool>,
    pub stale: Cell<bool>,
    pub full: Cell<bool>,
    pub unexpected: Cell<bool>,
    pub closes: Cell<usize>,
    pub reads: Cell<usize>,
}
pub struct Broker<F: AsyncFileSystem> {
    fs: F,
    control: Rc<Control>,
    completion: Option<Completion<F::File, F::Error>>,
}
impl<F: AsyncFileSystem> Broker<F> {
    pub fn new(fs: F, control: Rc<Control>) -> Self {
        Self {
            fs,
            control,
            completion: None,
        }
    }
}
impl<F: AsyncFileSystem> Transport for Broker<F> {
    type Handle = F::File;
    type Error = F::Error;
    type Unexpected = ();
    fn valid(&self, lease: MediaLease) -> bool {
        !self.control.stale.get() && self.fs.lease() == lease
    }
    fn kind(&self) -> FileSystemKind {
        self.fs.kind()
    }
    fn try_send(&mut self, request: Request<F::File>) -> Result<(), Request<F::File>> {
        if self.control.full.get() {
            return Err(request);
        }
        assert!(self.completion.is_none());
        self.completion = Some(match request {
            Request::Open(t, p) => Completion::Open(t, run(self.fs.open_read(p.as_str()))),
            Request::Read(t, mut h, b, n) => {
                self.control.reads.set(self.control.reads.get() + 1);
                let result = run(self.fs.read(&mut h, &mut b[..n]));
                Completion::Read(t, h, b, result)
            }
            Request::Close(t, h) => {
                self.control.closes.set(self.control.closes.get() + 1);
                Completion::Close(t, run(self.fs.close_file(h)))
            }
        });
        Ok(())
    }
    async fn receive(&mut self) -> Completion<F::File, F::Error> {
        let mut first = self.control.yield_receive.get();
        core::future::poll_fn(|cx| {
            if first {
                first = false;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            if self.control.pause.get() {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        if self.control.stale_on_receive.replace(false) {
            self.control.stale.set(true);
        }
        if self.control.wrong_ticket.replace(false) {
            let t = FileSystemRequestSequencer::new()
                .issue(FileSystemClientId::Library, self.fs.lease());
            return Completion::Unexpected(t, ());
        }
        if self.control.unexpected.replace(false) {
            // A wrong operation with the right ticket must not clear pending.
            return Completion::Unexpected(self.completion.as_ref().unwrap().ticket(), ());
        }
        self.completion.take().unwrap()
    }
}

/// SAFETY: only buffers created by Box::leak(Box::new([0; 4096])) in these
/// tests reach this helper, after idle retirement proves no broker borrow remains.
pub fn free_buffer(buffer: &'static mut [u8]) {
    assert_eq!(buffer.len(), 4096);
    unsafe {
        drop(Box::from_raw(buffer as *mut [u8]));
    }
}
