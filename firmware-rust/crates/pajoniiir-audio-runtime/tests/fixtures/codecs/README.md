# Synthetic codec fixtures (CC0)

All audio here is a generated chirp/sine sum or an integer boundary pattern.
No third-party recordings are included. `generate.py` reproduces the fixtures;
the retained baseline used Python 3 and Ubuntu FFmpeg 6.1.1 with libmp3lame.
Encoder versions can change compressed bytes and decoded golden PCM, so changing
the fixtures requires deliberate review.

WAV sources contain 12,000 frames. MP3 fixtures cover 48 kHz CBR stereo, 44.1 kHz
VBR stereo, 22.05 kHz mono, and CBR without Xing length/gapless metadata. An eight-second CBR stream exercises late loop checkpoints and recent seek restoration. Native
FLAC covers 16-bit stereo/mono, 24-bit 96 kHz stereo (9,216-frame blocks), and a
hand-built 32-bit VERBATIM stereo frame with valid header/frame CRCs.

`*-reference.pcm` is independently decoded signed PCM16 stereo from FFmpeg.
Mono is duplicated at unity explicitly; FFmpeg's automatic stereo upmix would
attenuate it. MP3 tests require matching frame count, >70 dB SNR and at most 16
sample units of decoder difference. FLAC16 matches source PCM exactly. Deeper
FLAC conversions permit one PCM16 unit for rounding differences.
