use core::cell::RefCell;

use crate::controller_profiles::{match_scanned_profile, wait_for_profile_catalog_change};
use embassy_futures::select::{Either, select};
use embassy_usb_driver::{
    Direction as UsbDirection, EndpointAddress, EndpointInfo, EndpointType,
    host::{
        DeviceEvent, PipeError, UsbHostAllocator, UsbHostController, UsbPipe, pipe,
    },
};
use embassy_usb_host::{BusController, BusRoute, BusState, handler::EnumerationInfo};
use embassy_sync::blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex};
use pajoniiir_controller_usb::{
    ControllerUsbCapabilities, ControllerUsbDescriptorError, UsbMidiEvent,
    find_midi_streaming_interface, inspect_configuration,
};

const USB1_ENUM_CONFIG_BYTES: usize = 512;
const USB1_MIDI_FS_MAX_PACKET_BYTES: u16 = 64;

static USB1_BUS_STATE: BusState = BusState::new();
static USB1_CONTROLLER_STATE: CriticalSectionMutex<RefCell<Usb1ControllerState>> =
    CriticalSectionMutex::new(RefCell::new(Usb1ControllerState::Idle));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Usb1ControllerProbe {
    pub(crate) device_address: u8,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) capabilities: ControllerUsbCapabilities,
    pub(crate) midi_transport_ready: bool,
    pub(crate) profile_index: Option<u8>,
    pub(crate) profile_catalog_revision: u32,
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


#[derive(Debug)]
pub(crate) enum Usb1MidiTransportError {
    Descriptor(ControllerUsbDescriptorError),
    NoMidiInterface,
    NoInputEndpoint,
    InvalidMaxPacketSize(u16),
    NoInputPipe,
    NoOutputPipe,
    OutputUnavailable,
    Transfer(PipeError),
}

impl From<PipeError> for Usb1MidiTransportError {
    fn from(error: PipeError) -> Self {
        Self::Transfer(error)
    }
}

struct Usb1MidiTransport<'d, A>
where
    A: UsbHostAllocator<'d>,
{
    input: A::Pipe<pipe::Bulk, pipe::In>,
    output: Option<A::Pipe<pipe::Bulk, pipe::Out>>,
    interface_number: u8,
    _lifetime: core::marker::PhantomData<&'d ()>,
}

impl<'d, A> Usb1MidiTransport<'d, A>
where
    A: UsbHostAllocator<'d>,
{
    fn new(
        allocator: &A,
        config: &[u8],
        enumeration: &EnumerationInfo,
    ) -> Result<Self, Usb1MidiTransportError> {
        let midi = find_midi_streaming_interface(config)
            .map_err(Usb1MidiTransportError::Descriptor)?
            .ok_or(Usb1MidiTransportError::NoMidiInterface)?;
        let input = midi
            .bulk_in
            .ok_or(Usb1MidiTransportError::NoInputEndpoint)?;

        validate_fs_bulk_packet_size(input.max_packet_size)?;
        if let Some(output) = midi.bulk_out {
            validate_fs_bulk_packet_size(output.max_packet_size)?;
        }

        let input_info = EndpointInfo {
            addr: EndpointAddress::from_parts(
                usize::from(input.address & 0x0f),
                UsbDirection::In,
            ),
            ep_type: EndpointType::Bulk,
            max_packet_size: input.max_packet_size,
            interval_ms: 0,
        };
        let input = allocator
            .alloc_pipe::<pipe::Bulk, pipe::In>(
                enumeration.device_address,
                &input_info,
                enumeration.split(),
            )
            .map_err(|_| Usb1MidiTransportError::NoInputPipe)?;

        let output = match midi.bulk_out {
            Some(output) => {
                let output_info = EndpointInfo {
                    addr: EndpointAddress::from_parts(
                        usize::from(output.address & 0x0f),
                        UsbDirection::Out,
                    ),
                    ep_type: EndpointType::Bulk,
                    max_packet_size: output.max_packet_size,
                    interval_ms: 0,
                };
                Some(
                    allocator
                        .alloc_pipe::<pipe::Bulk, pipe::Out>(
                            enumeration.device_address,
                            &output_info,
                            enumeration.split(),
                        )
                        .map_err(|_| Usb1MidiTransportError::NoOutputPipe)?,
                )
            }
            None => None,
        };

        Ok(Self {
            input,
            output,
            interface_number: midi.interface_number,
            _lifetime: core::marker::PhantomData,
        })
    }

    const fn interface_number(&self) -> u8 {
        self.interface_number
    }

    const fn has_output(&self) -> bool {
        self.output.is_some()
    }

    async fn read_raw(&mut self, output: &mut [u8]) -> Result<usize, Usb1MidiTransportError> {
        self.input.request_in(output).await.map_err(Into::into)
    }

    async fn write_event(&mut self, event: UsbMidiEvent) -> Result<(), Usb1MidiTransportError> {
        let output = self
            .output
            .as_mut()
            .ok_or(Usb1MidiTransportError::OutputUnavailable)?;
        let packet = event.as_usb_packet();
        output
            .request_out(&packet, false)
            .await
            .map_err(Into::into)
    }
}

fn validate_fs_bulk_packet_size(max_packet_size: u16) -> Result<(), Usb1MidiTransportError> {
    if max_packet_size == 0 || max_packet_size > USB1_MIDI_FS_MAX_PACKET_BYTES {
        Err(Usb1MidiTransportError::InvalidMaxPacketSize(
            max_packet_size,
        ))
    } else {
        Ok(())
    }
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
                pending_speed = wait_for_device_change(&mut controller).await;
                continue;
            }
        };

        let midi_transport =
            Usb1MidiTransport::new(&bus, &config[..config_len], &info).ok();
        let midi_transport_ready = midi_transport.is_some();
        let profile_match =
            match_scanned_profile(info.device_desc.vendor_id, info.device_desc.product_id);
        let mut probe = Usb1ControllerProbe {
            device_address: info.device_address,
            vendor_id: info.device_desc.vendor_id,
            product_id: info.device_desc.product_id,
            capabilities,
            midi_transport_ready,
            profile_index: profile_match.index,
            profile_catalog_revision: profile_match.revision,
        };
        set_state(Usb1ControllerState::Ready(probe));

        pending_speed = wait_for_ready_change(&mut controller, &mut probe).await;
        drop(midi_transport);
        bus.free_address(info.device_address);
    }
}

async fn wait_for_device_change<'d, C>(
    controller: &mut BusController<'d, C>,
) -> Option<embassy_usb_driver::Speed>
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

async fn wait_for_ready_change<'d, C>(
    controller: &mut BusController<'d, C>,
    probe: &mut Usb1ControllerProbe,
) -> Option<embassy_usb_driver::Speed>
where
    C: UsbHostController<'d>,
{
    loop {
        match select(
            controller.wait_for_device_event(),
            wait_for_profile_catalog_change(),
        )
        .await
        {
            Either::First(DeviceEvent::Disconnected) => {
                set_state(Usb1ControllerState::Detached);
                return None;
            }
            Either::First(DeviceEvent::Overcurrent) => {
                set_state(Usb1ControllerState::Failed(
                    Usb1ControllerFailure::Overcurrent,
                ));
                return None;
            }
            Either::First(DeviceEvent::Connected(speed)) => {
                set_state(Usb1ControllerState::Detached);
                return Some(speed);
            }
            Either::First(_) => {}
            Either::Second(_revision) => {
                let profile_match = match_scanned_profile(probe.vendor_id, probe.product_id);
                probe.profile_index = profile_match.index;
                probe.profile_catalog_revision = profile_match.revision;
                set_state(Usb1ControllerState::Ready(*probe));
            }
        }
    }
}
