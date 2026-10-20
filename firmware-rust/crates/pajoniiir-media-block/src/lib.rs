#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use core::num::NonZeroU64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockGeometry {
    pub block_size: u32,
    pub block_count: u64,
}

impl BlockGeometry {
    pub const fn is_valid(self) -> bool {
        self.block_size != 0 && self.block_count != 0
    }

    pub const fn byte_len(self) -> Option<u64> {
        (self.block_size as u64).checked_mul(self.block_count)
    }

    pub fn validate_transfer(
        self,
        first_block: u64,
        block_count: u32,
        buffer_len: usize,
    ) -> Result<(), TransferError> {
        if !self.is_valid() {
            return Err(TransferError::InvalidGeometry);
        }
        if block_count == 0 {
            return Err(TransferError::ZeroBlocks);
        }

        let end = first_block
            .checked_add(block_count as u64)
            .ok_or(TransferError::OutOfRange)?;
        if end > self.block_count {
            return Err(TransferError::OutOfRange);
        }

        let expected = (self.block_size as u64)
            .checked_mul(block_count as u64)
            .ok_or(TransferError::InvalidBufferLength)?;
        if expected != buffer_len as u64 {
            return Err(TransferError::InvalidBufferLength);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferError {
    InvalidGeometry,
    ZeroBlocks,
    InvalidBufferLength,
    OutOfRange,
}

pub trait BlockDevice {
    type Error;

    fn geometry(&self) -> BlockGeometry;

    fn read_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        output: &mut [u8],
    ) -> Result<(), Self::Error>;
}

pub trait WritableBlockDevice: BlockDevice {
    fn write_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        input: &[u8],
    ) -> Result<(), Self::Error>;

    fn flush(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRange {
    first_block: u64,
    block_count: NonZeroU64,
}

impl BlockRange {
    pub fn new(first_block: u64, block_count: NonZeroU64, geometry: BlockGeometry) -> Option<Self> {
        if !geometry.is_valid() {
            return None;
        }
        let end = first_block.checked_add(block_count.get())?;
        if end > geometry.block_count {
            return None;
        }
        Some(Self {
            first_block,
            block_count,
        })
    }

    pub fn from_start(first_block: u64, geometry: BlockGeometry) -> Option<Self> {
        if !geometry.is_valid() || first_block >= geometry.block_count {
            return None;
        }
        let count = NonZeroU64::new(geometry.block_count - first_block)?;
        Self::new(first_block, count, geometry)
    }

    pub const fn first_block(self) -> u64 {
        self.first_block
    }

    pub const fn block_count(self) -> NonZeroU64 {
        self.block_count
    }

    pub fn geometry(self, block_size: u32) -> BlockGeometry {
        BlockGeometry {
            block_size,
            block_count: self.block_count.get(),
        }
    }

    pub fn translate(self, first_block: u64, block_count: u32) -> Option<u64> {
        if block_count == 0 {
            return None;
        }
        let relative_end = first_block.checked_add(block_count as u64)?;
        if relative_end > self.block_count.get() {
            return None;
        }
        self.first_block.checked_add(first_block)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum PartitionDeviceError<E> {
    Transfer(TransferError),
    Inner(E),
}

impl<E: fmt::Display> fmt::Display for PartitionDeviceError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transfer(error) => write!(formatter, "partition transfer error: {error:?}"),
            Self::Inner(error) => write!(formatter, "partition backend error: {error}"),
        }
    }
}

impl<E> core::error::Error for PartitionDeviceError<E> where E: core::error::Error + 'static {}

pub struct PartitionDevice<D> {
    inner: D,
    range: BlockRange,
}

impl<D> PartitionDevice<D> {
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

impl<D: BlockDevice> BlockDevice for PartitionDevice<D> {
    type Error = PartitionDeviceError<D::Error>;

    fn geometry(&self) -> BlockGeometry {
        self.range.geometry(self.inner.geometry().block_size)
    }

    fn read_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        output: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.geometry()
            .validate_transfer(first_block, block_count, output.len())
            .map_err(PartitionDeviceError::Transfer)?;
        let absolute = self
            .range
            .translate(first_block, block_count)
            .ok_or(PartitionDeviceError::Transfer(TransferError::OutOfRange))?;
        self.inner
            .read_blocks(absolute, block_count, output)
            .map_err(PartitionDeviceError::Inner)
    }
}

impl<D: WritableBlockDevice> WritableBlockDevice for PartitionDevice<D> {
    fn write_blocks(
        &mut self,
        first_block: u64,
        block_count: u32,
        input: &[u8],
    ) -> Result<(), Self::Error> {
        self.geometry()
            .validate_transfer(first_block, block_count, input.len())
            .map_err(PartitionDeviceError::Transfer)?;
        let absolute = self
            .range
            .translate(first_block, block_count)
            .ok_or(PartitionDeviceError::Transfer(TransferError::OutOfRange))?;
        self.inner
            .write_blocks(absolute, block_count, input)
            .map_err(PartitionDeviceError::Inner)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(PartitionDeviceError::Inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        BadTransfer,
    }

    struct MemoryDevice {
        bytes: [u8; 4_096],
        read_calls: u32,
        write_calls: u32,
        flush_calls: u32,
    }

    impl MemoryDevice {
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
            }
        }
    }

    impl BlockDevice for MemoryDevice {
        type Error = TestError;

        fn geometry(&self) -> BlockGeometry {
            BlockGeometry {
                block_size: 512,
                block_count: 8,
            }
        }

        fn read_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            output: &mut [u8],
        ) -> Result<(), Self::Error> {
            self.geometry()
                .validate_transfer(first_block, block_count, output.len())
                .map_err(|_| TestError::BadTransfer)?;
            self.read_calls += 1;
            let start = first_block as usize * 512;
            output.copy_from_slice(&self.bytes[start..start + output.len()]);
            Ok(())
        }
    }

    impl WritableBlockDevice for MemoryDevice {
        fn write_blocks(
            &mut self,
            first_block: u64,
            block_count: u32,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            self.geometry()
                .validate_transfer(first_block, block_count, input.len())
                .map_err(|_| TestError::BadTransfer)?;
            self.write_calls += 1;
            let start = first_block as usize * 512;
            self.bytes[start..start + input.len()].copy_from_slice(input);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.flush_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn geometry_rejects_misaligned_and_out_of_range_transfers() {
        let geometry = BlockGeometry {
            block_size: 512,
            block_count: 8,
        };

        assert_eq!(geometry.validate_transfer(0, 1, 512), Ok(()));
        assert_eq!(
            geometry.validate_transfer(0, 1, 511),
            Err(TransferError::InvalidBufferLength)
        );
        assert_eq!(
            geometry.validate_transfer(8, 1, 512),
            Err(TransferError::OutOfRange)
        );
        assert_eq!(
            geometry.validate_transfer(0, 0, 0),
            Err(TransferError::ZeroBlocks)
        );
    }

    #[test]
    fn range_translation_is_checked_and_partition_relative() {
        let geometry = BlockGeometry {
            block_size: 512,
            block_count: 100,
        };
        let range = BlockRange::new(10, NonZeroU64::new(20).unwrap(), geometry).unwrap();

        assert_eq!(range.translate(0, 1), Some(10));
        assert_eq!(range.translate(19, 1), Some(29));
        assert_eq!(range.translate(20, 1), None);
        assert_eq!(range.translate(u64::MAX, 1), None);
    }

    #[test]
    fn from_start_covers_superfloppy_or_unknown_extent_to_device_end() {
        let geometry = BlockGeometry {
            block_size: 4_096,
            block_count: 1_000,
        };
        let range = BlockRange::from_start(25, geometry).unwrap();

        assert_eq!(range.first_block(), 25);
        assert_eq!(range.block_count().get(), 975);
        assert_eq!(
            range.geometry(4_096),
            BlockGeometry {
                block_size: 4_096,
                block_count: 975,
            }
        );
    }

    #[test]
    fn partition_device_translates_reads_and_blocks_escape_attempts() {
        let inner = MemoryDevice::new();
        let range = BlockRange::new(2, NonZeroU64::new(4).unwrap(), inner.geometry()).unwrap();
        let mut partition = PartitionDevice::new(inner, range);
        let mut block = [0u8; 512];

        partition.read_blocks(1, 1, &mut block).unwrap();
        assert_eq!(partition.inner().read_calls, 1);
        assert_eq!(block[0], 0);

        assert_eq!(
            partition.read_blocks(4, 1, &mut block),
            Err(PartitionDeviceError::Transfer(TransferError::OutOfRange))
        );
        assert_eq!(partition.inner().read_calls, 1);
    }

    #[test]
    fn partition_device_translates_writes_and_flush() {
        let inner = MemoryDevice::new();
        let range = BlockRange::new(1, NonZeroU64::new(2).unwrap(), inner.geometry()).unwrap();
        let mut partition = PartitionDevice::new(inner, range);
        let data = [0x5au8; 512];

        partition.write_blocks(0, 1, &data).unwrap();
        partition.flush().unwrap();

        assert_eq!(partition.inner().write_calls, 1);
        assert_eq!(partition.inner().flush_calls, 1);
        assert_eq!(&partition.inner().bytes[512..1_024], &data);
    }

    #[test]
    fn byte_len_is_checked_for_large_geometry() {
        assert_eq!(
            BlockGeometry {
                block_size: u32::MAX,
                block_count: u64::MAX,
            }
            .byte_len(),
            None
        );
    }
}
