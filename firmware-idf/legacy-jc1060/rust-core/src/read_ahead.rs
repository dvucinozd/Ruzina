//! Multi-sector read-ahead under the filesystem's one-sector cache.
//!
//! The exFAT volume asks for one 512-byte sector at a time. Each card request
//! costs about a millisecond of task handoff and command overhead, whatever
//! its size, so this layer turns a missed sector into one request for its
//! whole aligned line and serves the line's other sectors from memory.
//!
//! The card is read-only and owned by one lease for the life of the
//! transport, so a cached line never goes stale. Like the volume's own sector
//! cache, a hit performs no lease check: callers check the lease around each
//! filesystem operation and discard the volume when it ends.
#![forbid(unsafe_code)]

use alloc::{boxed::Box, vec::Vec};
use ruzina_media_block::{BlockGeometry, TransferError};
use ruzina_media_io::OwnedBlockDevice;

/// Sectors fetched per card request, and the bytes they fill.
pub const LINE_SECTORS: u32 = 16;
pub const LINE_BYTES: usize = LINE_SECTORS as usize * 512;
/// Lines held: two decks, each decoding and walking its MP3 frame index at
/// different positions, plus the filesystem's directory and FAT reads.
pub const LINES: usize = 8;

/// One heap line handed to the transport. Its length is the requested
/// sector count times 512, at most `LINE_BYTES`.
pub struct Line {
    data: Box<[u8]>,
    len: usize,
}

impl Line {
    /// A zeroed line in the heap, or `None` if it cannot be reserved.
    pub fn try_new(len: usize) -> Option<Self> {
        if len > LINE_BYTES {
            return None;
        }
        let mut data = Vec::new();
        data.try_reserve_exact(LINE_BYTES).ok()?;
        data.resize(LINE_BYTES, 0);
        Some(Self {
            data: data.into_boxed_slice(),
            len,
        })
    }

    fn set_len(&mut self, len: usize) {
        self.len = len.min(LINE_BYTES);
    }
}

impl AsRef<[u8]> for Line {
    fn as_ref(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl AsMut<[u8]> for Line {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error<E> {
    Transfer(TransferError),
    /// Every line is held by a canceled request; recover the device first.
    NoLine,
    Device(E),
}

#[derive(Clone, Copy)]
struct Tag {
    first: u64,
    count: u32,
    used: u32,
}

pub struct ReadAhead<D: OwnedBlockDevice<Buffer = Line>> {
    device: D,
    lines: [Option<Line>; LINES],
    tags: [Option<Tag>; LINES],
    clock: u32,
    requests: u32,
}

impl<D: OwnedBlockDevice<Buffer = Line>> ReadAhead<D> {
    /// Reserve every line up front, or hand the device back.
    pub fn try_new(device: D) -> Result<Self, D> {
        let mut lines: [Option<Line>; LINES] = Default::default();
        for line in &mut lines {
            match Line::try_new(0) {
                Some(new) => *line = Some(new),
                None => return Err(device),
            }
        }
        Ok(Self {
            device,
            lines,
            tags: [None; LINES],
            clock: 0,
            requests: 0,
        })
    }

    /// Card requests issued so far.
    pub fn requests(&self) -> u32 {
        self.requests
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    fn touch(&mut self, slot: usize) {
        self.clock = self.clock.wrapping_add(1);
        if let Some(tag) = self.tags[slot].as_mut() {
            tag.used = self.clock;
        }
    }

    fn lookup(&self, block: u64) -> Option<usize> {
        self.tags.iter().position(|tag| {
            tag.is_some_and(|t| block >= t.first && block - t.first < u64::from(t.count))
        })
    }

    /// An empty line first, then the least recently used one. Lines lent to
    /// a canceled request are skipped.
    fn victim(&self) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for slot in 0..LINES {
            if self.lines[slot].is_none() {
                continue;
            }
            let Some(tag) = self.tags[slot] else {
                return Some(slot);
            };
            let age = self.clock.wrapping_sub(tag.used);
            if best.is_none_or(|(_, oldest)| age > oldest) {
                best = Some((slot, age));
            }
        }
        best.map(|(slot, _)| slot)
    }

    fn copy_out(&self, slot: usize, first: u64, out: &mut [u8]) {
        let (tag, line) = (self.tags[slot].unwrap(), self.lines[slot].as_ref().unwrap());
        let start = (first - tag.first) as usize * 512;
        out.copy_from_slice(&line.as_ref()[start..start + out.len()]);
    }
}

impl<D: OwnedBlockDevice<Buffer = Line>> OwnedBlockDevice for ReadAhead<D> {
    type Error = Error<D::Error>;
    type Buffer = [u8; 512];

    fn geometry(&self) -> BlockGeometry {
        self.device.geometry()
    }

    async fn read_blocks_owned(
        &mut self,
        first_block: u64,
        block_count: u32,
        mut buffer: Self::Buffer,
    ) -> Result<Self::Buffer, (Self::Error, Self::Buffer)> {
        let geometry = self.device.geometry();
        if let Err(error) = geometry.validate_transfer(first_block, block_count, buffer.len()) {
            return Err((Error::Transfer(error), buffer));
        }
        if let Some(slot) = self.lookup(first_block) {
            self.touch(slot);
            self.copy_out(slot, first_block, &mut buffer);
            return Ok(buffer);
        }
        let Some(slot) = self.victim() else {
            return Err((Error::NoLine, buffer));
        };
        let start = first_block - first_block % u64::from(LINE_SECTORS);
        let count = (geometry.block_count - start).min(u64::from(LINE_SECTORS)) as u32;
        let mut line = self.lines[slot].take().expect("victim holds a line");
        line.set_len(count as usize * 512);
        self.tags[slot] = None;
        self.requests = self.requests.wrapping_add(1);
        match self.device.read_blocks_owned(start, count, line).await {
            Ok(line) => {
                self.lines[slot] = Some(line);
                self.tags[slot] = Some(Tag {
                    first: start,
                    count,
                    used: 0,
                });
                self.touch(slot);
                self.copy_out(slot, first_block, &mut buffer);
                Ok(buffer)
            }
            Err((error, line)) => {
                self.lines[slot] = Some(line);
                Err((Error::Device(error), buffer))
            }
        }
    }
}

#[cfg(test)]
mod tests;
