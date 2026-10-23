//! JC1060 BSP boundary; peripheral drivers await exact-unit bring-up.
use embassy_executor::Spawner;
use esp_hal::peripherals::Peripherals;
use ruzina_board_profile::{BoardProfile, jc1060};

pub(super) const PROFILE: &BoardProfile = &jc1060::PROFILE;

pub(super) fn start(spawner: Spawner, peripherals: Peripherals) {
    super::start_guition(spawner, peripherals, PROFILE);
}
