//! Documented board candidates, without HAL ownership or implicit activation.
//! Values describe the audited vendor bundles, not acceptance of arriving units.
#![no_std]

pub mod diagnostics;
pub mod jc1060;
pub mod jc8012;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardId {
    GuitionJc1060P470cIwY,
    GuitionJc8012P4a1cIwY,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct I2cPins {
    pub sda: u8,
    pub scl: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdmmcPins {
    pub data: [u8; 4],
    pub clock: u8,
    pub command: u8,
    pub power: SdPower,
    /// No card-detect connection was established by the audited sources.
    pub card_detect: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdPower {
    P4LdoChannel(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchController {
    Gt911,
    Gsl3680,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelController {
    Jd9165,
    Jd9365,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayCandidate {
    pub width: u16,
    pub height: u16,
    pub controller: PanelController,
    pub lanes: u8,
    pub phy_ldo_channel: u8,
    pub phy_voltage_mv: u16,
    pub reset_gpio: u8,
    pub backlight_gpio: u8,
    pub touch: TouchController,
    /// Both bundles contain differing panel tables; no table is selected here.
    pub exact_panel_selected: bool,
    /// JC8012 requires a controller payload; this crate never bundles it.
    pub requires_touch_firmware: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioCandidate {
    /// ES8311 + differential speaker amplifier, not a stereo MAIN DAC.
    pub output_channels: u8,
    pub mclk_gpio: u8,
    pub bclk_gpio: u8,
    pub lrck_gpio: u8,
    pub dout_gpio: u8,
    pub din_gpio: u8,
    pub amplifier_enable_gpio: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbCandidate {
    pub hs_connector_documented: bool,
    pub fs_connector_documented: bool,
    /// Schematic/BSP GPIO labels do not establish a v3.2 FS mapping.
    pub fs_dp_dm_gpio: Option<[u8; 2]>,
    /// The audited connectors share USB5V_IN; no host-power policy is qualified.
    pub independent_vbus_control: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardProfile {
    pub id: BoardId,
    pub model: &'static str,
    pub source_set: &'static str,
    pub min_chip_revision: u16,
    pub flash_bytes: u32,
    pub psram_bytes: u32,
    pub i2c: I2cPins,
    pub display: DisplayCandidate,
    pub sdmmc: SdmmcPins,
    pub audio: AudioCandidate,
    pub usb: UsbCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnitIdentity {
    pub chip_revision: u16,
    pub flash_bytes: u32,
    pub psram_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityMismatch {
    SiliconRevision,
    FlashCapacity,
    PsramCapacity,
}

impl BoardProfile {
    /// Check detected identity against the documented candidate. Success does
    /// not authorize peripheral activation or qualify the board's wiring.
    pub fn check_unit(&self, unit: UnitIdentity) -> Result<(), IdentityMismatch> {
        if unit.chip_revision < self.min_chip_revision {
            return Err(IdentityMismatch::SiliconRevision);
        }
        if unit.flash_bytes != self.flash_bytes {
            return Err(IdentityMismatch::FlashCapacity);
        }
        if unit.psram_bytes != self.psram_bytes {
            return Err(IdentityMismatch::PsramCapacity);
        }
        Ok(())
    }
}

pub const fn profile(id: BoardId) -> &'static BoardProfile {
    match id {
        BoardId::GuitionJc1060P470cIwY => &jc1060::PROFILE,
        BoardId::GuitionJc8012P4a1cIwY => &jc8012::PROFILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_identity_rejects_old_silicon_and_unreviewed_memory_variants() {
        for id in [
            BoardId::GuitionJc1060P470cIwY,
            BoardId::GuitionJc8012P4a1cIwY,
        ] {
            let board = profile(id);
            let unit = UnitIdentity {
                chip_revision: 302,
                flash_bytes: 16 * 1024 * 1024,
                psram_bytes: 32 * 1024 * 1024,
            };
            assert_eq!(board.check_unit(unit), Ok(()));
            assert_eq!(
                board.check_unit(UnitIdentity {
                    chip_revision: 301,
                    ..unit
                }),
                Err(IdentityMismatch::SiliconRevision)
            );
            assert_eq!(
                board.check_unit(UnitIdentity {
                    flash_bytes: 8 * 1024 * 1024,
                    ..unit
                }),
                Err(IdentityMismatch::FlashCapacity)
            );
            assert_eq!(
                board.check_unit(UnitIdentity {
                    flash_bytes: 32 * 1024 * 1024,
                    ..unit
                }),
                Err(IdentityMismatch::FlashCapacity)
            );
            assert_eq!(
                board.check_unit(UnitIdentity {
                    psram_bytes: 0,
                    ..unit
                }),
                Err(IdentityMismatch::PsramCapacity)
            );
        }
    }
}
