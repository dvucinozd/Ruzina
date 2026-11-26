//! Configuration descriptors the donor captured from real controllers
//! (`fixtures/README.md`). They record what Ruzina offers for each, and why
//! not, as the Inpulse 500 and S2 MK3 audio work changes it.

use ruzina_uac::descriptor::{
    DescriptorError, Rejection, SampleFormat, UsbSpeed, choose, parse_playback_formats,
};

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
fn s2_mk3_is_refused_as_a_malformed_configuration_today() {
    // UAC2: its 6-byte Format Type descriptor is shorter than UAC1's, and the
    // configuration ends in eight zero bytes inside wTotalLength.
    let error = parse_playback_formats(S2_MK3_HS).err().unwrap();
    let DescriptorError::Malformed { offset } = error else {
        panic!("{error:?}");
    };
    assert_eq!(&S2_MK3_HS[offset..offset + 3], &[6, 0x24, 2]);
    assert_eq!(&S2_MK3_HS[310..], &[0; 8]);
    assert_eq!(
        choose(S2_MK3_HS, UsbSpeed::High),
        Err(Rejection::Descriptor(error))
    );
}
