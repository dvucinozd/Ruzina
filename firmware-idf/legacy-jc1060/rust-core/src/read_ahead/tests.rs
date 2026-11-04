use super::*;
use crate::exfat_fixture::tests::{image, ready};
use ruzina_media_session::{ConnectResult, MediaSession, MediaSourceId};
use std::vec::Vec;

/// A whole card in memory that records every request it serves.
struct Card {
    bytes: Vec<u8>,
    requests: Vec<(u64, u32)>,
    fail_at: Option<u64>,
}

impl Card {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            requests: Vec::new(),
            fail_at: None,
        }
    }
}

impl OwnedBlockDevice for Card {
    type Error = i32;
    type Buffer = Line;
    fn geometry(&self) -> BlockGeometry {
        BlockGeometry {
            block_size: 512,
            block_count: (self.bytes.len() / 512) as u64,
        }
    }
    async fn read_blocks_owned(
        &mut self,
        first: u64,
        count: u32,
        mut line: Line,
    ) -> Result<Line, (i32, Line)> {
        assert!(
            self.geometry()
                .validate_transfer(first, count, line.as_ref().len())
                .is_ok()
        );
        if self.fail_at == Some(first) {
            return Err((-1, line));
        }
        self.requests.push((first, count));
        let start = first as usize * 512;
        let out = line.as_mut();
        let len = out.len();
        out.copy_from_slice(&self.bytes[start..start + len]);
        Ok(line)
    }
}

fn pattern(sectors: usize) -> Vec<u8> {
    (0..sectors * 512)
        .map(|i| (i * 7 + i / 512) as u8)
        .collect()
}

fn sector(cache: &mut ReadAhead<Card>, lba: u64) -> [u8; 512] {
    ready(cache.read_blocks_owned(lba, 1, [0; 512]))
        .ok()
        .unwrap()
}

#[test]
fn a_sequential_read_costs_one_request_per_line() {
    let bytes = pattern(64);
    let mut cache = ReadAhead::try_new(Card::new(bytes.clone())).ok().unwrap();
    for lba in 0..64u64 {
        let start = lba as usize * 512;
        assert_eq!(sector(&mut cache, lba)[..], bytes[start..start + 512]);
    }
    assert_eq!(
        cache.device.requests,
        [(0, 16), (16, 16), (32, 16), (48, 16)]
    );
    assert_eq!(cache.requests(), 4);
}

#[test]
fn a_miss_fetches_its_aligned_line_and_the_last_line_stops_at_the_card_end() {
    let bytes = pattern(40);
    let mut cache = ReadAhead::try_new(Card::new(bytes.clone())).ok().unwrap();
    assert_eq!(sector(&mut cache, 21)[..], bytes[21 * 512..22 * 512]);
    assert_eq!(sector(&mut cache, 39)[..], bytes[39 * 512..40 * 512]);
    assert_eq!(sector(&mut cache, 16)[..], bytes[16 * 512..17 * 512]);
    assert_eq!(cache.device.requests, [(16, 16), (32, 8)]);
    let (error, _) = ready(cache.read_blocks_owned(40, 1, [0; 512])).unwrap_err();
    assert_eq!(error, Error::Transfer(TransferError::OutOfRange));
}

#[test]
fn interleaved_streams_keep_their_lines_until_the_least_recent_is_evicted() {
    let bytes = pattern(16 * 12);
    let mut cache = ReadAhead::try_new(Card::new(bytes)).ok().unwrap();
    // Eight streams, one per line, read in turn: only first touches miss.
    for _ in 0..3 {
        for stream in 0..LINES as u64 {
            sector(&mut cache, stream * 16 + 1);
        }
    }
    assert_eq!(cache.device.requests.len(), LINES);
    // A ninth line evicts the least recently used (stream 0) and no other.
    sector(&mut cache, 8 * 16);
    sector(&mut cache, 7 * 16);
    sector(&mut cache, 0);
    assert_eq!(cache.device.requests[LINES..], [(128, 16), (0, 16)]);
}

#[test]
fn a_failed_request_returns_the_buffer_keeps_the_line_and_retries_later() {
    let bytes = pattern(32);
    let mut card = Card::new(bytes.clone());
    card.fail_at = Some(16);
    let mut cache = ReadAhead::try_new(card).ok().unwrap();
    let (error, returned) = ready(cache.read_blocks_owned(17, 1, [9; 512])).unwrap_err();
    assert_eq!((error, returned), (Error::Device(-1), [9; 512]));
    assert!(cache.lines.iter().all(Option::is_some));
    cache.device.fail_at = None;
    assert_eq!(sector(&mut cache, 17)[..], bytes[17 * 512..18 * 512]);
    assert_eq!(cache.device.requests, [(16, 16)]);
}

#[test]
fn the_exfat_fixture_verifies_through_the_read_ahead_with_far_fewer_requests() {
    let bytes = image();
    let sectors = (crate::exfat_fixture::SIZE as usize).div_ceil(512);
    let mut cache = ReadAhead::try_new(Card::new(bytes)).ok().unwrap();
    let ConnectResult::Accepted(lease) =
        MediaSession::new().on_connect(MediaSourceId::new(1).unwrap())
    else {
        panic!("a fresh session accepts its first source");
    };
    {
        let mut volume = ready(crate::exfat_fixture::mount(&mut cache, lease, &|| true))
            .ok()
            .unwrap();
        ready(crate::exfat_fixture::verify(&mut volume, &|| true, || {})).unwrap();
    }
    // Every file sector is read once at least; lines bring that down 16-fold.
    let requests = cache.device.requests.len();
    assert!(
        requests * 15 < sectors,
        "{requests} requests for {sectors} sectors"
    );
    assert!(
        cache
            .device
            .requests
            .iter()
            .all(|&(_, count)| count <= LINE_SECTORS)
    );
    let fetched: u64 = cache
        .device
        .requests
        .iter()
        .map(|&(_, count)| u64::from(count))
        .sum();
    assert!(
        fetched < 2 * sectors as u64 + 64,
        "{fetched} sectors fetched"
    );
}
