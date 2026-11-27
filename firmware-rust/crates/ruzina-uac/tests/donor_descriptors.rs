//! Configuration descriptors the donor captured from real controllers
//! (`fixtures/README.md`). They record what Ruzina offers for each, and why
//! not, as the Inpulse 500 and S2 MK3 audio work changes it.

use ruzina_uac::descriptor::{Rejection, SampleFormat, UsbSpeed, choose, parse_playback_formats};
use ruzina_uac::uac2;

const HERCULES_INPULSE_500_FS: &[u8] = include_bytes!("fixtures/hercules_inpulse_500_fs.bin");
const S2_MK3_HS: &[u8] = include_bytes!("fixtures/s2_mk3_hs.bin");

#[test]
fn fixtures_are_whole_configurations() {
    for (bytes, total) in [(HERCULES_INPULSE_500_FS, 270), (S2_MK3_HS, 318)] {
        assert_eq!(&bytes[..2], &[9, 2]);
        assert_eq!(usize::from(u16::from_le_bytes([bytes[2], bytes[3]])), total);
        assert_eq!(bytes.len(), total);
    }
}

#[test]
fn inpulse_500_is_asynchronous_44k1_packed_24_with_feedback() {
    let formats = parse_playback_formats(HERCULES_INPULSE_500_FS).unwrap();
    let [format] = formats.as_slice() else {
        panic!("one playback alternate, got {}", formats.as_slice().len());
    };
    assert_eq!(
        (format.interface, format.alternate, format.endpoint),
        (1, 1, 0x01)
    );
    assert_eq!((format.max_packet, format.interval), (540, 1));
    assert_eq!(format.channels, 4);
    assert_eq!(format.sample_format(), Some(SampleFormat::Packed24));
    assert_eq!(format.rates(), &[44_100]);
    assert!(format.pcm && format.feedback && !format.frequency_control);
    // 45 frames x 4 channels x 3 bytes: the whole endpoint.
    assert_eq!(format.packet_bytes(44_100), 540);
}

#[test]
fn inpulse_500_plays_through_its_explicit_feedback_at_full_speed() {
    let formats = parse_playback_formats(HERCULES_INPULSE_500_FS).unwrap();
    let format = formats.as_slice()[0];
    // OUT 0x01 names IN 0x81 as its synch endpoint: 3 bytes every frame,
    // a new value every 2^5 = 32 ms.
    assert!(format.asynchronous);
    assert_eq!(format.sync_address, 0x81);
    assert_eq!(
        (
            format.feedback_endpoint,
            format.feedback_interval,
            format.feedback_refresh
        ),
        (0x81, 1, 5)
    );
    assert!(format.explicit_feedback() && !format.feedback_unusable);
    assert_eq!(format.max_frames(), 45);
    let chosen = choose(HERCULES_INPULSE_500_FS, UsbSpeed::Full).unwrap();
    assert_eq!((chosen.format.alternate, chosen.rate), (1, 44_100));
    // The deck reads feedback at full speed only.
    assert_eq!(
        choose(HERCULES_INPULSE_500_FS, UsbSpeed::High),
        Err(Rejection::Feedback {
            interface: 1,
            alternate: 1
        })
    );
}

#[test]
fn s2_mk3_is_a_driveable_uac2_function_the_deck_does_not_stream_yet() {
    // Eight zero bytes end the configuration inside wTotalLength: padding.
    assert_eq!(&S2_MK3_HS[310..], &[0; 8]);
    // No UAC1 playback: its streaming alternates are UAC2 (protocol 0x20).
    assert!(
        parse_playback_formats(S2_MK3_HS)
            .unwrap()
            .as_slice()
            .is_empty()
    );
    let formats = uac2::parse(S2_MK3_HS).unwrap();
    let [format] = formats.as_slice() else {
        panic!("{formats:?}")
    };
    assert_eq!(
        (format.interface, format.alternate, format.endpoint),
        (1, 1, 0x01)
    );
    assert_eq!((format.max_packet, format.interval), (112, 1));
    assert_eq!(
        (format.channels, format.subslot_bytes, format.bits),
        (4, 4, 24)
    );
    assert!(format.pcm && format.asynchronous);
    assert_eq!(
        (
            format.feedback_endpoint,
            format.feedback_max_packet,
            format.feedback_interval
        ),
        (0x81, 4, 4)
    );
    // Terminal 1 runs on clock source 0x11 of AudioControl interface 0, which
    // the host may set and read the validity of.
    assert_eq!(
        (
            format.terminal,
            format.clock_source,
            format.control_interface
        ),
        (1, 0x11, 0)
    );
    assert!(format.clock_settable && format.clock_validity);
    assert_eq!(format.max_frames(), 7);
    assert!(format.supported(UsbSpeed::High));
    assert_eq!(
        choose(S2_MK3_HS, UsbSpeed::High),
        Err(Rejection::Uac2 {
            interface: 1,
            alternate: 1
        })
    );
    // At full speed the S2 MK3 would be nothing the deck can drive.
    assert_eq!(
        choose(S2_MK3_HS, UsbSpeed::Full),
        Err(Rejection::Uac2Unsupported {
            interface: 1,
            alternate: 1
        })
    );
}
