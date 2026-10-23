//! UAC transfers carry MAIN and headphone sink receipts: frames are credited
//! only by a completion of the current stream epoch, the two mixes stay on
//! their own channels at unity, and an unsupported audio format leaves MIDI
//! usable.

use pajoniiir_audio_dsp::PcmFrame;
use pajoniiir_audio_runtime::render::RENDER_BLOCK_FRAMES;
use pajoniiir_audio_runtime::routing::{
    ChannelPair, OutputAvailability, OutputRouter, ReadyStream,
};
use pajoniiir_audio_runtime::sink::{OutputDelivery, SinkBlock, SinkError, SinkReceipt};
use pajoniiir_controller_usb::find_midi_streaming_interface;
use pajoniiir_uac::descriptor::{SampleFormat, UsbSpeed, parse_playback_formats, select};
use pajoniiir_uac::packet::{Cadence, ChannelMap, Pair, pack};
use pajoniiir_uac::stream::{Cleanup, ControlNext, IsoNext, IsoStatus, TRANSFER_COUNT, UacStream};

fn config(alternate: &[u8]) -> Vec<u8> {
    let mut out = vec![9, 2, 0, 0, 3, 1, 0, 0x80, 50];
    out.extend_from_slice(&[9, 4, 0, 0, 0, 1, 1, 0, 0]);
    out.extend_from_slice(&[9, 4, 1, 0, 0, 1, 2, 0, 0]);
    out.extend_from_slice(alternate);
    out.extend_from_slice(&[9, 4, 2, 0, 2, 1, 3, 0, 0]);
    out.extend_from_slice(&[7, 5, 0x02, 0x02, 64, 0, 0]);
    out.extend_from_slice(&[7, 5, 0x82, 0x02, 64, 0, 0]);
    let total = out.len() as u16;
    out[2..4].copy_from_slice(&total.to_le_bytes());
    out
}

fn alternate(bits: u8, bytes: u8, rate: u32, attributes: u8, max_packet: u16) -> Vec<u8> {
    let mut out = vec![9, 4, 1, 1, 1, 1, 2, 0, 0, 7, 0x24, 1, 1, 1, 1, 0];
    out.extend_from_slice(&[11, 0x24, 2, 1, 4, bytes, bits, 1]);
    out.extend_from_slice(&rate.to_le_bytes()[..3]);
    out.extend_from_slice(&[
        9,
        5,
        0x01,
        attributes,
        max_packet as u8,
        (max_packet >> 8) as u8,
        1,
        0,
        0,
    ]);
    // Class-specific endpoint: sampling frequency control.
    out.extend_from_slice(&[7, 0x25, 1, 1, 0, 0, 0]);
    out
}

fn ready(stream_epoch: u32, pair: (u8, u8)) -> ReadyStream {
    ReadyStream {
        connection_epoch: 1,
        stream_epoch,
        interface: 1,
        channels: 4,
        pair: ChannelPair {
            left: pair.0,
            right: pair.1,
        },
        sample_rate_hz: 48_000,
        sample_bits: 24,
    }
}

fn routes(epoch: u32) -> OutputAvailability {
    OutputAvailability {
        controller_main: Some(ready(epoch, (0, 1))),
        controller_headphones: Some(ready(epoch, (2, 3))),
        ..OutputAvailability::default()
    }
}

fn main_pcm(frame: u64) -> PcmFrame {
    PcmFrame {
        left: 1_000 + (frame % 500) as i16,
        right: -1_000,
    }
}

fn phones_pcm(frame: u64) -> PcmFrame {
    PcmFrame {
        left: i16::MAX - (frame % 7) as i16,
        right: i16::MIN,
    }
}

#[test]
fn transfers_credit_receipts_only_for_the_current_epoch() {
    let formats = parse_playback_formats(&config(&alternate(24, 3, 48_000, 0x09, 576))).unwrap();
    let selected = select(&formats, UsbSpeed::Full).unwrap();
    assert_eq!(selected.rate, 48_000);
    let map = ChannelMap::new(
        4,
        Some(Pair { left: 0, right: 1 }),
        Some(Pair { left: 2, right: 3 }),
    )
    .unwrap();

    let mut router = OutputRouter::default();
    let transition = router.update(routes(1)).unwrap();
    assert!(router.commit_prepared(transition));
    let mut main_storage = [SinkBlock::EMPTY; 4];
    let mut phones_storage = [SinkBlock::EMPTY; 4];
    let mut delivery = OutputDelivery::new(&mut main_storage, &mut phones_storage).unwrap();
    delivery.sync_routes(&router, 0).unwrap();
    let main_token = delivery.main.token().unwrap();
    let phones_token = delivery.headphones.token().unwrap();
    delivery.main.set_mute(main_token, false, 0).unwrap();
    delivery
        .headphones
        .set_mute(phones_token, false, 0)
        .unwrap();

    let mut stream = UacStream::new();
    stream.start(&selected.format, 0).unwrap();
    assert_eq!(
        stream.control_done(true, 1),
        ControlNext::Submit(pajoniiir_uac::stream::ControlStep::SetSampleRate)
    );
    let ControlNext::Prime { epoch } = stream.control_done(true, 2) else {
        panic!("not primed");
    };

    let mut cadence = Cadence::new(selected.rate);
    let mut produced = 0u64;
    let mut clock = 0u64;
    let mut owned: [Option<(SinkReceipt, SinkReceipt, Vec<u8>)>; TRANSFER_COUNT] =
        Default::default();
    let fill = |delivery: &mut OutputDelivery<'_>, produced: &mut u64, clock: &mut u64| {
        while *produced < *clock + 512 {
            let block = |f: fn(u64) -> PcmFrame| -> [PcmFrame; RENDER_BLOCK_FRAMES] {
                core::array::from_fn(|i| f(*produced + i as u64))
            };
            let main = block(main_pcm);
            let phones = block(phones_pcm);
            if delivery.main.submit(main_token, *produced, &main).is_err()
                | delivery
                    .headphones
                    .submit(phones_token, *produced, &phones)
                    .is_err()
            {
                return;
            }
            *produced += RENDER_BLOCK_FRAMES as u64;
        }
    };
    let issue = |delivery: &mut OutputDelivery<'_>, clock: &mut u64, cadence: &mut Cadence| {
        let frames = cadence.next_frames();
        let mut main = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let mut phones = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let main_receipt = delivery
            .main
            .issue(main_token, *clock, frames, &mut main)
            .unwrap();
        let phones_receipt = delivery
            .headphones
            .issue(phones_token, *clock, frames, &mut phones)
            .unwrap();
        let mut packet = vec![0; selected.format.packet_bytes(selected.rate)];
        let written = pack(
            &mut packet,
            SampleFormat::Packed24,
            &map,
            Some(&main[..frames]),
            Some(&phones[..frames]),
            frames,
        )
        .unwrap();
        packet.truncate(written);
        *clock += frames as u64;
        (main_receipt, phones_receipt, packet)
    };

    fill(&mut delivery, &mut produced, &mut clock);
    for slot in owned.iter_mut() {
        *slot = Some(issue(&mut delivery, &mut clock, &mut cadence));
    }
    let mut checked = 0u64;
    for round in 0..30 {
        let index = round % TRANSFER_COUNT;
        let (main_receipt, phones_receipt, packet) = owned[index].take().unwrap();
        let IsoNext::Resubmit {
            epoch: next,
            credit: true,
        } = stream.iso_done(index, IsoStatus::Completed { lost_frames: 0 })
        else {
            panic!("expected resubmit");
        };
        assert_eq!(next, epoch);
        delivery.main.complete(main_receipt).unwrap();
        delivery.headphones.complete(phones_receipt).unwrap();
        for frame in packet.chunks(12) {
            let sample = |channel: usize| {
                i32::from_le_bytes([
                    0,
                    frame[channel * 3],
                    frame[channel * 3 + 1],
                    frame[channel * 3 + 2],
                ]) >> 16
            };
            let at = checked;
            assert_eq!(
                [sample(0), sample(1)],
                [i32::from(main_pcm(at).left), -1_000],
                "MAIN at {at}"
            );
            assert_eq!(
                [sample(2), sample(3)],
                [i32::from(phones_pcm(at).left), i32::from(i16::MIN)],
                "phones at {at}, unity"
            );
            checked += 1;
        }
        fill(&mut delivery, &mut produced, &mut clock);
        owned[index] = Some(issue(&mut delivery, &mut clock, &mut cadence));
    }
    assert_eq!(delivery.main.stats().completed_frames, checked);
    assert_eq!(delivery.headphones.stats().completed_frames, checked);

    // The controller re-enumerates its stream: teardown starts, the sinks
    // rebind, and late completions of the old epoch are never credited.
    stream.request_stop(false);
    let transition = router.update(routes(2)).unwrap();
    delivery.sync_routes(&router, clock).unwrap();
    assert_eq!(stream.poll(100), Cleanup::HaltAndFlush);
    for (index, slot) in owned.iter_mut().enumerate() {
        let (main_receipt, _, _) = slot.take().unwrap();
        assert_eq!(
            stream.iso_done(index, IsoStatus::Completed { lost_frames: 0 }),
            IsoNext::Retired { credit: false }
        );
        assert_eq!(
            delivery.main.complete(main_receipt),
            Err(SinkError::Stale),
            "defense in depth"
        );
    }
    assert_eq!(delivery.main.stats().completed_frames, checked);
    assert!(delivery.main.stats().revoked_in_flight_frames > 0);
    assert_eq!(
        stream.poll(101),
        Cleanup::Submit(pajoniiir_uac::stream::ControlStep::SetZeroAlternate)
    );
    stream.control_done(true, 102);
    assert_eq!(stream.poll(103), Cleanup::FreeAndRelease);
    assert!(router.commit_prepared(transition));
}

#[test]
fn unsupported_audio_keeps_midi_and_offers_no_stream() {
    for alt in [
        alternate(24, 3, 48_000, 0x05, 576),
        alternate(32, 4, 48_000, 0x09, 768),
        alternate(16, 2, 96_000, 0x09, 768),
    ] {
        let bytes = config(&alt);
        let formats = parse_playback_formats(&bytes).unwrap();
        assert_eq!(select(&formats, UsbSpeed::Full), None);
        let midi = find_midi_streaming_interface(&bytes).unwrap().unwrap();
        assert!(midi.is_bidirectional());
    }
}

#[test]
fn a_44_1_khz_only_controller_streams_through_the_converter() {
    use pajoniiir_uac::resample::Resampler;
    // FLX4-like: 4 ch, 16-bit, 44.1 kHz only.
    let formats = parse_playback_formats(&config(&alternate(16, 2, 44_100, 0x09, 360))).unwrap();
    let selected = select(&formats, UsbSpeed::Full).unwrap();
    assert_eq!(selected.rate, 44_100);
    let map = ChannelMap::new(
        4,
        Some(Pair { left: 0, right: 1 }),
        Some(Pair { left: 2, right: 3 }),
    )
    .unwrap();

    let mut router = OutputRouter::default();
    let mut available = routes(1);
    for stream in [
        available.controller_main.as_mut().unwrap(),
        available.controller_headphones.as_mut().unwrap(),
    ] {
        stream.sample_rate_hz = 44_100;
        stream.sample_bits = 16;
    }
    let transition = router.update(available).unwrap();
    assert!(router.commit_prepared(transition));
    let mut main_storage = [SinkBlock::EMPTY; 4];
    let mut phones_storage = [SinkBlock::EMPTY; 4];
    let mut delivery = OutputDelivery::new(&mut main_storage, &mut phones_storage).unwrap();
    delivery.sync_routes(&router, 0).unwrap();
    let main_token = delivery.main.token().unwrap();
    let phones_token = delivery.headphones.token().unwrap();
    delivery.main.set_mute(main_token, false, 0).unwrap();
    delivery
        .headphones
        .set_mute(phones_token, false, 0)
        .unwrap();

    let main_level = PcmFrame {
        left: 6_000,
        right: -6_000,
    };
    let phones_level = PcmFrame {
        left: 24_000,
        right: -24_000,
    };
    let mut resampler = Resampler::<4>::new(48_000, selected.rate).unwrap();
    let mut cadence = Cadence::new(selected.rate);
    let (mut produced, mut clock, mut wire_frames) = (0u64, 0u64, 0usize);
    for _ in 0..1_000 {
        while produced < clock + 256 {
            delivery
                .main
                .submit(main_token, produced, &[main_level; RENDER_BLOCK_FRAMES])
                .unwrap();
            delivery
                .headphones
                .submit(phones_token, produced, &[phones_level; RENDER_BLOCK_FRAMES])
                .unwrap();
            produced += RENDER_BLOCK_FRAMES as u64;
        }
        let frames = cadence.next_frames();
        let needed = resampler.input_needed(frames);
        let mut main = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let mut phones = [PcmFrame::default(); RENDER_BLOCK_FRAMES];
        let main_receipt = delivery
            .main
            .issue(main_token, clock, needed, &mut main)
            .unwrap();
        let phones_receipt = delivery
            .headphones
            .issue(phones_token, clock, needed, &mut phones)
            .unwrap();
        clock += needed as u64;
        let joined: Vec<[i16; 4]> = (0..needed)
            .map(|i| [main[i].left, main[i].right, phones[i].left, phones[i].right])
            .collect();
        let mut converted = [[0i16; 4]; 48];
        let n = resampler.process(&joined, &mut converted).unwrap();
        assert_eq!(n, frames);
        let split = |pick: usize| -> Vec<PcmFrame> {
            converted[..n]
                .iter()
                .map(|f| PcmFrame {
                    left: f[pick],
                    right: f[pick + 1],
                })
                .collect()
        };
        let mut packet = vec![0; selected.format.packet_bytes(selected.rate)];
        let written = pack(
            &mut packet,
            SampleFormat::Pcm16,
            &map,
            Some(&split(0)),
            Some(&split(2)),
            n,
        )
        .unwrap();
        for frame in packet[..written].chunks(8) {
            let s = |c: usize| i16::from_le_bytes([frame[c * 2], frame[c * 2 + 1]]);
            assert_eq!(
                [s(0), s(1), s(2), s(3)],
                [6_000, -6_000, 24_000, -24_000],
                "unity through the converter"
            );
        }
        wire_frames += n;
        delivery.main.complete(main_receipt).unwrap();
        delivery.headphones.complete(phones_receipt).unwrap();
    }
    assert_eq!(wire_frames, 44_100, "one second on the wire");
    assert!(
        (clock as i64 - 48_000).abs() <= 1,
        "one second of render frames consumed: {clock}"
    );
    assert_eq!(
        delivery.main.stats().completed_frames,
        clock,
        "receipts count render frames"
    );
    assert_eq!(delivery.main.stats().underrun_frames, 0);
}
