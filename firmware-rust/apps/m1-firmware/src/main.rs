#![no_std]
#![no_main]
#![deny(clippy::mem_forget)]

#[cfg(feature = "p4-psram-storage")]
extern crate alloc;

use embassy_executor::Spawner;
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};

#[allow(dead_code)]
mod usb0_library;
#[allow(dead_code)]
mod usb0_msc;
#[allow(dead_code)]
mod usb1_controller;

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_owner_task(usb_hs: esp_hal::peripherals::USB_HS<'static>) {
    usb0_msc::run_root_msc_owner(usb_hs).await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_media_task() {
    usb0_msc::run_usb0_media_worker().await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_filesystem_task() {
    usb0_msc::run_usb0_filesystem_worker().await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_library_task() {
    usb0_library::run_usb0_library_worker().await
}

#[cfg(feature = "usb1-controller-bringup")]
#[embassy_executor::task]
async fn usb1_owner_task(
    usb_fs: esp_hal::peripherals::USB_FS<'static>,
    usb_dp: esp_hal::peripherals::GPIO27<'static>,
    usb_dm: esp_hal::peripherals::GPIO26<'static>,
) {
    usb1_controller::run_root_controller_owner(usb_fs, usb_dp, usb_dm).await
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    esp_hal::system::software_reset()
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    #[cfg(feature = "p4-psram-storage")]
    {
        // Compile-verified placement boundary only. The allocator maps ESP32-P4
        // PSRAM as external heap storage, but DMA/cache behavior and the custom
        // M1 board's physical PSRAM remain HARDWARE_PENDING until EVT.
        esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);
    }

    // esp-rtos owns the Embassy scheduler/time driver. Start it before any
    // spawned task or timer is allowed to run.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // Compile-time anchors: the production target must keep the host-tested
    // product crates no_std-compatible on the real ESP32-P4 architecture.
    let _ = core::mem::size_of::<pajoniiir_controller_session::ControllerSession>();
    let _ = core::mem::size_of::<pajoniiir_controller_usb::ControllerUsbCapabilities>();
    let _ = core::mem::size_of::<pajoniiir_deck::DeckProductState>();
    let _ = core::mem::size_of::<pajoniiir_hot_cues::HotCueBank>();
    let _ = core::mem::size_of::<pajoniiir_mixer::MixerState>();
    let _ = core::mem::size_of::<pajoniiir_media_catalog::CatalogTrack>();
    let _ = core::mem::size_of::<pajoniiir_media_usb_msc::UsbMscCapacity>();
    let _ = core::mem::size_of::<pajoniiir_rekordbox::PdbHeader>();
    let _ = core::mem::size_of::<pajoniiir_ui_model::UiSnapshot>();
    let _ = core::mem::size_of::<pajoniiir_waveform::WaveformColumn>();

    #[cfg(feature = "usb0-hardware-bringup")]
    {
        spawner.spawn(
            usb0_owner_task(peripherals.USB_HS)
                .expect("USB0 owner task slot must be available"),
        );
        spawner.spawn(
            usb0_media_task().expect("USB0 media task slot must be available"),
        );
        spawner.spawn(
            usb0_filesystem_task().expect("USB0 filesystem task slot must be available"),
        );
        spawner.spawn(
            usb0_library_task().expect("USB0 library task slot must be available"),
        );
    }

    #[cfg(feature = "usb1-controller-bringup")]
    {
        spawner.spawn(
            usb1_owner_task(peripherals.USB_FS, peripherals.GPIO27, peripherals.GPIO26)
                .expect("USB1 owner task slot must be available"),
        );
    }

    #[cfg(not(any(
        feature = "usb0-hardware-bringup",
        feature = "usb1-controller-bringup"
    )))]
    let _ = spawner;

    // The default image intentionally does not touch either USB controller.
    // Feature gates transfer USB_HS and/or USB_FS ownership into persistent
    // Embassy owner tasks without weakening the hardware-verification boundary.
    match core::future::pending::<core::convert::Infallible>().await {}
}
