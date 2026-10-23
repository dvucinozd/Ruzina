use ruzina_track_analysis::{
    TrackAnalysis, WaveformColumn, color_preview_column, color_preview_peak,
};
use ruzina_ui_model::DeckPresentation;
use ruzina_waveform::{Rgb565Surface, WaveformView};

pub const WIDTH: usize = 700;
pub const HEIGHT: usize = 90;

const MEMORY_CUE_RGB565: u16 = 0xf800;

/// Compose existing neutral RGB565 raster output. Preview and detail share
/// explicit time bounds so playhead, beat ticks and effective cues stay aligned.
/// The preview uses optional PWV4 colors when imported (PWAV otherwise), and
/// imported memory points/loops are marked at the bottom, apart from hot cues.
pub fn compose(p: DeckPresentation, analysis: Option<TrackAnalysis<'_>>, detail: bool) -> Vec<u16> {
    let mut pixels = vec![0x0842; WIDTH * HEIGHT];
    let Some(analysis) = analysis else {
        return pixels;
    };
    let duration = p.transport.duration_ms as u32;
    let position = p.transport.position_ms as u32;
    let color: Vec<WaveformColumn>;
    let (columns, start, end) = if detail {
        let rate = u64::from(analysis.detail_rate_hz());
        if rate == 0 || analysis.detail().is_empty() {
            return pixels;
        }
        let count = WIDTH.min(analysis.detail().len());
        let center =
            (u64::from(position) * rate / 1000).min(analysis.detail().len() as u64) as usize;
        let first = center
            .saturating_sub(count / 2)
            .min(analysis.detail().len() - count);
        (
            &analysis.detail()[first..first + count],
            (first as u64 * 1000 / rate) as u32,
            ((first + count) as u64 * 1000 / rate) as u32,
        )
    } else if analysis.color_preview().is_empty() {
        (analysis.preview(), 0, duration)
    } else {
        let entries = analysis.color_preview();
        let peak = color_preview_peak(entries);
        color = (0..WIDTH)
            .filter_map(|x| color_preview_column(entries, x, WIDTH))
            .map(|column| column.waveform(peak))
            .collect();
        (&color[..], 0, duration)
    };
    let mut surface = Rgb565Surface::new(&mut pixels, WIDTH, HEIGHT).unwrap();
    surface.draw_centered_waveform(0, 8, WIDTH, HEIGHT - 16, WaveformView { columns });
    let x = |time: u32| -> Option<usize> {
        if end <= start || time < start || time > end {
            return None;
        }
        Some((u64::from(time - start) * (WIDTH - 1) as u64 / u64::from(end - start)) as usize)
    };
    if detail && let Some(grid) = analysis.beat_grid() {
        let beats = grid.beats();
        let first = beats.partition_point(|beat| beat.time_ms < start);
        for beat in beats[first..].iter().take_while(|beat| beat.time_ms <= end) {
            if let Some(x) = x(beat.time_ms) {
                surface.draw_playhead(x, 0, if beat.phase == 0 { 10 } else { 5 }, 0x94b2);
            }
        }
    }
    if let Some((start, end)) = p.loop_region {
        for time in [start, end] {
            if let Some(x) = x(time) {
                surface.draw_playhead(x, 8, HEIGHT - 16, 0x07e0);
            }
        }
    }
    for cue in analysis.memory_cues() {
        if let Some(x) = x(cue.start_ms) {
            surface.draw_playhead(x, HEIGHT - 12, 12, MEMORY_CUE_RGB565);
        }
        if let Some(x) = cue.end_ms.and_then(x) {
            surface.draw_playhead(x, HEIGHT - 6, 6, MEMORY_CUE_RGB565);
        }
    }
    for cue in p.cues.into_iter().flatten() {
        if let Some(x) = x(cue.pos_ms) {
            surface.draw_playhead(x, 0, 18, 0xfd20);
        }
        if cue.end_ms > cue.pos_ms
            && let Some(x) = x(cue.end_ms)
        {
            surface.draw_playhead(x, HEIGHT - 18, 18, 0xfd20);
        }
    }
    if let Some(x) = x(position) {
        surface.draw_playhead(x, 0, HEIGHT, 0xffff);
    }
    pixels
}

pub fn image(pixels: &[u16]) -> slint::Image {
    rgb565_image(pixels, WIDTH as u32, HEIGHT as u32)
}

pub fn rgb565_image(pixels: &[u16], width: u32, height: u32) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    for (pixel, rgb) in pixels.iter().zip(buffer.make_mut_slice()) {
        rgb.r = (((pixel >> 11) & 31) * 255 / 31) as u8;
        rgb.g = (((pixel >> 5) & 63) * 255 / 63) as u8;
        rgb.b = ((pixel & 31) * 255 / 31) as u8;
    }
    slint::Image::from_rgb8(buffer)
}
