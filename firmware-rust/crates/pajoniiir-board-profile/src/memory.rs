//! Bounded bring-up observations. No allocator, flash writes or acceptance claim.
use core::fmt::{self, Write};

pub const TEST_WORDS: usize = 16 * 1024;

/// Byte order is the SPI RDID receive-register order: manufacturer first.
/// Unknown/extended density encodings stay unknown; never use a board default.
pub fn flash_capacity(id: u32) -> Option<u32> {
    let [manufacturer, density_or_type, density, high] = id.to_le_bytes();
    if high != 0 || matches!(manufacturer, 0 | 0xff) {
        return None;
    }
    if manufacturer == 0x1f {
        match density_or_type & 0x1f {
            4..=9 => Some(1 << (15 + (density_or_type & 0x1f))),
            _ => None,
        }
    } else {
        match density {
            0x12..=0x1a => Some(1 << density),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashProbe {
    NotRequested,
    Busy,
    Timeout,
    ReadId(u32),
}
impl FlashProbe {
    pub fn bytes(self) -> Option<u32> {
        match self {
            Self::ReadId(id) => flash_capacity(id),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NotRequested => "NOT_REQUESTED",
            Self::Busy => "BUSY",
            Self::Timeout => "TIMEOUT",
            Self::ReadId(id) if flash_capacity(id).is_some() => "ID_DECODED",
            Self::ReadId(_) => "UNKNOWN_ID",
        }
    }
}

/// Access to an exclusively reserved CPU-visible scratch span, not general RAM.
pub trait ScratchWords {
    fn words(&self) -> usize;
    fn write(&mut self, index: usize, value: u32);
    fn read(&mut self, index: usize) -> u32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mismatch {
    pub word: u32,
    pub expected: u32,
    pub actual: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScratchError {
    Empty,
    Mismatch(Mismatch),
}

/// Three complete write/read passes detect stuck bits and address aliasing in
/// this span. Volatile access alone does NOT prove cache writeback or DMA safety.
pub fn test_scratch(memory: &mut impl ScratchWords) -> Result<(), ScratchError> {
    let words = memory.words().min(TEST_WORDS);
    if words == 0 {
        return Err(ScratchError::Empty);
    }
    for pattern in [0xaaaa_aaaa, 0x5555_5555, 0x693c_a5f0] {
        for index in 0..words {
            memory.write(index, pattern ^ index as u32);
        }
        for index in 0..words {
            let expected = pattern ^ index as u32;
            let actual = memory.read(index);
            if actual != expected {
                return Err(ScratchError::Mismatch(Mismatch {
                    word: index as u32,
                    expected,
                    actual,
                }));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PsramProbe {
    NotRequested,
    InvalidMapping,
    CapacityMismatch {
        mapped_bytes: u32,
    },
    ScratchPassed {
        mapped_bytes: u32,
    },
    ScratchFailed {
        mapped_bytes: u32,
        mismatch: Mismatch,
    },
}
impl PsramProbe {
    pub fn bytes(self) -> Option<u32> {
        match self {
            Self::NotRequested | Self::InvalidMapping => None,
            Self::CapacityMismatch { mapped_bytes }
            | Self::ScratchPassed { mapped_bytes }
            | Self::ScratchFailed { mapped_bytes, .. } => Some(mapped_bytes),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NotRequested => "NOT_REQUESTED",
            Self::InvalidMapping => "INVALID_MAPPING",
            Self::CapacityMismatch { .. } => "CAPACITY_MISMATCH",
            Self::ScratchPassed { .. } => "CPU_SCRATCH_PASSED",
            Self::ScratchFailed { .. } => "CPU_SCRATCH_FAILED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryReport {
    pub flash: FlashProbe,
    pub psram: PsramProbe,
}
impl Default for MemoryReport {
    fn default() -> Self {
        Self {
            flash: FlashProbe::NotRequested,
            psram: PsramProbe::NotRequested,
        }
    }
}
impl MemoryReport {
    pub fn write_to(&self, output: &mut impl Write, model: &str, image: &str) -> fmt::Result {
        writeln!(output, "M1_MEMORY_DIAGNOSTIC_V1")?;
        writeln!(output, "board_profile={model}")?;
        writeln!(output, "image_id={image}")?;
        writeln!(output, "flash_probe={}", self.flash.label())?;
        match self.flash {
            FlashProbe::ReadId(id) => writeln!(output, "flash_rdid_raw={id:06x}")?,
            _ => writeln!(output, "flash_rdid_raw=NOT_MEASURED")?,
        }
        write_bytes(output, "flash_bytes_rdid", self.flash.bytes())?;
        writeln!(output, "psram_probe={}", self.psram.label())?;
        write_bytes(output, "psram_bytes_mapped", self.psram.bytes())?;
        let tested = match self.psram {
            PsramProbe::ScratchPassed { .. } | PsramProbe::ScratchFailed { .. } => TEST_WORDS * 4,
            _ => 0,
        };
        writeln!(output, "psram_scratch_bytes={tested}")?;
        writeln!(
            output,
            "psram_clock_mhz={}",
            if self.psram == PsramProbe::NotRequested {
                0
            } else {
                80
            }
        )?;
        if let PsramProbe::ScratchFailed { mismatch, .. } = self.psram {
            writeln!(output, "psram_failure_word={}", mismatch.word)?;
            writeln!(output, "psram_expected={:08x}", mismatch.expected)?;
            writeln!(output, "psram_actual={:08x}", mismatch.actual)?;
        } else {
            writeln!(output, "psram_failure_word=NONE")?;
            writeln!(output, "psram_expected=NONE")?;
            writeln!(output, "psram_actual=NONE")?;
        }
        writeln!(output, "test_scope=CPU_CACHED_SCRATCH_ONLY")?;
        writeln!(output, "readiness=NOT_QUALIFIED")
    }
}
fn write_bytes(output: &mut impl Write, key: &str, value: Option<u32>) -> fmt::Result {
    match value {
        Some(bytes) => writeln!(output, "{key}={bytes}"),
        None => writeln!(output, "{key}=NOT_MEASURED"),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    struct Ram {
        words: [u32; 32],
        alias: bool,
        stuck: bool,
        highest: usize,
    }
    impl ScratchWords for Ram {
        fn words(&self) -> usize {
            32
        }
        fn write(&mut self, index: usize, value: u32) {
            self.highest = self.highest.max(index);
            self.words[if self.alias { index / 2 } else { index }] = value;
        }
        fn read(&mut self, index: usize) -> u32 {
            self.words[if self.alias { index / 2 } else { index }] | u32::from(self.stuck)
        }
    }
    #[test]
    fn rdid_byte_order_unknown_and_special_density_do_not_use_profile_defaults() {
        assert_eq!(flash_capacity(0x1840ef), Some(16 * 1024 * 1024));
        assert_eq!(flash_capacity(0x1740c8), Some(8 * 1024 * 1024));
        assert_eq!(flash_capacity(0x00491f), Some(16 * 1024 * 1024));
        for id in [0, 0xffffff, 0x181000, 0x0011ef, 0x1b40ef, 0x11840ef] {
            assert_eq!(flash_capacity(id), None, "{id:x}");
        }
    }
    #[test]
    fn scratch_detects_alias_and_stuck_bit_and_never_exceeds_span() {
        for (alias, stuck) in [(false, false), (true, false), (false, true)] {
            let mut ram = Ram {
                words: [0; 32],
                alias,
                stuck,
                highest: 0,
            };
            assert_eq!(test_scratch(&mut ram).is_ok(), !alias && !stuck);
            assert!(ram.highest < 32);
        }
    }
    #[test]
    fn empty_spans_fail_and_an_oversized_span_keeps_its_tail_untouched() {
        struct Span {
            data: std::vec::Vec<u32>,
        }
        impl ScratchWords for Span {
            fn words(&self) -> usize {
                self.data.len()
            }
            fn write(&mut self, index: usize, value: u32) {
                self.data[index] = value;
            }
            fn read(&mut self, index: usize) -> u32 {
                self.data[index]
            }
        }
        let mut empty = Span { data: std::vec![] };
        assert_eq!(test_scratch(&mut empty), Err(ScratchError::Empty));
        let mut large = Span {
            data: std::vec![0xdead_beef; TEST_WORDS + 3],
        };
        test_scratch(&mut large).unwrap();
        assert_eq!(&large.data[TEST_WORDS..], &[0xdead_beef; 3]);
    }
}
