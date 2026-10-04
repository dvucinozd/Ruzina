use pajoniiir_media_fs::{
    AsyncFileSystem, DirEntry, DirectoryHandle, EntryKind, FS_NAME_MAX, FS_PATH_MAX, FileHandle,
    FileStat, FileSystemCapabilities, FileSystemKind, LeaseBound,
};
use pajoniiir_media_io::OwnedBlockDevice;
use pajoniiir_media_session::MediaLease;

use crate::async_fs::{ExFatAsyncIoError, ExFatAsyncVolume};
use crate::entry_set::{ExFatEntrySetError, ExFatFileEntrySetParser, ExFatParsedFileEntry};

const EXFAT_FILE_ENTRY: u8 = 0x85;

#[derive(Debug)]
pub enum ExFatAsyncFsError<E> {
    StaleLease,
    Io(ExFatAsyncIoError<E>),
    EntrySet(ExFatEntrySetError),
    InvalidPath,
    PathNotFound,
    NotDirectory,
    IsDirectory,
    CorruptDirectory,
    UnexpectedEof,
    ClusterLoop,
    PositionOverflow,
    BeyondEnd,
}

impl<E> From<ExFatAsyncIoError<E>> for ExFatAsyncFsError<E> {
    fn from(error: ExFatAsyncIoError<E>) -> Self {
        Self::Io(error)
    }
}

impl<E> From<ExFatEntrySetError> for ExFatAsyncFsError<E> {
    fn from(error: ExFatEntrySetError) -> Self {
        Self::EntrySet(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExFatAllocationCursor {
    first_cluster: u32,
    current_cluster: u32,
    current_cluster_index: u32,
    data_length: Option<u64>,
    position: u64,
    no_fat_chain: bool,
    cluster_hops: u32,
}

impl ExFatAllocationCursor {
    const fn new(first_cluster: u32, data_length: Option<u64>, no_fat_chain: bool) -> Self {
        Self {
            first_cluster,
            current_cluster: first_cluster,
            current_cluster_index: 0,
            data_length,
            position: 0,
            no_fat_chain,
            cluster_hops: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatAsyncFileHandle {
    lease: MediaLease,
    cursor: ExFatAllocationCursor,
    valid_data_length: u64,
}

impl LeaseBound for ExFatAsyncFileHandle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

impl FileHandle for ExFatAsyncFileHandle {
    fn len(&self) -> u64 {
        self.cursor.data_length.unwrap_or(0)
    }

    fn position(&self) -> u64 {
        self.cursor.position
    }

    fn set_position(&mut self, position: u64) {
        self.cursor.position = position;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExFatAsyncDirectoryHandle {
    lease: MediaLease,
    cursor: ExFatAllocationCursor,
    ended: bool,
}

impl LeaseBound for ExFatAsyncDirectoryHandle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}

impl DirectoryHandle for ExFatAsyncDirectoryHandle {}

impl<D> ExFatAsyncVolume<D>
where
    D: OwnedBlockDevice,
{
    fn fs_ensure_lease(&self, lease: MediaLease) -> Result<(), ExFatAsyncFsError<D::Error>> {
        if lease == self.lease_value() {
            Ok(())
        } else {
            Err(ExFatAsyncFsError::StaleLease)
        }
    }

    fn fs_root_cursor(&self) -> ExFatAllocationCursor {
        ExFatAllocationCursor::new(self.boot_info().root_directory_cluster, None, false)
    }

    fn fs_cursor_from_entry(entry: ExFatParsedFileEntry) -> ExFatAllocationCursor {
        ExFatAllocationCursor::new(
            entry.first_cluster,
            Some(entry.data_length),
            entry.no_fat_chain,
        )
    }

    fn fs_validate_entry(
        &self,
        entry: ExFatParsedFileEntry,
    ) -> Result<(), ExFatAsyncFsError<D::Error>> {
        if entry.data_length == 0 {
            return Ok(());
        }

        self.cluster_first_sector(entry.first_cluster)?;
        if !entry.no_fat_chain {
            return Ok(());
        }

        let cluster_size = self.boot_info().cluster_size_bytes();
        if cluster_size == 0 {
            return Err(ExFatAsyncFsError::PositionOverflow);
        }
        let clusters = entry.data_length.div_ceil(cluster_size);
        let last_offset = clusters
            .checked_sub(1)
            .ok_or(ExFatAsyncFsError::PositionOverflow)?;
        let last_offset =
            u32::try_from(last_offset).map_err(|_| ExFatAsyncFsError::PositionOverflow)?;
        let last_cluster = entry
            .first_cluster
            .checked_add(last_offset)
            .ok_or(ExFatAsyncFsError::PositionOverflow)?;
        self.cluster_first_sector(last_cluster)?;
        Ok(())
    }

    async fn fs_cluster_for_index(
        &mut self,
        cursor: &mut ExFatAllocationCursor,
        target_index: u32,
    ) -> Result<Option<u32>, ExFatAsyncFsError<D::Error>> {
        if target_index >= self.boot_info().cluster_count {
            return Err(ExFatAsyncFsError::PositionOverflow);
        }

        if cursor.no_fat_chain {
            let cluster = cursor
                .first_cluster
                .checked_add(target_index)
                .ok_or(ExFatAsyncFsError::PositionOverflow)?;
            self.cluster_first_sector(cluster)?;
            cursor.current_cluster = cluster;
            cursor.current_cluster_index = target_index;
            return Ok(Some(cluster));
        }

        if target_index < cursor.current_cluster_index {
            cursor.current_cluster = cursor.first_cluster;
            cursor.current_cluster_index = 0;
            cursor.cluster_hops = 0;
        }

        while cursor.current_cluster_index < target_index {
            let Some(next) = self.next_cluster(cursor.current_cluster).await? else {
                return Ok(None);
            };
            cursor.current_cluster = next;
            cursor.current_cluster_index = cursor.current_cluster_index.saturating_add(1);
            cursor.cluster_hops = cursor.cluster_hops.saturating_add(1);
            if cursor.cluster_hops > self.boot_info().cluster_count {
                return Err(ExFatAsyncFsError::ClusterLoop);
            }
        }

        Ok(Some(cursor.current_cluster))
    }

    async fn fs_read_allocation(
        &mut self,
        cursor: &mut ExFatAllocationCursor,
        output: &mut [u8],
    ) -> Result<usize, ExFatAsyncFsError<D::Error>> {
        if output.is_empty() {
            return Ok(0);
        }
        if let Some(data_length) = cursor.data_length
            && cursor.position >= data_length
        {
            return Ok(0);
        }

        let boot = self.boot_info();
        let cluster_size = boot.cluster_size_bytes();
        let sector_size = u64::from(boot.bytes_per_sector);
        if cluster_size == 0 || sector_size == 0 {
            return Err(ExFatAsyncFsError::PositionOverflow);
        }

        let mut written = 0usize;
        while written < output.len() {
            if let Some(data_length) = cursor.data_length
                && cursor.position >= data_length
            {
                break;
            }

            let cluster_index = cursor.position / cluster_size;
            let cluster_index =
                u32::try_from(cluster_index).map_err(|_| ExFatAsyncFsError::PositionOverflow)?;
            let Some(cluster) = self.fs_cluster_for_index(cursor, cluster_index).await? else {
                if cursor.data_length.is_some() {
                    return Err(ExFatAsyncFsError::UnexpectedEof);
                }
                break;
            };

            let in_cluster = cursor.position % cluster_size;
            let sector_in_cluster = in_cluster / sector_size;
            let in_sector = usize::try_from(in_cluster % sector_size)
                .map_err(|_| ExFatAsyncFsError::PositionOverflow)?;
            let sector = self
                .cluster_first_sector(cluster)?
                .checked_add(sector_in_cluster)
                .ok_or(ExFatAsyncFsError::PositionOverflow)?;
            self.load_sector(sector).await?;

            let sector_bytes = self.loaded_bytes()?;
            let sector_remaining = sector_bytes
                .len()
                .checked_sub(in_sector)
                .ok_or(ExFatAsyncFsError::PositionOverflow)?;
            let cluster_remaining = usize::try_from(cluster_size - in_cluster)
                .map_err(|_| ExFatAsyncFsError::PositionOverflow)?;
            let mut take = (output.len() - written)
                .min(sector_remaining)
                .min(cluster_remaining);

            if let Some(data_length) = cursor.data_length {
                let remaining = usize::try_from(
                    data_length
                        .saturating_sub(cursor.position)
                        .min(usize::MAX as u64),
                )
                .map_err(|_| ExFatAsyncFsError::PositionOverflow)?;
                take = take.min(remaining);
            }
            if take == 0 {
                break;
            }

            output[written..written + take]
                .copy_from_slice(&sector_bytes[in_sector..in_sector + take]);
            written += take;
            cursor.position = cursor
                .position
                .checked_add(take as u64)
                .ok_or(ExFatAsyncFsError::PositionOverflow)?;
        }

        Ok(written)
    }

    async fn fs_next_raw_directory_record(
        &mut self,
        directory: &mut ExFatAsyncDirectoryHandle,
    ) -> Result<Option<[u8; 32]>, ExFatAsyncFsError<D::Error>> {
        if directory.ended {
            return Ok(None);
        }

        let mut raw = [0u8; 32];
        let read = self.fs_read_allocation(&mut directory.cursor, &mut raw).await?;
        if read == 0 {
            directory.ended = true;
            return Ok(None);
        }
        if read != raw.len() {
            return Err(ExFatAsyncFsError::CorruptDirectory);
        }
        if raw[0] == 0 {
            directory.ended = true;
            return Ok(None);
        }
        Ok(Some(raw))
    }

    async fn fs_next_parsed_directory_entry(
        &mut self,
        directory: &mut ExFatAsyncDirectoryHandle,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<ExFatParsedFileEntry>, ExFatAsyncFsError<D::Error>> {
        loop {
            let Some(primary) = self.fs_next_raw_directory_record(directory).await? else {
                return Ok(None);
            };
            if primary[0] != EXFAT_FILE_ENTRY {
                continue;
            }

            let mut parser = ExFatFileEntrySetParser::begin(&primary)?;
            let secondary_count = parser.expected_secondaries();
            for _ in 0..secondary_count {
                let secondary = self
                    .fs_next_raw_directory_record(directory)
                    .await?
                    .ok_or(ExFatAsyncFsError::CorruptDirectory)?;
                parser.push_secondary(&secondary)?;
            }

            let entry = parser.finish(name_storage)?;
            self.fs_validate_entry(entry)?;
            return Ok(Some(entry));
        }
    }

    async fn fs_find_in_directory(
        &mut self,
        cursor: ExFatAllocationCursor,
        wanted: &str,
    ) -> Result<ExFatParsedFileEntry, ExFatAsyncFsError<D::Error>> {
        let mut directory = ExFatAsyncDirectoryHandle {
            lease: self.lease_value(),
            cursor,
            ended: false,
        };
        let mut name_storage = [0u8; FS_NAME_MAX];

        while let Some(entry) = self
            .fs_next_parsed_directory_entry(&mut directory, &mut name_storage)
            .await?
        {
            let name = entry
                .name(&name_storage)
                .ok_or(ExFatAsyncFsError::CorruptDirectory)?;
            if names_equal(name, wanted) {
                return Ok(entry);
            }
        }

        Err(ExFatAsyncFsError::PathNotFound)
    }

    async fn fs_resolve_path(
        &mut self,
        path: &str,
    ) -> Result<Option<ExFatParsedFileEntry>, ExFatAsyncFsError<D::Error>> {
        if path.len() > FS_PATH_MAX || path.as_bytes().contains(&0) {
            return Err(ExFatAsyncFsError::InvalidPath);
        }

        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(None);
        }

        let mut components = trimmed.split('/').peekable();
        let mut directory = self.fs_root_cursor();

        while let Some(component) = components.next() {
            if component.is_empty() || matches!(component, "." | "..") {
                return Err(ExFatAsyncFsError::InvalidPath);
            }

            let entry = self.fs_find_in_directory(directory, component).await?;
            if components.peek().is_none() {
                return Ok(Some(entry));
            }
            if entry.kind != EntryKind::Directory {
                return Err(ExFatAsyncFsError::NotDirectory);
            }
            directory = Self::fs_cursor_from_entry(entry);
        }

        Err(ExFatAsyncFsError::InvalidPath)
    }
}

impl<D> AsyncFileSystem for ExFatAsyncVolume<D>
where
    D: OwnedBlockDevice,
{
    type Error = ExFatAsyncFsError<D::Error>;
    type File = ExFatAsyncFileHandle;
    type Directory = ExFatAsyncDirectoryHandle;

    fn kind(&self) -> FileSystemKind {
        FileSystemKind::ExFat
    }

    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities {
            read: true,
            write: false,
            directories: true,
            long_names: true,
            flush: false,
            random_seek: false,
            streaming_directory_visit: true,
        }
    }

    async fn stat(&mut self, path: &str) -> Result<FileStat, Self::Error> {
        let Some(entry) = self.fs_resolve_path(path).await? else {
            return Ok(FileStat {
                kind: EntryKind::Directory,
                len: 0,
                modified_unix_seconds: None,
            });
        };

        Ok(FileStat {
            kind: entry.kind,
            len: entry.data_length,
            modified_unix_seconds: None,
        })
    }

    async fn open_read(&mut self, path: &str) -> Result<Self::File, Self::Error> {
        let entry = self
            .fs_resolve_path(path)
            .await?
            .ok_or(ExFatAsyncFsError::IsDirectory)?;
        if entry.kind == EntryKind::Directory {
            return Err(ExFatAsyncFsError::IsDirectory);
        }

        Ok(ExFatAsyncFileHandle {
            lease: self.lease_value(),
            cursor: Self::fs_cursor_from_entry(entry),
            valid_data_length: entry.valid_data_length,
        })
    }

    async fn close_file(&mut self, file: Self::File) -> Result<(), Self::Error> {
        self.fs_ensure_lease(file.lease)
    }

    async fn read(
        &mut self,
        file: &mut Self::File,
        output: &mut [u8],
    ) -> Result<usize, Self::Error> {
        self.fs_ensure_lease(file.lease)?;
        if output.is_empty() {
            return Ok(0);
        }

        let data_length = file.len();
        if file.cursor.position >= data_length {
            return Ok(0);
        }
        let total = usize::try_from(
            data_length
                .saturating_sub(file.cursor.position)
                .min(output.len() as u64),
        )
        .map_err(|_| ExFatAsyncFsError::PositionOverflow)?;

        let initialized = if file.cursor.position < file.valid_data_length {
            usize::try_from(
                file.valid_data_length
                    .saturating_sub(file.cursor.position)
                    .min(total as u64),
            )
            .map_err(|_| ExFatAsyncFsError::PositionOverflow)?
        } else {
            0
        };

        if initialized != 0 {
            let read = self
                .fs_read_allocation(&mut file.cursor, &mut output[..initialized])
                .await?;
            if read != initialized {
                return Err(ExFatAsyncFsError::UnexpectedEof);
            }
        }

        if initialized < total {
            output[initialized..total].fill(0);
            file.cursor.position = file
                .cursor
                .position
                .checked_add((total - initialized) as u64)
                .ok_or(ExFatAsyncFsError::PositionOverflow)?;
        }

        Ok(total)
    }

    async fn seek_absolute(
        &mut self,
        file: &mut Self::File,
        position: u64,
    ) -> Result<(), Self::Error> {
        self.fs_ensure_lease(file.lease)?;
        if position > file.len() {
            return Err(ExFatAsyncFsError::BeyondEnd);
        }
        file.cursor.position = position;
        Ok(())
    }

    async fn open_directory(&mut self, path: &str) -> Result<Self::Directory, Self::Error> {
        let cursor = match self.fs_resolve_path(path).await? {
            None => self.fs_root_cursor(),
            Some(entry) => {
                if entry.kind != EntryKind::Directory {
                    return Err(ExFatAsyncFsError::NotDirectory);
                }
                Self::fs_cursor_from_entry(entry)
            }
        };

        Ok(ExFatAsyncDirectoryHandle {
            lease: self.lease_value(),
            cursor,
            ended: false,
        })
    }

    async fn next_entry(
        &mut self,
        directory: &mut Self::Directory,
        name_storage: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Self::Error> {
        self.fs_ensure_lease(directory.lease)?;
        let Some(entry) = self
            .fs_next_parsed_directory_entry(directory, name_storage)
            .await?
        else {
            return Ok(None);
        };

        Ok(Some(DirEntry {
            kind: entry.kind,
            len: entry.data_length,
            modified_unix_seconds: None,
            name_len: entry.name_len,
        }))
    }

    async fn close_directory(&mut self, directory: Self::Directory) -> Result<(), Self::Error> {
        self.fs_ensure_lease(directory.lease)
    }
}

fn names_equal(left: &[u8], right: &str) -> bool {
    let right = right.as_bytes();
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            if left.is_ascii() && right.is_ascii() {
                left.eq_ignore_ascii_case(right)
            } else {
                left == right
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll};
    use pajoniiir_media_block::BlockGeometry;
    use pajoniiir_media_session::{MediaGeneration, MediaSourceId};
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

    #[derive(Debug)]
    struct MemoryOwnedDevice {
        bytes: [u8; 64 * 512],
    }

    impl OwnedBlockDevice for MemoryOwnedDevice {
        type Error = ();
        type Buffer = [u8; 512];

        fn geometry(&self) -> BlockGeometry {
            BlockGeometry {
                block_size: 512,
                block_count: 64,
            }
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
                return Err(((), buffer));
            }
            let start = first_block as usize * 512;
            buffer.copy_from_slice(&self.bytes[start..start + buffer.len()]);
            Ok(buffer)
        }
    }

    fn lease() -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        }
    }

    fn boot() -> crate::ExFatAsyncBootInfo {
        crate::ExFatAsyncBootInfo {
            partition_offset: 0,
            volume_length: 64,
            fat_offset: 24,
            fat_length: 1,
            cluster_heap_offset: 25,
            cluster_count: 16,
            root_directory_cluster: 2,
            volume_serial: 0x1234_5678,
            file_system_revision: 0x0100,
            volume_flags: 0,
            bytes_per_sector: 512,
            sectors_per_cluster: 1,
            number_of_fats: 1,
            percent_in_use: 0,
        }
    }

    fn set_checksum(entries: &mut [[u8; 32]]) {
        let mut checksum = 0u16;
        for (entry_index, entry) in entries.iter().enumerate() {
            for (byte_index, byte) in entry.iter().copied().enumerate() {
                if entry_index == 0 && matches!(byte_index, 2 | 3) {
                    continue;
                }
                checksum = checksum.rotate_right(1).wrapping_add(u16::from(byte));
            }
        }
        entries[0][2..4].copy_from_slice(&checksum.to_le_bytes());
    }

    fn file_entry_set(
        name: &str,
        first_cluster: u32,
        data_length: u64,
        valid_data_length: u64,
        no_fat_chain: bool,
    ) -> [[u8; 32]; 3] {
        let units: std::vec::Vec<u16> = name.encode_utf16().collect();
        assert!(units.len() <= 15);

        let mut entries = [[0u8; 32]; 3];
        entries[0][0] = 0x85;
        entries[0][1] = 2;

        entries[1][0] = 0xc0;
        entries[1][1] = 0x01 | if no_fat_chain { 0x02 } else { 0 };
        entries[1][3] = units.len() as u8;
        entries[1][8..16].copy_from_slice(&valid_data_length.to_le_bytes());
        entries[1][20..24].copy_from_slice(&first_cluster.to_le_bytes());
        entries[1][24..32].copy_from_slice(&data_length.to_le_bytes());

        entries[2][0] = 0xc1;
        for (index, unit) in units.into_iter().enumerate() {
            let offset = 2 + index * 2;
            entries[2][offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }

        set_checksum(&mut entries);
        entries
    }

    fn filesystem() -> ExFatAsyncVolume<MemoryOwnedDevice> {
        let mut bytes = [0u8; 64 * 512];

        let fat = 24 * 512;
        bytes[fat + 2 * 4..fat + 2 * 4 + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        bytes[fat + 9 * 4..fat + 9 * 4 + 4].copy_from_slice(&11u32.to_le_bytes());
        bytes[fat + 11 * 4..fat + 11 * 4 + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());

        let root = 25 * 512;
        let contiguous = file_entry_set("contig.bin", 7, 600, 520, true);
        let chained = file_entry_set("chain.bin", 9, 600, 600, false);
        for (index, entry) in contiguous.into_iter().chain(chained).enumerate() {
            let offset = root + index * 32;
            bytes[offset..offset + 32].copy_from_slice(&entry);
        }
        bytes[root + 6 * 32] = 0;

        bytes[30 * 512..31 * 512].fill(b'A');
        bytes[31 * 512..32 * 512].fill(b'B');
        bytes[32 * 512..33 * 512].fill(b'C');
        bytes[34 * 512..35 * 512].fill(b'D');

        let scratch = [0u8; 512];
        ExFatAsyncVolume::new(MemoryOwnedDevice { bytes }, scratch, boot(), lease()).unwrap()
    }

    #[test]
    fn reads_contiguous_stream_and_zero_fills_uninitialized_tail() {
        let mut filesystem = filesystem();
        let mut file =
            block_on_ready(AsyncFileSystem::open_read(&mut filesystem, "/CONTIG.BIN")).unwrap();
        let mut output = [0u8; 600];

        assert_eq!(
            block_on_ready(AsyncFileSystem::read(
                &mut filesystem,
                &mut file,
                &mut output
            )),
            Ok(600)
        );
        assert!(output[..512].iter().all(|byte| *byte == b'A'));
        assert!(output[512..520].iter().all(|byte| *byte == b'B'));
        assert!(output[520..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn follows_fat_chain_for_fragmented_stream() {
        let mut filesystem = filesystem();
        let mut file =
            block_on_ready(AsyncFileSystem::open_read(&mut filesystem, "/chain.bin")).unwrap();
        let mut output = [0u8; 600];

        assert_eq!(
            block_on_ready(AsyncFileSystem::read(
                &mut filesystem,
                &mut file,
                &mut output
            )),
            Ok(600)
        );
        assert!(output[..512].iter().all(|byte| *byte == b'C'));
        assert!(output[512..].iter().all(|byte| *byte == b'D'));
    }

    #[test]
    fn directory_cursor_streams_file_entry_sets() {
        let mut filesystem = filesystem();
        let mut directory =
            block_on_ready(AsyncFileSystem::open_directory(&mut filesystem, "/")).unwrap();
        let mut name_storage = [0u8; FS_NAME_MAX];

        let first = block_on_ready(AsyncFileSystem::next_entry(
            &mut filesystem,
            &mut directory,
            &mut name_storage,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(first.name(&name_storage), Some(b"contig.bin".as_slice()));

        let second = block_on_ready(AsyncFileSystem::next_entry(
            &mut filesystem,
            &mut directory,
            &mut name_storage,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(second.name(&name_storage), Some(b"chain.bin".as_slice()));

        assert!(
            block_on_ready(AsyncFileSystem::next_entry(
                &mut filesystem,
                &mut directory,
                &mut name_storage
            ))
            .unwrap()
            .is_none()
        );
    }
}
