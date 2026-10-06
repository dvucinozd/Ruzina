use pajoniiir_audio_dsp::PcmFrame;

pub const RIFF_HEADER_BYTES: usize = 12;
pub const CHUNK_HEADER_BYTES: usize = 8;
pub const PCM_FORMAT_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WavError {
    TruncatedRiffHeader,
    InvalidRiffMagic,
    InvalidWaveMagic,
    TruncatedChunkHeader,
    TruncatedFormat,
    UnsupportedAudioFormat(u16),
    UnsupportedChannels(u16),
    UnsupportedBitsPerSample(u16),
    InvalidSampleRate,
    InvalidBlockAlign { expected: u16, actual: u16 },
    InvalidByteRate { expected: u32, actual: u32 },
    DataMisaligned { data_bytes: u64, block_align: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavChunkHeader {
    pub id: [u8; 4],
    pub len: u32,
}

impl WavChunkHeader {
    pub const fn padded_len(self) -> u64 {
        let len = self.len as u64;
        len + (len & 1)
    }

    pub fn is_format(self) -> bool {
        self.id == *b"fmt "
    }

    pub fn is_data(self) -> bool {
        self.id == *b"data"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavPcm16Format {
    pub channels: u16,
    pub sample_rate_hz: u32,
    pub byte_rate: u32,
    pub block_align: u16,
}

impl WavPcm16Format {
    pub const fn frame_bytes(self) -> usize {
        self.block_align as usize
    }
}

pub fn parse_riff_header(input: &[u8]) -> Result<u32, WavError> {
    if input.len() < RIFF_HEADER_BYTES {
        return Err(WavError::TruncatedRiffHeader);
    }
    if &input[0..4] != b"RIFF" {
        return Err(WavError::InvalidRiffMagic);
    }
    if &input[8..12] != b"WAVE" {
        return Err(WavError::InvalidWaveMagic);
    }
    Ok(u32::from_le_bytes([input[4], input[5], input[6], input[7]]))
}

pub fn parse_chunk_header(input: &[u8]) -> Result<WavChunkHeader, WavError> {
    if input.len() < CHUNK_HEADER_BYTES {
        return Err(WavError::TruncatedChunkHeader);
    }
    Ok(WavChunkHeader {
        id: [input[0], input[1], input[2], input[3]],
        len: u32::from_le_bytes([input[4], input[5], input[6], input[7]]),
    })
}

pub fn parse_pcm16_format(input: &[u8]) -> Result<WavPcm16Format, WavError> {
    if input.len() < PCM_FORMAT_BYTES {
        return Err(WavError::TruncatedFormat);
    }

    let audio_format = u16::from_le_bytes([input[0], input[1]]);
    if audio_format != 1 {
        return Err(WavError::UnsupportedAudioFormat(audio_format));
    }

    let channels = u16::from_le_bytes([input[2], input[3]]);
    if channels != 1 && channels != 2 {
        return Err(WavError::UnsupportedChannels(channels));
    }

    let sample_rate_hz = u32::from_le_bytes([input[4], input[5], input[6], input[7]]);
    if sample_rate_hz == 0 {
        return Err(WavError::InvalidSampleRate);
    }

    let byte_rate = u32::from_le_bytes([input[8], input[9], input[10], input[11]]);
    let block_align = u16::from_le_bytes([input[12], input[13]]);
    let bits_per_sample = u16::from_le_bytes([input[14], input[15]]);
    if bits_per_sample != 16 {
        return Err(WavError::UnsupportedBitsPerSample(bits_per_sample));
    }

    let expected_block_align = channels * 2;
    if block_align != expected_block_align {
        return Err(WavError::InvalidBlockAlign {
            expected: expected_block_align,
            actual: block_align,
        });
    }

    let expected_byte_rate = sample_rate_hz
        .checked_mul(u32::from(block_align))
        .ok_or(WavError::InvalidByteRate {
            expected: u32::MAX,
            actual: byte_rate,
        })?;
    if byte_rate != expected_byte_rate {
        return Err(WavError::InvalidByteRate {
            expected: expected_byte_rate,
            actual: byte_rate,
        });
    }

    Ok(WavPcm16Format {
        channels,
        sample_rate_hz,
        byte_rate,
        block_align,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavDecodeProgress {
    pub consumed_bytes: usize,
    pub produced_frames: usize,
    pub finished: bool,
}

pub struct WavPcm16Decoder {
    format: WavPcm16Format,
    remaining_data_bytes: u64,
    carry: [u8; 4],
    carry_len: u8,
    decoded_frames: u64,
}

impl WavPcm16Decoder {
    pub fn new(format: WavPcm16Format, data_bytes: u64) -> Result<Self, WavError> {
        if data_bytes % u64::from(format.block_align) != 0 {
            return Err(WavError::DataMisaligned {
                data_bytes,
                block_align: format.block_align,
            });
        }
        Ok(Self {
            format,
            remaining_data_bytes: data_bytes,
            carry: [0; 4],
            carry_len: 0,
            decoded_frames: 0,
        })
    }

    pub const fn format(&self) -> WavPcm16Format {
        self.format
    }

    pub const fn remaining_data_bytes(&self) -> u64 {
        self.remaining_data_bytes
    }

    pub const fn decoded_frames(&self) -> u64 {
        self.decoded_frames
    }

    pub const fn is_finished(&self) -> bool {
        self.remaining_data_bytes == 0 && self.carry_len == 0
    }

    pub fn decode(&mut self, input: &[u8], output: &mut [PcmFrame]) -> WavDecodeProgress {
        if input.is_empty() || self.remaining_data_bytes == 0 || output.is_empty() {
            return WavDecodeProgress {
                consumed_bytes: 0,
                produced_frames: 0,
                finished: self.is_finished(),
            };
        }

        let frame_bytes = self.format.frame_bytes();
        let allowed = core::cmp::min(input.len() as u64, self.remaining_data_bytes) as usize;
        let mut consumed = 0usize;
        let mut produced = 0usize;

        if self.carry_len != 0 {
            let carry_len = self.carry_len as usize;
            let needed = frame_bytes - carry_len;
            let take = core::cmp::min(needed, allowed);
            self.carry[carry_len..carry_len + take].copy_from_slice(&input[..take]);
            self.carry_len += take as u8;
            consumed += take;

            if self.carry_len as usize == frame_bytes {
                output[produced] = decode_frame(self.format, &self.carry[..frame_bytes]);
                produced += 1;
                self.carry_len = 0;
            } else {
                self.remaining_data_bytes -= consumed as u64;
                return WavDecodeProgress {
                    consumed_bytes: consumed,
                    produced_frames: produced,
                    finished: self.is_finished(),
                };
            }
        }

        while produced < output.len() && consumed + frame_bytes <= allowed {
            output[produced] =
                decode_frame(self.format, &input[consumed..consumed + frame_bytes]);
            consumed += frame_bytes;
            produced += 1;
        }

        if produced < output.len() && consumed < allowed {
            let tail = allowed - consumed;
            debug_assert!(tail < frame_bytes);
            self.carry[..tail].copy_from_slice(&input[consumed..allowed]);
            self.carry_len = tail as u8;
            consumed = allowed;
        }

        self.remaining_data_bytes -= consumed as u64;
        self.decoded_frames = self.decoded_frames.saturating_add(produced as u64);

        WavDecodeProgress {
            consumed_bytes: consumed,
            produced_frames: produced,
            finished: self.is_finished(),
        }
    }
}

fn decode_frame(format: WavPcm16Format, bytes: &[u8]) -> PcmFrame {
    let left = i16::from_le_bytes([bytes[0], bytes[1]]);
    if format.channels == 1 {
        PcmFrame { left, right: left }
    } else {
        PcmFrame {
            left,
            right: i16::from_le_bytes([bytes[2], bytes[3]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(channels: u16) -> [u8; PCM_FORMAT_BYTES] {
        let sample_rate = 48_000u32;
        let block_align = channels * 2;
        let byte_rate = sample_rate * u32::from(block_align);
        let mut bytes = [0u8; PCM_FORMAT_BYTES];
        bytes[0..2].copy_from_slice(&1u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&channels.to_le_bytes());
        bytes[4..8].copy_from_slice(&sample_rate.to_le_bytes());
        bytes[8..12].copy_from_slice(&byte_rate.to_le_bytes());
        bytes[12..14].copy_from_slice(&block_align.to_le_bytes());
        bytes[14..16].copy_from_slice(&16u16.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_riff_and_chunk_padding_without_allocation() {
        let mut riff = [0u8; RIFF_HEADER_BYTES];
        riff[0..4].copy_from_slice(b"RIFF");
        riff[4..8].copy_from_slice(&100u32.to_le_bytes());
        riff[8..12].copy_from_slice(b"WAVE");
        assert_eq!(parse_riff_header(&riff), Ok(100));

        let mut chunk = [0u8; CHUNK_HEADER_BYTES];
        chunk[0..4].copy_from_slice(b"JUNK");
        chunk[4..8].copy_from_slice(&5u32.to_le_bytes());
        let parsed = parse_chunk_header(&chunk).unwrap();
        assert_eq!(parsed.id, *b"JUNK");
        assert_eq!(parsed.padded_len(), 6);
    }

    #[test]
    fn validates_pcm16_mono_and_stereo_format_contract() {
        let mono = parse_pcm16_format(&fmt(1)).unwrap();
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.block_align, 2);
        assert_eq!(mono.byte_rate, 96_000);

        let stereo = parse_pcm16_format(&fmt(2)).unwrap();
        assert_eq!(stereo.channels, 2);
        assert_eq!(stereo.block_align, 4);
        assert_eq!(stereo.byte_rate, 192_000);
    }

    #[test]
    fn rejects_non_pcm_and_non_16_bit_sources() {
        let mut bytes = fmt(2);
        bytes[0..2].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            parse_pcm16_format(&bytes),
            Err(WavError::UnsupportedAudioFormat(3))
        );

        let mut bytes = fmt(2);
        bytes[14..16].copy_from_slice(&24u16.to_le_bytes());
        assert_eq!(
            parse_pcm16_format(&bytes),
            Err(WavError::UnsupportedBitsPerSample(24))
        );
    }

    #[test]
    fn decoder_preserves_stereo_samples_across_split_reads() {
        let format = parse_pcm16_format(&fmt(2)).unwrap();
        let source = [
            0x34, 0x12, 0xcc, 0xed, // 0x1234, -0x1234
            0x00, 0x80, 0xff, 0x7f, // -32768, 32767
        ];
        let mut decoder = WavPcm16Decoder::new(format, source.len() as u64).unwrap();
        let mut output = [PcmFrame::default(); 2];

        let first = decoder.decode(&source[..3], &mut output);
        assert_eq!(first.consumed_bytes, 3);
        assert_eq!(first.produced_frames, 0);

        let second = decoder.decode(&source[3..], &mut output);
        assert_eq!(second.consumed_bytes, 5);
        assert_eq!(second.produced_frames, 2);
        assert!(second.finished);
        assert_eq!(
            output,
            [
                PcmFrame {
                    left: 0x1234,
                    right: -0x1234,
                },
                PcmFrame {
                    left: i16::MIN,
                    right: i16::MAX,
                },
            ]
        );
    }

    #[test]
    fn decoder_duplicates_mono_into_canonical_stereo() {
        let format = parse_pcm16_format(&fmt(1)).unwrap();
        let source = [1, 0, 0xff, 0xff];
        let mut decoder = WavPcm16Decoder::new(format, source.len() as u64).unwrap();
        let mut output = [PcmFrame::default(); 2];

        let progress = decoder.decode(&source, &mut output);
        assert_eq!(progress.produced_frames, 2);
        assert!(progress.finished);
        assert_eq!(
            output,
            [
                PcmFrame { left: 1, right: 1 },
                PcmFrame { left: -1, right: -1 },
            ]
        );
    }

    #[test]
    fn output_backpressure_leaves_unconsumed_input_with_no_hidden_allocation() {
        let format = parse_pcm16_format(&fmt(2)).unwrap();
        let source = [1, 0, 2, 0, 3, 0, 4, 0];
        let mut decoder = WavPcm16Decoder::new(format, source.len() as u64).unwrap();
        let mut one = [PcmFrame::default(); 1];

        let first = decoder.decode(&source, &mut one);
        assert_eq!(first.consumed_bytes, 4);
        assert_eq!(first.produced_frames, 1);
        assert!(!first.finished);

        let second = decoder.decode(&source[first.consumed_bytes..], &mut one);
        assert_eq!(second.consumed_bytes, 4);
        assert_eq!(second.produced_frames, 1);
        assert!(second.finished);
        assert_eq!(decoder.decoded_frames(), 2);
    }

    #[test]
    fn misaligned_data_chunk_fails_before_decode() {
        let format = parse_pcm16_format(&fmt(2)).unwrap();
        assert_eq!(
            WavPcm16Decoder::new(format, 5).err(),
            Some(WavError::DataMisaligned {
                data_bytes: 5,
                block_align: 4,
            })
        );
    }
}
