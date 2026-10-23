use pajoniiir_media_catalog::artwork::{
    ARTWORK_DECK_EDGE, ArtworkDecks, ArtworkDecoder, ArtworkFallback, ArtworkIdentity, ArtworkView,
    JpegVerdict, probe_jpeg,
};
use pajoniiir_media_catalog::jpeg::BaselineJpegDecoder;

/// Name, JPEG, FFmpeg's RGB24 decode, width, height.
type Fixture = (&'static str, &'static [u8], &'static [u8], usize, usize);

const FIXTURES: [Fixture; 4] = [
    (
        "420",
        include_bytes!("fixtures/artwork/cover-420.jpg"),
        include_bytes!("fixtures/artwork/cover-420.rgb"),
        96,
        72,
    ),
    (
        "444",
        include_bytes!("fixtures/artwork/cover-444.jpg"),
        include_bytes!("fixtures/artwork/cover-444.rgb"),
        34,
        34,
    ),
    (
        "422",
        include_bytes!("fixtures/artwork/cover-422.jpg"),
        include_bytes!("fixtures/artwork/cover-422.rgb"),
        50,
        38,
    ),
    (
        "gray",
        include_bytes!("fixtures/artwork/cover-gray.jpg"),
        include_bytes!("fixtures/artwork/cover-gray.rgb"),
        30,
        20,
    ),
];

fn decode(jpeg: &[u8], edge: u16) -> Option<Vec<u16>> {
    let mut decoder = Box::new(BaselineJpegDecoder::new());
    let mut out = vec![0u16; usize::from(edge) * usize::from(edge)];
    decoder
        .decode(jpeg, probe_jpeg(jpeg), edge, &mut out)
        .then_some(out)
}

/// Area average of FFmpeg's full decode with the decoder's cell coverage.
fn reference(rgb: &[u8], width: usize, height: usize, edge: usize) -> Vec<[u32; 3]> {
    let mut sums = vec![[0u32; 4]; edge * edge];
    for y in 0..height {
        for x in 0..width {
            let pixel = &rgb[(y * width + x) * 3..][..3];
            for cy in y * edge / height..=((y + 1) * edge - 1) / height {
                for cx in x * edge / width..=((x + 1) * edge - 1) / width {
                    let cell = &mut sums[cy * edge + cx];
                    for c in 0..3 {
                        cell[c] += u32::from(pixel[c]);
                    }
                    cell[3] += 1;
                }
            }
        }
    }
    sums.iter()
        .map(|s| [0, 1, 2].map(|c| (s[c] + s[3] / 2) / s[3]))
        .collect()
}

fn fields(pixel: u16) -> [u32; 3] {
    [
        u32::from(pixel >> 11),
        u32::from((pixel >> 5) & 63),
        u32::from(pixel & 31),
    ]
}

#[test]
fn baseline_layouts_match_an_independent_decode_within_rounding() {
    for (name, jpeg, rgb, width, height) in FIXTURES {
        let info = probe_jpeg(jpeg);
        assert_eq!(info.verdict, JpegVerdict::Baseline, "{name}");
        assert_eq!(
            (usize::from(info.width), usize::from(info.height)),
            (width, height)
        );
        for edge in [ARTWORK_DECK_EDGE, 40, 7] {
            let ours = decode(jpeg, edge).unwrap_or_else(|| panic!("{name} edge {edge}"));
            let expected = reference(rgb, width, height, usize::from(edge));
            let mut worst = 0;
            for (pixel, want) in ours.iter().zip(&expected) {
                let want = [want[0] >> 3, want[1] >> 2, want[2] >> 3];
                for (got, want) in fields(*pixel).into_iter().zip(want) {
                    worst = worst.max(got.abs_diff(want));
                }
            }
            // IDCT and chroma upsampling differ from FFmpeg only in rounding:
            // every channel lands within one RGB565 step of its reference.
            assert!(worst <= 1, "{name} edge {edge}: {worst} RGB565 steps off");
        }
    }
    // M2.5's TJpgDec layouts only: progressive and FFmpeg's 1x2-everywhere
    // 4:4:4 are refused before decoding, so the deck shows the logo.
    for (jpeg, verdict) in [
        (
            include_bytes!("fixtures/artwork/cover-progressive.jpg").as_slice(),
            JpegVerdict::NotBaseline,
        ),
        (
            include_bytes!("fixtures/artwork/cover-ffmpeg-444.jpg").as_slice(),
            JpegVerdict::Sampling,
        ),
    ] {
        assert_eq!(probe_jpeg(jpeg).verdict, verdict);
        assert_eq!(decode(jpeg, ARTWORK_DECK_EDGE), None);
    }
}

/// A minimal baseline encoder for flat 8x8 grayscale blocks: DC-only
/// coefficients with quantization 1, the standard luminance DC table, an
/// EOB-only AC table and optional restart intervals.
fn flat_gray_jpeg(blocks_x: usize, blocks_y: usize, levels: &[u8], restart: usize) -> Vec<u8> {
    const DC_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
    let mut codes = [(0u32, 0u32); 12];
    let (mut code, mut symbol) = (0u32, 0usize);
    for (length, &count) in DC_BITS.iter().enumerate() {
        for _ in 0..count {
            codes[symbol] = (code, length as u32 + 1);
            code += 1;
            symbol += 1;
        }
        code <<= 1;
    }
    let mut out = vec![0xff, 0xd8];
    out.extend([0xff, 0xdb, 0x00, 0x43, 0x00]);
    out.extend([1u8; 64]);
    let (w, h) = ((blocks_x * 8) as u16, (blocks_y * 8) as u16);
    out.extend([0xff, 0xc0, 0x00, 0x0b, 0x08]);
    out.extend(h.to_be_bytes());
    out.extend(w.to_be_bytes());
    out.extend([0x01, 0x01, 0x11, 0x00]);
    out.extend([0xff, 0xc4, 0x00, 0x1f, 0x00]);
    out.extend(DC_BITS);
    out.extend(0u8..12);
    out.extend([0xff, 0xc4, 0x00, 0x14, 0x10, 0x01]);
    out.extend([0u8; 15]);
    out.push(0x00);
    if restart != 0 {
        out.extend([0xff, 0xdd, 0x00, 0x04]);
        out.extend((restart as u16).to_be_bytes());
    }
    out.extend([0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00]);

    let mut bits: Vec<bool> = Vec::new();
    let flush = |bits: &mut Vec<bool>, out: &mut Vec<u8>| {
        while !bits.len().is_multiple_of(8) {
            bits.push(true);
        }
        for byte in bits.chunks(8) {
            let value = byte.iter().fold(0u8, |v, &b| (v << 1) | u8::from(b));
            out.push(value);
            if value == 0xff {
                out.push(0x00);
            }
        }
        bits.clear();
    };
    let push = |bits: &mut Vec<bool>, value: u32, length: u32| {
        for i in (0..length).rev() {
            bits.push((value >> i) & 1 == 1);
        }
    };
    let mut pred = 0i32;
    let mut marker = 0u8;
    for (index, &level) in levels.iter().enumerate() {
        if restart != 0 && index != 0 && index % restart == 0 {
            flush(&mut bits, &mut out);
            out.extend([0xff, 0xd0 + marker]);
            marker = (marker + 1) & 7;
            pred = 0;
        }
        let dc = 8 * (i32::from(level) - 128);
        let diff = dc - pred;
        pred = dc;
        let size = 32 - diff.unsigned_abs().leading_zeros();
        let (code, length) = codes[size as usize];
        push(&mut bits, code, length);
        let magnitude = if diff < 0 { diff - 1 } else { diff } as u32 & ((1 << size) - 1);
        push(&mut bits, magnitude, size);
        push(&mut bits, 0, 1); // EOB
    }
    flush(&mut bits, &mut out);
    out.extend([0xff, 0xd9]);
    out
}

fn gray565(level: u8) -> u16 {
    let l = u16::from(level);
    (l >> 3) << 11 | (l >> 2) << 5 | l >> 3
}

#[test]
fn flat_blocks_decode_exactly_with_and_without_restart_intervals() {
    let levels = [0u8, 17, 128, 200, 255, 3, 90, 151, 64];
    for restart in [0, 1, 2, 4] {
        let jpeg = flat_gray_jpeg(3, 3, &levels, restart);
        assert_eq!(probe_jpeg(&jpeg).verdict, JpegVerdict::Baseline);
        let out = decode(&jpeg, 3).unwrap_or_else(|| panic!("restart {restart}"));
        assert_eq!(out, levels.map(gray565), "restart {restart}");
    }
}

#[test]
fn truncated_misordered_or_corrupt_streams_fail_without_panicking() {
    let levels = [10u8, 240, 30, 220, 50, 200, 70, 180, 90];
    let jpeg = flat_gray_jpeg(3, 3, &levels, 3);
    let scan = jpeg.windows(2).position(|w| w == [0xff, 0xda]).unwrap() + 10;
    // Cutting the entropy data short leaves later blocks without bits.
    for end in [scan + 1, scan + 3, jpeg.len() - 6] {
        let mut short = jpeg[..end].to_vec();
        short.extend([0xff, 0xd9]);
        assert_eq!(decode(&short, 3), None, "end {end}");
    }
    // A wrong restart marker number is refused.
    let mut wrong = jpeg.clone();
    let rst = wrong.windows(2).position(|w| w == [0xff, 0xd0]).unwrap();
    wrong[rst + 1] = 0xd3;
    assert_eq!(decode(&wrong, 3), None);
    // Byte damage and truncation anywhere must stay panic-free.
    for (_, jpeg, ..) in FIXTURES {
        for at in (0..jpeg.len()).step_by(3) {
            let mut damaged = jpeg.to_vec();
            damaged[at] ^= 0x5a;
            let _ = decode(&damaged, ARTWORK_DECK_EDGE);
            let _ = decode(&jpeg[..at], ARTWORK_DECK_EDGE);
        }
    }
    // Edge bounds are enforced by the decoder too.
    let (_, cover, ..) = FIXTURES[0];
    assert_eq!(decode(cover, 0), None);
    assert_eq!(decode(cover, 41), None);
}

#[test]
fn deck_owner_publishes_a_decoded_cover_and_the_logo_for_bad_files() {
    use pajoniiir_core::MediaTrackId;
    use pajoniiir_media_session::{MediaGeneration, MediaLease, MediaSourceId};
    let identity = ArtworkIdentity {
        track: MediaTrackId([4; 16]),
        lease: MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        },
        load_generation: 9,
    };
    let mut decks = ArtworkDecks::new();
    let mut decoder = Box::new(BaselineJpegDecoder::new());
    let (_, cover, ..) = FIXTURES[0];
    let ticket = decks.begin(0, identity, true).unwrap();
    assert!(decks.admit_file(ticket, cover.len() as u64));
    assert!(decks.complete(ticket, cover, decoder.as_mut()));
    let ArtworkView::Cover { pixels, .. } = decks.view(0, identity) else {
        panic!("fixture cover must decode");
    };
    assert_eq!(pixels, decode(cover, ARTWORK_DECK_EDGE).unwrap());

    let mut broken = cover.to_vec();
    broken.truncate(cover.len() / 2);
    let ticket = decks.begin(0, identity, true).unwrap();
    assert!(decks.complete(ticket, &broken, decoder.as_mut()));
    assert_eq!(
        decks.view(0, identity),
        ArtworkView::Fallback(ArtworkFallback::DecodeFailed)
    );
}
