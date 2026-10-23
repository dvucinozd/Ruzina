//! Isochronous packet cadence and PCM packing (M2.5 `flx4_uac_packetizer`).
//!
//! Each 1 ms packet carries `rate / 1000` frames with the remainder carried
//! forward, so 44.1 kHz is nine 44-frame packets and one 45-frame packet per
//! ten milliseconds, exact over every second.
//!
//! Packing is the single point where engine PCM meets the wire. MAIN and
//! headphone pairs are written to their own channels exactly as rendered:
//! the headphone mix and level were already applied once by the mixer, so
//! the transport never scales them again (donor `605bcd20` lost 12 dB that
//! way). A missing stream is silence on its channels, never a copy of the
//! other. A controller-specific MAIN calibration, if a profile ever needs
//! one, applies to the MAIN pair only.

use crate::descriptor::SampleFormat;
use pajoniiir_audio_dsp::PcmFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cadence {
    rate: u32,
    remainder: u32,
}

impl Cadence {
    pub const fn new(rate: u32) -> Self {
        Self { rate, remainder: 0 }
    }

    pub const fn rate(&self) -> u32 {
        self.rate
    }

    pub fn next_frames(&mut self) -> usize {
        self.remainder += self.rate;
        let frames = self.remainder / 1000;
        self.remainder -= frames * 1000;
        frames as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pair {
    pub left: u8,
    pub right: u8,
}

/// Where MAIN and headphones go in one interleaved stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelMap {
    channels: u8,
    main: Option<Pair>,
    headphones: Option<Pair>,
    /// Q15 MAIN-only calibration; `None` is unity.
    main_trim_q15: Option<i16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapError {
    Channels,
    Pair,
    Overlap,
}

impl ChannelMap {
    pub fn new(
        channels: u8,
        main: Option<Pair>,
        headphones: Option<Pair>,
    ) -> Result<Self, MapError> {
        if !matches!(channels, 2 | 4) {
            return Err(MapError::Channels);
        }
        let valid =
            |pair: Pair| pair.left != pair.right && pair.left < channels && pair.right < channels;
        if main.is_some_and(|pair| !valid(pair)) || headphones.is_some_and(|pair| !valid(pair)) {
            return Err(MapError::Pair);
        }
        if let (Some(main), Some(phones)) = (main, headphones)
            && [main.left, main.right]
                .iter()
                .any(|channel| *channel == phones.left || *channel == phones.right)
        {
            return Err(MapError::Overlap);
        }
        Ok(Self {
            channels,
            main,
            headphones,
            main_trim_q15: None,
        })
    }

    /// Controller-specific MAIN calibration from a qualified profile.
    pub const fn with_main_trim_q15(mut self, gain: i16) -> Self {
        self.main_trim_q15 = Some(gain);
        self
    }

    pub const fn channels(&self) -> u8 {
        self.channels
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackError {
    /// Destination smaller than the packet.
    Space,
    /// A supplied stream has fewer frames than the packet.
    Frames,
}

fn trim(sample: i16, gain: Option<i16>) -> i16 {
    match gain {
        None => sample,
        Some(gain) => ((i32::from(sample) * i32::from(gain)) >> 15) as i16,
    }
}

fn write(out: &mut [u8], sample: i16, format: SampleFormat) {
    match format {
        SampleFormat::Pcm16 => out.copy_from_slice(&sample.to_le_bytes()),
        // Full amplitude: the 16-bit sample is the 24-bit word's top bits.
        SampleFormat::Packed24 => out.copy_from_slice(&(i32::from(sample) << 8).to_le_bytes()[..3]),
    }
}

/// Packs `frames` frames into `out`; returns the bytes written.
pub fn pack(
    out: &mut [u8],
    format: SampleFormat,
    map: &ChannelMap,
    main: Option<&[PcmFrame]>,
    headphones: Option<&[PcmFrame]>,
    frames: usize,
) -> Result<usize, PackError> {
    let bytes = format.bytes();
    let frame_bytes = usize::from(map.channels) * bytes;
    let total = frames * frame_bytes;
    if out.len() < total {
        return Err(PackError::Space);
    }
    if main.is_some_and(|pcm| pcm.len() < frames)
        || headphones.is_some_and(|pcm| pcm.len() < frames)
    {
        return Err(PackError::Frames);
    }
    out[..total].fill(0);
    for index in 0..frames {
        let frame = &mut out[index * frame_bytes..(index + 1) * frame_bytes];
        let mut put = |pair: Option<Pair>, pcm: Option<&[PcmFrame]>, gain| {
            if let (Some(pair), Some(pcm)) = (pair, pcm) {
                let sample = pcm[index];
                let left = usize::from(pair.left) * bytes;
                let right = usize::from(pair.right) * bytes;
                write(
                    &mut frame[left..left + bytes],
                    trim(sample.left, gain),
                    format,
                );
                write(
                    &mut frame[right..right + bytes],
                    trim(sample.right, gain),
                    format,
                );
            }
        };
        put(map.main, main, map.main_trim_q15);
        put(map.headphones, headphones, None);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Pair = Pair { left: 0, right: 1 };
    const PHONES: Pair = Pair { left: 2, right: 3 };

    fn samples(out: &[u8], format: SampleFormat) -> [i32; 4] {
        let bytes = format.bytes();
        core::array::from_fn(|channel| {
            let raw = &out[channel * bytes..(channel + 1) * bytes];
            match format {
                SampleFormat::Pcm16 => i32::from(i16::from_le_bytes([raw[0], raw[1]])),
                SampleFormat::Packed24 => i32::from_le_bytes([0, raw[0], raw[1], raw[2]]) >> 8,
            }
        })
    }

    #[test]
    fn cadence_is_exact_per_second_at_both_rates() {
        for (rate, pattern) in [
            (44_100, [44, 44, 44, 44, 44, 44, 44, 44, 44, 45]),
            (48_000, [48; 10]),
        ] {
            let mut cadence = Cadence::new(rate);
            let first: [usize; 10] = core::array::from_fn(|_| cadence.next_frames());
            let mut sorted = first;
            sorted.sort_unstable();
            assert_eq!(sorted, pattern);
            let total: usize = first.iter().sum::<usize>()
                + (10..1000).map(|_| cadence.next_frames()).sum::<usize>();
            assert_eq!(total, rate as usize);
            assert!((0..100_000).all(|_| (44..=48).contains(&cadence.next_frames())));
        }
    }

    #[test]
    fn main_and_headphones_pack_once_at_unity_full_scale() {
        let map = ChannelMap::new(4, Some(MAIN), Some(PHONES)).unwrap();
        let main = [
            PcmFrame {
                left: 4_000,
                right: -4_000,
            },
            PcmFrame {
                left: i16::MAX,
                right: i16::MIN,
            },
        ];
        let phones = [
            PcmFrame {
                left: 16_000,
                right: -16_000,
            },
            PcmFrame {
                left: i16::MIN,
                right: 1,
            },
        ];
        for format in [SampleFormat::Pcm16, SampleFormat::Packed24] {
            let mut out = [0xaa; 64];
            let written = pack(&mut out, format, &map, Some(&main), Some(&phones), 2).unwrap();
            let frame = 4 * format.bytes();
            assert_eq!(written, 2 * frame);
            let scale = if format == SampleFormat::Packed24 {
                256
            } else {
                1
            };
            for index in 0..2 {
                let got = samples(&out[index * frame..], format);
                let want = [
                    main[index].left,
                    main[index].right,
                    phones[index].left,
                    phones[index].right,
                ]
                .map(|s| i32::from(s) * scale);
                assert_eq!(got, want, "{format:?} frame {index}");
            }
            assert_eq!(out[written], 0xaa, "nothing past the packet");
        }
    }

    #[test]
    fn a_missing_stream_is_silence_never_the_other_mix() {
        let map = ChannelMap::new(4, Some(MAIN), Some(PHONES)).unwrap();
        let phones = [PcmFrame { left: 9, right: -9 }];
        let main = [PcmFrame { left: 7, right: -7 }];
        let mut out = [0xff; 8];
        pack(&mut out, SampleFormat::Pcm16, &map, None, Some(&phones), 1).unwrap();
        assert_eq!(samples(&out, SampleFormat::Pcm16), [0, 0, 9, -9]);
        pack(&mut out, SampleFormat::Pcm16, &map, Some(&main), None, 1).unwrap();
        assert_eq!(samples(&out, SampleFormat::Pcm16), [7, -7, 0, 0]);
        let main_only = ChannelMap::new(4, Some(MAIN), None).unwrap();
        pack(
            &mut out,
            SampleFormat::Pcm16,
            &main_only,
            Some(&main),
            Some(&phones),
            1,
        )
        .unwrap();
        assert_eq!(samples(&out, SampleFormat::Pcm16), [7, -7, 0, 0]);
    }

    #[test]
    fn main_calibration_never_touches_headphones() {
        let map = ChannelMap::new(4, Some(MAIN), Some(PHONES))
            .unwrap()
            .with_main_trim_q15(8_192);
        let pcm = [PcmFrame {
            left: 20_000,
            right: -20_000,
        }];
        let mut out = [0; 8];
        pack(
            &mut out,
            SampleFormat::Pcm16,
            &map,
            Some(&pcm),
            Some(&pcm),
            1,
        )
        .unwrap();
        assert_eq!(
            samples(&out, SampleFormat::Pcm16),
            [5_000, -5_000, 20_000, -20_000]
        );
    }

    #[test]
    fn maps_and_buffers_are_validated() {
        assert_eq!(
            ChannelMap::new(6, Some(MAIN), None),
            Err(MapError::Channels)
        );
        assert_eq!(
            ChannelMap::new(2, Some(MAIN), Some(PHONES)),
            Err(MapError::Pair)
        );
        assert_eq!(
            ChannelMap::new(4, Some(MAIN), Some(Pair { left: 1, right: 2 })),
            Err(MapError::Overlap)
        );
        assert_eq!(
            ChannelMap::new(4, Some(Pair { left: 1, right: 1 }), None),
            Err(MapError::Pair)
        );
        let map = ChannelMap::new(2, Some(MAIN), None).unwrap();
        let pcm = [PcmFrame::default(); 2];
        assert_eq!(
            pack(&mut [0; 7], SampleFormat::Pcm16, &map, Some(&pcm), None, 2),
            Err(PackError::Space)
        );
        assert_eq!(
            pack(&mut [0; 64], SampleFormat::Pcm16, &map, Some(&pcm), None, 3),
            Err(PackError::Frames)
        );
    }
}
