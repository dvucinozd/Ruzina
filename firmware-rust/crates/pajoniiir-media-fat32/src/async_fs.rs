use core::char;

use pajoniiir_media_block::BlockGeometry;
use pajoniiir_media_fs::{
    AsyncFileSystem, DirEntry, DirectoryHandle, EntryKind, FS_NAME_MAX, FileHandle, FileStat,
    FileSystemCapabilities, FileSystemKind, LeaseBound,
};
use pajoniiir_media_io::OwnedBlockDevice;
use pajoniiir_media_session::MediaLease;

use crate::Fat32BootInfo;

const FAT32_ENTRY_SIZE: usize = 32;
const FAT32_LFN_ATTRIBUTE: u8 = 0x0f;
const FAT32_DIRECTORY_ATTRIBUTE: u8 = 0x10;
const FAT32_VOLUME_ATTRIBUTE: u8 = 0x08;
const FAT32_DELETED_ENTRY: u8 = 0xe5;
const FAT32_END_OF_DIRECTORY: u8 = 0x00;
const FAT32_BAD_CLUSTER: u32 = 0x0fff_fff7;
const FAT32_END_OF_CHAIN: u32 = 0x0fff_fff8;
const FAT32_CLUSTER_MASK: u32 = 0x0fff_ffff;
const FAT32_MAX_LFN_UNITS: usize = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fat32AsyncMountError {
    InvalidDeviceGeometry,
    SectorSizeMismatch { device: u32, boot: u16 },
    ScratchSize { expected: usize, actual: usize },
    VolumeBeyondDevice,
}

#[derive(Debug)]
pub enum Fat32AsyncFsError<E> {
    StaleLease,
    MissingScratch,
    Device(E),
    InvalidPath,
    PathNotFound,
    NotDirectory,
    IsDirectory,
    CorruptDirectory,
    CorruptCluster(u32),
    BadCluster(u32),
    ClusterLoop,
    NameTooLong,
    PositionOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirCursor {
    cluster: u32,
    sector_in_cluster: u32,
    entry_in_sector: u16,
    end: bool,
    cluster_hops: u32,
}

impl DirCursor {
    const fn new(cluster: u32) -> Self {
        Self {
            cluster,
            sector_in_cluster: 0,
            entry_in_sector: 0,
            end: false,
            cluster_hops: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolvedEntry {
    kind: EntryKind,
    first_cluster: u32,
    len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32AsyncFileHandle {
    lease: MediaLease,
    first_cluster: u32,
    current_cluster: u32,
    current_cluster_index: u32,
    len: u64,
    position: u64,
}

impl LeaseBound for Fat32AsyncFileHandle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

impl FileHandle for Fat32AsyncFileHandle {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat32AsyncDirectoryHandle {
    lease: MediaLease,
    cursor: DirCursor,
}

impl LeaseBound for Fat32AsyncDirectoryHandle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

impl DirectoryHandle for Fat32AsyncDirectoryHandle {}

pub struct Fat32AsyncFileSystem<D>
where
    D: OwnedBlockDevice,
{
    lease: MediaLease,
    device: D,
    scratch: Option<D::Buffer>,
    loaded_sector: Option<u64>,
    boot: Fat32BootInfo,
}

impl<D> Fat32AsyncFileSystem<D>
where
    D: OwnedBlockDevice,
{
    pub fn new(
        device: D,
        scratch: D::Buffer,
        boot: Fat32BootInfo,
        lease: MediaLease,
    ) -> Result<Self, (Fat32AsyncMountError, D, D::Buffer)> {
        let geometry = device.geometry();
        if !geometry.is_valid() {
            return Err((Fat32AsyncMountError::InvalidDeviceGeometry, device, scratch));
        }
        if geometry.block_size != u32::from(boot.bytes_per_sector) {
            return Err((
                Fat32AsyncMountError::SectorSizeMismatch {
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
                Fat32AsyncMountError::ScratchSize { expected, actual },
                device,
                scratch,
            ));
        }
        if u64::from(boot.total_sectors) > geometry.block_count {
            return Err((Fat32AsyncMountError::VolumeBeyondDevice, device, scratch));
        }

        Ok(Self {
            lease,
            device,
            scratch: Some(scratch),
            loaded_sector: None,
            boot,
        })
    }

    pub const fn lease_value(&self) -> MediaLease {
        self.lease
    }

    pub const fn boot_info(&self) -> Fat32BootInfo {
        self.boot
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn into_parts(mut self) -> (D, Option<D::Buffer>) {
        (self.device, self.scratch.take())
    }

    fn ensure_lease(&self, lease: MediaLease) -> Result<(), Fat32AsyncFsError<D::Error>> {
        if lease == self.lease {
            Ok(())
        } else {
            Err(Fat32AsyncFsError::StaleLease)
        }
    }

    fn cluster_size(&self) -> u64 {
        u64::from(self.boot.bytes_per_sector) * u64::from(self.boot.sectors_per_cluster)
    }

    fn validate_cluster(&self, cluster: u32) -> Result<(), Fat32AsyncFsError<D::Error>> {
        if (2..=self.boot.max_cluster).contains(&cluster) {
            Ok(())
        } else {
            Err(Fat32AsyncFsError::CorruptCluster(cluster))
        }
    }

    fn cluster_first_sector(&self, cluster: u32) -> Result<u64, Fat32AsyncFsError<D::Error>> {
        self.validate_cluster(cluster)?;
        let relative = u64::from(cluster - 2)
            .checked_mul(u64::from(self.boot.sectors_per_cluster))
            .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        self.boot
            .data_start_sector
            .checked_add(relative)
            .ok_or(Fat32AsyncFsError::PositionOverflow)
    }

    async fn load_sector(&mut self, sector: u64) -> Result<(), Fat32AsyncFsError<D::Error>> {
        if self.loaded_sector == Some(sector) {
            return Ok(());
        }

        let buffer = self
            .scratch
            .take()
            .ok_or(Fat32AsyncFsError::MissingScratch)?;
        match self.device.read_blocks_owned(sector, 1, buffer).await {
            Ok(buffer) => {
                self.scratch = Some(buffer);
                self.loaded_sector = Some(sector);
                Ok(())
            }
            Err((error, buffer)) => {
                self.scratch = Some(buffer);
                self.loaded_sector = None;
                Err(Fat32AsyncFsError::Device(error))
            }
        }
    }

    fn loaded_bytes(&self) -> Result<&[u8], Fat32AsyncFsError<D::Error>> {
        self.scratch
            .as_ref()
            .map(AsRef::as_ref)
            .ok_or(Fat32AsyncFsError::MissingScratch)
    }

    async fn next_cluster(
        &mut self,
        cluster: u32,
    ) -> Result<Option<u32>, Fat32AsyncFsError<D::Error>> {
        self.validate_cluster(cluster)?;
        let offset = u64::from(cluster)
            .checked_mul(4)
            .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        let bytes_per_sector = u64::from(self.boot.bytes_per_sector);
        let fat_sector = u64::from(self.boot.fat_start_sector)
            .checked_add(offset / bytes_per_sector)
            .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        let in_sector = (offset % bytes_per_sector) as usize;

        self.load_sector(fat_sector).await?;
        let bytes = self.loaded_bytes()?;
        let raw = u32::from_le_bytes(
            bytes
                .get(in_sector..in_sector + 4)
                .ok_or(Fat32AsyncFsError::CorruptDirectory)?
                .try_into()
                .map_err(|_| Fat32AsyncFsError::CorruptDirectory)?,
        ) & FAT32_CLUSTER_MASK;

        if raw >= FAT32_END_OF_CHAIN {
            Ok(None)
        } else if raw == FAT32_BAD_CLUSTER {
            Err(Fat32AsyncFsError::BadCluster(cluster))
        } else {
            self.validate_cluster(raw)?;
            Ok(Some(raw))
        }
    }

    async fn advance_dir_cluster(
        &mut self,
        cursor: &mut DirCursor,
    ) -> Result<(), Fat32AsyncFsError<D::Error>> {
        cursor.cluster_hops = cursor.cluster_hops.saturating_add(1);
        if cursor.cluster_hops > self.boot.cluster_count {
            return Err(Fat32AsyncFsError::ClusterLoop);
        }

        match self.next_cluster(cursor.cluster).await? {
            Some(next) => {
                cursor.cluster = next;
                cursor.sector_in_cluster = 0;
                cursor.entry_in_sector = 0;
            }
            None => {
                cursor.end = true;
            }
        }
        Ok(())
    }

    async fn next_raw_directory_record(
        &mut self,
        cursor: &mut DirCursor,
    ) -> Result<Option<[u8; FAT32_ENTRY_SIZE]>, Fat32AsyncFsError<D::Error>> {
        if cursor.end {
            return Ok(None);
        }

        if cursor.sector_in_cluster >= u32::from(self.boot.sectors_per_cluster) {
            self.advance_dir_cluster(cursor).await?;
            if cursor.end {
                return Ok(None);
            }
        }

        let entries_per_sector = usize::from(self.boot.bytes_per_sector) / FAT32_ENTRY_SIZE;
        if entries_per_sector == 0 {
            return Err(Fat32AsyncFsError::CorruptDirectory);
        }

        let sector = self
            .cluster_first_sector(cursor.cluster)?
            .checked_add(u64::from(cursor.sector_in_cluster))
            .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        self.load_sector(sector).await?;

        let index = usize::from(cursor.entry_in_sector);
        let offset = index
            .checked_mul(FAT32_ENTRY_SIZE)
            .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        let mut raw = [0u8; FAT32_ENTRY_SIZE];
        raw.copy_from_slice(
            self.loaded_bytes()?
                .get(offset..offset + FAT32_ENTRY_SIZE)
                .ok_or(Fat32AsyncFsError::CorruptDirectory)?,
        );

        cursor.entry_in_sector += 1;
        if usize::from(cursor.entry_in_sector) >= entries_per_sector {
            cursor.entry_in_sector = 0;
            cursor.sector_in_cluster += 1;
        }

        if raw[0] == FAT32_END_OF_DIRECTORY {
            cursor.end = true;
            return Ok(None);
        }

        Ok(Some(raw))
    }

    fn short_name_checksum(raw_name: &[u8]) -> u8 {
        raw_name
            .iter()
            .fold(0u8, |sum, byte| sum.rotate_right(1).wrapping_add(*byte))
    }

    fn decode_short_name(raw: &[u8; FAT32_ENTRY_SIZE], output: &mut [u8]) -> Option<usize> {
        let mut written = 0usize;
        let mut base = raw[0..8].to_vec_array();
        if base[0] == 0x05 {
            base[0] = 0xe5;
        }

        let base_len = base
            .iter()
            .rposition(|byte| *byte != b' ')
            .map_or(0, |i| i + 1);
        let ext = &raw[8..11];
        let ext_len = ext
            .iter()
            .rposition(|byte| *byte != b' ')
            .map_or(0, |i| i + 1);

        for byte in &base[..base_len] {
            let value = if raw[12] & 0x08 != 0 {
                byte.to_ascii_lowercase()
            } else {
                *byte
            };
            *output.get_mut(written)? = value;
            written += 1;
        }

        if ext_len != 0 {
            *output.get_mut(written)? = b'.';
            written += 1;
            for byte in &ext[..ext_len] {
                let value = if raw[12] & 0x10 != 0 {
                    byte.to_ascii_lowercase()
                } else {
                    *byte
                };
                *output.get_mut(written)? = value;
                written += 1;
            }
        }

        Some(written)
    }

    fn lfn_units(raw: &[u8; FAT32_ENTRY_SIZE]) -> [u16; 13] {
        let offsets = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        let mut units = [0u16; 13];
        for (index, offset) in offsets.into_iter().enumerate() {
            units[index] = u16::from_le_bytes([raw[offset], raw[offset + 1]]);
        }
        units
    }

    fn encode_utf16_name(units: &[u16], output: &mut [u8]) -> Option<usize> {
        let mut written = 0usize;
        for decoded in char::decode_utf16(
            units
                .iter()
                .copied()
                .take_while(|unit| *unit != 0 && *unit != 0xffff),
        ) {
            let ch = decoded.ok()?;
            let mut encoded = [0u8; 4];
            let text = ch.encode_utf8(&mut encoded);
            let end = written.checked_add(text.len())?;
            output
                .get_mut(written..end)?
                .copy_from_slice(text.as_bytes());
            written = end;
        }
        Some(written)
    }

    async fn next_parsed_directory_entry(
        &mut self,
        cursor: &mut DirCursor,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<(DirEntry, ResolvedEntry)>, Fat32AsyncFsError<D::Error>> {
        let mut lfn = [0xffffu16; FAT32_MAX_LFN_UNITS];
        let mut lfn_checksum = None;
        let mut lfn_valid = false;

        loop {
            let Some(raw) = self.next_raw_directory_record(cursor).await? else {
                return Ok(None);
            };

            if raw[0] == FAT32_DELETED_ENTRY {
                lfn_valid = false;
                lfn_checksum = None;
                continue;
            }

            if raw[11] == FAT32_LFN_ATTRIBUTE {
                let sequence = raw[0];
                let ordinal = usize::from(sequence & 0x1f);
                if ordinal == 0 || ordinal > 20 {
                    lfn_valid = false;
                    lfn_checksum = None;
                    continue;
                }
                if sequence & 0x40 != 0 {
                    lfn.fill(0xffff);
                    lfn_checksum = Some(raw[13]);
                    lfn_valid = true;
                }
                if lfn_checksum != Some(raw[13]) {
                    lfn_valid = false;
                }

                let start = (ordinal - 1) * 13;
                let units = Self::lfn_units(&raw);
                for (index, unit) in units.into_iter().enumerate() {
                    if let Some(slot) = lfn.get_mut(start + index) {
                        *slot = unit;
                    } else {
                        lfn_valid = false;
                    }
                }
                continue;
            }

            if raw[11] & FAT32_VOLUME_ATTRIBUTE != 0 {
                lfn_valid = false;
                lfn_checksum = None;
                continue;
            }

            let checksum_matches =
                lfn_valid && lfn_checksum == Some(Self::short_name_checksum(&raw[0..11]));
            let name_len = if checksum_matches {
                Self::encode_utf16_name(&lfn, name_storage).ok_or(Fat32AsyncFsError::NameTooLong)?
            } else {
                Self::decode_short_name(&raw, name_storage).ok_or(Fat32AsyncFsError::NameTooLong)?
            };

            let first_cluster = (u32::from(u16::from_le_bytes([raw[20], raw[21]])) << 16)
                | u32::from(u16::from_le_bytes([raw[26], raw[27]]));
            let len = u64::from(u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]));
            let kind = if raw[11] & FAT32_DIRECTORY_ATTRIBUTE != 0 {
                EntryKind::Directory
            } else {
                EntryKind::File
            };

            let entry = DirEntry {
                kind,
                len,
                modified_unix_seconds: None,
                name_len: name_len as u16,
            };
            let resolved = ResolvedEntry {
                kind,
                first_cluster,
                len,
            };
            return Ok(Some((entry, resolved)));
        }
    }

    fn names_equal(left: &[u8], right: &str) -> bool {
        let right = right.as_bytes();
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    async fn find_in_directory(
        &mut self,
        cluster: u32,
        wanted: &str,
    ) -> Result<ResolvedEntry, Fat32AsyncFsError<D::Error>> {
        self.validate_cluster(cluster)?;
        let mut cursor = DirCursor::new(cluster);
        let mut name_storage = [0u8; FS_NAME_MAX];

        while let Some((entry, resolved)) = self
            .next_parsed_directory_entry(&mut cursor, &mut name_storage)
            .await?
        {
            let name = entry
                .name(&name_storage)
                .ok_or(Fat32AsyncFsError::CorruptDirectory)?;
            if Self::names_equal(name, wanted) {
                return Ok(resolved);
            }
        }

        Err(Fat32AsyncFsError::PathNotFound)
    }

    async fn resolve_path(
        &mut self,
        path: &str,
    ) -> Result<Option<ResolvedEntry>, Fat32AsyncFsError<D::Error>> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(None);
        }

        let mut components = trimmed.split('/').peekable();
        let mut directory_cluster = self.boot.root_cluster;

        while let Some(component) = components.next() {
            if component.is_empty() || component == "." || component == ".." {
                return Err(Fat32AsyncFsError::InvalidPath);
            }

            let entry = self.find_in_directory(directory_cluster, component).await?;
            if components.peek().is_none() {
                return Ok(Some(entry));
            }
            if entry.kind != EntryKind::Directory {
                return Err(Fat32AsyncFsError::NotDirectory);
            }
            self.validate_cluster(entry.first_cluster)?;
            directory_cluster = entry.first_cluster;
        }

        Err(Fat32AsyncFsError::InvalidPath)
    }

    async fn file_cluster_for_position(
        &mut self,
        file: &mut Fat32AsyncFileHandle,
        position: u64,
    ) -> Result<u32, Fat32AsyncFsError<D::Error>> {
        if file.len == 0 {
            return Err(Fat32AsyncFsError::CorruptCluster(file.first_cluster));
        }

        self.validate_cluster(file.first_cluster)?;
        let cluster_size = self.cluster_size();
        let target_index = u32::try_from(position / cluster_size)
            .map_err(|_| Fat32AsyncFsError::PositionOverflow)?;

        let (mut cluster, mut index) = if target_index >= file.current_cluster_index {
            (file.current_cluster, file.current_cluster_index)
        } else {
            (file.first_cluster, 0)
        };

        while index < target_index {
            cluster = self
                .next_cluster(cluster)
                .await?
                .ok_or(Fat32AsyncFsError::CorruptCluster(cluster))?;
            index += 1;
            if index > self.boot.cluster_count {
                return Err(Fat32AsyncFsError::ClusterLoop);
            }
        }

        file.current_cluster = cluster;
        file.current_cluster_index = index;
        Ok(cluster)
    }
}

impl<D> LeaseBound for Fat32AsyncFileSystem<D>
where
    D: OwnedBlockDevice,
{
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

impl<D> AsyncFileSystem for Fat32AsyncFileSystem<D>
where
    D: OwnedBlockDevice,
{
    type Error = Fat32AsyncFsError<D::Error>;
    type File = Fat32AsyncFileHandle;
    type Directory = Fat32AsyncDirectoryHandle;

    fn kind(&self) -> FileSystemKind {
        FileSystemKind::Fat32
    }

    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities {
            read: true,
            write: false,
            directories: true,
            long_names: true,
            flush: false,
            random_seek: false,
            streaming_directory_visit: false,
        }
    }

    async fn stat(&mut self, path: &str) -> Result<FileStat, Self::Error> {
        match self.resolve_path(path).await? {
            None => Ok(FileStat {
                kind: EntryKind::Directory,
                len: 0,
                modified_unix_seconds: None,
            }),
            Some(entry) => Ok(FileStat {
                kind: entry.kind,
                len: entry.len,
                modified_unix_seconds: None,
            }),
        }
    }

    async fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
        let entry = self
            .resolve_path(path)
            .await?
            .ok_or(Fat32AsyncFsError::IsDirectory)?;
        if entry.kind != EntryKind::File {
            return Err(Fat32AsyncFsError::IsDirectory);
        }
        if entry.len != 0 {
            self.validate_cluster(entry.first_cluster)?;
        }

        Ok(Fat32AsyncFileHandle {
            lease: self.lease,
            first_cluster: entry.first_cluster,
            current_cluster: entry.first_cluster,
            current_cluster_index: 0,
            len: entry.len,
            position: 0,
        })
    }

    async fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)
    }

    async fn read(
        &mut self,
        file: &mut Self::File,
        output: &mut [u8],
    ) -> Result<usize, Self::Error> {
        self.ensure_lease(file.lease)?;
        if output.is_empty() || file.position >= file.len {
            return Ok(0);
        }

        let mut written = 0usize;
        let bytes_per_sector = u64::from(self.boot.bytes_per_sector);
        let cluster_size = self.cluster_size();

        while written < output.len() && file.position < file.len {
            let cluster = self.file_cluster_for_position(file, file.position).await?;
            let offset_in_cluster = file.position % cluster_size;
            let sector_in_cluster = offset_in_cluster / bytes_per_sector;
            let offset_in_sector = (offset_in_cluster % bytes_per_sector) as usize;
            let sector = self
                .cluster_first_sector(cluster)?
                .checked_add(sector_in_cluster)
                .ok_or(Fat32AsyncFsError::PositionOverflow)?;

            self.load_sector(sector).await?;
            let remaining_file = usize::try_from(file.len - file.position).unwrap_or(usize::MAX);
            let available_sector = usize::from(self.boot.bytes_per_sector) - offset_in_sector;
            let count = (output.len() - written)
                .min(remaining_file)
                .min(available_sector);

            output[written..written + count].copy_from_slice(
                self.loaded_bytes()?
                    .get(offset_in_sector..offset_in_sector + count)
                    .ok_or(Fat32AsyncFsError::PositionOverflow)?,
            );
            written += count;
            file.position = file
                .position
                .checked_add(count as u64)
                .ok_or(Fat32AsyncFsError::PositionOverflow)?;
        }

        Ok(written)
    }

    async fn seek_absolute(
        &mut self,
        file: &mut Self::File,
        position: u64,
    ) -> Result<(), Self::Error> {
        self.ensure_lease(file.lease)?;
        file.position = position;
        if position == 0 {
            file.current_cluster = file.first_cluster;
            file.current_cluster_index = 0;
        }
        Ok(())
    }

    async fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
        let cluster = match self.resolve_path(path).await? {
            None => self.boot.root_cluster,
            Some(entry) => {
                if entry.kind != EntryKind::Directory {
                    return Err(Fat32AsyncFsError::NotDirectory);
                }
                self.validate_cluster(entry.first_cluster)?;
                entry.first_cluster
            }
        };

        Ok(Fat32AsyncDirectoryHandle {
            lease: self.lease,
            cursor: DirCursor::new(cluster),
        })
    }

    async fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Self::Error> {
        self.ensure_lease(directory.lease)?;
        self.next_parsed_directory_entry(&mut directory.cursor, name_storage)
            .await
            .map(|entry| entry.map(|(entry, _)| entry))
    }

    async fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error> {
        self.ensure_lease(directory.lease)
    }
}

trait ToVecArray {
    fn to_vec_array(&self) -> [u8; 8];
}

impl ToVecArray for [u8] {
    fn to_vec_array(&self) -> [u8; 8] {
        let mut output = [0u8; 8];
        output.copy_from_slice(&self[..8]);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll};
    use pajoniiir_media_fs::AsyncFileSystem;
    use pajoniiir_media_session::{MediaGeneration, MediaSourceId};

    const SECTOR_SIZE: usize = 512;
    const SECTORS: usize = 16;

    fn lease() -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(7).unwrap(),
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestError {
        InvalidTransfer,
    }

    struct MemoryDevice {
        bytes: [u8; SECTOR_SIZE * SECTORS],
        reads: u32,
    }

    impl MemoryDevice {
        fn new(bytes: [u8; SECTOR_SIZE * SECTORS]) -> Self {
            Self { bytes, reads: 0 }
        }
    }

    impl OwnedBlockDevice for MemoryDevice {
        type Error = TestError;
        type Buffer = [u8; SECTOR_SIZE];

        fn geometry(&self) -> BlockGeometry {
            BlockGeometry {
                block_size: SECTOR_SIZE as u32,
                block_count: SECTORS as u64,
            }
        }

        async fn read_blocks_owned(
            &mut self,
            first_block: u64,
            block_count: u32,
            mut buffer: Self::Buffer,
        ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
            if block_count != 1 || first_block >= SECTORS as u64 {
                return Err((TestError::InvalidTransfer, buffer));
            }
            let start = first_block as usize * SECTOR_SIZE;
            buffer.copy_from_slice(&self.bytes[start..start + SECTOR_SIZE]);
            self.reads += 1;
            Ok(buffer)
        }
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = core::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    fn put_u32(sector: &mut [u8], offset: usize, value: u32) {
        sector[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn set_fat_entry(image: &mut [u8], cluster: u32, value: u32) {
        let fat = &mut image[SECTOR_SIZE..SECTOR_SIZE * 2];
        put_u32(fat, cluster as usize * 4, value);
    }

    fn short_entry(name: &[u8; 11], attributes: u8, cluster: u32, len: u32) -> [u8; 32] {
        let mut entry = [0u8; 32];
        entry[..11].copy_from_slice(name);
        entry[11] = attributes;
        entry[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        entry[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        entry[28..32].copy_from_slice(&len.to_le_bytes());
        entry
    }

    fn lfn_entry(name: &str, checksum: u8) -> [u8; 32] {
        let mut entry = [0xffu8; 32];
        entry[0] = 0x41;
        entry[11] = FAT32_LFN_ATTRIBUTE;
        entry[12] = 0;
        entry[13] = checksum;
        entry[26] = 0;
        entry[27] = 0;

        let offsets = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        let mut units = [0xffffu16; 13];
        for (index, unit) in name.encode_utf16().enumerate() {
            units[index] = unit;
        }
        let terminator = name.encode_utf16().count();
        if terminator < units.len() {
            units[terminator] = 0;
        }
        for (unit, offset) in units.into_iter().zip(offsets) {
            entry[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
        entry
    }

    fn fixture() -> ([u8; SECTOR_SIZE * SECTORS], Fat32BootInfo) {
        let mut image = [0u8; SECTOR_SIZE * SECTORS];

        set_fat_entry(&mut image, 0, 0x0fff_fff8);
        set_fat_entry(&mut image, 1, 0xffff_ffff);
        set_fat_entry(&mut image, 2, 0x0fff_ffff);
        set_fat_entry(&mut image, 3, 5);
        set_fat_entry(&mut image, 4, 0x0fff_ffff);
        set_fat_entry(&mut image, 5, 0x0fff_ffff);
        set_fat_entry(&mut image, 6, 0x0fff_ffff);
        set_fat_entry(&mut image, 7, 0x0fff_ffff);

        let root = SECTOR_SIZE * 2;
        let track = short_entry(b"TRACK   WAV", 0x20, 3, 700);
        image[root..root + 32].copy_from_slice(&track);
        let music = short_entry(b"MUSIC      ", FAT32_DIRECTORY_ATTRIBUTE, 4, 0);
        image[root + 32..root + 64].copy_from_slice(&music);

        let alias = *b"LONGTR~1WAV";
        let checksum = Fat32AsyncFileSystem::<MemoryDevice>::short_name_checksum(&alias);
        let lfn = lfn_entry("Long Track.wav", checksum);
        image[root + 64..root + 96].copy_from_slice(&lfn);
        let long_file = short_entry(&alias, 0x20, 7, 5);
        image[root + 96..root + 128].copy_from_slice(&long_file);
        image[root + 128] = 0;

        let subdir = SECTOR_SIZE * 4;
        let nested = short_entry(b"DEEP    WAV", 0x20, 6, 4);
        image[subdir..subdir + 32].copy_from_slice(&nested);
        image[subdir + 32] = 0;

        image[SECTOR_SIZE * 3..SECTOR_SIZE * 4].fill(b'A');
        image[SECTOR_SIZE * 5..SECTOR_SIZE * 6].fill(b'B');
        image[SECTOR_SIZE * 6..SECTOR_SIZE * 6 + 4].copy_from_slice(b"DEEP");
        image[SECTOR_SIZE * 7..SECTOR_SIZE * 7 + 5].copy_from_slice(b"LONG!");

        let boot = Fat32BootInfo {
            bytes_per_sector: SECTOR_SIZE as u16,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            fat_count: 1,
            sectors_per_fat: 1,
            total_sectors: SECTORS as u32,
            root_cluster: 2,
            fs_info_sector: 1,
            backup_boot_sector: 0,
            fat_start_sector: 1,
            data_start_sector: 2,
            cluster_count: 8,
            max_cluster: 9,
            volume_id: 0x1234_5678,
        };

        (image, boot)
    }

    fn filesystem() -> Fat32AsyncFileSystem<MemoryDevice> {
        let (image, boot) = fixture();
        match Fat32AsyncFileSystem::new(MemoryDevice::new(image), [0u8; SECTOR_SIZE], boot, lease())
        {
            Ok(fs) => fs,
            Err(_) => panic!("fixture mount failed"),
        }
    }

    #[test]
    fn opens_and_reads_file_across_fragmented_cluster_chain() {
        let mut fs = filesystem();
        let mut file = block_on_ready(AsyncFileSystem::open_read(&mut fs, "/TRACK.WAV")).unwrap();
        let mut output = [0u8; 700];

        assert_eq!(
            block_on_ready(AsyncFileSystem::read(&mut fs, &mut file, &mut output)).unwrap(),
            700
        );
        assert!(output[..512].iter().all(|byte| *byte == b'A'));
        assert!(output[512..].iter().all(|byte| *byte == b'B'));
        assert_eq!(file.position(), 700);
    }

    #[test]
    fn resolves_nested_directory_and_file() {
        let mut fs = filesystem();
        let stat = block_on_ready(AsyncFileSystem::stat(&mut fs, "/MUSIC/DEEP.WAV")).unwrap();
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.len, 4);

        let mut file =
            block_on_ready(AsyncFileSystem::open_read(&mut fs, "/music/deep.wav")).unwrap();
        let mut output = [0u8; 4];
        block_on_ready(AsyncFileSystem::read_exact(&mut fs, &mut file, &mut output)).unwrap();
        assert_eq!(&output, b"DEEP");
    }

    #[test]
    fn directory_iteration_decodes_lfn_without_allocation() {
        let mut fs = filesystem();
        let mut directory = block_on_ready(AsyncFileSystem::open_directory(&mut fs, "/")).unwrap();
        let mut name = [0u8; FS_NAME_MAX];

        let first = block_on_ready(AsyncFileSystem::next_entry(
            &mut fs,
            &mut directory,
            &mut name,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(first.name(&name), Some(b"TRACK.WAV".as_slice()));

        let second = block_on_ready(AsyncFileSystem::next_entry(
            &mut fs,
            &mut directory,
            &mut name,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(second.name(&name), Some(b"MUSIC".as_slice()));

        let third = block_on_ready(AsyncFileSystem::next_entry(
            &mut fs,
            &mut directory,
            &mut name,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(third.name(&name), Some(b"Long Track.wav".as_slice()));
    }

    #[test]
    fn long_filename_lookup_is_case_insensitive_for_ascii() {
        let mut fs = filesystem();
        let mut file =
            block_on_ready(AsyncFileSystem::open_read(&mut fs, "/long track.wav")).unwrap();
        let mut output = [0u8; 5];

        block_on_ready(AsyncFileSystem::read_exact(&mut fs, &mut file, &mut output)).unwrap();
        assert_eq!(&output, b"LONG!");
    }

    #[test]
    fn stale_handles_fail_closed_after_generation_change() {
        let mut fs = filesystem();
        let mut file = block_on_ready(AsyncFileSystem::open_read(&mut fs, "/TRACK.WAV")).unwrap();
        fs.lease = MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(8).unwrap(),
        };

        assert!(matches!(
            block_on_ready(AsyncFileSystem::read(&mut fs, &mut file, &mut [0u8; 1])),
            Err(Fat32AsyncFsError::StaleLease)
        ));
    }

    #[test]
    fn constructor_returns_owned_scratch_on_geometry_failure() {
        let (image, mut boot) = fixture();
        boot.bytes_per_sector = 1_024;
        let result =
            Fat32AsyncFileSystem::new(MemoryDevice::new(image), [0u8; SECTOR_SIZE], boot, lease());

        match result {
            Err((Fat32AsyncMountError::SectorSizeMismatch { device, boot }, _, scratch)) => {
                assert_eq!(device, 512);
                assert_eq!(boot, 1_024);
                assert_eq!(scratch.len(), SECTOR_SIZE);
            }
            _ => panic!("unexpected mount result"),
        }
    }
}
