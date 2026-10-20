#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

use core::fmt;

use pajoniiir_media_block::{BlockGeometry, BlockRange, TransferError};

/// Owned-buffer asynchronous block I/O contract.
///
/// Buffers move into the transport while an operation is in flight and are
/// always returned on both success and failure. This matches DMA/USB ownership
/// without requiring heap allocation or a synchronous block_on bridge.
#[allow(async_fn_in_trait)]
pub trait OwnedBlockDevice {
    type Error;
    type Buffer: AsRef<[u8]> + AsMut<[u8]>;

    fn geometry(&self) -> BlockGeometry;

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)>;
}

#[allow(async_fn_in_trait)]
pub trait OwnedWritableBlockDevice: OwnedBlockDevice {
    async fn write_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)>;

    async fn flush_owned(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum AsyncPartitionError<E> {
    Transfer(TransferError),
    Inner(E),
}

impl<E: fmt::Display> fmt::Display for AsyncPartitionError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transfer(error) => write!(formatter, "async partition transfer error: {error:?}"),
            Self::Inner(error) => write!(formatter, "async partition backend error: {error}"),
        }
    }
}

impl<E> core::error::Error for AsyncPartitionError<E> where E: core::error::Error + 'static {}

pub struct AsyncPartitionDevice<D> {
    inner: D,
    range: BlockRange,
}

impl<D> AsyncPartitionDevice<D> {
    pub const fn new(inner: D, range: BlockRange) -> Self {
        Self { inner, range }
    }

    pub const fn range(&self) -> BlockRange {
        self.range
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

impl<D: OwnedBlockDevice> OwnedBlockDevice for AsyncPartitionDevice<D> {
    type Error = AsyncPartitionError<D::Error>;
    type Buffer = D::Buffer;

    fn geometry(&self) -> BlockGeometry {
        self.range.geometry(self.inner.geometry().block_size)
    }

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        if let Err(error) =
            self.geometry()
                .validate_transfer(first_block, block_count, buffer.as_ref().len())
        {
            return Err((AsyncPartitionError::Transfer(error), buffer));
        }
        let Some(absolute) = self.range.translate(first_block, block_count) else {
            return Err((
                AsyncPartitionError::Transfer(TransferError::OutOfRange),
                buffer,
            ));
        };

        match self
            .inner
            .read_blocks_owned(absolute, block_count, buffer)
            .await
        {
            Ok(buffer) => Ok(buffer),
            Err((error, buffer)) => Err((AsyncPartitionError::Inner(error), buffer)),
        }
    }
}

impl<D: OwnedWritableBlockDevice> OwnedWritableBlockDevice for AsyncPartitionDevice<D> {
    async fn write_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        if let Err(error) =
            self.geometry()
                .validate_transfer(first_block, block_count, buffer.as_ref().len())
        {
            return Err((AsyncPartitionError::Transfer(error), buffer));
        }
        let Some(absolute) = self.range.translate(first_block, block_count) else {
            return Err((
                AsyncPartitionError::Transfer(TransferError::OutOfRange),
                buffer,
            ));
        };

        match self
            .inner
            .write_blocks_owned(absolute, block_count, buffer)
            .await
        {
            Ok(buffer) => Ok(buffer),
            Err((error, buffer)) => Err((AsyncPartitionError::Inner(error), buffer)),
        }
    }

    async fn flush_owned(&mut self) -> Result<(), Self::Error> {
        self.inner
            .flush_owned()
            .await
            .map_err(AsyncPartitionError::Inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll};
    use std::sync::Arc;
    use std::task::{Wake, Waker};

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

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        Failed,
    }

    struct MemoryOwnedDevice {
        bytes: [u8; 4_096],
        read_calls: u32,
        write_calls: u32,
        flush_calls: u32,
        last_block: u64,
        fail: bool,
    }

    impl MemoryOwnedDevice {
        fn new() -> Self {
            let mut bytes = [0u8; 4_096];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = (index & 0xff) as u8;
            }
            Self {
                bytes,
                read_calls: 0,
                write_calls: 0,
                flush_calls: 0,
                last_block: 0,
                fail: false,
            }
        }
    }

    impl OwnedBlockDevice for MemoryOwnedDevice {
        type Error = TestError;
        type Buffer = [u8; 512];

        fn geometry(&self) -> BlockGeometry {
            BlockGeometry {
                block_size: 512,
                block_count: 8,
            }
        }

        async fn read_blocks_owned(
            &mut self,
            first_block: u64,
            block_count: u32,
            mut buffer: Self::Buffer,
        ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
            if self.fail {
                return Err((TestError::Failed, buffer));
            }
            if self
                .geometry()
                .validate_transfer(first_block, block_count, buffer.len())
                .is_err()
            {
                return Err((TestError::Failed, buffer));
            }
            self.read_calls += 1;
            self.last_block = first_block;
            let start = first_block as usize * 512;
            let len = buffer.len();
            buffer.copy_from_slice(&self.bytes[start..start + len]);
            Ok(buffer)
        }
    }

    impl OwnedWritableBlockDevice for MemoryOwnedDevice {
        async fn write_blocks_owned(
            &mut self,
            first_block: u64,
            block_count: u32,
            buffer: Self::Buffer,
        ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
            if self.fail {
                return Err((TestError::Failed, buffer));
            }
            if self
                .geometry()
                .validate_transfer(first_block, block_count, buffer.len())
                .is_err()
            {
                return Err((TestError::Failed, buffer));
            }
            self.write_calls += 1;
            self.last_block = first_block;
            let start = first_block as usize * 512;
            let len = buffer.len();
            self.bytes[start..start + len].copy_from_slice(&buffer);
            Ok(buffer)
        }

        async fn flush_owned(&mut self) -> Result<(), Self::Error> {
            if self.fail {
                return Err(TestError::Failed);
            }
            self.flush_calls += 1;
            Ok(())
        }
    }

    fn partition() -> AsyncPartitionDevice<MemoryOwnedDevice> {
        let inner = MemoryOwnedDevice::new();
        let range =
            BlockRange::new(2, core::num::NonZeroU64::new(4).unwrap(), inner.geometry()).unwrap();
        AsyncPartitionDevice::new(inner, range)
    }

    #[test]
    fn async_partition_translates_owned_reads() {
        let mut device = partition();
        let output = block_on_ready(device.read_blocks_owned(1, 1, [0u8; 512])).unwrap();

        assert_eq!(device.inner().read_calls, 1);
        assert_eq!(device.inner().last_block, 3);
        assert_eq!(output[0], 0);
        assert_eq!(device.geometry().block_count, 4);
    }

    #[test]
    fn async_partition_preserves_buffer_on_contract_failure() {
        let mut device = partition();
        let buffer = [0x5au8; 512];
        let (error, returned) = block_on_ready(device.read_blocks_owned(4, 1, buffer)).unwrap_err();

        assert_eq!(
            error,
            AsyncPartitionError::Transfer(TransferError::OutOfRange)
        );
        assert_eq!(returned, [0x5a; 512]);
        assert_eq!(device.inner().read_calls, 0);
    }

    #[test]
    fn async_partition_translates_writes_and_flush() {
        let mut device = partition();
        let input = [0xa5u8; 512];

        let returned = block_on_ready(device.write_blocks_owned(0, 1, input)).unwrap();
        block_on_ready(device.flush_owned()).unwrap();

        assert_eq!(returned, [0xa5; 512]);
        assert_eq!(device.inner().write_calls, 1);
        assert_eq!(device.inner().last_block, 2);
        assert_eq!(device.inner().flush_calls, 1);
        assert_eq!(&device.inner().bytes[1_024..1_536], &[0xa5; 512]);
    }

    #[test]
    fn backend_error_preserves_owned_buffer() {
        let mut device = partition();
        device.inner_mut().fail = true;
        let buffer = [0x3cu8; 512];

        let (error, returned) = block_on_ready(device.read_blocks_owned(0, 1, buffer)).unwrap_err();

        assert_eq!(error, AsyncPartitionError::Inner(TestError::Failed));
        assert_eq!(returned, [0x3c; 512]);
    }
}
