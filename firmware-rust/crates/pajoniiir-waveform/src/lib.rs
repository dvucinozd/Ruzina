#![no_std]
#![forbid(unsafe_code)]

/// One pre-reduced waveform column.
///
/// `peak` is normalized to the full u16 range. Color is native RGB565.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaveformColumn {
    pub peak: u16,
    pub color_rgb565: u16,
}

/// Borrowed waveform view ready for rasterization.
#[derive(Clone, Copy, Debug)]
pub struct WaveformView<'a> {
    pub columns: &'a [WaveformColumn],
}

/// Minimal RGB565 surface used by the hot raster path.
///
/// The public surface is intentionally toolkit-agnostic. embedded-graphics can
/// be layered on top for primitives without making the core waveform span path
/// depend on a display driver.
pub struct Rgb565Surface<'a> {
    pixels: &'a mut [u16],
    width: usize,
    height: usize,
}

impl<'a> Rgb565Surface<'a> {
    pub fn new(pixels: &'a mut [u16], width: usize, height: usize) -> Option<Self> {
        if width.checked_mul(height)? != pixels.len() {
            return None;
        }
        Some(Self {
            pixels,
            width,
            height,
        })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn clear(&mut self, color_rgb565: u16) {
        self.pixels.fill(color_rgb565);
    }

    pub fn draw_centered_waveform(
        &mut self,
        x0: usize,
        y0: usize,
        width: usize,
        height: usize,
        view: WaveformView<'_>,
    ) {
        if width == 0 || height == 0 || view.columns.is_empty() {
            return;
        }

        let drawable_width = width.min(self.width.saturating_sub(x0));
        let drawable_height = height.min(self.height.saturating_sub(y0));
        if drawable_width == 0 || drawable_height == 0 {
            return;
        }

        let center = y0 + drawable_height / 2;
        let half = drawable_height / 2;

        for screen_x in 0..drawable_width {
            let source_index = screen_x.saturating_mul(view.columns.len()) / drawable_width;
            let column = view.columns[source_index.min(view.columns.len() - 1)];
            let amplitude = half.saturating_mul(column.peak as usize) / u16::MAX as usize;

            let start_y = center.saturating_sub(amplitude);
            let end_y = (center + amplitude).min(y0 + drawable_height - 1);
            self.draw_vspan(x0 + screen_x, start_y, end_y, column.color_rgb565);
        }
    }

    pub fn draw_playhead(&mut self, x: usize, y0: usize, height: usize, color_rgb565: u16) {
        if x >= self.width || y0 >= self.height || height == 0 {
            return;
        }
        let end_y = (y0 + height - 1).min(self.height - 1);
        self.draw_vspan(x, y0, end_y, color_rgb565);
    }

    fn draw_vspan(&mut self, x: usize, start_y: usize, end_y: usize, color: u16) {
        if x >= self.width || start_y >= self.height || start_y > end_y {
            return;
        }
        let last = end_y.min(self.height - 1);
        for y in start_y..=last {
            self.pixels[y * self.width + x] = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_buffer_geometry() {
        let mut pixels = [0u16; 7];
        assert!(Rgb565Surface::new(&mut pixels, 4, 2).is_none());
    }

    #[test]
    fn renders_bounded_columns() {
        let mut pixels = [0u16; 8 * 8];
        let mut surface = Rgb565Surface::new(&mut pixels, 8, 8).unwrap();
        let columns = [WaveformColumn {
            peak: u16::MAX,
            color_rgb565: 0xffff,
        }; 8];

        surface.draw_centered_waveform(0, 0, 8, 8, WaveformView { columns: &columns });

        assert!(pixels.contains(&0xffff));
    }

    #[test]
    fn rgb565_golden_fixture_is_deterministic() {
        const WAVE: u16 = 0x07e0;
        const PLAYHEAD: u16 = 0xf800;

        let columns = [
            WaveformColumn {
                peak: 0,
                color_rgb565: WAVE,
            },
            WaveformColumn {
                peak: 16_384,
                color_rgb565: WAVE,
            },
            WaveformColumn {
                peak: 32_768,
                color_rgb565: WAVE,
            },
            WaveformColumn {
                peak: 49_152,
                color_rgb565: WAVE,
            },
            WaveformColumn {
                peak: u16::MAX,
                color_rgb565: WAVE,
            },
        ];

        let mut pixels = [0u16; 25];
        {
            let mut surface = Rgb565Surface::new(&mut pixels, 5, 5).unwrap();
            surface.clear(0);
            surface.draw_centered_waveform(0, 0, 5, 5, WaveformView { columns: &columns });
            surface.draw_playhead(2, 0, 5, PLAYHEAD);
        }

        let expected = [
            0, 0, PLAYHEAD, 0, WAVE, 0, 0, PLAYHEAD, WAVE, WAVE, WAVE, WAVE, PLAYHEAD, WAVE, WAVE,
            0, 0, PLAYHEAD, WAVE, WAVE, 0, 0, PLAYHEAD, 0, WAVE,
        ];

        assert_eq!(pixels, expected);
    }
}
