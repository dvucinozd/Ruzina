//! JC8012 BSP boundary; no panel/touch payload is included or activated.
use embassy_executor::Spawner;
use esp_hal::peripherals::Peripherals;
use ruzina_board_profile::{BoardProfile, jc8012};

pub(super) const PROFILE: &BoardProfile = &jc8012::PROFILE;

pub(super) fn start(spawner: Spawner, peripherals: Peripherals) {
    super::start_guition(spawner, peripherals, PROFILE);
}
