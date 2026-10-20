#![no_std]
#![no_main]
#![deny(clippy::mem_forget)]

#[cfg(feature = "p4-psram-storage")]
extern crate alloc;

use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;

#[cfg(feature = "render-graph-bringup")]
mod audio_render_owner;
#[allow(dead_code)]
mod controller_profiles;
#[cfg(feature = "wav-audio-source-bringup")]
mod deck_audio_source;
#[allow(dead_code)]
mod deck_effect_owners;
#[cfg(feature = "hotcue-flash-storage")]
mod hotcue_flash;
#[cfg(feature = "library-deck-load")]
mod library_deck_owner;
#[allow(dead_code)]
mod product_runtime;
#[allow(dead_code)]
mod sdmmc_profile;
#[allow(dead_code)]
mod usb0_library;
#[allow(dead_code)]
mod usb0_msc;
#[allow(dead_code)]
mod usb1_controller;

mod platform;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    esp_hal::system::software_reset()
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    platform::start(spawner, peripherals);
    match core::future::pending::<core::convert::Infallible>().await {}
}
