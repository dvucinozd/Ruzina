//! Diagnostic-only, exclusive boot-time ownership; no allocator registration.
//! SPI RDID semantics follow pinned esp-storage 0.10.0 hardware.rs. Unlike its
//! constructor this probe has a finite polling budget and retains the raw ID.
#[cfg(feature = "guition-flash-probe")]
use pajoniiir_board_profile::memory::FlashProbe;
#[cfg(feature = "guition-psram-test")]
use pajoniiir_board_profile::memory::{
    PsramProbe, ScratchError, ScratchWords, TEST_WORDS, test_scratch,
};

#[cfg(feature = "guition-flash-probe")]
pub(super) fn flash_probe(_flash: esp_hal::peripherals::FLASH<'static>) -> FlashProbe {
    // Called before starting the RTOS or second core, with exclusive FLASH
    // ownership. Never changes flash status, mode, protection or storage bytes.
    critical_section::with(|_| read_id())
}

#[cfg(feature = "guition-flash-probe")]
#[esp_hal::ram]
fn read_id() -> FlashProbe {
    let spi = esp_hal::peripherals::SPI1::regs();
    if spi.cmd().read().bits() != 0 {
        return FlashProbe::Busy;
    }
    spi.cmd().write(|w| w.flash_rdid().set_bit());
    for _ in 0..100_000 {
        if spi.cmd().read().flash_rdid().bit_is_clear() {
            return FlashProbe::ReadId(spi.w(0).read().buf().bits() & 0x00ff_ffff);
        }
    }
    // Do not guess an abort/reset sequence for an unqualified controller.
    FlashProbe::Timeout
}

#[cfg(feature = "guition-psram-test")]
pub(super) fn psram_test(
    peripheral: esp_hal::peripherals::PSRAM<'static>,
    expected: u32,
) -> PsramProbe {
    use esp_hal::psram::{Psram, PsramConfig, PsramSize, PsramTimingParams};
    // Explicit 80 MHz instead of the HAL's 250 MHz default. This is an opt-in
    // diagnostic configuration, not qualification of the documented 200 MHz.
    let psram = Psram::new(
        peripheral,
        PsramConfig {
            size: PsramSize::AutoDetect,
            timing: PsramTimingParams::MHZ_80,
            ..Default::default()
        },
    );
    let (base, bytes) = psram.raw_parts();
    if base.is_null()
        || !(base as usize).is_multiple_of(4)
        || bytes < TEST_WORDS * 4
        || bytes > u32::MAX as usize
        || (base as usize).checked_add(bytes).is_none()
    {
        return PsramProbe::InvalidMapping;
    }
    let mapped_bytes = bytes as u32;
    if mapped_bytes != expected {
        return PsramProbe::CapacityMismatch { mapped_bytes };
    }
    // SAFETY: PSRAM was initialized by this sole owner, before either core's
    // application tasks/allocator/DMA. Only the first 64 KiB is reserved and
    // touched here. The Psram capability remains alive for all raw accesses.
    let mut scratch = VolatileScratch {
        base: base.cast::<u32>(),
    };
    let result = test_scratch(&mut scratch);
    // Scratch contents are intentionally disposable and never published as PCM
    // or heap storage. Cache maintenance/DMA/full-array qualification is absent.
    match result {
        Ok(()) => PsramProbe::ScratchPassed { mapped_bytes },
        Err(ScratchError::Mismatch(mismatch)) => PsramProbe::ScratchFailed {
            mapped_bytes,
            mismatch,
        },
        Err(ScratchError::Empty) => PsramProbe::InvalidMapping,
    }
}

#[cfg(feature = "guition-psram-test")]
struct VolatileScratch {
    base: *mut u32,
}
#[cfg(feature = "guition-psram-test")]
impl ScratchWords for VolatileScratch {
    fn words(&self) -> usize {
        TEST_WORDS
    }
    fn write(&mut self, index: usize, value: u32) {
        assert!(index < TEST_WORDS);
        // SAFETY: private type constructed only for the exclusive checked span.
        unsafe { self.base.add(index).write_volatile(value) };
    }
    fn read(&mut self, index: usize) -> u32 {
        assert!(index < TEST_WORDS);
        // SAFETY: the complete span was written before each read pass.
        unsafe { self.base.add(index).read_volatile() }
    }
}
