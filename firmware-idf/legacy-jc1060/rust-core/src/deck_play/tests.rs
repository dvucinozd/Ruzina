use super::*;
use std::boxed::Box;
use std::vec::Vec;

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(
            "../../../../../firmware-rust/crates/ruzina-audio-runtime/tests/fixtures/codecs/",
            $name
        ))
        .as_slice()
    };
}

const WAV: &[u8] = fixture!("stereo.wav");
const FLAC: &[u8] = fixture!("stereo.flac");
const CBR: &[u8] = fixture!("cbr.mp3");
const VBR_44K: &[u8] = fixture!("vbr.mp3");
const LONG_ID3: &[u8] = fixture!("long-vbr.mp3");

#[test]
fn in_place_core_reset_matches_fresh_decoder_across_sources() {
    fn decode(decoder: &mut nanomp3_core::Decoder, data: &[u8]) -> Vec<i16> {
        let mut rest = data;
        let mut output = Vec::new();
        let mut pcm = [0i16; nanomp3_core::MAX_SAMPLES_PER_FRAME];
        while !rest.is_empty() {
            let (consumed, result) = decoder.decode(rest, &mut pcm);
            if consumed == 0 {
                break;
            }
            assert!(consumed <= rest.len());
            rest = &rest[consumed..];
            if let Ok(info) = result {
                output.extend_from_slice(
                    &pcm[..info.samples_produced * info.channels.num() as usize],
                );
            }
        }
        assert!(!output.is_empty());
        output
    }
    let mut reused = nanomp3_core::Decoder::new_minimp3_compat();
    for data in [CBR, VBR_44K, fixture!("mono.mp3"), LONG_ID3, CBR] {
        let _dirty_state = decode(&mut reused, LONG_ID3);
        reused.reset();
        reused.reset();
        let mut fresh = nanomp3_core::Decoder::new();
        assert_eq!(decode(&mut reused, data), decode(&mut fresh, data));
    }
}

#[cfg(feature = "producer-profile")]
mod service_model;

#[cfg(feature = "producer-profile")]
#[test]
fn source_profile_measures_slow_read_and_keeps_real_pcm_and_failure() {
    struct Slow {
        inner: Memory,
        clock: core::cell::Cell<u64>,
    }
    impl SourceFiles for Slow {
        type File = &'static [u8];
        fn open(&mut self, path: &str) -> Option<Self::File> {
            self.inner.open(path)
        }
        fn len(&self, file: &Self::File) -> u64 {
            self.inner.len(file)
        }
        fn close(&mut self, file: Self::File) {
            self.inner.close(file)
        }
        fn now_us(&self) -> u64 {
            self.clock.get()
        }
        fn read_at(&mut self, file: &mut Self::File, offset: u64, out: &mut [u8]) -> Option<usize> {
            self.clock.set(self.clock.get() + 5000);
            self.inner.read_at(file, offset, out)
        }
    }
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Slow {
        inner: Memory::new(&[WAV]),
        clock: core::cell::Cell::new(1),
    };
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    commands.load(0);
    let mut pcm = Vec::new();
    for _ in 0..100_000 {
        let start = files.now_us();
        let head = ring.profile_head();
        let (busy, _) = producer.poll(&mut files, ring, &commands, path);
        producer.profile_poll(ring, start, files.now_us(), busy, head);
        pcm.extend(drain(&mut reader, ring));
        if !busy {
            break;
        }
    }
    assert_eq!(pcm, wav_pcm());
    let words = producer.profile.words();
    let total =
        |stage: usize| u64::from(words[17 + stage * 4]) | (u64::from(words[18 + stage * 4]) << 32);
    assert_eq!(total(1), (2..6).map(total).sum::<u64>());
    assert!(total(3) >= 5000);
    assert_eq!(words[19 + 3 * 4], 5000);
    assert_eq!(words[11] as usize, pcm.len());
    assert_eq!((words[13], words[14]), (0, 0));
    files.inner.fail_reads_after = Some(files.inner.reads);
    commands.load(0);
    let mut failed = false;
    for _ in 0..10 {
        failed |= matches!(
            producer.poll(&mut files, ring, &commands, path).1,
            Some(Event::Failed { .. })
        );
        if failed {
            break;
        }
    }
    assert!(failed);
}

/// Tracks by catalog index, read from memory.
pub(crate) struct Memory {
    files: Vec<&'static [u8]>,
    reads: usize,
    pub(crate) fail_reads_after: Option<usize>,
}

impl Memory {
    pub(crate) fn new(files: &[&'static [u8]]) -> Self {
        Self {
            files: files.to_vec(),
            reads: 0,
            fail_reads_after: None,
        }
    }
}

impl SourceFiles for Memory {
    type File = &'static [u8];
    fn open(&mut self, path: &str) -> Option<Self::File> {
        self.files.get(path.parse::<usize>().ok()?).copied()
    }
    fn len(&self, file: &Self::File) -> u64 {
        file.len() as u64
    }
    fn read_at(&mut self, file: &mut Self::File, offset: u64, out: &mut [u8]) -> Option<usize> {
        self.reads += 1;
        if self
            .fail_reads_after
            .is_some_and(|limit| self.reads > limit)
        {
            return None;
        }
        let rest = file.get(offset as usize..)?;
        let n = rest.len().min(out.len());
        out[..n].copy_from_slice(&rest[..n]);
        Some(n)
    }
    fn close(&mut self, _: Self::File) {}
}

fn ring() -> &'static Ring {
    let ring = Box::leak(Box::new(Ring::new()));
    let storage = std::vec![PcmFrame::default(); RING_STORAGE_FRAMES].into_boxed_slice();
    assert!(ring.attach(Box::leak(storage)));
    ring
}

fn path(index: u32) -> Option<std::string::String> {
    (index < 100).then(|| std::format!("{index}"))
}

/// Polls until the producer goes idle; returns every event.
fn run(
    producer: &mut Producer<&'static [u8]>,
    files: &mut Memory,
    ring: &Ring,
    commands: &Commands,
) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..100_000 {
        let (busy, event) = producer.poll(files, ring, commands, path);
        events.extend(event);
        if !busy {
            return events;
        }
    }
    panic!("producer never went idle");
}

/// Drains everything the ring holds for the reader's epoch.
fn drain(reader: &mut Reader, ring: &Ring) -> Vec<PcmFrame> {
    reader.sync(ring);
    let mut frames = Vec::new();
    while let Some(frame) = reader.pop(ring) {
        frames.push(frame);
    }
    reader.commit(ring);
    frames
}

fn wav_pcm() -> Vec<PcmFrame> {
    WAV[44..]
        .chunks_exact(4)
        .map(|b| PcmFrame {
            left: i16::from_le_bytes([b[0], b[1]]),
            right: i16::from_le_bytes([b[2], b[3]]),
        })
        .collect()
}

#[test]
fn wav_and_flac_tracks_reach_the_ring_bit_exact() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[WAV, FLAC]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    for (index, codec) in [(0, 1), (1, 3)] {
        commands.load(index);
        let events = run(&mut producer, &mut files, ring, &commands);
        assert_eq!(
            events[0],
            Event::Ready {
                index,
                codec,
                rate: 48_000
            }
        );
        assert_eq!(
            events[1..],
            [Event::Ended {
                index,
                faulted: false
            }]
        );
        assert_eq!(drain(&mut reader, ring), wav_pcm());
        assert!(reader.ended(ring));
    }
}

#[test]
fn mp3_tracks_decode_at_their_own_rate() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[CBR, VBR_44K]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    for (index, rate) in [(0, 48_000), (1, 44_100)] {
        commands.load(index);
        let events = run(&mut producer, &mut files, ring, &commands);
        assert_eq!(
            events[0],
            Event::Ready {
                index,
                codec: 2,
                rate
            }
        );
        assert_eq!(drain(&mut reader, ring).len(), 12_000);
        assert_eq!(reader.rate, rate);
    }
}

#[test]
fn a_long_track_fills_the_ring_then_follows_the_consumer() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[LONG_ID3]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    commands.load(0);
    let events = run(&mut producer, &mut files, ring, &commands);
    // Once the ring is full, the MP3 frame walk runs to the end of the file.
    assert_eq!(
        events,
        [
            Event::Ready {
                index: 0,
                codec: 2,
                rate: 44_100
            },
            Event::Indexed {
                index: 0,
                elapsed_ms: 0
            }
        ]
    );
    reader.sync(ring);
    assert!(reader.buffered(ring) > RING_FRAMES - PCM_STREAM_BLOCK_FRAMES as u32);
    // Eight seconds of 44.1 kHz in a ring of under two: the producer refills
    // as the consumer drains, and the track ends exactly once.
    let mut total = 0;
    let mut ended = Vec::new();
    for _ in 0..64 {
        total += drain(&mut reader, ring).len();
        ended.extend(run(&mut producer, &mut files, ring, &commands));
    }
    assert_eq!(
        ended,
        [Event::Ended {
            index: 0,
            faulted: false
        }]
    );
    assert!((total as i64 - 8 * 44_100).abs() < 2_000, "{total}");
}

#[test]
fn cue_restarts_the_track_and_load_replaces_a_full_ring() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[LONG_ID3, WAV]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    commands.load(0);
    run(&mut producer, &mut files, ring, &commands);
    reader.sync(ring);
    let first: Vec<_> = (0..1000).filter_map(|_| reader.pop(ring)).collect();
    reader.commit(ring);
    commands.cue();
    // A reload walks the frames again.
    assert_eq!(
        run(&mut producer, &mut files, ring, &commands),
        [
            Event::Ready {
                index: 0,
                codec: 2,
                rate: 44_100
            },
            Event::Indexed {
                index: 0,
                elapsed_ms: 0
            }
        ]
    );
    let again = drain(&mut reader, ring);
    assert_eq!(again[..1000], first[..]);
    // The ring is full of the long track and the consumer has not caught up:
    // the new track's first block waits, then follows once the reader syncs.
    run(&mut producer, &mut files, ring, &commands);
    assert!(ring.free() < PCM_STREAM_BLOCK_FRAMES as u32);
    commands.load(1);
    assert_eq!(
        run(&mut producer, &mut files, ring, &commands),
        [Event::Ready {
            index: 1,
            codec: 1,
            rate: 48_000
        }]
    );
    reader.sync(ring);
    reader.commit(ring);
    let events = run(&mut producer, &mut files, ring, &commands);
    assert_eq!(
        events,
        [Event::Ended {
            index: 1,
            faulted: false
        }]
    );
    assert_eq!(drain(&mut reader, ring), wav_pcm());
}

#[test]
fn unusable_tracks_fail_by_stage_and_silence_the_deck() {
    let (ring, commands) = (ring(), Commands::new());
    let text: &'static [u8] = b"not an audio file at all, just text";
    let mut files = Memory::new(&[WAV, text, &WAV[..40]]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    commands.load(0);
    run(&mut producer, &mut files, ring, &commands);
    for (index, stage) in [(100, 1), (50, 2), (1, 3), (2, 3)] {
        commands.load(index);
        assert_eq!(
            run(&mut producer, &mut files, ring, &commands),
            [Event::Failed { index, stage }]
        );
        assert!(drain(&mut reader, ring).is_empty());
        assert_eq!(reader.rate, 0);
    }
    // A read failure mid-track ends it as faulted.
    files.fail_reads_after = Some(files.reads + 40);
    commands.load(0);
    let events = run(&mut producer, &mut files, ring, &commands);
    assert_eq!(
        events[1..],
        [Event::Ended {
            index: 0,
            faulted: true
        }]
    );
}

#[test]
fn output_resamples_while_playing_and_counts_only_real_underruns() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[VBR_44K]);
    let mut producer = Producer::new();
    let mut output = Output::new();
    let mut packet = [PcmFrame { left: 1, right: 1 }; 48];
    output.render(ring, &mut packet, 48_000, true, 1 << 15);
    assert!(packet.iter().all(|f| *f == PcmFrame::default()));
    commands.load(0);
    run(&mut producer, &mut files, ring, &commands);
    // Paused: silence, nothing consumed.
    output.render(ring, &mut packet, 48_000, false, 1 << 15);
    assert!(packet.iter().all(|f| *f == PcmFrame::default()));
    assert_eq!(output.reader.buffered(ring), 12_000);
    // 0.2 s at 48 kHz takes 0.2 s of the 44.1 kHz source.
    for _ in 0..200 {
        output.render(ring, &mut packet, 48_000, true, 1 << 15);
    }
    let consumed = 12_000 - output.reader.buffered(ring);
    assert!((i64::from(consumed) - 8_820).abs() <= 2, "{consumed}");
    assert_eq!(output.underruns, 0);
    assert_eq!(output.played, 200 * 48);
    // Running out at the end of the track is not an underrun.
    for _ in 0..400 {
        output.render(ring, &mut packet, 48_000, true, 1 << 15);
    }
    assert_eq!(output.underruns, 0);
    assert!(packet.iter().all(|f| *f == PcmFrame::default()));
    // A track that cannot keep up is.
    let mut files = Memory::new(&[LONG_ID3]);
    let mut producer = Producer::new();
    commands.load(0);
    producer.poll(&mut files, ring, &commands, path);
    while producer.priming() {
        producer.poll(&mut files, ring, &commands, path);
    }
    for _ in 0..400 {
        output.render(ring, &mut packet, 48_000, true, 1 << 15);
    }
    assert!(output.underruns > 0);
}

#[test]
fn output_gain_follows_the_headphone_level() {
    assert_eq!(headphone_gain_q15(Some((0x3fff, 0x3fff))), 8192);
    assert_eq!(headphone_gain_q15(Some((0, 0x3fff))), 0);
    assert_eq!(headphone_gain_q15(None), 512);
    assert_eq!(headphone_gain_q15(Some((5, 0))), 512);
    assert_eq!(scale(i16::MAX, 8192), i16::MAX / 4);
    assert_eq!(scale(i16::MIN, 8192), i16::MIN / 4);
    assert_eq!(scale(-1000, 1 << 15), -1000);
}

#[test]
fn a_seek_reopens_the_track_at_its_position() {
    let (ring, commands) = (ring(), Commands::new());
    let mut files = Memory::new(&[WAV, LONG_ID3]);
    let mut producer = Producer::new();
    let mut reader = Reader::new();
    // WAV: frame-exact. 100 ms at 48 kHz is frame 4800.
    commands.load(0);
    run(&mut producer, &mut files, ring, &commands);
    commands.seek(100);
    let events = run(&mut producer, &mut files, ring, &commands);
    assert_eq!(
        events[0],
        Event::Seeked {
            index: 0,
            position_ms: 100,
            ok: true
        }
    );
    reader.sync(ring);
    assert_eq!(reader.position_ms(), 100);
    assert_eq!(drain(&mut reader, ring), wav_pcm()[4_800..]);
    assert_eq!(reader.position_ms(), 250);
    // A seek after the end revives the track.
    commands.seek(0);
    run(&mut producer, &mut files, ring, &commands);
    assert_eq!(drain(&mut reader, ring), wav_pcm());
    // MP3 behind an ID3 tag: forward, then back past the start, each epoch
    // starting at its target.
    commands.load(1);
    run(&mut producer, &mut files, ring, &commands);
    for target in [6_000, 1_000, 7_900] {
        reader.sync(ring);
        commands.seek(target);
        let events = run(&mut producer, &mut files, ring, &commands);
        assert!(
            matches!(events[0], Event::Seeked { ok: true, .. }),
            "{events:?}"
        );
        reader.sync(ring);
        assert_eq!(reader.position_ms(), target);
        // The full ring held the first block back until the reader followed.
        run(&mut producer, &mut files, ring, &commands);
        assert!(reader.pop(ring).is_some());
    }
    // Two seeks before the producer runs: the newer wins.
    commands.seek(2_000);
    commands.seek(3_000);
    let events = run(&mut producer, &mut files, ring, &commands);
    assert!(matches!(
        events[0],
        Event::Seeked {
            position_ms: 3_000,
            ..
        }
    ));
    // Without a track there is nothing to seek.
    let mut idle = Producer::new();
    commands.seek(10);
    assert_eq!(idle.poll(&mut files, ring, &commands, path), (false, None));
}

#[test]
fn ring_keeps_its_first_storage() {
    let ring = ring();
    let other = std::vec![PcmFrame::default(); RING_STORAGE_FRAMES].into_boxed_slice();
    assert!(!ring.attach(Box::leak(other)));
    let short = std::vec![PcmFrame::default(); 8].into_boxed_slice();
    assert!(!Ring::new().attach(Box::leak(short)));
    assert!(!Ring::new().push(&[PcmFrame::default()]));
}

#[test]
fn descriptor_publication_rejects_busy_and_interleaved_reads() {
    let ring = ring();
    ring.restart_at(44_100, 123);
    let first = ring.snapshot().unwrap();
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    let tail = ring.tail.load(Ordering::Acquire);
    // Pause a writer before its first field, then after partial mutation.
    ring.descriptor_seq.fetch_add(1, Ordering::SeqCst);
    assert_eq!(ring.snapshot(), None);
    ring.origin.store(456, Ordering::SeqCst);
    assert!(!reader.sync(ring));
    assert_eq!((reader.epoch, reader.position), (first.epoch, 123));
    assert_eq!(ring.tail.load(Ordering::Acquire), tail);
    ring.origin.store(123, Ordering::SeqCst);
    ring.descriptor_seq.fetch_add(1, Ordering::SeqCst);
    // A complete restart between field reads and validation must be rejected.
    assert_eq!(ring.snapshot_with(|| ring.restart_at(48_000, 789)), None);
    assert!(reader.sync(ring));
    assert_eq!((reader.rate, reader.position), (48_000, 789));
    ring.end();
    assert!(reader.ended(ring));
    ring.restart_at(44_100, 10);
    assert!(!reader.ended(ring));
    assert!(reader.sync(ring));
    assert!(!reader.ended(ring));
}

#[test]
fn descriptor_wrap_and_full_ring_preserve_pcm_ownership() {
    let ring = ring();
    ring.head.store(u32::MAX - 99, Ordering::Relaxed);
    ring.tail.store(u32::MAX - 99, Ordering::Relaxed);
    ring.epoch.store(u32::MAX, Ordering::SeqCst);
    ring.descriptor_seq.store(u32::MAX - 1, Ordering::SeqCst);
    ring.restart_at(48_000, 10);
    assert_eq!(ring.snapshot().unwrap().epoch, 1);
    assert_eq!(ring.descriptor_seq.load(Ordering::SeqCst), 0);
    let frames = std::vec![PcmFrame { left: 12, right: 34 }; RING_FRAMES as usize];
    assert!(ring.push(&frames));
    assert_eq!(ring.free(), 0);
    ring.restart_at(44_100, 20);
    assert!(!ring.push(&[PcmFrame::default()]));
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    assert_eq!(reader.position, 20);
    assert_eq!(reader.buffered(ring), 0);
    assert_eq!(ring.free(), RING_FRAMES);
    assert!(ring.push(&frames[..10]));
    assert_eq!(drain(&mut reader, ring), frames[..10]);
}

#[test]
fn descriptor_snapshot_is_coherent_under_concurrent_restarts() {
    let ring = ring();
    let writer = std::thread::spawn(move || {
        for origin in 1..=200_000 {
            ring.restart_at(if origin % 2 == 0 { 48_000 } else { 44_100 }, origin);
            if origin % 7 == 0 {
                ring.end();
            }
        }
    });
    let mut reader = Reader::new();
    for _ in 0..200_000 {
        if reader.sync(ring) {
            assert_eq!(reader.position, reader.epoch);
            assert_eq!(
                reader.rate,
                if reader.epoch.is_multiple_of(2) {
                    48_000
                } else {
                    44_100
                }
            );
            assert_eq!(reader.tail, 0);
        }
    }
    writer.join().unwrap();
    reader.sync(ring);
    assert_eq!(reader.position, 200_000);
}

#[test]
fn slow_large_tag_prime_is_incremental_and_does_not_starve_playing_deck() {
    use std::cell::Cell;
    struct Slow {
        inner: Memory,
        clock: Cell<u64>,
    }
    impl SourceFiles for Slow {
        type File = &'static [u8];
        fn open(&mut self, p: &str) -> Option<Self::File> {
            self.inner.open(p)
        }
        fn len(&self, f: &Self::File) -> u64 {
            self.inner.len(f)
        }
        fn read_at(&mut self, f: &mut Self::File, o: u64, b: &mut [u8]) -> Option<usize> {
            self.clock.set(self.clock.get() + 500);
            self.inner.read_at(f, o, b)
        }
        fn close(&mut self, f: Self::File) {
            self.inner.close(f)
        }
        fn now_us(&self) -> u64 {
            self.clock.get()
        }
    }
    // Retain the real fixture's MP3 frames, replacing just its ID3 padding.
    let old_tag = 10
        + LONG_ID3[6..10]
            .iter()
            .fold(0usize, |n, b| (n << 7) | usize::from(*b));
    let tag_len = 512 * 1024usize;
    let mut tagged = std::vec![0u8;tag_len+10];
    tagged[..6].copy_from_slice(b"ID3\x04\x00\x00");
    for (i, b) in tagged[6..10].iter_mut().enumerate() {
        *b = ((tag_len >> (7 * (3 - i))) & 127) as u8;
    }
    tagged.extend_from_slice(&LONG_ID3[old_tag..]);
    let tagged = Box::leak(tagged.into_boxed_slice());
    let mut files = [
        Slow {
            inner: Memory::new(&[LONG_ID3]),
            clock: Cell::new(0),
        },
        Slow {
            inner: Memory::new(&[tagged]),
            clock: Cell::new(0),
        },
    ];
    let rings = [ring(), ring()];
    let commands = [Commands::new(), Commands::new()];
    let mut producers = [Producer::new(), Producer::new()];
    commands[0].load(0);
    run(
        &mut producers[0],
        &mut files[0].inner,
        rings[0],
        &commands[0],
    );
    let mut reader = Reader::new();
    reader.sync(rings[0]);
    commands[1].load(0);
    let mut primed = false;
    let mut rounds = 0;
    let mut minimum = RING_FRAMES;
    let mut max_reads = 0;
    for turn in 0..10_000 {
        rounds += 1;
        let before_time = files[0].clock.get() + files[1].clock.get();
        let order = crate::producer_schedule::order(rings, [true, false], [0, 0], turn % 2 == 0);
        assert_eq!(order, [0, 1]);
        for i in order {
            let reads = files[i].inner.reads;
            let (_, event) = producers[i].poll(&mut files[i], rings[i], &commands[i], path);
            max_reads = max_reads.max(files[i].inner.reads - reads);
            if i == 1 && matches!(event, Some(Event::Ready { .. })) {
                primed = true;
            }
        }
        let elapsed = files[0].clock.get() + files[1].clock.get() - before_time + 1000;
        let frames = (elapsed * 44_100).div_ceil(1_000_000);
        for _ in 0..frames {
            assert!(
                reader.pop(rings[0]).is_some(),
                "playing source starved while priming other deck"
            );
        }
        reader.commit(rings[0]);
        minimum = minimum.min(reader.buffered(rings[0]));
        if primed {
            break;
        }
    }
    assert!(primed);
    assert!(rings[1].prime_max_us.load(Ordering::Relaxed) >= 1000);
    // The decoder may skip the complete ID3 padding by offset; require
    // continuation across turns without assuming an inefficient linear scan.
    assert!(rounds > 1);
    assert!(max_reads <= 2, "per-turn read budget exceeded: {max_reads}");
    assert!(
        minimum > RING_FRAMES / 2,
        "unexpectedly low headroom: {minimum}"
    );
    // Scheduling compares time coverage, not raw frame counts.
    rings[0].restart(48_000);
    rings[1].restart(44_100);
    assert_eq!(
        crate::producer_schedule::order(rings, [false, true], [0, 0], false),
        [1, 0]
    );
}

#[test]
fn riff_metadata_walk_yields_between_chunks() {
    let mut wav = std::vec::Vec::from(&WAV[..12]);
    for _ in 0..300 {
        wav.extend_from_slice(b"JUNK\x00\x00\x00\x00");
    }
    wav.extend_from_slice(&WAV[12..]);
    let len = wav.len() as u32 - 8;
    wav[4..8].copy_from_slice(&len.to_le_bytes());
    let fixture = Box::leak(wav.into_boxed_slice());
    let mut files = Memory::new(&[fixture]);
    let ring = ring();
    let commands = Commands::new();
    let mut producer = Producer::new();
    commands.load(0);
    let mut ready = false;
    for turn in 0..1000 {
        let reads = files.reads;
        let (busy, event) = producer.poll(&mut files, ring, &commands, path);
        assert!(busy);
        if turn < 300 {
            assert!(files.reads - reads <= 1);
            assert!(event.is_none());
        }
        if matches!(event, Some(Event::Ready { .. })) {
            ready = true;
            break;
        }
    }
    assert!(ready);
    let mut reader = Reader::new();
    run(&mut producer, &mut files, ring, &commands);
    assert_eq!(drain(&mut reader, ring), wav_pcm());
}

#[test]
fn producer_priority_uses_rate_and_pitch_time_coverage() {
    let rings = [ring(), ring()];
    rings[0].restart(44_100);
    rings[1].restart(48_000);
    let frames = std::vec![PcmFrame::default();2000];
    assert!(rings[0].push(&frames));
    assert!(rings[1].push(&frames));
    assert_eq!(
        crate::producer_schedule::order(rings, [true, true], [0, 0], false),
        [1, 0]
    );
    assert_eq!(
        crate::producer_schedule::order(rings, [true, true], [1000, 0], false),
        [0, 1]
    );
    assert_eq!(rings[0].low_supply_us.load(Ordering::Relaxed), u32::MAX);
    rings[0].record_supply(0);
    let first = rings[0].low_supply_us.load(Ordering::Relaxed);
    rings[0].record_supply(1000);
    assert!(rings[0].low_supply_us.load(Ordering::Relaxed) < first);
}

/// A far MP3 seek decodes from the start of the file until the frame walk
/// has run; with it, CBR, Xing-tagged VBR and untagged VBR seeks resume at
/// an indexed frame. Untagged VBR resets the codec there and counts the
/// walked frames itself. The PCM after the seek is identical either way.
#[test]
fn the_mp3_frame_walk_makes_a_far_seek_cheap() {
    const TARGET_MS: u32 = 7_000;
    fn seek(
        producer: &mut Producer<&'static [u8]>,
        files: &mut Memory,
        ring: &Ring,
        commands: &Commands,
    ) -> (usize, Vec<PcmFrame>) {
        let before = files.reads;
        commands.seek(TARGET_MS);
        let mut reads = None;
        for _ in 0..100_000 {
            if let (_, Some(Event::Seeked { ok, .. })) = producer.poll(files, ring, commands, path)
            {
                assert!(ok);
                reads = Some(files.reads - before);
                break;
            }
        }
        let reads = reads.expect("the seek completes");
        let mut reader = Reader::new();
        reader.sync(ring);
        assert_eq!(reader.position_ms(), TARGET_MS);
        let mut frames = Vec::new();
        while frames.len() < 4_096 {
            run(producer, files, ring, commands);
            reader.sync(ring);
            while let Some(frame) = reader.pop(ring) {
                frames.push(frame);
            }
            reader.commit(ring);
        }
        (reads, frames)
    }
    for data in [
        fixture!("long.mp3"),
        LONG_ID3,
        fixture!("long-vbr-untagged.mp3"),
    ] {
        // Cold: seek as soon as the load is ready, before the walk runs
        // (it only runs while the ring is full).
        let (ring, commands) = (ring(), Commands::new());
        let mut files = Memory::new(&[data]);
        let mut producer = Producer::new();
        commands.load(0);
        while !matches!(
            producer.poll(&mut files, ring, &commands, path).1,
            Some(Event::Ready { .. })
        ) {}
        assert!(!producer.indexed());
        let (cold_reads, cold) = seek(&mut producer, &mut files, ring, &commands);

        let (ring, commands) = (self::ring(), Commands::new());
        let mut files = Memory::new(&[data]);
        let mut producer = Producer::new();
        commands.load(0);
        // Once the ring is full the producer stays busy with the walk, one
        // read per turn, until the end of the file.
        run(&mut producer, &mut files, ring, &commands);
        assert!(producer.indexed());
        let (busy, event) = producer.poll(&mut files, ring, &commands, path);
        assert!(!busy && event.is_none());
        let (warm_reads, warm) = seek(&mut producer, &mut files, ring, &commands);
        assert!(
            warm_reads * 4 < cold_reads,
            "indexed seek {warm_reads} reads, cold {cold_reads}"
        );
        assert_eq!(warm, cold);
    }
}

/// Source frame `n` as a recognizable PCM value.
fn numbered(n: u32) -> PcmFrame {
    PcmFrame {
        left: (n % 30_000) as i16,
        right: -((n % 30_000) as i16),
    }
}

/// Pushes source frames `from..from + count` (fits the ring).
fn feed(ring: &Ring, from: u32, count: u32) {
    let frames: Vec<PcmFrame> = (from..from + count).map(numbered).collect();
    assert!(ring.push(&frames));
}

/// The source frame numbers of the next `count` frames the reader plays.
fn take(reader: &mut Reader, ring: &Ring, count: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for _ in 0..count {
        let frame = reader.pop(ring).expect("a frame");
        out.push(frame.left as u32);
    }
    reader.commit(ring);
    out
}

fn span(from: u32, to: u32) -> Vec<u32> {
    (from..to).collect()
}

#[test]
fn a_loop_replays_its_history_exactly_and_exits_into_the_ring() {
    let ring = ring();
    ring.restart_at(44_100, 1_000);
    feed(ring, 1_000, 20_000);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    assert_eq!(take(&mut reader, ring, 500), span(1_000, 1_500));
    // Loop In behind the playhead, Loop Out ahead of it.
    reader.set_loop(1_200, 1_600);
    let played = take(&mut reader, ring, 1_000);
    let expected = [
        span(1_500, 1_600),
        span(1_200, 1_600),
        span(1_200, 1_600),
        span(1_200, 1_300),
    ]
    .concat();
    assert_eq!(played, expected);
    // The ring waits at the loop end while the history replays.
    assert_eq!(reader.buffered(ring), 20_000 - 600);
    assert_eq!(reader.position_ms(), 1_300 * 1000 / 44_100);
    assert!(reader.looping());
    // Exit: the rest of the loop from the history, then the ring onwards.
    reader.clear_loop();
    assert_eq!(take(&mut reader, ring, 400), span(1_300, 1_700));
    assert!(!reader.looping());
    assert_eq!(reader.buffered(ring), 20_000 - 700);
}

#[test]
fn a_loop_out_just_behind_the_playhead_wraps_at_once() {
    let ring = ring();
    ring.restart_at(44_100, 0);
    feed(ring, 0, 4_000);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 1_500);
    // The reported position rounds down to the millisecond.
    reader.set_loop(1_000, 1_498);
    assert_eq!(
        take(&mut reader, ring, 600),
        [span(1_000, 1_498), span(1_000, 1_102)].concat()
    );
}

#[test]
fn a_longer_loop_plays_on_from_the_ring_before_it_wraps() {
    let ring = ring();
    ring.restart_at(44_100, 0);
    feed(ring, 0, 8_000);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 1_000);
    reader.set_loop(500, 1_000);
    assert_eq!(take(&mut reader, ring, 200), span(500, 700));
    // Loop Double while replaying: history up to the ring, the ring to the
    // new end, then the wrap.
    reader.set_loop(500, 1_500);
    assert_eq!(
        take(&mut reader, ring, 1_000),
        [span(700, 1_500), span(500, 700)].concat()
    );
}

#[test]
fn a_loop_start_before_the_history_is_reported_once_and_played_through() {
    let ring = ring();
    ring.restart_at(44_100, 5_000);
    feed(ring, 5_000, 4_000);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 100);
    // Loop In before this epoch (the playhead was seeked past it).
    reader.set_loop(4_000, 5_200);
    assert_eq!(take(&mut reader, ring, 150), span(5_100, 5_250));
    assert_eq!(reader.take_loop_miss(90), Some(90));
    assert_eq!(reader.take_loop_miss(90), None);
    // The caller seeks to the loop start (landing one frame late); from then
    // on the history holds the whole loop and every wrap is seamless.
    ring.restart_at(44_100, 4_001);
    feed(ring, 4_001, 4_000);
    assert!(reader.sync(ring));
    assert_eq!(
        take(&mut reader, ring, 1_300),
        [span(4_001, 5_200), span(4_001, 4_102)].concat()
    );
    assert_eq!(reader.take_loop_miss(90), None);
}

#[test]
fn a_loop_older_than_the_history_is_reported() {
    let ring = ring();
    ring.restart_at(48_000, 0);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    let mut next = 0;
    while next < HISTORY_FRAMES + 2_000 {
        feed(ring, next, 1_000);
        next += 1_000;
        take(&mut reader, ring, 1_000);
    }
    feed(ring, next, 1_000);
    // [100, HISTORY + 2_500): its start fell out of the history.
    reader.set_loop(100, next + 500);
    take(&mut reader, ring, 600);
    assert_eq!(reader.take_loop_miss(2), Some(2));
}

#[test]
fn a_loop_end_past_the_end_of_the_track_wraps_at_the_last_frame() {
    let ring = ring();
    ring.restart_at(44_100, 0);
    feed(ring, 0, 4_000);
    ring.end();
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 3_800);
    // 4 BEAT near the end of the track: the loop end is never reached.
    reader.set_loop(2_000, 50_000);
    assert_eq!(
        take(&mut reader, ring, 4_200),
        [span(3_800, 4_000), span(2_000, 4_000), span(2_000, 4_000)].concat()
    );
    assert!(reader.looping());
    // Exit: the rest of the history, then the track finishes.
    assert_eq!(take(&mut reader, ring, 100), span(2_000, 2_100));
    reader.clear_loop();
    assert_eq!(take(&mut reader, ring, 1_900), span(2_100, 4_000));
    assert!(!reader.looping());
    assert_eq!(reader.pop(ring), None);
}

#[test]
fn a_loop_set_at_the_very_end_lets_the_track_finish() {
    let ring = ring();
    ring.restart_at(44_100, 0);
    feed(ring, 0, 4_000);
    ring.end();
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 3_900);
    reader.set_loop(3_900, 50_000);
    assert_eq!(take(&mut reader, ring, 100), span(3_900, 4_000));
    assert_eq!(reader.pop(ring), None);
    assert!(!reader.looping());
}

#[test]
fn a_seek_past_the_loop_end_returns_to_the_loop() {
    let ring = ring();
    ring.restart_at(44_100, 0);
    feed(ring, 0, 4_000);
    let mut reader = Reader::new();
    assert!(reader.sync(ring));
    take(&mut reader, ring, 1_100);
    reader.set_loop(1_000, 1_200);
    // Beat Jump past the loop end: a new epoch, without the loop start.
    ring.restart_at(44_100, 3_000);
    feed(ring, 3_000, 4_000);
    assert!(reader.sync(ring));
    assert_eq!(take(&mut reader, ring, 10), span(3_000, 3_010));
    assert_eq!(reader.take_loop_miss(22), Some(22));
    // The engine's seek to the start: from there the loop replays.
    ring.restart_at(44_100, 1_000);
    feed(ring, 1_000, 4_000);
    assert!(reader.sync(ring));
    assert_eq!(
        take(&mut reader, ring, 500),
        [span(1_000, 1_200), span(1_000, 1_200), span(1_000, 1_100)].concat()
    );
    assert_eq!(reader.take_loop_miss(22), None);
}
