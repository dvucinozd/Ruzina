use pajoniiir_media_block::BlockGeometry;
use pajoniiir_media_fs::LeaseBound;
use pajoniiir_media_io::OwnedBlockDevice;
use pajoniiir_media_session::MediaLease;

const EXFAT_FS_NAME: &[u8; 8] = b"EXFAT   ";
const EXFAT_BOOT_SIGNATURE_OFFSET: usize = 510;
const EXFAT_MIN_SECTOR_BYTES: usize = 512;
const EXFAT_BOOT_REGION_DATA_SECTORS: u64 = 11;
const EXFAT_BOOT_CHECKSUM_SECTOR: u64 = 11;
const EXFAT_RESERVED_BOOT_SECTORS: u32 = 24;
const EXFAT_FIRST_DATA_CLUSTER: u32 = 2;
const EXFAT_END_OF_CHAIN_MIN: u32 = 0xffff_fff8;
const EXFAT_BAD_CLUSTER: u32 = 0xffff_fff7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatAsyncBootInfo {
    pub partition_offset: u64,
    pub volume_length: u64,
    pub fat_offset: u32,
    pub fat_length: u32,
    pub cluster_heap_offset: u32,
    pub cluster_count: u32,
    pub root_directory_cluster: u32,
    pub volume_serial: u32,
    pub file_system_revision: u16,
    pub volume_flags: u16,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub number_of_fats: u8,
    pub percent_in_use: u8,
}

impl ExFatAsyncBootInfo {
    pub const fn max_cluster(self) -> u32 {
        self.cluster_count.saturating_add(1)
    }

    pub const fn cluster_size_bytes(self) -> u64 {
        (self.bytes_per_sector as u64) * (self.sectors_per_cluster as u64)
    }

    pub const fn active_fat_offset(self) -> u64 {
        let active = if self.number_of_fats == 2 && (self.volume_flags & 1) != 0 {
            self.fat_length as u64
        } else {
            0
        };
        (self.fat_offset as u64) + active
    }

    pub fn cluster_first_sector(self, cluster: u32) -> Option<u64> {
        if cluster < EXFAT_FIRST_DATA_CLUSTER || cluster > self.max_cluster() {
            return None;
        }
        let relative = u64::from(cluster - EXFAT_FIRST_DATA_CLUSTER)
            .checked_mul(u64::from(self.sectors_per_cluster))?;
        u64::from(self.cluster_heap_offset).checked_add(relative)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExFatAsyncBootSectorError {
    InvalidDeviceGeometry,
    InvalidSectorLength { expected: usize, actual: usize },
    MissingSignature,
    MissingBootSignature,
    InvalidBytesPerSectorShift(u8),
    InvalidSectorsPerClusterShift(u8),
    SectorSizeMismatch { device: u32, boot: u32 },
    InvalidFatCount(u8),
    InvalidVolumeLength(u64),
    InvalidFatLayout,
    FatTooSmall,
    InvalidClusterHeap,
    InvalidClusterCount,
    InvalidRootDirectoryCluster(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExFatAsyncMountError {
    InvalidDeviceGeometry,
    SectorSizeMismatch { device: u32, boot: u32 },
    ScratchSize { expected: usize, actual: usize },
    VolumeBeyondDevice,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ExFatAsyncIoError<E> {
    MissingScratch,
    Device(E),
    BootChecksumMismatch { expected: u32, observed: u32 },
    InvalidCluster(u32),
    BadCluster(u32),
    CorruptFat,
}

pub fn parse_exfat_boot_sector(
    sector: &[u8],
    geometry: BlockGeometry,
) -> Result<ExFatAsyncBootInfo, ExFatAsyncBootSectorError> {
    if !geometry.is_valid() {
        return Err(ExFatAsyncBootSectorError::InvalidDeviceGeometry);
    }

    let expected = geometry.block_size as usize;
    if expected < EXFAT_MIN_SECTOR_BYTES || sector.len() != expected {
        return Err(ExFatAsyncBootSectorError::InvalidSectorLength {
            expected,
            actual: sector.len(),
        });
    }

    if sector.get(3..11) != Some(EXFAT_FS_NAME.as_slice()) {
        return Err(ExFatAsyncBootSectorError::MissingSignature);
    }
    if sector.get(EXFAT_BOOT_SIGNATURE_OFFSET..EXFAT_BOOT_SIGNATURE_OFFSET + 2)
        != Some([0x55, 0xaa].as_slice())
    {
        return Err(ExFatAsyncBootSectorError::MissingBootSignature);
    }

    let bytes_per_sector_shift = sector[108];
    if !(9..=12).contains(&bytes_per_sector_shift) {
        return Err(ExFatAsyncBootSectorError::InvalidBytesPerSectorShift(
            bytes_per_sector_shift,
        ));
    }
    let bytes_per_sector = 1u32 << bytes_per_sector_shift;
    if bytes_per_sector != geometry.block_size {
        return Err(ExFatAsyncBootSectorError::SectorSizeMismatch {
            device: geometry.block_size,
            boot: bytes_per_sector,
        });
    }

    let sectors_per_cluster_shift = sector[109];
    if u16::from(bytes_per_sector_shift) + u16::from(sectors_per_cluster_shift) > 25 {
        return Err(ExFatAsyncBootSectorError::InvalidSectorsPerClusterShift(
            sectors_per_cluster_shift,
        ));
    }
    let sectors_per_cluster = 1u32
        .checked_shl(u32::from(sectors_per_cluster_shift))
        .ok_or(ExFatAsyncBootSectorError::InvalidSectorsPerClusterShift(
            sectors_per_cluster_shift,
        ))?;

    let number_of_fats = sector[110];
    if !(1..=2).contains(&number_of_fats) {
        return Err(ExFatAsyncBootSectorError::InvalidFatCount(number_of_fats));
    }

    let partition_offset =
        read_u64(sector, 64).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let volume_length = read_u64(sector, 72).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    if volume_length == 0 || volume_length > geometry.block_count {
        return Err(ExFatAsyncBootSectorError::InvalidVolumeLength(
            volume_length,
        ));
    }

    let fat_offset = read_u32(sector, 80).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let fat_length = read_u32(sector, 84).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let cluster_heap_offset =
        read_u32(sector, 88).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let cluster_count = read_u32(sector, 92).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let root_directory_cluster =
        read_u32(sector, 96).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let volume_serial = read_u32(sector, 100).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let file_system_revision =
        read_u16(sector, 104).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let volume_flags = read_u16(sector, 106).ok_or(ExFatAsyncBootSectorError::MissingSignature)?;
    let percent_in_use = sector[112];

    if fat_offset < EXFAT_RESERVED_BOOT_SECTORS || fat_length == 0 {
        return Err(ExFatAsyncBootSectorError::InvalidFatLayout);
    }

    let fats_end = u64::from(fat_offset)
        .checked_add(
            u64::from(fat_length)
                .checked_mul(u64::from(number_of_fats))
                .ok_or(ExFatAsyncBootSectorError::InvalidFatLayout)?,
        )
        .ok_or(ExFatAsyncBootSectorError::InvalidFatLayout)?;
    if fats_end > u64::from(cluster_heap_offset) {
        return Err(ExFatAsyncBootSectorError::InvalidFatLayout);
    }

    if cluster_count == 0 {
        return Err(ExFatAsyncBootSectorError::InvalidClusterCount);
    }
    let max_cluster = cluster_count
        .checked_add(1)
        .ok_or(ExFatAsyncBootSectorError::InvalidClusterCount)?;
    if !(EXFAT_FIRST_DATA_CLUSTER..=max_cluster).contains(&root_directory_cluster) {
        return Err(ExFatAsyncBootSectorError::InvalidRootDirectoryCluster(
            root_directory_cluster,
        ));
    }

    let fat_bytes = u64::from(fat_length)
        .checked_mul(u64::from(bytes_per_sector))
        .ok_or(ExFatAsyncBootSectorError::InvalidFatLayout)?;
    let required_fat_bytes = u64::from(cluster_count)
        .checked_add(2)
        .and_then(|entries| entries.checked_mul(4))
        .ok_or(ExFatAsyncBootSectorError::InvalidClusterCount)?;
    if fat_bytes < required_fat_bytes {
        return Err(ExFatAsyncBootSectorError::FatTooSmall);
    }

    let heap_sectors = u64::from(cluster_count)
        .checked_mul(u64::from(sectors_per_cluster))
        .ok_or(ExFatAsyncBootSectorError::InvalidClusterHeap)?;
    let heap_end = u64::from(cluster_heap_offset)
        .checked_add(heap_sectors)
        .ok_or(ExFatAsyncBootSectorError::InvalidClusterHeap)?;
    if heap_end > volume_length {
        return Err(ExFatAsyncBootSectorError::InvalidClusterHeap);
    }

    Ok(ExFatAsyncBootInfo {
        partition_offset,
        volume_length,
        fat_offset,
        fat_length,
        cluster_heap_offset,
        cluster_count,
        root_directory_cluster,
        volume_serial,
        file_system_revision,
        volume_flags,
        bytes_per_sector,
        sectors_per_cluster,
        number_of_fats,
        percent_in_use,
    })
}

pub struct ExFatAsyncVolume<D>
where
    D: OwnedBlockDevice,
{
    lease: MediaLease,
    device: D,
    scratch: Option<D::Buffer>,
    loaded_sector: Option<u64>,
    boot: ExFatAsyncBootInfo,
}

impl<D> ExFatAsyncVolume<D>
where
    D: OwnedBlockDevice,
{
    pub fn new(
        device: D,
        scratch: D::Buffer,
        boot: ExFatAsyncBootInfo,
        lease: MediaLease,
    ) -> Result<Self, (ExFatAsyncMountError, D, D::Buffer)> {
        let geometry = device.geometry();
        if !geometry.is_valid() {
            return Err((ExFatAsyncMountError::InvalidDeviceGeometry, device, scratch));
        }
        if geometry.block_size != boot.bytes_per_sector {
            return Err((
                ExFatAsyncMountError::SectorSizeMismatch {
                    device: geometry.block_size,
                    boot: boot.bytes_per_sector,
                },
                device,
                scratch,
            ));
        }
        let expected = geometry.block_size as usize;
        if scratch.as_ref().len() != expected {
            let actual = scratch.as_ref().len();
            return Err((
                ExFatAsyncMountError::ScratchSize { expected, actual },
                device,
                scratch,
            ));
        }
        if boot.volume_length > geometry.block_count {
            return Err((ExFatAsyncMountError::VolumeBeyondDevice, device, scratch));
        }

        Ok(Self {
            lease,
            device,
            scratch: Some(scratch),
            loaded_sector: Some(0),
            boot,
        })
    }

    pub const fn lease_value(&self) -> MediaLease {
        self.lease
    }

    pub const fn boot_info(&self) -> ExFatAsyncBootInfo {
        self.boot
    }

    pub const fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn into_parts(mut self) -> (D, Option<D::Buffer>) {
        (self.device, self.scratch.take())
    }

    pub async fn validate_main_boot_region(&mut self) -> Result<(), ExFatAsyncIoError<D::Error>> {
        let mut checksum = 0u32;

        for sector_index in 0..EXFAT_BOOT_REGION_DATA_SECTORS {
            self.load_sector(sector_index).await?;
            let bytes = self.loaded_bytes()?;
            for (byte_index, byte) in bytes.iter().copied().enumerate() {
                if sector_index == 0 && matches!(byte_index, 106 | 107 | 112) {
                    continue;
                }
                checksum = checksum.rotate_right(1).wrapping_add(u32::from(byte));
            }
        }

        self.load_sector(EXFAT_BOOT_CHECKSUM_SECTOR).await?;
        let bytes = self.loaded_bytes()?;
        for chunk in bytes.chunks_exact(4) {
            let observed = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if observed != checksum {
                return Err(ExFatAsyncIoError::BootChecksumMismatch {
                    expected: checksum,
                    observed,
                });
            }
        }

        Ok(())
    }

    pub async fn next_cluster(
        &mut self,
        cluster: u32,
    ) -> Result<Option<u32>, ExFatAsyncIoError<D::Error>> {
        self.validate_cluster(cluster)?;
        let fat_byte_offset = u64::from(cluster)
            .checked_mul(4)
            .ok_or(ExFatAsyncIoError::CorruptFat)?;
        let sector_size = u64::from(self.boot.bytes_per_sector);
        let fat_sector = self
            .boot
            .active_fat_offset()
            .checked_add(fat_byte_offset / sector_size)
            .ok_or(ExFatAsyncIoError::CorruptFat)?;
        let in_sector = usize::try_from(fat_byte_offset % sector_size)
            .map_err(|_| ExFatAsyncIoError::CorruptFat)?;

        self.load_sector(fat_sector).await?;
        let bytes = self.loaded_bytes()?;
        let raw = bytes
            .get(in_sector..in_sector + 4)
            .ok_or(ExFatAsyncIoError::CorruptFat)?;
        let next = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);

        if next >= EXFAT_END_OF_CHAIN_MIN {
            Ok(None)
        } else if next == EXFAT_BAD_CLUSTER {
            Err(ExFatAsyncIoError::BadCluster(cluster))
        } else {
            self.validate_cluster(next)?;
            Ok(Some(next))
        }
    }

    pub fn cluster_first_sector(&self, cluster: u32) -> Result<u64, ExFatAsyncIoError<D::Error>> {
        self.boot
            .cluster_first_sector(cluster)
            .ok_or(ExFatAsyncIoError::InvalidCluster(cluster))
    }

    async fn load_sector(&mut self, sector: u64) -> Result<(), ExFatAsyncIoError<D::Error>> {
        if self.loaded_sector == Some(sector) {
            return Ok(());
        }
        if sector >= self.boot.volume_length {
            return Err(ExFatAsyncIoError::CorruptFat);
        }

        let buffer = self
            .scratch
            .take()
            .ok_or(ExFatAsyncIoError::MissingScratch)?;
        match self.device.read_blocks_owned(sector, 1, buffer).await {
            Ok(buffer) => {
                self.scratch = Some(buffer);
                self.loaded_sector = Some(sector);
                Ok(())
            }
            Err((error, buffer)) => {
                self.scratch = Some(buffer);
                self.loaded_sector = None;
                Err(ExFatAsyncIoError::Device(error))
            }
        }
    }

    fn loaded_bytes(&self) -> Result<&[u8], ExFatAsyncIoError<D::Error>> {
        self.scratch
            .as_ref()
            .map(AsRef::as_ref)
            .ok_or(ExFatAsyncIoError::MissingScratch)
    }

    fn validate_cluster(&self, cluster: u32) -> Result<(), ExFatAsyncIoError<D::Error>> {
        if (EXFAT_FIRST_DATA_CLUSTER..=self.boot.max_cluster()).contains(&cluster) {
            Ok(())
        } else {
            Err(ExFatAsyncIoError::InvalidCluster(cluster))
        }
    }
}

impl<D> LeaseBound for ExFatAsyncVolume<D>
where
    D: OwnedBlockDevice,
{
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let raw = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]))
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

    fn boot_sector(block_count: u64, cluster_count: u32) -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[3..11].copy_from_slice(EXFAT_FS_NAME);
        sector[64..72].copy_from_slice(&0u64.to_le_bytes());
        sector[72..80].copy_from_slice(&block_count.to_le_bytes());
        sector[80..84].copy_from_slice(&24u32.to_le_bytes());
        sector[84..88].copy_from_slice(&1u32.to_le_bytes());
        sector[88..92].copy_from_slice(&25u32.to_le_bytes());
        sector[92..96].copy_from_slice(&cluster_count.to_le_bytes());
        sector[96..100].copy_from_slice(&2u32.to_le_bytes());
        sector[100..104].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        sector[104..106].copy_from_slice(&0x0100u16.to_le_bytes());
        sector[106..108].copy_from_slice(&0u16.to_le_bytes());
        sector[108] = 9;
        sector[109] = 0;
        sector[110] = 1;
        sector[112] = 0;
        sector[510] = 0x55;
        sector[511] = 0xaa;
        sector
    }

    fn geometry() -> BlockGeometry {
        BlockGeometry {
            block_size: 512,
            block_count: 64,
        }
    }

    #[test]
    fn boot_parser_accepts_bounded_exfat_geometry() {
        let sector = boot_sector(64, 16);
        let boot = parse_exfat_boot_sector(&sector, geometry()).unwrap();

        assert_eq!(boot.volume_serial, 0x1234_5678);
        assert_eq!(boot.bytes_per_sector, 512);
        assert_eq!(boot.sectors_per_cluster, 1);
        assert_eq!(boot.cluster_count, 16);
        assert_eq!(boot.root_directory_cluster, 2);
        assert_eq!(boot.cluster_first_sector(2), Some(25));
        assert_eq!(boot.cluster_first_sector(17), Some(40));
        assert_eq!(boot.cluster_first_sector(18), None);
    }

    #[test]
    fn boot_parser_rejects_sector_and_cluster_geometry_mismatch() {
        let mut sector = boot_sector(64, 16);
        sector[108] = 10;
        assert_eq!(
            parse_exfat_boot_sector(&sector, geometry()),
            Err(ExFatAsyncBootSectorError::SectorSizeMismatch {
                device: 512,
                boot: 1_024,
            })
        );

        let mut sector = boot_sector(64, 16);
        sector[96..100].copy_from_slice(&18u32.to_le_bytes());
        assert_eq!(
            parse_exfat_boot_sector(&sector, geometry()),
            Err(ExFatAsyncBootSectorError::InvalidRootDirectoryCluster(18))
        );
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        BadTransfer,
    }

    #[derive(Debug)]
    struct MemoryOwnedDevice {
        bytes: [u8; 64 * 512],
    }

    impl OwnedBlockDevice for MemoryOwnedDevice {
        type Error = TestError;
        type Buffer = [u8; 512];

        fn geometry(&self) -> BlockGeometry {
            geometry()
        }

        async fn read_blocks_owned(
            &mut self,
            first_block: u64,
            block_count: u32,
            mut buffer: Self::Buffer,
        ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
            if self
                .geometry()
                .validate_transfer(first_block, block_count, buffer.len())
                .is_err()
            {
                return Err((TestError::BadTransfer, buffer));
            }
            let start = first_block as usize * 512;
            buffer.copy_from_slice(&self.bytes[start..start + 512]);
            Ok(buffer)
        }
    }

    fn valid_boot_region() -> ([[u8; 512]; 12], ExFatAsyncBootInfo) {
        let mut sectors = [[0u8; 512]; 12];
        sectors[0] = boot_sector(64, 16);
        let boot = parse_exfat_boot_sector(&sectors[0], geometry()).unwrap();

        let mut checksum = 0u32;
        for (sector_index, sector) in sectors.iter().take(11).enumerate() {
            for (byte_index, byte) in sector.iter().copied().enumerate() {
                if sector_index == 0 && matches!(byte_index, 106 | 107 | 112) {
                    continue;
                }
                checksum = checksum.rotate_right(1).wrapping_add(u32::from(byte));
            }
        }
        for chunk in sectors[11].chunks_exact_mut(4) {
            chunk.copy_from_slice(&checksum.to_le_bytes());
        }
        (sectors, boot)
    }

    fn volume_from_region(
        sectors: [[u8; 512]; 12],
        boot: ExFatAsyncBootInfo,
    ) -> ExFatAsyncVolume<MemoryOwnedDevice> {
        let mut bytes = [0u8; 64 * 512];
        for (index, sector) in sectors.into_iter().enumerate() {
            bytes[index * 512..(index + 1) * 512].copy_from_slice(&sector);
        }
        ExFatAsyncVolume::new(
            MemoryOwnedDevice { bytes },
            bytes[0..512].try_into().unwrap(),
            boot,
            MediaLease {
                generation: pajoniiir_media_session::MediaGeneration::initial(),
                source: pajoniiir_media_session::MediaSourceId::new(1).unwrap(),
            },
        )
        .unwrap()
    }

    #[test]
    fn async_volume_validates_main_boot_checksum() {
        let (sectors, boot) = valid_boot_region();
        let mut volume = volume_from_region(sectors, boot);
        assert_eq!(block_on_ready(volume.validate_main_boot_region()), Ok(()));
    }

    #[test]
    fn async_volume_rejects_corrupt_main_boot_region() {
        let (mut sectors, boot) = valid_boot_region();
        sectors[3][17] ^= 0x80;
        let mut volume = volume_from_region(sectors, boot);
        assert!(matches!(
            block_on_ready(volume.validate_main_boot_region()),
            Err(ExFatAsyncIoError::BootChecksumMismatch { .. })
        ));
    }
}
