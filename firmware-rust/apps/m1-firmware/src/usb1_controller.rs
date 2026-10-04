use core::cell::RefCell;

use embassy_usb_driver::host::{DeviceEvent, UsbHostController};
use embassy_usb_host::{BusController, BusRoute, BusState};
use embassy_sync::blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex};
use pajoniiir_controller_usb::{ControllerUsbCapabilities, inspect_configuration};

const USB1_ENUM_CONFIG_BYTES: usize = 512;

static USB1_BUS_STATE: BusState = BusState::new();
static USB1_CONTROLLER_STATE: CriticalSectionMutex<RefCell<Usb1ControllerState>> =
    CriticalSectionMutex::new(RefCell::new(Usb1ControllerState::Idle));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb1ControllerProbe {
    pub(crate) device_address: u8,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) capabilities: ControllerUsbCapabilities,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb1ControllerFailure {
    Enumeration,
    MalformedConfiguration,
    Overcurrent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb1ControllerState {
    Idle,
    Enumerating,
    Ready(Usb1ControllerProbe),
    Failed(Usb1ControllerFailure),
    Detached,
}

fn set_state(state: Usb1ControllerState) {
    USB1_CONTROLLER_STATE.lock(|cell| *cell.borrow_mut() = state);
}

pub(crate) fn controller_state() -> Usb1ControllerState {
    USB1_CONTROLLER_STATE.lock(|cell| *cell.borrow())
}

/// Persistent owner for the M1 USB1 Full-Speed controller port.
///
/// This compile boundary stops after enumeration and capability classification.
/// MIDI pipes and UAC streaming are layered on later.
pub(crate) async fn run_root_controller_owner(
    usb_fs: esp_hal::peripherals::USB_FS<'static>,
    usb_dp: esp_hal::peripherals::GPIO27<'static>,
    usb_dm: esp_hal::peripherals::GPIO26<'static>,
) -> ! {
    let usb = esp_hal::usb::otg::Usb::new_fs(usb_fs, usb_dp, usb_dm);
    let driver = esp_hal::usb::otg::embassy_usb_host::Driver::new(usb);
    let (mut controller, bus) = embassy_usb_host::bus(driver, &USB1_BUS_STATE);
    let mut pending_speed = None;

    loop {
        let speed = match pending_speed.take() {
            Some(speed) => speed,
            None => loop {
                match controller.wait_for_device_event().await {
                    DeviceEvent::Connected(speed) => break speed,
                    DeviceEvent::Overcurrent => {
                        set_state(Usb1ControllerState::Failed(
                            Usb1ControllerFailure::Overcurrent,
                        ));
                    }
                    DeviceEvent::Disconnected => set_state(Usb1ControllerState::Detached),
                    _ => {}
                }
            },
        };

        set_state(Usb1ControllerState::Enumerating);
        let mut config = [0u8; USB1_ENUM_CONFIG_BYTES];
        let (info, config_len) = match bus.enumerate(BusRoute::Direct(speed), &mut config).await {
            Ok(enumerated) => enumerated,
            Err(_) => {
                set_state(Usb1ControllerState::Failed(
                    Usb1ControllerFailure::Enumeration,
                ));
                continue;
            }
        };

        let capabilities = match inspect_configuration(&config[..config_len]) {
            Ok(capabilities) => capabilities,
            Err(_) => {
                set_state(Usb1ControllerState::Failed(
                    Usb1ControllerFailure::MalformedConfiguration,
                ));
                bus.free_address(info.device_address);
                pending_speed = wait_for_change(&mut controller).await;
                continue;
            }
        };

        set_state(Usb1ControllerState::Ready(Usb1ControllerProbe {
            device_address: info.device_address,
            vendor_id: info.device_desc.vendor_id,
            product_id: info.device_desc.product_id,
            capabilities,
        }));

        pending_speed = wait_for_change(&mut controller).await;
        bus.free_address(info.device_address);
    }
}

async fn wait_for_change<'d, C>(controller: &mut BusController<'d, C>) -> Option<embassy_usb_driver::Speed>
where
    C: UsbHostController<'d>,
{
    loop {
        match controller.wait_for_device_event().await {
            DeviceEvent::Disconnected => {
                set_state(Usb1ControllerState::Detached);
                return None;
            }
            DeviceEvent::Overcurrent => {
                set_state(Usb1ControllerState::Failed(
                    Usb1ControllerFailure::Overcurrent,
                ));
                return None;
            }
            DeviceEvent::Connected(speed) => {
                set_state(Usb1ControllerState::Detached);
                return Some(speed);
            }
            _ => {}
        }
    }
}
