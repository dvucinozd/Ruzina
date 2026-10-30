use super::*;
use crate::media_bridge::{
    Binding, ReadClient, ReadPort, ReadQueue, ReadRequest, ReadResult, Ticket,
};
use core::future::Future;
use core::task::{Context, Poll, Waker};
use ruzina_media_block::BlockGeometry;
use ruzina_media_session::{MediaHandle, MediaSourceId};
use std::{boxed::Box, cell::RefCell, rc::Rc, vec, vec::Vec};

const CLUSTERS: u32 = SIZE.div_ceil(512) + 10;
const FAT_SECTORS: u32 = ((CLUSTERS + 2) * 4).div_ceil(512);
const HEAP: u32 = 24 + FAT_SECTORS;
const VOLUME: u32 = HEAP + CLUSTERS;
const FILE_LBA: u64 = (1 + HEAP + 2) as u64;

fn entries(name: &str, cluster: u32, len: u64, dir: bool) -> Vec<[u8; 32]> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut out = vec![[0u8; 32]; 2 + units.len().div_ceil(15)];
    out[0][0] = 0x85;
    out[0][1] = (out.len() - 1) as u8;
    out[0][4] = if dir { 0x10 } else { 0x20 };
    out[1][0] = 0xc0;
    out[1][1] = 3;
    out[1][3] = units.len() as u8;
    out[1][8..16].copy_from_slice(&len.to_le_bytes());
    out[1][20..24].copy_from_slice(&cluster.to_le_bytes());
    out[1][24..32].copy_from_slice(&len.to_le_bytes());
    for (index, unit) in units.into_iter().enumerate() {
        let entry = 2 + index / 15;
        out[entry][0] = 0xc1;
        let offset = 2 + (index % 15) * 2;
        out[entry][offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
    }
    let mut checksum = 0u16;
    for (index, entry) in out.iter().enumerate() {
        for (offset, &byte) in entry.iter().enumerate() {
            if index == 0 && (offset == 2 || offset == 3) {
                continue;
            }
            checksum = checksum.rotate_right(1).wrapping_add(byte.into());
        }
    }
    out[0][2..4].copy_from_slice(&checksum.to_le_bytes());
    out
}

fn image() -> Vec<u8> {
    let mut bytes = vec![0u8; (VOLUME as usize + 1) * 512];
    bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
    bytes[450] = 0x07;
    bytes[454..458].copy_from_slice(&1u32.to_le_bytes());
    bytes[458..462].copy_from_slice(&VOLUME.to_le_bytes());
    let boot = &mut bytes[512..1024];
    boot[0..3].copy_from_slice(&[0xeb, 0x76, 0x90]);
    boot[3..11].copy_from_slice(b"EXFAT   ");
    boot[64..72].copy_from_slice(&1u64.to_le_bytes());
    boot[72..80].copy_from_slice(&(VOLUME as u64).to_le_bytes());
    boot[80..84].copy_from_slice(&24u32.to_le_bytes());
    boot[84..88].copy_from_slice(&FAT_SECTORS.to_le_bytes());
    boot[88..92].copy_from_slice(&HEAP.to_le_bytes());
    boot[92..96].copy_from_slice(&CLUSTERS.to_le_bytes());
    boot[96..100].copy_from_slice(&2u32.to_le_bytes());
    boot[104..106].copy_from_slice(&0x100u16.to_le_bytes());
    boot[108] = 9;
    boot[109] = 0;
    boot[110] = 1;
    boot[510..512].copy_from_slice(&[0x55, 0xaa]);
    let mut checksum = 0u32;
    for sector in 0..11 {
        for (i, &byte) in bytes[(sector + 1) * 512..(sector + 2) * 512]
            .iter()
            .enumerate()
        {
            if sector == 0 && matches!(i, 106 | 107 | 112) {
                continue;
            }
            checksum = checksum.rotate_right(1).wrapping_add(byte.into());
        }
    }
    for chunk in bytes[12 * 512..13 * 512].chunks_exact_mut(4) {
        chunk.copy_from_slice(&checksum.to_le_bytes());
    }
    for (lba, set) in [
        ((1 + HEAP) as usize, entries("RUZINA_TEST", 3, 512, true)),
        (
            (2 + HEAP) as usize,
            entries("Čitanje i seek 64GB.bin", 4, SIZE.into(), false),
        ),
    ] {
        for (i, entry) in set.iter().enumerate() {
            bytes[lba * 512 + i * 32..lba * 512 + (i + 1) * 32].copy_from_slice(entry);
        }
    }
    for p in 0..SIZE {
        bytes[FILE_LBA as usize * 512 + p as usize] =
            (p * 73 + (p >> 8) * 19 + (p >> 17) * 131 + 0x5a) as u8;
    }
    bytes
}

struct State {
    queue: ReadQueue<[u8; 512]>,
    bytes: Vec<u8>,
    paused: Option<u64>,
    active: Option<ReadRequest<[u8; 512]>>,
    reads: usize,
}
#[derive(Clone)]
struct Port(Rc<RefCell<State>>);
impl ReadPort for Port {
    type Buffer = [u8; 512];
    fn is_current(&self, binding: Binding) -> bool {
        self.0.borrow().queue.is_current(binding)
    }
    fn submit(
        &mut self,
        binding: Binding,
        lba: u64,
        count: u32,
        buffer: Self::Buffer,
    ) -> Result<Ticket, (crate::media_bridge::Error, Self::Buffer)> {
        let mut result = None;
        {
            let mut s = self.0.borrow_mut();
            result.replace(s.queue.submit(binding, lba, count, buffer));
        }
        result.unwrap()
    }
    fn poll_completion(
        &mut self,
        ticket: Ticket,
        _cx: &mut Context<'_>,
    ) -> Poll<ReadResult<Self::Buffer>> {
        let mut s = self.0.borrow_mut();
        if s.active.is_none() {
            s.active = s.queue.take_request();
        }
        if s.active
            .as_ref()
            .is_some_and(|r| Some(r.first_block()) != s.paused)
        {
            let mut request = s.active.take().unwrap();
            let start = request.first_block() as usize * 512;
            request
                .buffer_mut()
                .copy_from_slice(&s.bytes[start..start + 512]);
            s.reads += 1;
            assert!(s.queue.complete(request, Ok(512)).is_ok());
        }
        match s.queue.take_completion(ticket) {
            Some(result) => Poll::Ready(result),
            None => Poll::Pending,
        }
    }
}
fn setup() -> (Port, Binding, ReadClient<Port>) {
    let mut queue = ReadQueue::new();
    let binding = queue
        .activate(
            MediaSourceId::new(1).unwrap(),
            MediaHandle::new(1).unwrap(),
            BlockGeometry {
                block_size: 512,
                block_count: u64::from(VOLUME) + 1,
            },
        )
        .unwrap();
    let port = Port(Rc::new(RefCell::new(State {
        queue,
        bytes: image(),
        paused: None,
        active: None,
        reads: 0,
    })));
    (port.clone(), binding, ReadClient::new(port, binding))
}
fn ready<F: Future>(future: F) -> F::Output {
    match Box::pin(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending"),
    }
}

fn partition_offset(port: &Port, value: u64) {
    let mut state = port.0.borrow_mut();
    state.bytes[512 + 64..512 + 72].copy_from_slice(&value.to_le_bytes());
    let mut checksum = 0u32;
    for sector in 0..11 {
        for (index, &byte) in state.bytes[(sector + 1) * 512..(sector + 2) * 512]
            .iter()
            .enumerate()
        {
            if sector == 0 && matches!(index, 106 | 107 | 112) {
                continue;
            }
            checksum = checksum.rotate_right(1).wrapping_add(byte.into());
        }
    }
    for chunk in state.bytes[12 * 512..13 * 512].chunks_exact_mut(4) {
        chunk.copy_from_slice(&checksum.to_le_bytes());
    }
}

#[test]
fn exfat_partition_offset_zero_uses_bounded_partition_extent() {
    let (port, binding, mut client) = setup();
    partition_offset(&port, 0);
    let current = || port.is_current(binding);
    let mut volume = match ready(mount(&mut client, binding.lease, &current)) {
        Ok(volume) => volume,
        Err(error) => panic!("valid zero PartitionOffset was rejected: {error:?}"),
    };
    assert_eq!(volume.device().range().first_block(), 1);
    assert_eq!(ready(verify(&mut volume, &current, || {})), Ok(()));
}

#[test]
fn exfat_partition_offset_nonzero_mismatch_stays_rejected() {
    let (port, binding, mut client) = setup();
    partition_offset(&port, 2); // actual MBR partition starts at1
    let current = || port.is_current(binding);
    assert_eq!(
        ready(mount(&mut client, binding.lease, &current)).err(),
        Some(Error::Boot)
    );
}

#[test]
fn shared_partition_exfat_full_utf8_fixture_sha_eof_and_seeks() {
    let (port, binding, mut client) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    assert_eq!(volume.device().range().first_block(), 1);
    assert_eq!(volume.boot_info().root_directory_cluster, 2);
    assert_eq!(ready(verify(&mut volume, &current, || {})), Ok(()));
    assert!(port.0.borrow().reads > SIZE as usize / 512);
}

#[test]
fn corrupt_fixture_is_rejected() {
    let (port, binding, mut client) = setup();
    port.0.borrow_mut().bytes[FILE_LBA as usize * 512 + 77] ^= 1;
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    assert_eq!(
        ready(verify(&mut volume, &current, || {})),
        Err(Error::Content)
    );
}

#[test]
fn corrupt_boot_checksum_is_rejected_before_open() {
    let (port, binding, mut client) = setup();
    port.0.borrow_mut().bytes[12 * 512] ^= 1;
    let current = || port.is_current(binding);
    assert_eq!(
        ready(mount(&mut client, binding.lease, &current)).err(),
        Some(Error::Checksum)
    );
}

#[test]
fn canceled_mount_keeps_outer_client_ticket_for_drain() {
    let (port, binding, mut client) = setup();
    port.0.borrow_mut().paused = Some(0);
    let current = || port.is_current(binding);
    let mut mounting = Box::pin(mount(&mut client, binding.lease, &current));
    assert!(
        mounting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(mounting);
    assert!(client.has_pending());
    port.0.borrow_mut().paused = None;
    assert!(ready(client.recover_pending()).unwrap().is_ok());
    assert!(ready(mount(&mut client, binding.lease, &current)).is_ok());
}

#[test]
fn canceled_filesystem_read_drains_then_remounts_with_new_lease() {
    let (port, binding, mut client) = setup();
    let current = || port.is_current(binding);
    {
        let mut volume = ready(mount(&mut client, binding.lease, &current))
            .ok()
            .unwrap();
        let mut file = ready(volume.open_read(PATH)).unwrap();
        let mut output = [0u8; 512];
        port.0.borrow_mut().paused = Some(FILE_LBA);
        let mut reading = Box::pin(volume.read(&mut file, &mut output));
        assert!(
            reading
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(reading);
        let (_partition, scratch) = volume.into_parts();
        assert!(scratch.is_none());
    }
    assert!(client.has_pending());
    port.0.borrow_mut().paused = None;
    assert!(ready(client.recover_pending()).unwrap().is_ok());
    let fresh = {
        let mut s = port.0.borrow_mut();
        s.queue.invalidate();
        s.queue
            .activate(
                MediaSourceId::new(1).unwrap(),
                MediaHandle::new(1).unwrap(),
                client.geometry(),
            )
            .unwrap()
    };
    assert_ne!(fresh.lease, binding.lease);
    let mut fresh_client = ReadClient::new(port.clone(), fresh);
    let current = || port.is_current(fresh);
    let mut volume = ready(mount(&mut fresh_client, fresh.lease, &current))
        .ok()
        .unwrap();
    assert_eq!(ready(verify(&mut volume, &current, || {})), Ok(()));
}

#[test]
fn cached_file_bytes_cannot_bypass_fixture_lease_gate() {
    let (port, binding, mut client) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    let mut file = ready(volume.open_read(PATH)).unwrap();
    let mut output = [0u8; 1];
    assert_eq!(ready(volume.read(&mut file, &mut output)).unwrap(), 1);
    port.0.borrow_mut().queue.invalidate();
    // Demonstrates why the existing shared FS alone is not an external lease gate.
    assert_eq!(ready(volume.read(&mut file, &mut output)).unwrap(), 1);
    assert_eq!(
        ready(verify(&mut volume, &current, || {})),
        Err(Error::Stale)
    );
}

#[test]
fn invalidation_during_long_verify_never_returns_success() {
    let (port, binding, mut client) = setup();
    let current = || port.is_current(binding);
    let mut volume = ready(mount(&mut client, binding.lease, &current))
        .ok()
        .unwrap();
    assert_eq!(
        ready(verify(&mut volume, &current, || {
            port.0.borrow_mut().queue.invalidate();
        })),
        Err(Error::Stale)
    );
}
