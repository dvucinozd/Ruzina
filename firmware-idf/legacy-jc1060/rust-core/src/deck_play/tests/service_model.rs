//! Offline service sensitivity, not a calibrated target/CPU model.
use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

const RATE: u64 = 44_100;

struct Timeline {
    now: u64,
    remainder: u64,
    playing: [bool; 2],
    rings: [&'static Ring; 2],
    readers: [Reader; 2],
    missing: [u64; 2],
    consumed: [u64; 2],
    first_missing: Option<u64>,
    consume_until: Option<u64>,
}

impl Timeline {
    /// Consume during blocking reads as well as between polls. Carry the
    /// fractional sample debt; repeated small intervals must not round up.
    fn advance(&mut self, us: u64) {
        let consume_us = self
            .consume_until
            .map_or(us, |deadline| us.min(deadline.saturating_sub(self.now)));
        self.now += us;
        let frames = self.remainder + consume_us * RATE;
        self.remainder = frames % 1_000_000;
        for deck in 0..2 {
            if !self.playing[deck] {
                continue;
            }
            for _ in 0..frames / 1_000_000 {
                if self.readers[deck].pop(self.rings[deck]).is_some() {
                    self.consumed[deck] += 1;
                } else {
                    self.missing[deck] += 1;
                    self.first_missing.get_or_insert(self.now);
                }
            }
            self.readers[deck].commit(self.rings[deck]);
        }
    }
}

struct VirtualFiles {
    inner: Memory,
    timeline: Rc<RefCell<Timeline>>,
    read_delay_us: u64,
}

impl SourceFiles for VirtualFiles {
    type File = &'static [u8];
    fn open(&mut self, p: &str) -> Option<Self::File> {
        self.inner.open(p)
    }
    fn len(&self, f: &Self::File) -> u64 {
        self.inner.len(f)
    }
    fn read_at(&mut self, f: &mut Self::File, o: u64, out: &mut [u8]) -> Option<usize> {
        self.timeline.borrow_mut().advance(self.read_delay_us);
        self.inner.read_at(f, o, out)
    }
    fn close(&mut self, f: Self::File) {
        self.inner.close(f);
    }
    fn now_us(&self) -> u64 {
        self.timeline.borrow().now
    }
}

#[derive(Debug)]
struct Observation {
    elapsed_us: u64,
    missing: [u64; 2],
    consumed: [u64; 2],
    reads: [usize; 2],
    max_poll_us: u64,
}

fn simulate(playing: [bool; 2], read_delay_us: u64, duration_us: u64) -> Observation {
    let rings = [ring(), ring()];
    let timeline = Rc::new(RefCell::new(Timeline {
        now: 0,
        remainder: 0,
        playing: [false; 2],
        rings,
        readers: [Reader::new(), Reader::new()],
        missing: [0; 2],
        consumed: [0; 2],
        first_missing: None,
        consume_until: None,
    }));
    let mut files = core::array::from_fn::<_, 2, _>(|_| VirtualFiles {
        inner: Memory::new(&[LONG_ID3]),
        timeline: Rc::clone(&timeline),
        read_delay_us,
    });
    let mut producers = [Producer::new(), Producer::new()];
    let commands = [Commands::new(), Commands::new()];
    // Begin with the real full PCM rings. No playback consumes load/prime
    // time. The eight-second CC0 source is longer than the bounded run.
    for deck in 0..2 {
        commands[deck].load(0);
        let mut idle = false;
        for _ in 0..100_000 {
            let (busy, event) =
                producers[deck].poll(&mut files[deck], rings[deck], &commands[deck], path);
            assert!(!matches!(event, Some(Event::Failed { .. })));
            if !busy {
                idle = true;
                break;
            }
        }
        assert!(idle && rings[deck].rate.load(Ordering::Acquire) == RATE as u32);
    }
    let start_reads = files.each_ref().map(|f| f.inner.reads);
    let started = {
        let mut state = timeline.borrow_mut();
        for (deck, ring) in rings.iter().enumerate() {
            assert!(state.readers[deck].sync(ring));
        }
        state.playing = playing;
        state.remainder = 0;
        state.consume_until = Some(state.now + duration_us);
        state.now
    };
    let mut max_poll_us = 0;
    for turn in 0..100_000 {
        if timeline.borrow().now - started >= duration_us
            || timeline.borrow().first_missing.is_some()
        {
            break;
        }
        let order = crate::producer_schedule::order(rings, playing, [0, 0], turn % 2 == 0);
        for deck in order {
            let before = files[deck].now_us();
            let (_, event) =
                producers[deck].poll(&mut files[deck], rings[deck], &commands[deck], path);
            assert!(!matches!(
                event,
                Some(Event::Failed { .. } | Event::Ended { .. })
            ));
            max_poll_us = max_poll_us.max(files[deck].now_us() - before);
            if timeline.borrow().first_missing.is_some() {
                break;
            }
        }
        // Explicit synthetic owner overhead. Decoder compute/preemption is
        // zero-cost here; only reads advance time inside the real poll budget.
        timeline.borrow_mut().advance(100);
    }
    let state = timeline.borrow();
    assert!(
        state.now - started >= duration_us || state.first_missing.is_some(),
        "model turn bound exhausted"
    );
    Observation {
        elapsed_us: state.now - started,
        missing: state.missing,
        consumed: state.consumed,
        reads: core::array::from_fn(|d| files[d].inner.reads - start_reads[d]),
        max_poll_us,
    }
}

#[test]
fn shared_read_timeline_exposes_dual_source_pressure() {
    for delay in [0, 500, 2_000, 5_000, 10_000, 20_000, 40_000] {
        let single = simulate([true, false], delay, 5_000_000);
        let dual = simulate([true, true], delay, 5_000_000);
        std::println!("SERVICE_MODEL delay_us={delay} single={single:?} dual={dual:?}");
        if delay == 0 {
            assert_eq!(single.missing, [0, 0]);
            assert_eq!(dual.missing, [0, 0]);
            assert_eq!(dual.consumed, [5 * RATE; 2]);
        }
        // The elapsed-time check occurs after an indivisible action. A
        // single read can exceed 1000 us, but the next action must yield.
        assert!(single.max_poll_us <= delay.max(POLL_BUDGET_US));
        assert!(dual.max_poll_us <= delay.max(POLL_BUDGET_US));
        assert!(single.elapsed_us >= 5_000_000 || single.missing != [0, 0]);
        assert!(dual.reads.into_iter().all(|n| n > 0));
        if delay == 40_000 {
            assert_eq!(single.missing, [0, 0]);
            assert!(dual.missing.into_iter().all(|n| n > 0));
            assert!(dual.elapsed_us < 5_000_000);
        }
    }
}

#[test]
fn consumption_stops_at_the_requested_bound_inside_a_slow_read() {
    let single = simulate([true, false], 40_000, 4_000_000);
    let dual = simulate([true, true], 40_000, 4_000_000);
    assert_eq!(single.missing, [0, 0]);
    assert_eq!(dual.missing, [0, 0]);
    assert_eq!(dual.consumed, [4 * RATE; 2]);
    assert!(
        dual.elapsed_us > 4_000_000,
        "read should straddle the bound"
    );
}

struct HostFiles {
    inner: Memory,
    started: Instant,
    read_delay: Duration,
}

impl SourceFiles for HostFiles {
    type File = &'static [u8];
    fn open(&mut self, p: &str) -> Option<Self::File> {
        self.inner.open(p)
    }
    fn len(&self, f: &Self::File) -> u64 {
        self.inner.len(f)
    }
    fn read_at(&mut self, f: &mut Self::File, o: u64, out: &mut [u8]) -> Option<usize> {
        if !self.read_delay.is_zero() {
            std::thread::sleep(self.read_delay);
        }
        self.inner.read_at(f, o, out)
    }
    fn close(&mut self, f: Self::File) {
        self.inner.close(f);
    }
    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }
}

/// Explicit manual release-mode measurement. No host duration threshold is a
/// target acceptance gate; delayed and memory readers must produce exact PCM.
#[test]
#[ignore = "manual host release benchmark; not ESP32-P4 timing"]
fn host_source_service_benchmark() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "run with --release"
    );
    let mut baseline = None;
    for delay in [0, 200] {
        for repetition in 0..5 {
            let ring = ring();
            let commands = Commands::new();
            let mut producer = Producer::new();
            let mut reader = Reader::new();
            let mut pcm_output = Vec::with_capacity(8 * RATE as usize);
            let mut files = HostFiles {
                inner: Memory::new(&[LONG_ID3]),
                started: Instant::now(),
                read_delay: Duration::from_micros(delay),
            };
            commands.load(0);
            let mut frames = 0u64;
            let mut hash = 0xcbf29ce484222325u64;
            let mut finished = false;
            for _ in 0..1_000_000 {
                let start = files.now_us();
                let head = ring.profile_head();
                let (busy, event) = producer.poll(&mut files, ring, &commands, path);
                producer.profile_poll(ring, start, files.now_us(), busy, head);
                assert!(!matches!(event, Some(Event::Failed { .. })));
                reader.sync(ring);
                while let Some(pcm) = reader.pop(ring) {
                    pcm_output.push(pcm);
                    for byte in pcm
                        .left
                        .to_le_bytes()
                        .into_iter()
                        .chain(pcm.right.to_le_bytes())
                    {
                        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                    }
                    frames += 1;
                }
                reader.commit(ring);
                if !busy {
                    finished = true;
                    break;
                }
            }
            let elapsed = files.now_us();
            assert!(finished && frames > 7 * RATE);
            let expected = baseline.get_or_insert_with(|| (frames, hash, pcm_output.clone()));
            assert_eq!((frames, hash), (expected.0, expected.1));
            assert_eq!(pcm_output, expected.2, "reader delay changed decoded PCM");
            let words = producer.profile.words();
            let stage_us = |stage: usize| {
                u64::from(words[17 + stage * 4]) | (u64::from(words[18 + stage * 4]) << 32)
            };
            std::println!(
                "HOST_SOURCE_BENCH delay_us={delay} repetition={repetition} elapsed_us={elapsed} frames={frames} pcm_fnv64={hash:016x} reads={} poll_us={} step_us={} read_us={}",
                files.inner.reads,
                stage_us(0),
                stage_us(2),
                stage_us(3)
            );
        }
    }
    // An explicit host-only output permits byte-for-byte cross-build PCM
    // comparison. Never replace an existing artifact or open a serial port.
    if let Some(path) = std::env::var_os("RUZINA_BENCH_PCM_OUT") {
        use std::io::Write;
        let mut output = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap(),
        );
        for pcm in baseline.unwrap().2 {
            output.write_all(&pcm.left.to_le_bytes()).unwrap();
            output.write_all(&pcm.right.to_le_bytes()).unwrap();
        }
        output.flush().unwrap();
    }
}
