use super::*;
use std::vec::Vec;

const RATE: f64 = 48_000.0;

/// A source of `frames` frames from `sample`, frame by frame from frame 0.
fn source(sample: impl Fn(f64) -> f64) -> impl FnMut() -> Option<PcmFrame> {
    let mut n = 0u32;
    move || {
        let value = sample(f64::from(n) / RATE) as i16;
        n += 1;
        Some(PcmFrame {
            left: value,
            right: value,
        })
    }
}

fn sine(hz: f64) -> impl Fn(f64) -> f64 {
    move |t| (2.0 * core::f64::consts::PI * hz * t).sin() * 12_000.0
}

/// A chord-like tone: a 120 Hz fundamental and four harmonics, so one period is exactly
/// 400 frames at 48 kHz.
fn chord(t: f64) -> f64 {
    (1..=5)
        .map(|k| (2.0 * core::f64::consts::PI * 120.0 * f64::from(k) * t + f64::from(k)).sin())
        .sum::<f64>()
        * 4_000.0
}

/// `frames` stretched frames at `tempo`, and the frames that took from the source.
fn stretch(
    tempo: f32,
    frames: usize,
    mut pull: impl FnMut() -> Option<PcmFrame>,
) -> (Vec<PcmFrame>, u64) {
    let mut keylocks = Keylock::boxed_pair();
    let keylock = &mut keylocks[0];
    let output = (0..frames)
        .map(|_| keylock.next(tempo, &mut pull).expect("a frame"))
        .collect();
    (output, keylock.pulled())
}

/// Upward zero crossings of the left channel: a sine's periods.
fn periods(frames: &[PcmFrame]) -> usize {
    frames
        .windows(2)
        .filter(|pair| pair[0].left < 0 && pair[1].left >= 0)
        .count()
}

#[test]
fn at_the_tracks_own_tempo_the_output_is_the_input() {
    let (output, _) = stretch(1.0, 20_000, source(chord));
    let mut input = source(chord);
    for (n, frame) in output.iter().enumerate() {
        assert_eq!(*frame, input().unwrap(), "frame {n}");
    }
}

#[test]
fn the_key_stays_and_the_track_moves_at_the_tempo() {
    for tempo in [0.8f32, 0.92, 1.06, 1.25, 1.6] {
        let (output, pulled) = stretch(tempo, 96_000, source(sine(1_000.0)));
        let found = periods(&output) as f64;
        assert!(
            (found - 2_000.0).abs() <= 10.0,
            "tempo {tempo}: {found} periods"
        );
        let expected = 96_000.0 * f64::from(tempo);
        assert!(
            (pulled as f64 - expected).abs() <= 2_500.0,
            "tempo {tempo}: pulled {pulled}, expected {expected}"
        );
    }
}

#[test]
fn a_chord_stays_whole_across_the_grain_joins() {
    for tempo in [0.87f32, 1.13, 1.4] {
        let (output, _) = stretch(tempo, 96_000, source(chord));
        // A perfect stretch of a periodic tone is the same tone: each frame repeats one
        // period (400 frames) later. Badly joined grains break that at every join.
        let left: Vec<f64> = output[4_800..].iter().map(|f| f64::from(f.left)).collect();
        let residual: f64 = left.windows(401).map(|w| (w[400] - w[0]).powi(2)).sum();
        let energy: f64 = left.iter().map(|s| s * s).sum();
        let ratio = residual / energy;
        assert!(ratio < 0.005, "tempo {tempo}: residual {ratio}");
    }
}

/// A bright tone: harmonics of 1200 Hz up to 8.4 kHz, so one period is exactly 40 frames
/// at 48 kHz. The search compares only every 4th or 16th frame, far below these
/// frequencies; the joins must still line the waveforms up.
fn bright(t: f64) -> f64 {
    [1.0f64, 2.0, 3.0, 5.0, 7.0]
        .into_iter()
        .map(|k| (2.0 * core::f64::consts::PI * 1_200.0 * k * t + k).sin())
        .sum::<f64>()
        * 3_000.0
}

#[test]
fn a_bright_tone_stays_whole_across_the_grain_joins() {
    for tempo in [0.87f32, 0.97, 1.03, 1.13] {
        let (output, _) = stretch(tempo, 96_000, source(bright));
        // As for the chord: each frame of a perfect stretch repeats a period later.
        let left: Vec<f64> = output[4_800..].iter().map(|f| f64::from(f.left)).collect();
        let residual: f64 = left.windows(41).map(|w| (w[40] - w[0]).powi(2)).sum();
        let energy: f64 = left.iter().map(|s| s * s).sum();
        let ratio = residual / energy;
        // 0.0114 at tempos 0.87 and 1.13; with the fine search on every 8th frame, 0.0148.
        assert!(ratio < 0.0125, "tempo {tempo}: residual {ratio}");
    }
}

/// `sample` on the left and its inverse on the right: both channels as one are silent, so
/// every candidate the search compares scores the same.
fn opposed(sample: impl Fn(f64) -> f64) -> impl FnMut() -> Option<PcmFrame> {
    let mut mono = source(sample);
    move || {
        let frame = mono()?;
        Some(PcmFrame {
            left: frame.left,
            right: frame.left.wrapping_neg(),
        })
    }
}

#[test]
fn a_run_does_not_hear_the_run_before_it() {
    for tempo in [0.5f32, 0.6, 1.5, 2.0] {
        // One keylock plays a loud other track first, round its whole window, and stops; a
        // fresh one does not. The track after it gives the search nothing to choose by, so
        // it takes the earliest start it may.
        let mut keylocks = Keylock::boxed_pair();
        let mut before = source(chord);
        for _ in 0..2 * WINDOW {
            keylocks[0].next(1.0, &mut before).unwrap();
        }
        keylocks[0].stop();
        let (mut used, mut fresh) = (opposed(sine(700.0)), opposed(sine(700.0)));
        for n in 0..4_000 {
            assert_eq!(
                keylocks[0].next(tempo, &mut used),
                keylocks[1].next(tempo, &mut fresh),
                "tempo {tempo}: frame {n}"
            );
        }
    }
}

#[test]
fn a_starved_keylock_waits_and_a_stopped_one_starts_over() {
    let mut keylocks = Keylock::boxed_pair();
    let keylock = &mut keylocks[1];
    let mut tone = source(sine(500.0));
    let mut produced = 0;
    // The deck supplies in bursts with gaps; every frame still comes out, in order.
    let mut reference = source(sine(500.0));
    for round in 0..4_000 {
        let mut pull = || (round % 3 != 0).then(|| tone().unwrap());
        if let Some(frame) = keylock.next(1.0, &mut pull) {
            assert_eq!(frame, reference().unwrap());
            produced += 1;
        }
    }
    assert!(produced > 2_000);
    assert!(keylock.lag() <= 2 * GRAIN);
    keylock.stop();
    assert_eq!(keylock.lag(), 0);
    assert!(keylock.next(1.2, || None).is_none());
    assert_eq!(keylock.pulled(), 0);
}

#[test]
fn the_lead_on_the_heard_position_stays_within_two_grains() {
    let mut keylocks = Keylock::boxed_pair();
    let keylock = &mut keylocks[0];
    let mut pull = source(chord);
    let mut most = 0;
    for _ in 0..48_000 {
        keylock.next(1.5, &mut pull).unwrap();
        most = most.max(keylock.lag());
    }
    assert!(most <= 2 * GRAIN + SEEK, "lead {most}");
}

#[test]
fn the_heard_position_follows_the_tempo_not_the_grains() {
    for tempo in [0.92f32, 1.08, 1.5] {
        let mut keylocks = Keylock::boxed_pair();
        let keylock = &mut keylocks[0];
        let mut pull = source(chord);
        let (mut heard, mut played) = (0.0f64, 0.0f64);
        for n in 1..=48_000u32 {
            keylock.next(tempo, &mut pull).unwrap();
            // Where the deck is after `n` frames at the tempo, as each lead tells it.
            let line = f64::from(n) * f64::from(tempo);
            let pulled = keylock.pulled() as f64;
            heard = heard.max((pulled - f64::from(keylock.heard_lag()) - line).abs());
            played = played.max((pulled - f64::from(keylock.lag()) - line).abs());
        }
        std::println!("tempo {tempo}: heard {heard} frames, grain {played} frames off");
        assert!(
            heard <= 2.0,
            "tempo {tempo}: heard {heard} frames off the line"
        );
        // The grains themselves wander about the line by the search's moves.
        assert!(played > 32.0, "tempo {tempo}: grain {played}");
    }
}

#[test]
#[ignore = "timing on the host, run by hand"]
fn bench_keylock_cost() {
    for tempo in [1.0f32, 1.13] {
        let mut keylocks = Keylock::boxed_pair();
        let keylock = &mut keylocks[0];
        let mut tone = source(chord);
        let frames: Vec<PcmFrame> = (0..800_000).map(|_| tone().unwrap()).collect();
        let mut at = 0;
        let mut pull = || {
            at += 1;
            Some(frames[at - 1])
        };
        let start = std::time::Instant::now();
        let mut sum = 0i64;
        for _ in 0..480_000 {
            sum += i64::from(keylock.next(tempo, &mut pull).unwrap().left);
        }
        let per = start.elapsed().as_nanos() as f64 / 480_000.0;
        std::println!("tempo {tempo}: {per:.1} ns a frame ({sum})");
    }
}
