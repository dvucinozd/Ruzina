use super::*;
use core::future::Future;
use core::pin::Pin;
use core::task::Waker;
use std::boxed::Box;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::Wake;
use std::vec;
use std::vec::Vec;

#[derive(Debug)]
struct Buffer {
    bytes: Vec<u8>,
    id: u32,
    drops: Rc<Cell<usize>>,
}
impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
fn buffer(id: u32, bytes: usize) -> Buffer {
    Buffer {
        bytes: vec![0; bytes],
        id,
        drops: Rc::default(),
    }
}
fn geometry() -> BlockGeometry {
    BlockGeometry {
        block_size: 512,
        block_count: 100,
    }
}
fn source() -> MediaSourceId {
    MediaSourceId::new(0x5344_0001).unwrap()
}
fn handle() -> MediaHandle {
    MediaHandle::new(1).unwrap()
}
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
fn ready<F: Future>(future: F) -> F::Output {
    match poll(Box::pin(future).as_mut()) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending"),
    }
}

struct State {
    queue: ReadQueue<Buffer>,
    waker: Option<Waker>,
}
#[derive(Clone)]
struct Port(Rc<RefCell<State>>);
impl ReadPort for Port {
    type Buffer = Buffer;
    fn is_current(&self, binding: Binding) -> bool {
        self.0.borrow().queue.is_current(binding)
    }
    fn submit(
        &mut self,
        binding: Binding,
        first: u64,
        count: u32,
        buffer: Buffer,
    ) -> Result<Ticket, (Error, Buffer)> {
        self.0
            .borrow_mut()
            .queue
            .submit(binding, first, count, buffer)
    }
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Buffer>> {
        let mut state = self.0.borrow_mut();
        if let Some(result) = state.queue.take_completion(ticket) {
            return Poll::Ready(result);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
fn setup() -> (Port, Binding, ReadClient<Port>) {
    let mut queue = ReadQueue::new();
    let binding = queue.activate(source(), handle(), geometry()).unwrap();
    let port = Port(Rc::new(RefCell::new(State { queue, waker: None })));
    let client = ReadClient::new(port.clone(), binding);
    (port, binding, client)
}
fn complete(port: &Port, request: ReadRequest<Buffer>, result: Result<usize, i32>) {
    let waker = {
        let mut state = port.0.borrow_mut();
        assert!(state.queue.complete(request, result).is_ok());
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

#[test]
fn success_returns_the_same_owned_allocation() {
    let (port, _, mut client) = setup();
    let input = buffer(7, 1024);
    let pointer = input.bytes.as_ptr();
    let drops = input.drops.clone();
    let mut future = Box::pin(client.read_blocks_owned(9, 2, input));
    assert!(poll(future.as_mut()).is_pending());
    let mut request = port.0.borrow_mut().queue.take_request().unwrap();
    assert_eq!((request.first_block(), request.block_count()), (9, 2));
    request.buffer_mut().fill(0x5a);
    complete(&port, request, Ok(1024));
    let Poll::Ready(Ok(output)) = poll(future.as_mut()) else {
        panic!("not complete")
    };
    drop(future);
    assert!(!client.has_pending());
    assert_eq!((output.id, output.bytes.as_ptr()), (7, pointer));
    assert!(output.bytes.iter().all(|&value| value == 0x5a));
    assert_eq!(drops.get(), 0);
    drop(output);
    assert_eq!(drops.get(), 1);
}

#[test]
fn invalid_transfers_never_reach_the_worker() {
    let (port, _, mut client) = setup();
    for (first, count, size, expected) in [
        (0, 0, 512, Error::Transfer(TransferError::ZeroBlocks)),
        (
            0,
            1,
            511,
            Error::Transfer(TransferError::InvalidBufferLength),
        ),
        (99, 2, 1024, Error::Transfer(TransferError::OutOfRange)),
        (u64::MAX, 1, 512, Error::Transfer(TransferError::OutOfRange)),
        (0, 9, 4608, Error::TooLarge),
    ] {
        let input = buffer(4, size);
        let pointer = input.bytes.as_ptr();
        let (error, returned) = ready(client.read_blocks_owned(first, count, input)).unwrap_err();
        assert_eq!(error, expected);
        assert_eq!(returned.bytes.as_ptr(), pointer);
        assert!(port.0.borrow_mut().queue.take_request().is_none());
    }
}

#[test]
fn removal_before_submission_rejects_the_old_lease() {
    let (port, _, mut client) = setup();
    port.0.borrow_mut().queue.invalidate();
    let (error, returned) = ready(client.read_blocks_owned(0, 1, buffer(12, 512))).unwrap_err();
    assert_eq!((error, returned.id), (Error::Stale, 12));
    assert!(port.0.borrow_mut().queue.take_request().is_none());
    assert!(port.0.borrow().queue.can_deinitialize());
}

#[test]
fn queued_removal_returns_buffer_without_starting_dma() {
    let (port, _, mut client) = setup();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(13, 512)));
    assert!(poll(future.as_mut()).is_pending());
    port.0.borrow_mut().queue.invalidate();
    assert!(port.0.borrow_mut().queue.take_request().is_none());
    let Poll::Ready(Err((Error::Stale, returned))) = poll(future.as_mut()) else {
        panic!("not stale")
    };
    assert_eq!(returned.id, 13);
}

#[test]
fn active_removal_fences_late_success_and_delays_teardown() {
    let (port, _, mut client) = setup();
    let input = buffer(15, 512);
    let drops = input.drops.clone();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, input));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    port.0.borrow_mut().queue.invalidate();
    assert!(!port.0.borrow().queue.can_deinitialize());
    assert_eq!(drops.get(), 0);
    assert!(poll(future.as_mut()).is_pending());
    complete(&port, request, Ok(512));
    assert!(port.0.borrow().queue.can_deinitialize());
    let Poll::Ready(Err((Error::Stale, returned))) = poll(future.as_mut()) else {
        panic!("accepted stale success")
    };
    assert_eq!(returned.id, 15);
}

#[test]
fn removal_after_completion_before_delivery_also_rejects_success() {
    let (port, _, mut client) = setup();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(16, 512)));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    complete(&port, request, Ok(512));
    port.0.borrow_mut().queue.invalidate();
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Err((Error::Stale, _)))
    ));
}

#[test]
fn cancel_active_then_recover_preserves_owner_buffer_and_blocks_new_read() {
    let (port, _, mut client) = setup();
    let input = buffer(20, 512);
    let drops = input.drops.clone();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, input));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    drop(future);
    assert!(client.has_pending());
    assert_eq!(drops.get(), 0);
    let (error, returned) = ready(client.read_blocks_owned(0, 1, buffer(21, 512))).unwrap_err();
    assert_eq!((error, returned.id), (Error::Busy, 21));
    let mut recovery = Box::pin(client.recover_pending());
    assert!(poll(recovery.as_mut()).is_pending());
    drop(recovery); // Canceling recovery also keeps the pending ticket.
    complete(&port, request, Ok(512));
    let returned = ready(client.recover_pending()).unwrap().unwrap();
    assert_eq!(returned.id, 20);
    assert!(!client.has_pending());
    assert!(ready(client.recover_pending()).is_none());
}

#[test]
fn cancel_queued_then_invalidate_can_be_drained() {
    let (port, _, mut client) = setup();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(22, 512)));
    assert!(poll(future.as_mut()).is_pending());
    drop(future);
    port.0.borrow_mut().queue.invalidate();
    let (error, returned) = ready(client.recover_pending()).unwrap().unwrap_err();
    assert_eq!((error, returned.id), (Error::Stale, 22));
}

#[test]
fn dropping_unpolled_future_never_submits() {
    let (port, _, mut client) = setup();
    let input = buffer(23, 512);
    let drops = input.drops.clone();
    drop(client.read_blocks_owned(0, 1, input));
    assert_eq!(drops.get(), 1);
    assert!(!client.has_pending());
    assert!(port.0.borrow_mut().queue.take_request().is_none());
}

#[test]
fn dropping_client_does_not_free_an_active_worker_buffer() {
    let (port, _, mut client) = setup();
    let input = buffer(24, 512);
    let drops = input.drops.clone();
    let mut future = Box::pin(client.read_blocks_owned(0, 1, input));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    let ticket = request.ticket();
    drop(future);
    drop(client);
    assert_eq!(drops.get(), 0);
    port.0.borrow_mut().queue.invalidate();
    complete(&port, request, Ok(512));
    let result = port.0.borrow_mut().queue.take_completion(ticket).unwrap();
    assert!(matches!(result, Err((Error::Stale, _))));
    drop(result);
    assert_eq!(drops.get(), 1);
}

#[test]
fn remount_waits_for_drain_and_never_revives_an_old_client() {
    let (port, old, mut old_client) = setup();
    let mut future = Box::pin(old_client.read_blocks_owned(0, 1, buffer(25, 512)));
    assert!(poll(future.as_mut()).is_pending());
    drop(future);
    port.0.borrow_mut().queue.invalidate();
    assert_eq!(
        port.0
            .borrow_mut()
            .queue
            .activate(source(), handle(), geometry()),
        Err(Error::Busy)
    );
    assert!(ready(old_client.recover_pending()).unwrap().is_err());
    let fresh = port
        .0
        .borrow_mut()
        .queue
        .activate(source(), handle(), geometry())
        .unwrap();
    assert_ne!(fresh.lease.generation, old.lease.generation);
    let (error, _) = ready(old_client.read_blocks_owned(0, 1, buffer(26, 512))).unwrap_err();
    assert_eq!(error, Error::Stale);
    let mut fresh_client = ReadClient::new(port.clone(), fresh);
    let mut future = Box::pin(fresh_client.read_blocks_owned(99, 1, buffer(27, 512)));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    complete(&port, request, Ok(512));
    assert!(matches!(poll(future.as_mut()), Poll::Ready(Ok(_))));
}

#[test]
fn io_errors_and_short_reads_return_the_original_buffer() {
    for (result, expected) in [
        (Err(0x107), Error::Device(0x107)),
        (
            Ok(511),
            Error::ShortRead {
                expected: 512,
                actual: 511,
            },
        ),
        (
            Ok(513),
            Error::ShortRead {
                expected: 512,
                actual: 513,
            },
        ),
    ] {
        let (port, _, mut client) = setup();
        let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(30, 512)));
        assert!(poll(future.as_mut()).is_pending());
        let request = port.0.borrow_mut().queue.take_request().unwrap();
        complete(&port, request, result);
        let Poll::Ready(Err((error, returned))) = poll(future.as_mut()) else {
            panic!("not failed")
        };
        assert_eq!((error, returned.id), (expected, 30));
    }
}

#[test]
fn wrong_ticket_cannot_consume_or_overwrite_a_completion() {
    let mut a = ReadQueue::new();
    let mut b = ReadQueue::new();
    let a_binding = a.activate(source(), handle(), geometry()).unwrap();
    let b_binding = b
        .activate(source(), MediaHandle::new(2).unwrap(), geometry())
        .unwrap();
    let a_ticket = a.submit(a_binding, 0, 1, buffer(31, 512)).unwrap();
    let b_ticket = b.submit(b_binding, 0, 1, buffer(32, 512)).unwrap();
    let a_request = a.take_request().unwrap();
    let b_request = b.take_request().unwrap();
    let a_request = b.complete(a_request, Ok(512)).err().unwrap();
    assert!(a.complete(a_request, Ok(512)).is_ok());
    assert!(b.complete(b_request, Ok(512)).is_ok());
    assert!(a.take_completion(b_ticket).is_none());
    assert_eq!(a.take_completion(a_ticket).unwrap().unwrap().id, 31);
    assert_eq!(b.take_completion(b_ticket).unwrap().unwrap().id, 32);
}

#[test]
fn slot_backpressure_preserves_both_buffers() {
    let (port, binding, _) = setup();
    let mut state = port.0.borrow_mut();
    let ticket = state.queue.submit(binding, 0, 1, buffer(33, 512)).unwrap();
    let (error, returned) = state
        .queue
        .submit(binding, 1, 1, buffer(34, 512))
        .unwrap_err();
    assert_eq!((error, returned.id), (Error::Busy, 34));
    let request = state.queue.take_request().unwrap();
    assert!(state.queue.complete(request, Ok(512)).is_ok());
    assert_eq!(state.queue.take_completion(ticket).unwrap().unwrap().id, 33);
}

#[test]
fn unsupported_geometry_and_sequence_wrap_are_rejected() {
    let mut queue = ReadQueue::new();
    for geometry in [
        BlockGeometry {
            block_size: 1024,
            block_count: 10,
        },
        BlockGeometry {
            block_size: 512,
            block_count: 0,
        },
        BlockGeometry {
            block_size: 512,
            block_count: u32::MAX as u64 + 1,
        },
    ] {
        assert_eq!(
            queue.activate(source(), handle(), geometry),
            Err(Error::UnsupportedGeometry)
        );
    }
    let binding = queue.activate(source(), handle(), geometry()).unwrap();
    queue.sequence = u64::MAX;
    let (error, returned) = queue.submit(binding, 0, 1, buffer(35, 512)).unwrap_err();
    assert_eq!((error, returned.id), (Error::SequenceExhausted, 35));
}

struct Counter(AtomicUsize);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn completion_wakes_the_registered_waiter() {
    let (port, _, mut client) = setup();
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(36, 512)));
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    complete(&port, request, Ok(512));
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    assert!(poll(future.as_mut()).is_ready());
}

struct DeliveryRace(Port);
impl ReadPort for DeliveryRace {
    type Buffer = Buffer;
    fn is_current(&self, binding: Binding) -> bool {
        self.0.is_current(binding)
    }
    fn submit(
        &mut self,
        binding: Binding,
        first: u64,
        count: u32,
        buffer: Buffer,
    ) -> Result<Ticket, (Error, Buffer)> {
        self.0.submit(binding, first, count, buffer)
    }
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Buffer>> {
        let completion = self.0.poll_completion(ticket, cx);
        if completion.is_ready() {
            self.0.0.borrow_mut().queue.invalidate();
        }
        completion
    }
}

#[test]
fn client_rechecks_lease_after_transport_delivers_success() {
    let (port, binding, _) = setup();
    let mut client = ReadClient::new(DeliveryRace(port.clone()), binding);
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(37, 512)));
    assert!(poll(future.as_mut()).is_pending());
    let request = port.0.borrow_mut().queue.take_request().unwrap();
    complete(&port, request, Ok(512));
    // The queue returned Ok before the lease changed; the client must gate it.
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Err((Error::Stale, _)))
    ));
}

#[test]
fn queued_invalidation_wakes_waiter_and_does_not_wait_for_worker() {
    let (port, _, mut client) = setup();
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut future = Box::pin(client.read_blocks_owned(0, 1, buffer(38, 512)));
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let waker = {
        let mut state = port.0.borrow_mut();
        state.queue.invalidate();
        state.waker.take()
    };
    waker.unwrap().wake();
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Err((Error::Stale, _)))
    ));
}

struct EagerPort(Port);
impl ReadPort for EagerPort {
    type Buffer = Buffer;
    fn is_current(&self, binding: Binding) -> bool {
        self.0.is_current(binding)
    }
    fn submit(
        &mut self,
        binding: Binding,
        first: u64,
        count: u32,
        buffer: Buffer,
    ) -> Result<Ticket, (Error, Buffer)> {
        let ticket = self.0.submit(binding, first, count, buffer)?;
        let request = self.0.0.borrow_mut().queue.take_request().unwrap();
        complete(&self.0, request, Ok(count as usize * 512));
        Ok(ticket)
    }
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Buffer>> {
        self.0.poll_completion(ticket, cx)
    }
}

#[test]
fn completion_before_first_wait_is_not_lost() {
    let (port, binding, _) = setup();
    let mut client = ReadClient::new(EagerPort(port), binding);
    assert_eq!(
        ready(client.read_blocks_owned(0, 1, buffer(39, 512)))
            .unwrap()
            .id,
        39
    );
    assert!(!client.has_pending());
}
