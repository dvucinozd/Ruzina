use super::*;
use std::vec;
use std::vec::Vec;

#[cfg(feature = "idf-uac-profile")]
#[test]
fn profiling_six_packet_fill_preserves_pcm_and_failed_exit() {
    use crate::uac_profile::{self, Stage};
    let selected = select(
        &parse_playback_formats(&flx4_config()).unwrap(),
        UsbSpeed::High,
    )
    .unwrap();
    let make_state = || State {
        selected: Some(selected),
        cadence: Cadence::new(selected.rate),
        map: Some(ChannelMap::new(4, Some(MAIN), Some(HEADPHONES)).unwrap()),
        phase: 0,
        packets: 0,
        frames: 0,
        toned_packets: 0,
        rejection: [0; 3],
        feedback: [0; 3],
    };
    let run = |enabled: bool, capacity: usize| {
        let mut state = make_state();
        let mut output = vec![0; capacity];
        let mut sizes = [0; 6];
        let mut ordinal = 0i16;
        if enabled {
            uac_profile::begin();
        }
        let bytes = {
            let _fill = uac_profile::fill_scope();
            fill_from(&mut state, &mut output, &mut sizes, |_, main, hp| {
                let _graph = uac_profile::scope(Stage::Graph);
                uac_profile::advance(7);
                for (m, h) in main.iter_mut().zip(hp) {
                    *m = PcmFrame {
                        left: ordinal,
                        right: -ordinal,
                    };
                    *h = PcmFrame {
                        left: ordinal / 2,
                        right: ordinal / 3,
                    };
                    ordinal += 1;
                }
                true
            })
        };
        let mut stats = [0; 20];
        if enabled {
            assert_eq!(unsafe { uac_profile::end(stats.as_mut_ptr(), 20) }, 20);
        }
        (bytes, output, sizes, stats)
    };
    let plain = run(false, 6 * 384);
    let profiled = run(true, 6 * 384);
    assert_eq!(
        (&plain.0, &plain.1, &plain.2),
        (&profiled.0, &profiled.1, &profiled.2)
    );
    assert_eq!(profiled.0, 6 * 384);
    assert_eq!(profiled.3[2], 6);
    assert_eq!(profiled.3[4 + Stage::Graph as usize], 42);
    assert_eq!(profiled.3[4..12].iter().sum::<u32>(), profiled.3[0]);
    let fail = run(true, 384);
    assert_eq!(fail.0, 0);
    assert_eq!(fail.3[2], 2);
    assert_eq!(fail.3[1], 0);
}

/// AS alternate as in the FLX4: Type I PCM, one 48 kHz rate, isochronous
/// synchronous OUT endpoint 0x01 at a 1 ms high-speed interval.
fn alternate(alternate: u8, bytes: u8, bits: u8, max_packet: u16) -> Vec<u8> {
    let mut out = vec![9, 4, 1, alternate, 1, 1, 2, 0, 0];
    out.extend_from_slice(&[7, 0x24, 1, 1, 1, 1, 0]);
    out.extend_from_slice(&[11, 0x24, 2, 1, 4, bytes, bits, 1, 0x80, 0xbb, 0x00]);
    out.extend_from_slice(&[
        9,
        5,
        0x01,
        0x0d,
        max_packet as u8,
        (max_packet >> 8) as u8,
        4,
        0,
        0,
    ]);
    out.extend_from_slice(&[7, 0x25, 1, 1, 0, 0, 0]);
    out
}

/// AudioControl, zero-bandwidth alternate, 24-bit alt 1 and 16-bit alt 2,
/// then a MIDI streaming interface.
fn flx4_config() -> Vec<u8> {
    let mut out = vec![9, 2, 0, 0, 3, 1, 0, 0x80, 250];
    out.extend_from_slice(&[9, 4, 0, 0, 0, 1, 1, 0, 0]);
    out.extend_from_slice(&[9, 4, 1, 0, 0, 1, 2, 0, 0]);
    out.extend_from_slice(&alternate(1, 3, 24, 576));
    out.extend_from_slice(&alternate(2, 2, 16, 384));
    out.extend_from_slice(&[9, 4, 4, 0, 2, 1, 3, 0, 0]);
    out.extend_from_slice(&[7, 5, 0x82, 0x02, 0x00, 0x02, 0]);
    out.extend_from_slice(&[7, 5, 0x03, 0x02, 0x00, 0x02, 0]);
    let total = out.len() as u16;
    out[2..4].copy_from_slice(&total.to_le_bytes());
    out
}

fn sample16(packet: &[u8], frame: usize, channel: usize) -> i16 {
    let at = frame * 8 + channel * 2;
    i16::from_le_bytes([packet[at], packet[at + 1]])
}

#[test]
fn flx4_headphone_tone_is_selected_requested_and_packed() {
    a_refused_configuration_names_its_reason();
    let config = flx4_config();
    let mut info = [0u32; 10];
    // SAFETY: `config` and `info` outlive the call.
    let selected = unsafe {
        ruzina_legacy_uac_select(config.as_ptr(), config.len() as u32, 1, info.as_mut_ptr())
    };
    assert_eq!(selected, 1);
    // Interface 1, the 16-bit alternate 2 (scored highest), endpoint 0x01,
    // 384-byte packets, four channels of two bytes at 48 kHz.
    assert_eq!(&info[..7], &[1, 2, 0x01, 384, 4, 2, 48_000]);

    let mut request = [0u8; 11];
    // SAFETY: eleven writable bytes.
    let len = unsafe { ruzina_legacy_uac_control(0, request.as_mut_ptr()) };
    assert_eq!(&request[..len as usize], &[0x01, 0x0b, 2, 0, 1, 0, 0, 0]);
    // SAFETY: as above.
    let len = unsafe { ruzina_legacy_uac_control(1, request.as_mut_ptr()) };
    assert_eq!(
        &request[..len as usize],
        &[0x22, 0x01, 0, 1, 0x01, 0, 3, 0, 0x80, 0xbb, 0]
    );
    // SAFETY: as above.
    let len = unsafe { ruzina_legacy_uac_control(2, request.as_mut_ptr()) };
    assert_eq!(&request[..len as usize], &[0x01, 0x0b, 0, 0, 1, 0, 0, 0]);

    let mut out = vec![0u8; 8 * 384];
    let mut sizes = [0u32; 8];
    let total = with(|state| fill(state, &mut out, &mut sizes, true, false));
    assert_eq!(sizes, [384; 8]);
    assert_eq!(total, 8 * 384);
    // Left headphone (channel 3 of 4) carries the sine; right headphone and
    // MAIN stay silent. Frame 12 is the positive peak.
    let first = &out[..384];
    assert_eq!(sample16(first, 12, 2), 2067);
    assert_eq!(sample16(first, 36, 2), -2067);
    for frame in 0..48 {
        assert_eq!(sample16(first, frame, 0), 0);
        assert_eq!(sample16(first, frame, 1), 0);
        assert_eq!(sample16(first, frame, 3), 0);
    }

    // Silence when no deck plays; counters cover both calls.
    with(|state| fill(state, &mut out, &mut sizes, false, false));
    assert!(out.iter().all(|byte| *byte == 0));
    let mut stats = [0u32; 3];
    // SAFETY: three writable slots.
    unsafe { ruzina_legacy_uac_stats(stats.as_mut_ptr()) };
    assert_eq!(stats, [16, 16 * 48, 8]);
}

fn reject() -> [u32; 3] {
    let mut out = [u32::MAX; 3];
    // SAFETY: three writable slots.
    unsafe { ruzina_legacy_uac_reject(out.as_mut_ptr()) };
    out
}

fn select_config(config: &[u8], high_speed: bool) -> i32 {
    let mut info = [0u32; 10];
    // SAFETY: `config` and `info` outlive the call.
    unsafe {
        ruzina_legacy_uac_select(
            config.as_ptr(),
            config.len() as u32,
            i32::from(high_speed),
            info.as_mut_ptr(),
        )
    }
}

/// 10.14 feedback bytes for `frames_per_ms`.
fn feedback_wire(frames_per_ms: f64) -> [u8; 3] {
    let value = (frames_per_ms * 16_384.0).round() as u32;
    [value as u8, (value >> 8) as u8, (value >> 16) as u8]
}

fn feedback(bytes: &[u8]) -> i32 {
    // SAFETY: `bytes` outlives the call.
    unsafe { ruzina_legacy_uac_feedback(bytes.as_ptr(), bytes.len() as u32) }
}

fn feedback_stats() -> [u32; 4] {
    let mut out = [0u32; 4];
    // SAFETY: four writable slots.
    unsafe { ruzina_legacy_uac_feedback_stats(out.as_mut_ptr()) };
    out
}

/// The Inpulse 500 at full speed: selected with its feedback endpoint, then
/// paced by what it reads back.
fn inpulse_500_is_paced_by_its_feedback(config: &[u8]) {
    let mut info = [0u32; 10];
    // SAFETY: `config` and `info` outlive the call.
    let chosen = unsafe {
        ruzina_legacy_uac_select(config.as_ptr(), config.len() as u32, 0, info.as_mut_ptr())
    };
    assert_eq!(chosen, 1);
    // Interface 1 alt 1, OUT 0x01 of 540 bytes, 4 x 3 bytes at 44.1 kHz,
    // no SET_CUR; feedback on IN 0x81, refreshed every 2^5 ms.
    assert_eq!(info, [1, 1, 0x01, 540, 4, 3, 44_100, 0, 0x81, 5]);
    assert_eq!(reject(), [0, 0, 0]);
    // Before any reading: the nominal 44.1 frames per ms.
    assert_eq!(feedback_stats(), [0, 0, 0, 44_100 * 65_536 / 1000]);
    let mut out = vec![0u8; 6 * 540];
    let mut sizes = [0u32; 6];
    let mut frames = 0;
    for _ in 0..100 {
        assert!(with(|state| fill(state, &mut out, &mut sizes, false, false)) > 0);
        frames += sizes.iter().map(|size| size / 12).sum::<u32>();
    }
    assert!((i64::from(frames) - 26_460).abs() <= 1, "{frames}");
    // Between refreshes the device answers with nothing: no new pace.
    assert_eq!(feedback(&[]), 0);
    // A device 2 % fast: 600 ms then take 2 % more frames, no packet over 45.
    assert_eq!(feedback(&feedback_wire(44.982)), 1);
    let mut frames = 0;
    for _ in 0..100 {
        with(|state| fill(state, &mut out, &mut sizes, false, false));
        assert!(sizes.iter().all(|size| *size <= 540 && *size % 12 == 0));
        frames += sizes.iter().map(|size| size / 12).sum::<u32>();
    }
    assert!((i64::from(frames) - 26_989).abs() <= 1, "{frames}");
    // Refused: 10 % fast, and a reading of the wrong length.
    assert_eq!(feedback(&feedback_wire(48.5)), -1);
    assert_eq!(feedback(&[1, 2]), -1);
    let [updates, empty, invalid, q16] = feedback_stats();
    assert_eq!([updates, empty, invalid], [1, 1, 2]);
    assert!((q16 as f64 / 65_536.0 - 44.982).abs() < 0.001, "{q16}");
}

/// Part of the FLX4 test below: both use the one selection state.
fn a_refused_configuration_names_its_reason() {
    // The donor's captures (firmware-rust/crates/ruzina-uac/tests/fixtures).
    let hercules = include_bytes!(
        "../../../../../firmware-rust/crates/ruzina-uac/tests/fixtures/hercules_inpulse_500_fs.bin"
    );
    let s2 = include_bytes!(
        "../../../../../firmware-rust/crates/ruzina-uac/tests/fixtures/s2_mk3_hs.bin"
    );
    // Inpulse 500 at high speed: FEEDBACK on interface 1, alternate 1 (it
    // is a full-speed device; the deck follows feedback at full speed only).
    assert_eq!(select_config(hercules, true), 0);
    assert_eq!(reject(), [7, 1, 1]);
    inpulse_500_is_paced_by_its_feedback(hercules);
    // S2 MK3 at high speed: MALFORMED at its 6-byte UAC2 Format Type.
    assert_eq!(select_config(s2, true), 0);
    let [code, offset, _] = reject();
    assert_eq!(code, 2);
    assert_eq!(&s2[offset as usize..offset as usize + 3], &[6, 0x24, 2]);
    // A selection clears the reason; the FLX4 has no feedback to follow.
    assert_eq!(select_config(&flx4_config(), true), 1);
    assert_eq!(reject(), [0, 0, 0]);
    assert_eq!(feedback(&feedback_wire(48.0)), -1);
    assert_eq!(feedback_stats(), [0, 0, 0, 0]);
}
