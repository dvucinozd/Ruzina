//! Read-only single-slot protocol for an IDF-owned blocking SD task.
//!
//! This module has no RTOS/FFI dependency. A platform port must serialize every
//! queue access and wake the client after completion/invalidation. The worker
//! owns `ReadRequest` throughout DMA and may complete it only after DMA stops.
//! No blocking SDMMC call belongs in `ReadPort` or the client's async poll.
#![forbid(unsafe_code)]

use core::future::poll_fn;
use core::task::{Context, Poll};
use ruzina_media_block::{BlockGeometry, TransferError};
use ruzina_media_io::OwnedBlockDevice;
use ruzina_media_session::{ConnectResult, MediaHandle, MediaLease, MediaSession, MediaSourceId};

pub const MAX_READ_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Transfer(TransferError),
    UnsupportedGeometry,
    TooLarge,
    Busy,
    Stale,
    SequenceExhausted,
    Device(i32),
    ShortRead { expected: usize, actual: usize },
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "IDF read bridge: {self:?}")
    }
}

impl core::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub lease: MediaLease,
    pub handle: MediaHandle,
    pub geometry: BlockGeometry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ticket {
    binding: Binding,
    sequence: u64,
}

pub struct ReadRequest<B> {
    ticket: Ticket,
    first_block: u64,
    block_count: u32,
    buffer: B,
}

impl<B: AsMut<[u8]>> ReadRequest<B> {
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }
    pub fn first_block(&self) -> u64 {
        self.first_block
    }
    pub fn block_count(&self) -> u32 {
        self.block_count
    }
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        self.buffer.as_mut()
    }
}

pub type ReadResult<B> = Result<B, (Error, B)>;

enum Slot<B> {
    Empty,
    Queued(ReadRequest<B>),
    Active(Ticket),
    Complete {
        ticket: Ticket,
        result: ReadResult<B>,
    },
}

/// Exactly one hardware owner and one outstanding request, without allocation.
/// Handles must uniquely identify that hardware owner within this runtime.
pub struct ReadQueue<B> {
    session: MediaSession,
    binding: Option<Binding>,
    sequence: u64,
    slot: Slot<B>,
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> Default for ReadQueue<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> ReadQueue<B> {
    pub const fn new() -> Self {
        Self {
            session: MediaSession::new(),
            binding: None,
            sequence: 0,
            slot: Slot::Empty,
        }
    }

    /// Call only after card init succeeds. This is block availability, not a
    /// filesystem mount; shared `MediaSession::commit_mounted` is not implied.
    pub fn activate(
        &mut self,
        source: MediaSourceId,
        handle: MediaHandle,
        geometry: BlockGeometry,
    ) -> Result<Binding, Error> {
        if self.binding.is_some() || !matches!(self.slot, Slot::Empty) {
            return Err(Error::Busy);
        }
        if geometry.block_size != 512
            || geometry.block_count == 0
            || geometry.block_count > u32::MAX as u64
        {
            return Err(Error::UnsupportedGeometry);
        }
        let lease = match self.session.on_connect(source) {
            ConnectResult::Accepted(lease) => lease,
            _ => return Err(Error::Busy),
        };
        self.session.bind_handle(lease, handle);
        let binding = Binding {
            lease,
            handle,
            geometry,
        };
        self.binding = Some(binding);
        Ok(binding)
    }

    pub fn is_current(&self, binding: Binding) -> bool {
        self.binding == Some(binding)
            && self.session.validate(binding.lease)
            && self.session.handle() == Some(binding.handle)
    }

    /// Invalidate before host/card teardown. Queued IO becomes a stale completion;
    /// active IO retains its buffer in the worker until `complete` returns it.
    pub fn invalidate(&mut self) {
        if let Some(binding) = self.binding.take() {
            self.session.on_disconnect(Some(binding.handle));
        }
        let slot = core::mem::replace(&mut self.slot, Slot::Empty);
        self.slot = match slot {
            Slot::Queued(request) => Slot::Complete {
                ticket: request.ticket,
                result: Err((Error::Stale, request.buffer)),
            },
            Slot::Complete { ticket, result } => {
                let buffer = match result {
                    Ok(buffer) | Err((_, buffer)) => buffer,
                };
                Slot::Complete {
                    ticket,
                    result: Err((Error::Stale, buffer)),
                }
            }
            other => other,
        };
    }

    /// False while a worker can still be using DMA/card memory. A completed
    /// buffer may await client recovery after it is safe to deinitialize hardware.
    pub fn can_deinitialize(&self) -> bool {
        self.binding.is_none() && !matches!(self.slot, Slot::Active(_))
    }

    pub fn submit(
        &mut self,
        binding: Binding,
        first_block: u64,
        block_count: u32,
        buffer: B,
    ) -> Result<Ticket, (Error, B)> {
        if !self.is_current(binding) {
            return Err((Error::Stale, buffer));
        }
        if let Err(error) =
            binding
                .geometry
                .validate_transfer(first_block, block_count, buffer.as_ref().len())
        {
            return Err((Error::Transfer(error), buffer));
        }
        if buffer.as_ref().len() > MAX_READ_BYTES {
            return Err((Error::TooLarge, buffer));
        }
        if !matches!(self.slot, Slot::Empty) {
            return Err((Error::Busy, buffer));
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            return Err((Error::SequenceExhausted, buffer));
        };
        self.sequence = sequence;
        let ticket = Ticket { binding, sequence };
        self.slot = Slot::Queued(ReadRequest {
            ticket,
            first_block,
            block_count,
            buffer,
        });
        Ok(ticket)
    }

    pub fn take_request(&mut self) -> Option<ReadRequest<B>> {
        if !matches!(self.slot, Slot::Queued(_)) {
            return None;
        }
        let Slot::Queued(request) = core::mem::replace(&mut self.slot, Slot::Empty) else {
            unreachable!()
        };
        self.slot = Slot::Active(request.ticket);
        Some(request)
    }

    /// Worker retains the request on an unmatched completion. Nothing is freed,
    /// silently discarded or overwritten; drain/fault handling stays with owner.
    pub fn complete(
        &mut self,
        request: ReadRequest<B>,
        result: Result<usize, i32>,
    ) -> Result<(), ReadRequest<B>> {
        if !matches!(self.slot, Slot::Active(ticket) if ticket == request.ticket) {
            return Err(request);
        }
        let expected = request.block_count as usize * 512;
        let status = if !self.is_current(request.ticket.binding) {
            Err(Error::Stale)
        } else {
            match result {
                Ok(actual) if actual == expected => Ok(()),
                Ok(actual) => Err(Error::ShortRead { expected, actual }),
                Err(code) => Err(Error::Device(code)),
            }
        };
        let result = match status {
            Ok(()) => Ok(request.buffer),
            Err(error) => Err((error, request.buffer)),
        };
        self.slot = Slot::Complete {
            ticket: request.ticket,
            result,
        };
        Ok(())
    }

    pub fn take_completion(&mut self, expected: Ticket) -> Option<ReadResult<B>> {
        if !matches!(self.slot, Slot::Complete { ticket, .. } if ticket == expected) {
            return None;
        }
        let Slot::Complete { result, .. } = core::mem::replace(&mut self.slot, Slot::Empty) else {
            unreachable!()
        };
        Some(result)
    }
}

/// Platform transport contract, implemented by a synchronized IDF queue/notify
/// bridge. Submission is nonblocking and returns the same buffer if rejected.
/// Completion polling must register a waker and atomically recheck readiness to
/// avoid lost wakes. Owner/queue outlive active IO even if this port is dropped.
/// Never retain a borrowed Rust slice or access a returned buffer from DMA.
pub trait ReadPort {
    type Buffer: AsRef<[u8]> + AsMut<[u8]>;
    fn is_current(&self, binding: Binding) -> bool;
    fn submit(
        &mut self,
        binding: Binding,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Ticket, (Error, Self::Buffer)>;
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Self::Buffer>>;
}

/// A canceled read leaves `pending` intact. Recover it before another read or
/// remount. Dropping an unpolled future simply drops its never-submitted buffer;
/// after submission, the task/queue retains ownership until completion/drain.
pub struct ReadClient<P> {
    port: P,
    binding: Binding,
    pending: Option<Ticket>,
}

impl<P: ReadPort> ReadClient<P> {
    pub fn new(port: P, binding: Binding) -> Self {
        Self {
            port,
            binding,
            pending: None,
        }
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    async fn receive(&mut self, ticket: Ticket) -> ReadResult<P::Buffer> {
        let result = poll_fn(|cx| self.port.poll_completion(ticket, cx)).await;
        self.pending = None;
        if !self.port.is_current(self.binding) {
            let buffer = match result {
                Ok(buffer) | Err((_, buffer)) => buffer,
            };
            Err((Error::Stale, buffer))
        } else {
            result
        }
    }

    pub async fn recover_pending(&mut self) -> Option<ReadResult<P::Buffer>> {
        let ticket = self.pending?;
        Some(self.receive(ticket).await)
    }
}

impl<P: ReadPort> OwnedBlockDevice for ReadClient<P> {
    type Error = Error;
    type Buffer = P::Buffer;
    fn geometry(&self) -> BlockGeometry {
        self.binding.geometry
    }

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> ReadResult<Self::Buffer> {
        if let Err(error) =
            self.binding
                .geometry
                .validate_transfer(first_block, block_count, buffer.as_ref().len())
        {
            return Err((Error::Transfer(error), buffer));
        }
        if buffer.as_ref().len() > MAX_READ_BYTES {
            return Err((Error::TooLarge, buffer));
        }
        if self.pending.is_some() {
            return Err((Error::Busy, buffer));
        }
        if !self.port.is_current(self.binding) {
            return Err((Error::Stale, buffer));
        }
        match self
            .port
            .submit(self.binding, first_block, block_count, buffer)
        {
            Err(error) => Err(error),
            Ok(ticket) => {
                self.pending = Some(ticket);
                self.receive(ticket).await
            }
        }
    }
}

#[cfg(test)]
mod tests;
