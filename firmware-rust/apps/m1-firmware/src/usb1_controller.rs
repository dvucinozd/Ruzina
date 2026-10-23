use core::cell::RefCell;

#[cfg(feature = "render-graph-bringup")]
use crate::audio_render_owner::publish_product_render_control;
use crate::controller_profiles::{
    ActiveProfileLedError, ActiveProfileMidiError, active_channel_filter_always,
    bind_controller_descriptor,
    dispatch_session_deliveries, finish_profile_initialization, map_active_led_update,
    match_scanned_profile, on_controller_disconnect, pending_profile_initialization,
    process_active_midi_transfer, profile_initialization_is_current, request_profile_activation,
    wait_for_profile_activation_change, wait_for_profile_catalog_change,
};
use crate::deck_effect_owners::{dispatch_product_effects, wait_for_effect_owner_progress};
#[cfg(feature = "library-deck-load")]
use crate::library_deck_owner::{dispatch_library_deck, wait_for_library_deck_progress};
use crate::product_runtime::ProductRuntime;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_usb_driver::{
    Direction as UsbDirection, EndpointAddress, EndpointInfo, EndpointType,
    host::{DeviceEvent, PipeError, UsbHostAllocator, UsbHostController, UsbPipe, pipe},
};
use embassy_usb_host::{BusController, BusRoute, BusState, handler::EnumerationInfo};
use pajoniiir_controller_profile_storage::{MAX_INIT_SYSEX, ProfileActivationState};
use pajoniiir_controller_session::{ControllerSession, UsbMidiBatchReport};
use pajoniiir_controller_usb::{
    ControllerUsbCapabilities, ControllerUsbDescriptorError, UsbMidiEvent, UsbMidiSysExPackets,
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
    pub(crate) led_transport_ready: bool,
    pub(crate) controller_epoch: u32,
    pub(crate) profile_index: Option<u8>,
    pub(crate) profile_catalog_revision: u32,
    pub(crate) profile_activation_state: ProfileActivationState,
    pub(crate) midi_ingest: UsbMidiBatchReport,
    pub(crate) product_pending_deck_effects: u8,
    pub(crate) product_pending_library_effects: u8,
    pub(crate) product_backpressured: bool,
    pub(crate) led_updates_sent: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Usb1ControllerFailure {
    Enumeration,
    MalformedConfiguration,
    MalformedMidiTransfer,
    ActiveProfile,
    MidiTransport,
    LedOutput,
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
    let terminal_changed = USB1_CONTROLLER_STATE.lock(|cell| {
        let mut previous = cell.borrow_mut();
        let changed = *previous != state
            && matches!(
                state,
                Usb1ControllerState::Detached | Usb1ControllerState::Failed(_)
            );
        *previous = state;
        changed
    });
    let _ = terminal_changed;
    #[cfg(feature = "runtime-health")]
    if terminal_changed {
        crate::runtime_health::restart_control();
    }
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
            addr: EndpointAddress::from_parts(usize::from(input.address & 0x0f), UsbDirection::In),
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
        output.request_out(&packet, false).await.map_err(Into::into)
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

/// Controller owner accepting an explicitly supplied, board-qualified USB instance.
///
/// This compile boundary owns enumeration, controller-profile activation and
/// USB-MIDI Bulk-IN ingestion through the semantic controller session plus
/// profile-driven Bulk-OUT LED feedback. UAC remains a later gate.
pub(crate) async fn run_root_controller_owner(usb: esp_hal::usb::otg::Usb<'static>) -> ! {
    let driver = esp_hal::usb::otg::embassy_usb_host::Driver::new(usb);
    let (mut controller, bus) = embassy_usb_host::bus(driver, &USB1_BUS_STATE);
    let mut pending_speed = None;
    let mut controller_epoch = 0u32;
    let mut session = ControllerSession::new();
    let mut product = ProductRuntime::new();
    #[cfg(feature = "runtime-health")]
    crate::runtime_health::restart_control();

    loop {
        let speed = match pending_speed.take() {
            Some(speed) => speed,
            None => loop {
                #[cfg(feature = "runtime-health")]
                crate::runtime_health::publish_control(&product, &session, false);
                #[cfg(all(feature = "apta-usb0-bringup", feature = "runtime-health"))]
                let Some(event) = crate::usb0_analysis::product_wait(
                    &mut product,
                    crate::analysis_io::owner_tick(
                        controller.wait_for_device_event(),
                        embassy_time::Timer::after_millis(pajoniiir_service::owners::OWNER_TICK_MS),
                    ),
                )
                .await
                else {
                    continue;
                };
                #[cfg(all(feature = "apta-usb0-bringup", not(feature = "runtime-health")))]
                let event = crate::usb0_analysis::product_wait(
                    &mut product,
                    controller.wait_for_device_event(),
                )
                .await;
                #[cfg(all(feature = "runtime-health", not(feature = "apta-usb0-bringup")))]
                let event = match select(
                    controller.wait_for_device_event(),
                    embassy_time::Timer::after_millis(pajoniiir_service::owners::OWNER_TICK_MS),
                )
                .await
                {
                    Either::First(event) => event,
                    Either::Second(()) => continue,
                };
                #[cfg(not(any(feature = "runtime-health", feature = "apta-usb0-bringup")))]
                let event = controller.wait_for_device_event().await;
                match event {
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
                pending_speed =
                    wait_for_device_change(&mut controller, &session, &mut product).await;
                continue;
            }
        };

        let mut midi_transport = Usb1MidiTransport::new(&bus, &config[..config_len], &info).ok();
        let midi_transport_ready = midi_transport.is_some();
        let led_transport_ready = midi_transport
            .as_ref()
            .is_some_and(|transport| transport.has_output());
        controller_epoch = controller_epoch.wrapping_add(1);
        if controller_epoch == 0 {
            controller_epoch = 1;
        }
        let profile_match = bind_controller_descriptor(
            info.device_desc.vendor_id,
            info.device_desc.product_id,
            capability_flags(capabilities),
            controller_epoch,
        );
        let mut probe = Usb1ControllerProbe {
            device_address: info.device_address,
            vendor_id: info.device_desc.vendor_id,
            product_id: info.device_desc.product_id,
            capabilities,
            midi_transport_ready,
            led_transport_ready,
            controller_epoch,
            profile_index: profile_match.index,
            profile_catalog_revision: profile_match.revision,
            profile_activation_state: profile_match.activation_state,
            midi_ingest: UsbMidiBatchReport::default(),
            product_pending_deck_effects: 0,
            product_pending_library_effects: 0,
            product_backpressured: false,
            led_updates_sent: 0,
        };
        refresh_profile_probe(&mut probe, true);
        #[cfg(feature = "runtime-health")]
        crate::runtime_health::restart_control();
        session.set_connected(true);
        session.reset_profile_state();
        drain_product_dispatch(&mut probe, &mut session, &mut product).await;
        set_state(Usb1ControllerState::Ready(probe));

        pending_speed = wait_for_ready_change(
            &mut controller,
            &mut probe,
            midi_transport.as_mut(),
            &mut session,
            &mut product,
        )
        .await;
        // Detach restores the default policy even on paths that skip a drain.
        product.set_channel_filter_always(false);
        #[cfg(feature = "render-graph-bringup")]
        publish_product_render_control(&product);
        drop(midi_transport);
        bus.free_address(info.device_address);
    }
}

async fn wait_for_device_change<'d, C>(
    controller: &mut BusController<'d, C>,
    session: &ControllerSession,
    product: &mut ProductRuntime,
) -> Option<embassy_usb_driver::Speed>
where
    C: UsbHostController<'d>,
{
    #[cfg(not(feature = "runtime-health"))]
    let _ = (session, &*product);
    loop {
        #[cfg(feature = "runtime-health")]
        crate::runtime_health::publish_control(product, session, false);
        #[cfg(all(feature = "apta-usb0-bringup", feature = "runtime-health"))]
        let Some(event) = crate::usb0_analysis::product_wait(
            product,
            crate::analysis_io::owner_tick(
                controller.wait_for_device_event(),
                embassy_time::Timer::after_millis(pajoniiir_service::owners::OWNER_TICK_MS),
            ),
        )
        .await
        else {
            continue;
        };
        #[cfg(all(feature = "apta-usb0-bringup", not(feature = "runtime-health")))]
        let event =
            crate::usb0_analysis::product_wait(product, controller.wait_for_device_event()).await;
        #[cfg(all(feature = "runtime-health", not(feature = "apta-usb0-bringup")))]
        let event = match select(
            controller.wait_for_device_event(),
            embassy_time::Timer::after_millis(pajoniiir_service::owners::OWNER_TICK_MS),
        )
        .await
        {
            Either::First(event) => event,
            Either::Second(()) => continue,
        };
        #[cfg(not(any(feature = "runtime-health", feature = "apta-usb0-bringup")))]
        let event = controller.wait_for_device_event().await;
        match event {
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

fn capability_flags(capabilities: ControllerUsbCapabilities) -> u16 {
    let mut flags = 0u16;
    if capabilities.has_midi_streaming() {
        flags |= 1 << 0;
    }
    if capabilities.has_bidirectional_midi() {
        flags |= 1 << 1;
    }
    if capabilities.has_usb_audio() {
        flags |= 1 << 2;
    }
    if capabilities.has_audio_output() {
        flags |= 1 << 3;
    }
    if capabilities.hid_interfaces != 0 {
        flags |= 1 << 4;
    }
    if capabilities.vendor_interfaces != 0 {
        flags |= 1 << 5;
    }
    flags
}

fn refresh_profile_probe(probe: &mut Usb1ControllerProbe, request_activation: bool) {
    let mut profile_match = match_scanned_profile(probe.vendor_id, probe.product_id);
    probe.profile_index = profile_match.index;
    probe.profile_catalog_revision = profile_match.revision;
    probe.profile_activation_state = profile_match.activation_state;

    if request_activation
        && profile_match.activation_state == ProfileActivationState::Matched
        && let Some(index) = profile_match.index
        && request_profile_activation(index)
    {
        profile_match = match_scanned_profile(probe.vendor_id, probe.product_id);
        probe.profile_index = profile_match.index;
        probe.profile_catalog_revision = profile_match.revision;
        probe.profile_activation_state = profile_match.activation_state;
    }
}

async fn wait_for_ready_change<'d, C, A>(
    controller: &mut BusController<'d, C>,
    probe: &mut Usb1ControllerProbe,
    mut midi_transport: Option<&mut Usb1MidiTransport<'d, A>>,
    session: &mut ControllerSession,
    product: &mut ProductRuntime,
) -> Option<embassy_usb_driver::Speed>
where
    C: UsbHostController<'d>,
    A: UsbHostAllocator<'d>,
{
    let mut midi_buffer = [0u8; USB1_MIDI_FS_MAX_PACKET_BYTES as usize];

    loop {
        #[cfg(feature = "runtime-health")]
        crate::runtime_health::publish_control(product, session, probe.product_backpressured);
        if probe.profile_activation_state == ProfileActivationState::Initializing {
            let outcome = initialize_controller_profile(
                &mut *controller,
                probe,
                midi_transport.as_deref_mut(),
            )
            .await;
            match outcome {
                ProfileInitOutcome::Initialized | ProfileInitOutcome::Failed => {
                    refresh_profile_probe(probe, false);
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                    continue;
                }
                ProfileInitOutcome::ProfileChanged => {
                    session.reset_profile_state();
                    refresh_profile_probe(probe, true);
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                    continue;
                }
                ProfileInitOutcome::Disconnected => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return None;
                }
                ProfileInitOutcome::Overcurrent => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Failed(
                        Usb1ControllerFailure::Overcurrent,
                    ));
                    return None;
                }
                ProfileInitOutcome::Connected(speed) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return Some(speed);
                }
            }
        }

        if probe.profile_activation_state == ProfileActivationState::Active
            && product.has_dirty_leds()
            && let Some(transport) = midi_transport.as_mut()
            && transport.has_output()
        {
            match flush_controller_leds(&mut *controller, probe, transport, product).await {
                LedFlushOutcome::Complete => {
                    set_state(Usb1ControllerState::Ready(*probe));
                }
                LedFlushOutcome::ProfileChanged => {
                    session.reset_profile_state();
                    refresh_profile_probe(probe, true);
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                    continue;
                }
                LedFlushOutcome::Disconnected => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return None;
                }
                LedFlushOutcome::Overcurrent => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Failed(
                        Usb1ControllerFailure::Overcurrent,
                    ));
                    return None;
                }
                LedFlushOutcome::Connected(speed) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return Some(speed);
                }
                LedFlushOutcome::Failed => {
                    product.mark_led_resync();
                    set_state(Usb1ControllerState::Failed(
                        Usb1ControllerFailure::LedOutput,
                    ));
                    return None;
                }
            }
        }

        let profile_change = select(
            wait_for_profile_catalog_change(),
            wait_for_profile_activation_change(),
        );
        let runtime_change = select(profile_change, wait_for_product_owner_progress());
        #[cfg(feature = "runtime-health")]
        let runtime_change = async {
            match select(
                runtime_change,
                embassy_time::Timer::after_millis(pajoniiir_service::owners::OWNER_TICK_MS),
            )
            .await
            {
                Either::First(change) => change,
                Either::Second(()) => Either::Second(()),
            }
        };

        if probe.profile_activation_state == ProfileActivationState::Active
            && !probe.product_backpressured
            && let Some(transport) = midi_transport.as_mut()
        {
            let device_or_midi = select(
                controller.wait_for_device_event(),
                transport.read_raw(&mut midi_buffer),
            );
            let change = select(device_or_midi, runtime_change);
            #[cfg(feature = "apta-usb0-bringup")]
            let change = crate::usb0_analysis::product_wait(product, change).await;
            #[cfg(not(feature = "apta-usb0-bringup"))]
            let change = change.await;
            match change {
                Either::First(Either::First(DeviceEvent::Disconnected)) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return None;
                }
                Either::First(Either::First(DeviceEvent::Overcurrent)) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Failed(
                        Usb1ControllerFailure::Overcurrent,
                    ));
                    return None;
                }
                Either::First(Either::First(DeviceEvent::Connected(speed))) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return Some(speed);
                }
                Either::First(Either::First(_)) => {}
                Either::First(Either::Second(Ok(read))) => {
                    match process_active_midi_transfer(
                        probe.controller_epoch,
                        session,
                        &midi_buffer[..read],
                    )
                    .await
                    {
                        Ok(report) => {
                            probe.midi_ingest.accumulate(report);
                            drain_product_dispatch(probe, session, product).await;
                            set_state(Usb1ControllerState::Ready(*probe));
                        }
                        Err(ActiveProfileMidiError::NotActive)
                        | Err(ActiveProfileMidiError::StaleController) => {
                            session.reset_profile_state();
                            refresh_profile_probe(probe, true);
                            set_state(Usb1ControllerState::Ready(*probe));
                        }
                        Err(ActiveProfileMidiError::Batch(_)) => {
                            session.set_connected(false);
                            on_controller_disconnect();
                            set_state(Usb1ControllerState::Failed(
                                Usb1ControllerFailure::MalformedMidiTransfer,
                            ));
                            return None;
                        }
                        Err(ActiveProfileMidiError::InvalidActiveProfile) => {
                            session.set_connected(false);
                            on_controller_disconnect();
                            set_state(Usb1ControllerState::Failed(
                                Usb1ControllerFailure::ActiveProfile,
                            ));
                            return None;
                        }
                    }
                }
                Either::First(Either::Second(Err(Usb1MidiTransportError::Transfer(
                    PipeError::Disconnected,
                )))) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Detached);
                    return None;
                }
                Either::First(Either::Second(Err(_))) => {
                    session.set_connected(false);
                    on_controller_disconnect();
                    set_state(Usb1ControllerState::Failed(
                        Usb1ControllerFailure::MidiTransport,
                    ));
                    return None;
                }
                Either::Second(Either::First(Either::First(_revision))) => {
                    session.reset_profile_state();
                    refresh_profile_probe(probe, true);
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                }
                Either::Second(Either::First(Either::Second(_revision))) => {
                    session.reset_profile_state();
                    refresh_profile_probe(probe, false);
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                }
                Either::Second(Either::Second(())) => {
                    drain_product_dispatch(probe, session, product).await;
                    set_state(Usb1ControllerState::Ready(*probe));
                }
            }
            continue;
        }

        let change = select(controller.wait_for_device_event(), runtime_change);
        #[cfg(feature = "apta-usb0-bringup")]
        let change = crate::usb0_analysis::product_wait(product, change).await;
        #[cfg(not(feature = "apta-usb0-bringup"))]
        let change = change.await;
        match change {
            Either::First(DeviceEvent::Disconnected) => {
                session.set_connected(false);
                on_controller_disconnect();
                set_state(Usb1ControllerState::Detached);
                return None;
            }
            Either::First(DeviceEvent::Overcurrent) => {
                session.set_connected(false);
                on_controller_disconnect();
                set_state(Usb1ControllerState::Failed(
                    Usb1ControllerFailure::Overcurrent,
                ));
                return None;
            }
            Either::First(DeviceEvent::Connected(speed)) => {
                session.set_connected(false);
                on_controller_disconnect();
                set_state(Usb1ControllerState::Detached);
                return Some(speed);
            }
            Either::First(_) => {}
            Either::Second(Either::First(Either::First(_revision))) => {
                session.reset_profile_state();
                refresh_profile_probe(probe, true);
                drain_product_dispatch(probe, session, product).await;
                set_state(Usb1ControllerState::Ready(*probe));
            }
            Either::Second(Either::First(Either::Second(_revision))) => {
                session.reset_profile_state();
                refresh_profile_probe(probe, false);
                drain_product_dispatch(probe, session, product).await;
                set_state(Usb1ControllerState::Ready(*probe));
            }
            Either::Second(Either::Second(())) => {
                drain_product_dispatch(probe, session, product).await;
                set_state(Usb1ControllerState::Ready(*probe));
            }
        }
    }
}

async fn wait_for_product_owner_progress() {
    #[cfg(feature = "library-deck-load")]
    {
        let _ = select(
            wait_for_effect_owner_progress(),
            wait_for_library_deck_progress(),
        )
        .await;
    }
    #[cfg(not(feature = "library-deck-load"))]
    wait_for_effect_owner_progress().await;
}

#[cfg(feature = "library-deck-load")]
fn dispatch_library_runtime(product: &mut ProductRuntime) -> bool {
    dispatch_library_deck(product).made_progress()
}

#[cfg(not(feature = "library-deck-load"))]
fn dispatch_library_runtime(_product: &mut ProductRuntime) -> bool {
    false
}

async fn drain_product_dispatch(
    probe: &mut Usb1ControllerProbe,
    session: &mut ControllerSession,
    product: &mut ProductRuntime,
) {
    // Mirror the profile capability before any control or render publication.
    product.set_channel_filter_always(
        active_channel_filter_always(probe.controller_epoch).await,
    );
    loop {
        let library_before = dispatch_library_runtime(product);
        let before = dispatch_product_effects(product);
        let report =
            dispatch_session_deliveries(probe.controller_epoch, session, product, 32).await;
        let after = dispatch_product_effects(product);
        let library_after = dispatch_library_runtime(product);

        if report.backpressured {
            if library_before || before.made_progress() || after.made_progress() || library_after {
                continue;
            }
            probe.product_backpressured = true;
            break;
        }
        if report.idle {
            probe.product_backpressured = false;
            break;
        }

        // Budget exhaustion is cooperative work chunking, not downstream
        // backpressure. No producer can mutate this ControllerSession while this
        // owner is draining it, so another bounded round must make progress or
        // reach an actual sink refusal.
        if !report.budget_exhausted {
            probe.product_backpressured = false;
            break;
        }
    }

    let _ = dispatch_product_effects(product);
    let _ = dispatch_library_runtime(product);
    probe.product_pending_deck_effects = product.pending_deck_effects().min(u8::MAX as usize) as u8;
    probe.product_pending_library_effects =
        product.pending_library_effects().min(u8::MAX as usize) as u8;

    #[cfg(feature = "apta-usb0-bringup")]
    crate::usb0_analysis::product_turn(product);
    #[cfg(feature = "render-graph-bringup")]
    publish_product_render_control(product);
    #[cfg(feature = "runtime-health")]
    crate::runtime_health::publish_control(product, session, probe.product_backpressured);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProfileInitOutcome {
    Initialized,
    ProfileChanged,
    Failed,
    Disconnected,
    Overcurrent,
    Connected(embassy_usb_driver::Speed),
}

/// Enqueue the active profile's bounded initial SysEx before MIDI input or LED
/// output open. The binding is rechecked before every packet; a profile change
/// stops after at most the packet already in flight. Any transfer failure or
/// unrelated device event closes this activation as FAILED rather than leaving
/// a truncated initialization behind an open profile.
async fn initialize_controller_profile<'d, C, A>(
    controller: &mut BusController<'d, C>,
    probe: &Usb1ControllerProbe,
    transport: Option<&mut Usb1MidiTransport<'d, A>>,
) -> ProfileInitOutcome
where
    C: UsbHostController<'d>,
    A: UsbHostAllocator<'d>,
{
    let mut sysex = [0u8; MAX_INIT_SYSEX];
    let Ok((ticket, len)) =
        pending_profile_initialization(probe.controller_epoch, &mut sysex).await
    else {
        return ProfileInitOutcome::ProfileChanged;
    };
    let (Some(transport), Ok(packets)) = (
        transport.filter(|transport| transport.has_output()),
        UsbMidiSysExPackets::new(0, &sysex[..len]),
    ) else {
        finish_profile_initialization(ticket, false).await;
        return ProfileInitOutcome::Failed;
    };

    for event in packets {
        if !profile_initialization_is_current(ticket, probe.controller_epoch) {
            return ProfileInitOutcome::ProfileChanged;
        }
        match select(
            controller.wait_for_device_event(),
            transport.write_event(event),
        )
        .await
        {
            Either::First(DeviceEvent::Disconnected) => return ProfileInitOutcome::Disconnected,
            Either::First(DeviceEvent::Overcurrent) => return ProfileInitOutcome::Overcurrent,
            Either::First(DeviceEvent::Connected(speed)) => {
                return ProfileInitOutcome::Connected(speed);
            }
            Either::Second(Ok(())) => {}
            Either::Second(Err(Usb1MidiTransportError::Transfer(PipeError::Disconnected))) => {
                return ProfileInitOutcome::Disconnected;
            }
            Either::First(_) | Either::Second(Err(_)) => {
                finish_profile_initialization(ticket, false).await;
                return ProfileInitOutcome::Failed;
            }
        }
    }

    if finish_profile_initialization(ticket, true).await {
        ProfileInitOutcome::Initialized
    } else {
        ProfileInitOutcome::ProfileChanged
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LedFlushOutcome {
    Complete,
    ProfileChanged,
    Disconnected,
    Overcurrent,
    Connected(embassy_usb_driver::Speed),
    Failed,
}

async fn flush_controller_leds<'d, C, A>(
    controller: &mut BusController<'d, C>,
    probe: &mut Usb1ControllerProbe,
    transport: &mut Usb1MidiTransport<'d, A>,
    product: &mut ProductRuntime,
) -> LedFlushOutcome
where
    C: UsbHostController<'d>,
    A: UsbHostAllocator<'d>,
{
    while let Some(update) = product.take_led_update() {
        let event = match map_active_led_update(probe.controller_epoch, update).await {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(ActiveProfileLedError::NotActive | ActiveProfileLedError::StaleController) => {
                product.mark_led_resync();
                return LedFlushOutcome::ProfileChanged;
            }
            Err(
                ActiveProfileLedError::InvalidActiveProfile
                | ActiveProfileLedError::InvalidMidiOutput,
            ) => {
                product.mark_led_resync();
                return LedFlushOutcome::Failed;
            }
        };

        let device_or_write = select(
            controller.wait_for_device_event(),
            transport.write_event(event),
        );
        let profile_change = select(
            wait_for_profile_catalog_change(),
            wait_for_profile_activation_change(),
        );

        match select(device_or_write, profile_change).await {
            Either::First(Either::First(DeviceEvent::Disconnected)) => {
                product.mark_led_resync();
                return LedFlushOutcome::Disconnected;
            }
            Either::First(Either::First(DeviceEvent::Overcurrent)) => {
                product.mark_led_resync();
                return LedFlushOutcome::Overcurrent;
            }
            Either::First(Either::First(DeviceEvent::Connected(speed))) => {
                product.mark_led_resync();
                return LedFlushOutcome::Connected(speed);
            }
            Either::First(Either::First(_)) => {
                product.mark_led_resync();
            }
            Either::First(Either::Second(Ok(()))) => {
                probe.led_updates_sent = probe.led_updates_sent.saturating_add(1);
            }
            Either::First(Either::Second(Err(Usb1MidiTransportError::Transfer(
                PipeError::Disconnected,
            )))) => {
                product.mark_led_resync();
                return LedFlushOutcome::Disconnected;
            }
            Either::First(Either::Second(Err(_))) => {
                product.mark_led_resync();
                return LedFlushOutcome::Failed;
            }
            Either::Second(_) => {
                product.mark_led_resync();
                return LedFlushOutcome::ProfileChanged;
            }
        }
    }

    LedFlushOutcome::Complete
}
