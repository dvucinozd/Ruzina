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
    let config = flx4_config();
    let mut info = [0u32; 8];
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
