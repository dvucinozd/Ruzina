//! Bounded baseline JPEG thumbnail decoder (RP-07, M2.5 TJpgDec behavior).
//!
//! Decodes one interleaved baseline scan MCU by MCU and area-averages the
//! pixels straight into a square RGB565 thumbnail, so no full-size image is
//! ever held. All state is fixed-size and owned by the caller. A truncated
//! entropy stream, an undefined table, a second scan or anything outside the
//! probed baseline layouts fails the decode; the owner then shows the logo.

use crate::artwork::{ArtworkDecoder, JpegInfo, JpegVerdict};

/// Largest thumbnail edge the accumulators hold (deck 34, Library row 40).
pub const JPEG_THUMB_MAX_EDGE: usize = 40;

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// `round(8192 * s(u) * cos((2x + 1) u pi / 16))`, s(0) = 1/(2 sqrt 2), else
/// 1/2, so a row and a column pass together give the 2-D IDCT scaled by 2^26.
const IDCT: [[i64; 8]; 8] = [
    [2896, 4017, 3784, 3406, 2896, 2276, 1567, 799],
    [2896, 3406, 1567, -799, -2896, -4017, -3784, -2276],
    [2896, 2276, -1567, -4017, -2896, 799, 3784, 3406],
    [2896, 799, -3784, -2276, 2896, 3406, -1567, -4017],
    [2896, -799, -3784, 2276, 2896, -3406, -1567, 4017],
    [2896, -2276, -1567, 4017, -2896, -799, 3784, -3406],
    [2896, -3406, 1567, 799, -2896, 4017, -3784, 2276],
    [2896, -4017, 3784, -3406, 2896, -2276, 1567, -799],
];

#[derive(Clone, Copy)]
struct Huffman {
    defined: bool,
    maxcode: [i32; 17],
    valptr: [i32; 17],
    mincode: [i32; 17],
    values: [u8; 256],
}

impl Huffman {
    const EMPTY: Self = Self {
        defined: false,
        maxcode: [-1; 17],
        valptr: [0; 17],
        mincode: [0; 17],
        values: [0; 256],
    };

    fn build(counts: &[u8], values: &[u8]) -> Option<Self> {
        let mut table = Self::EMPTY;
        let (mut code, mut index) = (0i32, 0i32);
        for length in 1..=16 {
            let count = i32::from(counts[length - 1]);
            if count != 0 {
                table.valptr[length] = index;
                table.mincode[length] = code;
                code += count;
                index += count;
                table.maxcode[length] = code - 1;
                if code > 1 << length {
                    return None;
                }
            }
            code <<= 1;
        }
        table.values[..values.len()].copy_from_slice(values);
        table.defined = true;
        Some(table)
    }
}

#[derive(Clone, Copy, Default)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    quant: usize,
    dc: usize,
    ac: usize,
    pred: i32,
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    count: u32,
    at_marker: bool,
    starved: bool,
}

impl Bits<'_> {
    fn bit(&mut self) -> u32 {
        if self.count == 0 {
            self.fill();
        }
        self.count -= 1;
        (self.acc >> self.count) & 1
    }

    fn fill(&mut self) {
        let byte = match (
            self.at_marker,
            self.data.get(self.pos),
            self.data.get(self.pos + 1),
        ) {
            (false, Some(0xff), Some(0x00)) => {
                self.pos += 2;
                0xff
            }
            (false, Some(0xff), _) | (false, None, _) | (true, _, _) => {
                // A marker or the end of data: whatever is read now is
                // padding, so the scan is short of entropy-coded data.
                self.at_marker = true;
                self.starved = true;
                0
            }
            (false, Some(&byte), _) => {
                self.pos += 1;
                byte
            }
        };
        self.acc = (self.acc << 8) | u32::from(byte);
        self.count += 8;
    }

    fn receive(&mut self, bits: u32) -> i32 {
        let mut value = 0i32;
        for _ in 0..bits {
            value = (value << 1) | self.bit() as i32;
        }
        value
    }

    fn decode(&mut self, table: &Huffman) -> Option<u8> {
        let mut code = 0i32;
        for length in 1..=16 {
            code = (code << 1) | self.bit() as i32;
            if code <= table.maxcode[length] {
                let index = table.valptr[length] + code - table.mincode[length];
                return table.values.get(usize::try_from(index).ok()?).copied();
            }
        }
        None
    }

    /// Byte-aligns and consumes the expected RSTn marker.
    fn restart(&mut self, expected: u8) -> bool {
        self.acc = 0;
        self.count = 0;
        self.at_marker = false;
        if self.data.get(self.pos) == Some(&0xff) && self.data.get(self.pos + 1) == Some(&expected)
        {
            self.pos += 2;
            true
        } else {
            false
        }
    }
}

fn extend(value: i32, bits: u32) -> i32 {
    if bits == 0 || value >= 1 << (bits - 1) {
        value
    } else {
        value - (1 << bits) + 1
    }
}

pub struct BaselineJpegDecoder {
    quant: [[u16; 64]; 4],
    quant_defined: [bool; 4],
    dc: [Huffman; 4],
    ac: [Huffman; 4],
    acc: [[u32; 4]; JPEG_THUMB_MAX_EDGE * JPEG_THUMB_MAX_EDGE],
    samples: [[u8; 256]; 3],
}

impl Default for BaselineJpegDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl BaselineJpegDecoder {
    pub const fn new() -> Self {
        Self {
            quant: [[0; 64]; 4],
            quant_defined: [false; 4],
            dc: [Huffman::EMPTY; 4],
            ac: [Huffman::EMPTY; 4],
            acc: [[0; 4]; JPEG_THUMB_MAX_EDGE * JPEG_THUMB_MAX_EDGE],
            samples: [[0; 256]; 3],
        }
    }

    fn decode_inner(&mut self, jpeg: &[u8], edge: usize, out: &mut [u16]) -> Option<()> {
        self.quant_defined = [false; 4];
        self.dc = [Huffman::EMPTY; 4];
        self.ac = [Huffman::EMPTY; 4];
        if jpeg.get(..2)? != [0xff, 0xd8] {
            return None;
        }
        let mut pos = 2;
        let mut frame: Option<(usize, usize)> = None;
        let mut components = [Component::default(); 3];
        let mut count = 0usize;
        let mut restart_interval = 0usize;
        loop {
            while jpeg.get(pos) == Some(&0xff) && jpeg.get(pos + 1) == Some(&0xff) {
                pos += 1;
            }
            if *jpeg.get(pos)? != 0xff {
                return None;
            }
            let marker = *jpeg.get(pos + 1)?;
            let length = usize::from(u16::from_be_bytes([
                *jpeg.get(pos + 2)?,
                *jpeg.get(pos + 3)?,
            ]));
            let segment = jpeg
                .get(pos + 4..pos + 2 + length)
                .filter(|_| length >= 2)?;
            match marker {
                0xdb => self.quant_tables(segment)?,
                0xc4 => self.huffman_tables(segment)?,
                0xdd => {
                    restart_interval =
                        usize::from(u16::from_be_bytes(segment.get(..2)?.try_into().ok()?));
                }
                0xc0 => {
                    if frame.is_some() || *segment.first()? != 8 {
                        return None;
                    }
                    let height =
                        usize::from(u16::from_be_bytes(segment.get(1..3)?.try_into().ok()?));
                    let width =
                        usize::from(u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?));
                    count = usize::from(*segment.get(5)?);
                    if width == 0 || height == 0 || !matches!(count, 1 | 3) {
                        return None;
                    }
                    for (index, component) in components[..count].iter_mut().enumerate() {
                        let spec = segment.get(6 + index * 3..9 + index * 3)?;
                        *component = Component {
                            id: spec[0],
                            h: usize::from(spec[1] >> 4),
                            v: usize::from(spec[1] & 15),
                            quant: usize::from(spec[2]),
                            ..Component::default()
                        };
                        if component.quant > 3 {
                            return None;
                        }
                    }
                    if count == 1 {
                        components[0].h = 1;
                        components[0].v = 1;
                    } else if !matches!(
                        (components[0].h, components[0].v),
                        (1, 1) | (2, 1) | (2, 2)
                    ) || components[1..3].iter().any(|c| (c.h, c.v) != (1, 1))
                    {
                        return None;
                    }
                    frame = Some((width, height));
                }
                0xc1..=0xcf => return None,
                0xda => {
                    let (width, height) = frame?;
                    let scan = usize::from(*segment.first()?);
                    if scan != count {
                        return None;
                    }
                    for index in 0..scan {
                        let spec = segment.get(1 + index * 2..3 + index * 2)?;
                        let component = components[..count].iter_mut().find(|c| c.id == spec[0])?;
                        component.dc = usize::from(spec[1] >> 4);
                        component.ac = usize::from(spec[1] & 15);
                        if component.dc > 3
                            || component.ac > 3
                            || !self.dc[component.dc].defined
                            || !self.ac[component.ac].defined
                            || !self.quant_defined[component.quant]
                        {
                            return None;
                        }
                    }
                    let data = jpeg.get(pos + 2 + length..)?;
                    return self.scan(
                        data,
                        &mut components[..count],
                        width,
                        height,
                        restart_interval,
                        edge,
                        out,
                    );
                }
                0xd9 => return None,
                _ => {}
            }
            pos += 2 + length;
        }
    }

    fn quant_tables(&mut self, mut segment: &[u8]) -> Option<()> {
        while let Some((&spec, rest)) = segment.split_first() {
            let (precision, id) = (spec >> 4, usize::from(spec & 15));
            if id > 3 || precision > 1 {
                return None;
            }
            let bytes = if precision == 0 { 64 } else { 128 };
            let table = rest.get(..bytes)?;
            for (k, value) in self.quant[id].iter_mut().enumerate() {
                *value = if precision == 0 {
                    u16::from(table[k])
                } else {
                    u16::from_be_bytes([table[2 * k], table[2 * k + 1]])
                };
            }
            self.quant_defined[id] = true;
            segment = &rest[bytes..];
        }
        Some(())
    }

    fn huffman_tables(&mut self, mut segment: &[u8]) -> Option<()> {
        while let Some((&spec, rest)) = segment.split_first() {
            let (class, id) = (spec >> 4, usize::from(spec & 15));
            let counts = rest.get(..16)?;
            let total: usize = counts.iter().map(|&n| usize::from(n)).sum();
            if class > 1 || id > 3 || total > 256 {
                return None;
            }
            let values = rest.get(16..16 + total)?;
            let table = Huffman::build(counts, values)?;
            if class == 0 {
                self.dc[id] = table;
            } else {
                self.ac[id] = table;
            }
            segment = &rest[16 + total..];
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn scan(
        &mut self,
        data: &[u8],
        components: &mut [Component],
        width: usize,
        height: usize,
        restart_interval: usize,
        edge: usize,
        out: &mut [u16],
    ) -> Option<()> {
        let hmax = components.iter().map(|c| c.h).max()?;
        let vmax = components.iter().map(|c| c.v).max()?;
        let (mcu_w, mcu_h) = (8 * hmax, 8 * vmax);
        let mcus_x = width.div_ceil(mcu_w);
        let mcus_y = height.div_ceil(mcu_h);
        self.acc[..edge * edge].fill([0; 4]);
        let mut bits = Bits {
            data,
            pos: 0,
            acc: 0,
            count: 0,
            at_marker: false,
            starved: false,
        };
        let mut block = [0i32; 64];
        let mut restart = 0u8;
        for mcu in 0..mcus_x * mcus_y {
            if restart_interval != 0 && mcu != 0 && mcu % restart_interval == 0 {
                if !bits.restart(0xd0 + restart) {
                    return None;
                }
                restart = (restart + 1) & 7;
                components.iter_mut().for_each(|c| c.pred = 0);
            }
            for (index, component) in components.iter_mut().enumerate() {
                let stride = 8 * component.h;
                for by in 0..component.v {
                    for bx in 0..component.h {
                        self.block(&mut bits, component, &mut block)?;
                        let samples = &mut self.samples[index];
                        for y in 0..8 {
                            for x in 0..8 {
                                samples[(by * 8 + y) * stride + bx * 8 + x] =
                                    block[y * 8 + x] as u8;
                            }
                        }
                    }
                }
            }
            let (x0, y0) = ((mcu % mcus_x) * mcu_w, (mcu / mcus_x) * mcu_h);
            for py in 0..mcu_h.min(height - y0) {
                for px in 0..mcu_w.min(width - x0) {
                    let sample = |index: usize| {
                        let c = components[index];
                        let (sx, sy) = (px * c.h / hmax, py * c.v / vmax);
                        i32::from(self.samples[index][sy * 8 * c.h + sx])
                    };
                    let rgb = if components.len() == 1 {
                        let y = sample(0);
                        [y, y, y]
                    } else {
                        ycbcr(sample(0), sample(1), sample(2))
                    };
                    self.accumulate(x0 + px, y0 + py, width, height, edge, rgb);
                }
            }
        }
        if bits.starved {
            return None;
        }
        for (cell, pixel) in self.acc[..edge * edge].iter().zip(out.iter_mut()) {
            let n = cell[3].max(1);
            let channel = |sum: u32| ((sum + n / 2) / n) as u16;
            *pixel = (channel(cell[0]) >> 3) << 11
                | (channel(cell[1]) >> 2) << 5
                | channel(cell[2]) >> 3;
        }
        Some(())
    }

    fn block(
        &self,
        bits: &mut Bits<'_>,
        component: &mut Component,
        out: &mut [i32; 64],
    ) -> Option<()> {
        let quant = &self.quant[component.quant];
        let mut coefficients = [0i64; 64];
        let size = u32::from(bits.decode(&self.dc[component.dc])?);
        if size > 11 {
            return None;
        }
        component.pred += extend(bits.receive(size), size);
        coefficients[0] = i64::from(component.pred) * i64::from(quant[0]);
        let mut k = 1;
        while k < 64 {
            let rs = bits.decode(&self.ac[component.ac])?;
            let (run, size) = (usize::from(rs >> 4), u32::from(rs & 15));
            if size == 0 {
                if run != 15 {
                    break;
                }
                k += 16;
                continue;
            }
            k += run;
            if k > 63 || size > 10 {
                return None;
            }
            coefficients[usize::from(ZIGZAG[k])] =
                i64::from(extend(bits.receive(size), size)) * i64::from(quant[k]);
            k += 1;
        }
        if k > 64 {
            return None;
        }
        let mut rows = [0i64; 64];
        for v in 0..8 {
            for x in 0..8 {
                rows[v * 8 + x] = (0..8).map(|u| IDCT[x][u] * coefficients[v * 8 + u]).sum();
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                let value: i64 = (0..8).map(|v| IDCT[y][v] * rows[v * 8 + x]).sum();
                out[y * 8 + x] = (((value + (1 << 25)) >> 26) + 128).clamp(0, 255) as i32;
            }
        }
        Some(())
    }

    /// Adds one source pixel to every thumbnail cell it covers, so an
    /// upscaled small cover leaves no empty cell.
    fn accumulate(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        edge: usize,
        rgb: [i32; 3],
    ) {
        let (cx0, cx1) = (x * edge / width, ((x + 1) * edge - 1) / width);
        let (cy0, cy1) = (y * edge / height, ((y + 1) * edge - 1) / height);
        for cy in cy0..=cy1.min(edge - 1) {
            for cx in cx0..=cx1.min(edge - 1) {
                let cell = &mut self.acc[cy * edge + cx];
                for (sum, value) in cell.iter_mut().zip(rgb) {
                    *sum += value as u32;
                }
                cell[3] += 1;
            }
        }
    }
}

/// JFIF full-range YCbCr to RGB, 16.16 fixed point.
fn ycbcr(y: i32, cb: i32, cr: i32) -> [i32; 3] {
    let (cb, cr) = (cb - 128, cr - 128);
    let round = 1 << 15;
    [
        y + ((91_881 * cr + round) >> 16),
        y - ((22_554 * cb + 46_802 * cr + round) >> 16),
        y + ((116_130 * cb + round) >> 16),
    ]
    .map(|value| value.clamp(0, 255))
}

impl ArtworkDecoder for BaselineJpegDecoder {
    fn decode(&mut self, jpeg: &[u8], info: JpegInfo, edge: u16, out: &mut [u16]) -> bool {
        let edge = usize::from(edge);
        info.verdict == JpegVerdict::Baseline
            && (1..=JPEG_THUMB_MAX_EDGE).contains(&edge)
            && out.len() == edge * edge
            && self.decode_inner(jpeg, edge, out).is_some()
    }
}
