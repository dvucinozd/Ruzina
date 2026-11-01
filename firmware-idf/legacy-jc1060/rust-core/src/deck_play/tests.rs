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

/// Tracks by catalog index, read from memory.
struct Memory {
    files: Vec<&'static [u8]>,
    reads: usize,
    fail_reads_after: Option<usize>,
}

impl Memory {
    fn new(files: &[&'static [u8]]) -> Self {
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
    let storage = std::vec![PcmFrame::default(); RING_FRAMES as usize].into_boxed_slice();
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
    assert_eq!(
        events,
        [Event::Ready {
            index: 0,
            codec: 2,
            rate: 44_100
        }]
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
    assert_eq!(
        run(&mut producer, &mut files, ring, &commands),
        [Event::Ready {
            index: 0,
            codec: 2,
            rate: 44_100
        }]
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
fn ring_keeps_its_first_storage() {
    let ring = ring();
    let other = std::vec![PcmFrame::default(); RING_FRAMES as usize].into_boxed_slice();
    assert!(!ring.attach(Box::leak(other)));
    let short = std::vec![PcmFrame::default(); 8].into_boxed_slice();
    assert!(!Ring::new().attach(Box::leak(short)));
    assert!(!Ring::new().push(&[PcmFrame::default()]));
}
