//! Candidate values from the owner-supplied JC1060 P4V3X bundle.
use crate::*;

pub const PROFILE: BoardProfile = BoardProfile {
    id: BoardId::GuitionJc1060P470cIwY,
    model: "JC1060P470C_I_W_Y",
    source_set: "jc1060-owner-p4v3x-2026-10-20",
    min_chip_revision: 302,
    flash_bytes: 16 * 1024 * 1024,
    psram_bytes: 32 * 1024 * 1024,
    i2c: I2cPins { sda: 7, scl: 8 },
    display: DisplayCandidate {
        width: 1024,
        height: 600,
        controller: PanelController::Jd9165,
        lanes: 2,
        phy_ldo_channel: 3,
        phy_voltage_mv: 2500,
        reset_gpio: 0,
        backlight_gpio: 23,
        touch: TouchController::Gt911,
        exact_panel_selected: false,
        requires_touch_firmware: false,
    },
    sdmmc: SdmmcPins {
        data: [39, 40, 41, 42],
        clock: 43,
        command: 44,
        power: SdPower::P4LdoChannel(4),
        card_detect: None,
    },
    audio: AudioCandidate {
        output_channels: 1,
        mclk_gpio: 13,
        bclk_gpio: 12,
        lrck_gpio: 10,
        dout_gpio: 9,
        din_gpio: 48,
        amplifier_enable_gpio: 11,
    },
    usb: UsbCandidate {
        hs_connector_documented: true,
        fs_connector_documented: true,
        fs_dp_dm_gpio: None,
        independent_vbus_control: false,
    },
};
