//! Diagnostic shared Rust partition/exFAT read path. No FatFs, writes or heap.
#![forbid(unsafe_code)]
use ruzina_media_block::BlockRange;
use ruzina_media_exfat::{ExFatAsyncVolume, parse_exfat_boot_sector};
use ruzina_media_fs::{AsyncFileSystem, FileHandle};
use ruzina_media_io::{AsyncPartitionDevice, OwnedBlockDevice};
use ruzina_media_partition::{
    GptEntryStream, PartitionLayout, PartitionScanResult, parse_gpt_header, scan_mbr_or_superfloppy,
};
use ruzina_media_session::MediaLease;
use sha2::{Digest, Sha256};

pub const PATH: &str = "/RUZINA_TEST/Čitanje i seek 64GB.bin";
pub const SIZE: u32 = 8 * 1024 * 1024 + 513;
pub const SHA: [u8; 32] = [
    0x1c, 0xb1, 0x88, 0x15, 0x70, 0x2b, 0x14, 0xff, 0x92, 0xad, 0x02, 0xf4, 0x97, 0x6a, 0x16, 0x24,
    0x81, 0xd8, 0xd9, 0x1b, 0xeb, 0x42, 0x9b, 0xa5, 0x60, 0x7b, 0x13, 0x1d, 0x4a, 0xb2, 0x81, 0xde,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Stale,
    Read,
    Partition,
    Boot,
    Mount,
    Checksum,
    Open,
    Size,
    Content,
    Eof,
    Seek,
    Close,
}
pub type Volume<D> = ExFatAsyncVolume<AsyncPartitionDevice<D>>;

/// Retain the client outside a mount future so cancellation never loses its
/// pending ticket. Drop the borrowed volume before calling client recovery.
pub struct ReadBorrow<'a, D>(&'a mut D);
impl<D: OwnedBlockDevice> OwnedBlockDevice for ReadBorrow<'_, D> {
    type Error = D::Error;
    type Buffer = D::Buffer;
    fn geometry(&self) -> ruzina_media_block::BlockGeometry {
        self.0.geometry()
    }
    async fn read_blocks_owned(
        &mut self,
        first: u64,
        count: u32,
        buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        self.0.read_blocks_owned(first, count, buffer).await
    }
}

fn current(is_current: &impl Fn() -> bool) -> Result<(), Error> {
    if is_current() {
        Ok(())
    } else {
        Err(Error::Stale)
    }
}

async fn sector<D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    device: &mut D,
    lba: u64,
    buffer: [u8; 512],
    is_current: &impl Fn() -> bool,
) -> Result<[u8; 512], Error> {
    current(is_current)?;
    let buffer = device
        .read_blocks_owned(lba, 1, buffer)
        .await
        .map_err(|_| Error::Read)?;
    current(is_current)?;
    Ok(buffer)
}

/// Caller owns the device/lease. A canceled mount/read must be discarded and its
/// transport ticket drained before another mount. No C/FatFs handle is reused.
pub async fn mount<'a, D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    device: &'a mut D,
    lease: MediaLease,
    is_current: &impl Fn() -> bool,
) -> Result<Volume<ReadBorrow<'a, D>>, Error> {
    let mut device = ReadBorrow(device);
    let mut buffer = sector(&mut device, 0, [0; 512], is_current).await?;
    let mut layout = PartitionLayout::new();
    match scan_mbr_or_superfloppy(&buffer, &mut layout) {
        PartitionScanResult::Ok => {}
        PartitionScanResult::NeedsGpt => {
            buffer = sector(&mut device, 1, buffer, is_current).await?;
            let info = parse_gpt_header(&buffer).ok_or(Error::Partition)?;
            let mut stream = GptEntryStream::new(info).ok_or(Error::Partition)?;
            let mut lba = info.entries_lba;
            layout.clear();
            while !stream.is_complete() {
                buffer = sector(&mut device, lba, buffer, is_current).await?;
                if stream.push(&buffer, &mut layout) == 0 {
                    return Err(Error::Partition);
                }
                lba = lba.checked_add(1).ok_or(Error::Partition)?;
            }
            if !stream.is_valid_complete() {
                return Err(Error::Partition);
            }
        }
        _ => return Err(Error::Partition),
    }
    for candidate in layout.candidates() {
        let geometry = device.geometry();
        let range = match candidate.sector_count {
            Some(count) => BlockRange::new(candidate.first_lba, count, geometry),
            None => BlockRange::from_start(candidate.first_lba, geometry),
        }
        .ok_or(Error::Partition)?;
        buffer = sector(&mut device, candidate.first_lba, buffer, is_current).await?;
        let Ok(boot) = parse_exfat_boot_sector(&buffer, range.geometry(512)) else {
            continue;
        };
        // exFAT 3.1.4 defines zero as "ignore this field". A nonzero mismatch
        // still rejects; all IO remains bounded by the actual partition extent.
        if boot.partition_offset != 0 && boot.partition_offset != candidate.first_lba {
            return Err(Error::Boot);
        }
        let mut volume = ExFatAsyncVolume::new(
            AsyncPartitionDevice::new(device, range),
            buffer,
            boot,
            lease,
        )
        .map_err(|_| Error::Mount)?;
        volume
            .validate_main_boot_region()
            .await
            .map_err(|_| Error::Checksum)?;
        current(is_current)?;
        return Ok(volume);
    }
    Err(Error::Boot)
}

pub fn matches(bytes: &[u8], offset: u32) -> bool {
    bytes.iter().enumerate().all(|(i, &byte)| {
        let p = offset.wrapping_add(i as u32);
        byte == p
            .wrapping_mul(73)
            .wrapping_add((p >> 8) * 19)
            .wrapping_add((p >> 17) * 131)
            .wrapping_add(0x5a) as u8
    })
}

/// Check the lease on both sides of filesystem operations, including cache hits,
/// EOF, seek and close. Only a final current lease can publish a successful pass.
/// On cancellation, discard file/volume; an interrupted cursor is never resumed.
pub async fn verify<D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    volume: &mut Volume<D>,
    is_current: &impl Fn() -> bool,
    mut yield_read: impl FnMut(),
) -> Result<(), Error> {
    current(is_current)?;
    let mut file = volume.open_read(PATH).await.map_err(|_| Error::Open)?;
    current(is_current)?;
    if file.len() != u64::from(SIZE) {
        return Err(Error::Size);
    }
    let mut output = [0u8; 513];
    let mut offset = 0;
    let mut sha = Sha256::new();
    while offset < SIZE {
        let wanted = usize::min(512, (SIZE - offset) as usize);
        current(is_current)?;
        let got = volume
            .read(&mut file, &mut output[..wanted])
            .await
            .map_err(|_| Error::Read)?;
        current(is_current)?;
        if got != wanted || !matches(&output[..got], offset) {
            return Err(Error::Content);
        }
        sha.update(&output[..got]);
        offset += got as u32;
        if offset.is_multiple_of(16384) {
            yield_read();
        }
    }
    let digest: [u8; 32] = sha.finalize().into();
    if digest != SHA {
        return Err(Error::Content);
    }
    current(is_current)?;
    if volume
        .read(&mut file, &mut output)
        .await
        .map_err(|_| Error::Read)?
        != 0
    {
        return Err(Error::Eof);
    }
    current(is_current)?;
    for position in [
        SIZE - 513,
        0,
        131071,
        511,
        131072,
        4095,
        512,
        SIZE - 1,
        4096,
        SIZE,
    ] {
        current(is_current)?;
        volume
            .seek_absolute(&mut file, position.into())
            .await
            .map_err(|_| Error::Seek)?;
        current(is_current)?;
        if file.position() != u64::from(position) {
            return Err(Error::Seek);
        }
        let wanted = usize::min(513, (SIZE - position) as usize);
        let got = volume
            .read(&mut file, &mut output[..wanted])
            .await
            .map_err(|_| Error::Read)?;
        current(is_current)?;
        if got != wanted || !matches(&output[..got], position) {
            return Err(Error::Seek);
        }
    }
    volume.close_file(file).await.map_err(|_| Error::Close)?;
    current(is_current)
}

#[cfg(test)]
pub(crate) mod tests;
