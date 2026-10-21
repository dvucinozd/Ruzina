//! Opt-in service transport only. Confirm the unit's service connector before
//! use: a USB-UART bridge is not this native USB Serial/JTAG peripheral.
use core::fmt::Write;
use embassy_time::{Duration, Timer};
use esp_hal::{peripherals::USB_DEVICE, usb::usb_serial_jtag::UsbSerialJtag};
use pajoniiir_board_profile::{
    BoardProfile,
    diagnostics::{BootReport, ReportBuffer},
};
use pajoniiir_service::{Environment, Identity, RuntimeStatus, Service};

#[embassy_executor::task]
pub(super) async fn report_task(usb: USB_DEVICE<'static>, profile: &'static BoardProfile) {
    let mut serial = UsbSerialJtag::new(usb);
    let mut buffer = ReportBuffer::<2048>::default();
    let report = BootReport {
        profile,
        chip_revision: esp_hal::efuse::chip_revision().combined(),
        image_id: option_env!("M1_IMAGE_ID").unwrap_or("UNSPECIFIED"),
        observed_flash_bytes: None,
        observed_psram_bytes: None,
    };
    if report.write_to(&mut buffer).is_err()
        || writeln!(buffer, "reset_reason={:?}", esp_hal::system::reset_reason()).is_err()
    {
        // No truncated success record can escape as a valid boot report.
        return;
    }
    // One immutable boot sample, repeatedly emitted below. Repeated emission is
    // not a heartbeat and must not refresh this observation timestamp.
    let mut service = Service::new(Identity {
        environment: Environment::DeviceBootSnapshot,
        board: profile.id,
        image_id: option_env!("M1_IMAGE_ID"),
        chip_revision: Some(report.chip_revision),
        flash_bytes: report.observed_flash_bytes,
        psram_bytes: report.observed_psram_bytes,
        reset_code: esp_hal::system::reset_reason().map(|reason| reason as u32),
    });
    service.advance(embassy_time::Instant::now().as_millis());
    if service
        .snapshot(RuntimeStatus::BOOT_ONLY)
        .write_to(&mut buffer)
        .is_err()
    {
        return;
    }
    let mut offset = 0;
    loop {
        // Never use fmt::Write on USB Serial/JTAG: its blocking flush can wait
        // indefinitely without a host. Each scheduler turn attempts <=64 bytes.
        for _ in 0..64 {
            let Some(byte) = buffer.bytes().get(offset) else {
                break;
            };
            if serial.write_byte_nb(*byte).is_err() {
                break;
            }
            offset += 1;
        }
        let _ = serial.flush_tx_nb();
        if offset == buffer.bytes().len() {
            offset = 0;
            Timer::after(Duration::from_secs(2)).await;
        } else {
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}
