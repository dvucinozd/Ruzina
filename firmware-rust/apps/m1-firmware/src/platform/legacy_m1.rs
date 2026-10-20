//! Historical custom-M1 wiring. Never used by a Guition selector.
use embassy_executor::Spawner;
use esp_hal::timer::timg::TimerGroup;

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_owner_task(usb_hs: esp_hal::peripherals::USB_HS<'static>) {
    crate::usb0_msc::run_root_msc_owner(usb_hs).await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_media_task() {
    crate::usb0_msc::run_usb0_media_worker().await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_filesystem_task() {
    crate::usb0_msc::run_usb0_filesystem_worker().await
}

#[cfg(feature = "usb0-hardware-bringup")]
#[embassy_executor::task]
async fn usb0_library_task() {
    crate::usb0_library::run_usb0_library_worker().await
}

#[cfg(feature = "library-deck-load")]
#[embassy_executor::task]
async fn library_deck_owner_task() {
    crate::library_deck_owner::run_library_deck_owner().await
}

#[cfg(feature = "wav-audio-source-bringup")]
#[embassy_executor::task]
async fn deck_a_audio_source_owner_task() {
    crate::deck_audio_source::run_deck_a_audio_source_owner().await
}

#[cfg(feature = "wav-audio-source-bringup")]
#[embassy_executor::task]
async fn deck_b_audio_source_owner_task() {
    crate::deck_audio_source::run_deck_b_audio_source_owner().await
}

#[cfg(feature = "render-graph-bringup")]
#[embassy_executor::task]
async fn audio_render_owner_task() {
    crate::audio_render_owner::run_audio_render_owner().await
}

#[cfg(feature = "hotcue-flash-storage")]
#[embassy_executor::task]
async fn deck_hot_cue_persistence_owner_task(flash: esp_hal::peripherals::FLASH<'static>) {
    crate::deck_effect_owners::run_persistence_owner(flash).await
}

#[cfg(feature = "usb1-controller-bringup")]
#[embassy_executor::task]
async fn deck_transport_owner_task() {
    crate::deck_effect_owners::run_transport_owner().await
}

#[cfg(feature = "usb1-controller-bringup")]
#[embassy_executor::task]
async fn deck_audio_dsp_owner_task() {
    crate::deck_effect_owners::run_audio_dsp_owner().await
}

#[cfg(feature = "usb1-controller-bringup")]
#[embassy_executor::task]
async fn usb1_owner_task(
    usb_fs: esp_hal::peripherals::USB_FS<'static>,
    usb_dp: esp_hal::peripherals::GPIO27<'static>,
    usb_dm: esp_hal::peripherals::GPIO26<'static>,
) {
    crate::usb1_controller::run_root_controller_owner(usb_fs, usb_dp, usb_dm).await
}

#[cfg(feature = "sdmmc-profile-storage-bringup")]
#[embassy_executor::task]
async fn sdmmc_profile_owner_task(
    sdhost: esp_hal::peripherals::SDHOST<'static>,
    d0: esp_hal::peripherals::GPIO39<'static>,
    d1: esp_hal::peripherals::GPIO40<'static>,
    d2: esp_hal::peripherals::GPIO41<'static>,
    d3: esp_hal::peripherals::GPIO42<'static>,
    clk: esp_hal::peripherals::GPIO43<'static>,
    cmd: esp_hal::peripherals::GPIO44<'static>,
    power_enable: esp_hal::peripherals::GPIO45<'static>,
    card_detect: esp_hal::peripherals::GPIO46<'static>,
) {
    crate::sdmmc_profile::run_profile_storage_owner(
        sdhost,
        d0,
        d1,
        d2,
        d3,
        clk,
        cmd,
        power_enable,
        card_detect,
    )
    .await
}

pub(super) fn start(spawner: Spawner, peripherals: esp_hal::peripherals::Peripherals) {
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
            usb0_owner_task(peripherals.USB_HS).expect("USB0 owner task slot must be available"),
        );
        spawner.spawn(usb0_media_task().expect("USB0 media task slot must be available"));
        spawner.spawn(usb0_filesystem_task().expect("USB0 filesystem task slot must be available"));
        spawner.spawn(usb0_library_task().expect("USB0 library task slot must be available"));
    }

    #[cfg(feature = "library-deck-load")]
    {
        spawner.spawn(
            library_deck_owner_task().expect("Library deck-load owner task slot must be available"),
        );
    }

    #[cfg(feature = "wav-audio-source-bringup")]
    {
        spawner.spawn(
            deck_a_audio_source_owner_task()
                .expect("Deck A audio-source owner task slot must be available"),
        );
        spawner.spawn(
            deck_b_audio_source_owner_task()
                .expect("Deck B audio-source owner task slot must be available"),
        );
    }

    #[cfg(feature = "render-graph-bringup")]
    {
        spawner.spawn(
            audio_render_owner_task().expect("audio render owner task slot must be available"),
        );
    }

    #[cfg(feature = "hotcue-flash-storage")]
    {
        spawner.spawn(
            deck_hot_cue_persistence_owner_task(peripherals.FLASH)
                .expect("Hot Cue persistence owner task slot must be available"),
        );
    }

    #[cfg(feature = "usb1-controller-bringup")]
    {
        spawner.spawn(
            deck_transport_owner_task().expect("deck transport owner task slot must be available"),
        );
        spawner.spawn(
            deck_audio_dsp_owner_task().expect("deck audio DSP owner task slot must be available"),
        );
        spawner.spawn(
            usb1_owner_task(peripherals.USB_FS, peripherals.GPIO27, peripherals.GPIO26)
                .expect("USB1 owner task slot must be available"),
        );
    }

    #[cfg(feature = "sdmmc-profile-storage-bringup")]
    {
        spawner.spawn(
            sdmmc_profile_owner_task(
                peripherals.SDHOST,
                peripherals.GPIO39,
                peripherals.GPIO40,
                peripherals.GPIO41,
                peripherals.GPIO42,
                peripherals.GPIO43,
                peripherals.GPIO44,
                peripherals.GPIO45,
                peripherals.GPIO46,
            )
            .expect("SDMMC profile owner task slot must be available"),
        );
    }

    #[cfg(not(any(
        feature = "usb0-hardware-bringup",
        feature = "usb1-controller-bringup",
        feature = "hotcue-flash-storage",
        feature = "sdmmc-profile-storage-bringup"
    )))]
    let _ = spawner;
}
