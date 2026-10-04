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

    #[cfg(not(feature = "usb0-hardware-bringup"))]
    let _ = spawner;

    // The default image intentionally does not touch USB0 yet. Enabling
    // usb0-hardware-bringup transfers USB_HS ownership into the persistent
    // Embassy owner task above.
    match core::future::pending::<core::convert::Infallible>().await {}
}
