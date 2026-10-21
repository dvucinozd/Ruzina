//! First-boot observations never substitute documented capacities for readings.
use crate::BoardProfile;
use core::fmt::{self, Write};

pub struct BootReport<'a> {
    pub profile: &'a BoardProfile,
    pub chip_revision: u16,
    pub image_id: &'a str,
    pub observed_flash_bytes: Option<u32>,
    pub observed_psram_bytes: Option<u32>,
}

impl BootReport<'_> {
    pub fn write_to(&self, output: &mut impl Write) -> fmt::Result {
        writeln!(output, "M1_BOOT_DIAGNOSTIC_V1")?;
        writeln!(output, "board_profile={}", self.profile.model)?;
        writeln!(output, "image_id={}", self.image_id)?;
        writeln!(output, "chip_revision_detected={}", self.chip_revision)?;
        writeln!(
            output,
            "chip_minimum_required={}",
            self.profile.min_chip_revision
        )?;
        writeln!(
            output,
            "chip_policy_match={}",
            self.chip_revision >= self.profile.min_chip_revision
        )?;
        writeln!(
            output,
            "flash_bytes_documented={}",
            self.profile.flash_bytes
        )?;
        writeln!(
            output,
            "psram_bytes_documented={}",
            self.profile.psram_bytes
        )?;
        for (name, value) in [
            ("flash_bytes_observed", self.observed_flash_bytes),
            ("psram_bytes_observed", self.observed_psram_bytes),
        ] {
            match value {
                Some(bytes) => writeln!(output, "{name}={bytes}")?,
                None => writeln!(output, "{name}=NOT_MEASURED")?,
            }
        }
        writeln!(output, "readiness=NOT_QUALIFIED")
    }
}

pub struct ReportBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Default for ReportBuffer<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}
impl<const N: usize> ReportBuffer<N> {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
impl<const N: usize> Write for ReportBuffer<N> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self
            .len
            .checked_add(value.len())
            .filter(|end| *end <= N)
            .ok_or(fmt::Error)?;
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmeasured_memory_is_not_reported_as_a_detected_capacity_or_acceptance() {
        let report = BootReport {
            profile: &crate::jc1060::PROFILE,
            chip_revision: 302,
            image_id: "review-test-image",
            observed_flash_bytes: None,
            observed_psram_bytes: None,
        };
        let mut buffer = ReportBuffer::<512>::default();
        report.write_to(&mut buffer).unwrap();
        let text = core::str::from_utf8(buffer.bytes()).unwrap();
        assert!(text.contains("chip_revision_detected=302\n"));
        assert!(text.contains("flash_bytes_documented=16777216\n"));
        assert!(text.contains("flash_bytes_observed=NOT_MEASURED\n"));
        assert!(text.contains("psram_bytes_observed=NOT_MEASURED\n"));
        assert!(text.contains("readiness=NOT_QUALIFIED\n"));
    }

    #[test]
    fn actual_mismatches_and_buffer_exhaustion_remain_explicit() {
        let report = BootReport {
            profile: &crate::jc8012::PROFILE,
            chip_revision: 301,
            image_id: "test",
            observed_flash_bytes: Some(8 * 1024 * 1024),
            observed_psram_bytes: Some(0),
        };
        let mut buffer = ReportBuffer::<512>::default();
        report.write_to(&mut buffer).unwrap();
        let text = core::str::from_utf8(buffer.bytes()).unwrap();
        assert!(text.contains("chip_policy_match=false\n"));
        assert!(text.contains("flash_bytes_observed=8388608\n"));
        assert!(text.contains("psram_bytes_observed=0\n"));
        assert!(report.write_to(&mut ReportBuffer::<16>::default()).is_err());
    }
}
