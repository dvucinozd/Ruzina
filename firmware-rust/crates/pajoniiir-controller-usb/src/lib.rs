#![no_std]
#![forbid(unsafe_code)]

const DESCRIPTOR_CONFIGURATION: u8 = 0x02;
const DESCRIPTOR_INTERFACE: u8 = 0x04;
const DESCRIPTOR_ENDPOINT: u8 = 0x05;
const CLASS_AUDIO: u8 = 0x01;
const CLASS_HID: u8 = 0x03;
const CLASS_VENDOR: u8 = 0xff;
const AUDIO_SUBCLASS_CONTROL: u8 = 0x01;
const AUDIO_SUBCLASS_STREAMING: u8 = 0x02;
const AUDIO_SUBCLASS_MIDI_STREAMING: u8 = 0x03;
const ENDPOINT_TRANSFER_ISOCHRONOUS: u8 = 0x01;
const ENDPOINT_TRANSFER_BULK: u8 = 0x02;
const ENDPOINT_TRANSFER_INTERRUPT: u8 = 0x03;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControllerUsbCapabilities {
    pub interface_descriptors: u8,
    pub midi_streaming_interfaces: u8,
    pub midi_bulk_in_endpoints: u8,
    pub midi_bulk_out_endpoints: u8,
    pub audio_control_interfaces: u8,
    pub audio_streaming_interfaces: u8,
    pub audio_iso_in_endpoints: u8,
    pub audio_iso_out_endpoints: u8,
    pub hid_interfaces: u8,
    pub hid_interrupt_in_endpoints: u8,
    pub hid_interrupt_out_endpoints: u8,
    pub vendor_interfaces: u8,
    pub other_interfaces: u8,
}

impl ControllerUsbCapabilities {
    pub const fn has_midi_streaming(self) -> bool {
        self.midi_streaming_interfaces != 0
    }

    pub const fn has_bidirectional_midi(self) -> bool {
        self.has_midi_streaming()
            && self.midi_bulk_in_endpoints != 0
            && self.midi_bulk_out_endpoints != 0
    }

    pub const fn has_usb_audio(self) -> bool {
        self.audio_control_interfaces != 0 || self.audio_streaming_interfaces != 0
    }

    pub const fn has_audio_output(self) -> bool {
        self.audio_streaming_interfaces != 0 && self.audio_iso_out_endpoints != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerUsbDescriptorError {
    TooShort,
    NotConfiguration,
    InvalidConfigurationLength,
    DescriptorTooShort { offset: usize },
    DescriptorOutOfBounds { offset: usize },
    InterfaceTooShort { offset: usize },
    EndpointTooShort { offset: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbEndpointDescriptor {
    pub address: u8,
    pub max_packet_size: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MidiStreamingInterface {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub bulk_in: Option<UsbEndpointDescriptor>,
    pub bulk_out: Option<UsbEndpointDescriptor>,
}

impl MidiStreamingInterface {
    pub const fn has_input(self) -> bool {
        self.bulk_in.is_some()
    }

    pub const fn has_output(self) -> bool {
        self.bulk_out.is_some()
    }

    pub const fn is_bidirectional(self) -> bool {
        self.has_input() && self.has_output()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbMidiEvent {
    pub cable: u8,
    pub cin: u8,
    pub bytes: [u8; 3],
    pub len: u8,
}

impl UsbMidiEvent {
    pub fn decode(packet: [u8; 4]) -> Option<Self> {
        let cin = packet[0] & 0x0f;
        let len = cin_payload_len(cin)?;
        Some(Self {
            cable: packet[0] >> 4,
            cin,
            bytes: [packet[1], packet[2], packet[3]],
            len,
        })
    }

    pub const fn as_usb_packet(self) -> [u8; 4] {
        [
            (self.cable << 4) | (self.cin & 0x0f),
            self.bytes[0],
            self.bytes[1],
            self.bytes[2],
        ]
    }

    pub const fn channel_voice(self) -> Option<MidiChannelVoice> {
        if self.cin < 0x08 || self.cin > 0x0e {
            return None;
        }
        if self.bytes[0] >> 4 != self.cin {
            return None;
        }
        Some(MidiChannelVoice {
            cable: self.cable,
            status: self.bytes[0],
            data1: self.bytes[1],
            data2: if self.len == 3 {
                Some(self.bytes[2])
            } else {
                None
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MidiChannelVoice {
    pub cable: u8,
    pub status: u8,
    pub data1: u8,
    pub data2: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMidiEncodeError {
    InvalidCable(u8),
    NotChannelVoice(u8),
    InvalidDataByte(u8),
    MissingData2,
}

impl MidiChannelVoice {
    pub fn new(
        cable: u8,
        status: u8,
        data1: u8,
        data2: Option<u8>,
    ) -> Result<Self, UsbMidiEncodeError> {
        if cable > 0x0f {
            return Err(UsbMidiEncodeError::InvalidCable(cable));
        }
        let family = status >> 4;
        if !(0x08..=0x0e).contains(&family) {
            return Err(UsbMidiEncodeError::NotChannelVoice(status));
        }
        if data1 > 0x7f {
            return Err(UsbMidiEncodeError::InvalidDataByte(data1));
        }

        let two_byte = matches!(family, 0x0c | 0x0d);
        if !two_byte && data2.is_none() {
            return Err(UsbMidiEncodeError::MissingData2);
        }
        if let Some(value) = data2
            && value > 0x7f
        {
            return Err(UsbMidiEncodeError::InvalidDataByte(value));
        }

        Ok(Self {
            cable,
            status,
            data1,
            data2: if two_byte { None } else { data2 },
        })
    }

    pub fn to_usb_event(self) -> UsbMidiEvent {
        let cin = self.status >> 4;
        let len = if matches!(cin, 0x0c | 0x0d) { 2 } else { 3 };
        UsbMidiEvent {
            cable: self.cable,
            cin,
            bytes: [self.status, self.data1, self.data2.unwrap_or(0)],
            len,
        }
    }
}

const fn cin_payload_len(cin: u8) -> Option<u8> {
    match cin {
        0x02 | 0x06 | 0x0c | 0x0d => Some(2),
        0x03 | 0x04 | 0x07 | 0x08 | 0x09 | 0x0a | 0x0b | 0x0e => Some(3),
        0x05 | 0x0f => Some(1),
        _ => None,
    }
}

pub fn find_midi_streaming_interface(
    bytes: &[u8],
) -> Result<Option<MidiStreamingInterface>, ControllerUsbDescriptorError> {
    inspect_configuration(bytes)?;

    let header_len = usize::from(bytes[0]);
    let total_len = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    let mut current: Option<MidiStreamingInterface> = None;
    let mut offset = header_len;

    while offset < total_len {
        let descriptor_len = usize::from(bytes[offset]);
        let end = offset + descriptor_len;

        if bytes[offset + 1] == DESCRIPTOR_INTERFACE {
            if let Some(found) = current
                && (found.bulk_in.is_some() || found.bulk_out.is_some())
            {
                return Ok(Some(found));
            }

            let class = bytes[offset + 5];
            let subclass = bytes[offset + 6];
            let alternate_setting = bytes[offset + 3];
            current = if class == CLASS_AUDIO
                && subclass == AUDIO_SUBCLASS_MIDI_STREAMING
                && alternate_setting == 0
            {
                Some(MidiStreamingInterface {
                    interface_number: bytes[offset + 2],
                    alternate_setting,
                    bulk_in: None,
                    bulk_out: None,
                })
            } else {
                None
            };
        } else if bytes[offset + 1] == DESCRIPTOR_ENDPOINT
            && let Some(interface) = current.as_mut()
            && bytes[offset + 3] & 0x03 == ENDPOINT_TRANSFER_BULK
        {
            let endpoint = UsbEndpointDescriptor {
                address: bytes[offset + 2],
                max_packet_size: u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]),
            };
            if endpoint.address & 0x80 != 0 {
                if interface.bulk_in.is_none() {
                    interface.bulk_in = Some(endpoint);
                }
            } else if interface.bulk_out.is_none() {
                interface.bulk_out = Some(endpoint);
            }
        }

        offset = end;
    }

    Ok(current.filter(|found| found.bulk_in.is_some() || found.bulk_out.is_some()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterfaceKind {
    MidiStreaming,
    AudioControl,
    AudioStreaming,
    Hid,
    Vendor,
    Other,
}

pub fn inspect_configuration(
    bytes: &[u8],
) -> Result<ControllerUsbCapabilities, ControllerUsbDescriptorError> {
    if bytes.len() < 9 {
        return Err(ControllerUsbDescriptorError::TooShort);
    }
    if bytes[1] != DESCRIPTOR_CONFIGURATION {
        return Err(ControllerUsbDescriptorError::NotConfiguration);
    }

    let header_len = usize::from(bytes[0]);
    let total_len = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    if header_len < 9 || total_len < header_len || total_len > bytes.len() {
        return Err(ControllerUsbDescriptorError::InvalidConfigurationLength);
    }

    let mut capabilities = ControllerUsbCapabilities::default();
    let mut current_interface = None;
    let mut offset = header_len;

    while offset < total_len {
        if offset + 2 > total_len {
            return Err(ControllerUsbDescriptorError::DescriptorTooShort { offset });
        }
        let descriptor_len = usize::from(bytes[offset]);
        if descriptor_len < 2 {
            return Err(ControllerUsbDescriptorError::DescriptorTooShort { offset });
        }
        let end = offset
            .checked_add(descriptor_len)
            .ok_or(ControllerUsbDescriptorError::DescriptorOutOfBounds { offset })?;
        if end > total_len {
            return Err(ControllerUsbDescriptorError::DescriptorOutOfBounds { offset });
        }

        match bytes[offset + 1] {
            DESCRIPTOR_INTERFACE => {
                if descriptor_len < 9 {
                    return Err(ControllerUsbDescriptorError::InterfaceTooShort { offset });
                }
                let kind = classify_interface(bytes[offset + 5], bytes[offset + 6]);
                current_interface = Some(kind);
                capabilities.interface_descriptors =
                    capabilities.interface_descriptors.saturating_add(1);
                match kind {
                    InterfaceKind::MidiStreaming => {
                        capabilities.midi_streaming_interfaces =
                            capabilities.midi_streaming_interfaces.saturating_add(1);
                    }
                    InterfaceKind::AudioControl => {
                        capabilities.audio_control_interfaces =
                            capabilities.audio_control_interfaces.saturating_add(1);
                    }
                    InterfaceKind::AudioStreaming => {
                        capabilities.audio_streaming_interfaces =
                            capabilities.audio_streaming_interfaces.saturating_add(1);
                    }
                    InterfaceKind::Hid => {
                        capabilities.hid_interfaces = capabilities.hid_interfaces.saturating_add(1);
                    }
                    InterfaceKind::Vendor => {
                        capabilities.vendor_interfaces =
                            capabilities.vendor_interfaces.saturating_add(1);
                    }
                    InterfaceKind::Other => {
                        capabilities.other_interfaces =
                            capabilities.other_interfaces.saturating_add(1);
                    }
                }
            }
            DESCRIPTOR_ENDPOINT => {
                if descriptor_len < 7 {
                    return Err(ControllerUsbDescriptorError::EndpointTooShort { offset });
                }
                if let Some(kind) = current_interface {
                    classify_endpoint(
                        &mut capabilities,
                        kind,
                        bytes[offset + 3] & 0x03,
                        bytes[offset + 2] & 0x80 != 0,
                    );
                }
            }
            _ => {}
        }

        offset = end;
    }

    Ok(capabilities)
}

const fn classify_interface(class: u8, subclass: u8) -> InterfaceKind {
    match (class, subclass) {
        (CLASS_AUDIO, AUDIO_SUBCLASS_MIDI_STREAMING) => InterfaceKind::MidiStreaming,
        (CLASS_AUDIO, AUDIO_SUBCLASS_CONTROL) => InterfaceKind::AudioControl,
        (CLASS_AUDIO, AUDIO_SUBCLASS_STREAMING) => InterfaceKind::AudioStreaming,
        (CLASS_HID, _) => InterfaceKind::Hid,
        (CLASS_VENDOR, _) => InterfaceKind::Vendor,
        _ => InterfaceKind::Other,
    }
}

fn classify_endpoint(
    capabilities: &mut ControllerUsbCapabilities,
    kind: InterfaceKind,
    transfer_type: u8,
    input: bool,
) {
    match (kind, transfer_type, input) {
        (InterfaceKind::MidiStreaming, ENDPOINT_TRANSFER_BULK, true) => {
            capabilities.midi_bulk_in_endpoints =
                capabilities.midi_bulk_in_endpoints.saturating_add(1);
        }
        (InterfaceKind::MidiStreaming, ENDPOINT_TRANSFER_BULK, false) => {
            capabilities.midi_bulk_out_endpoints =
                capabilities.midi_bulk_out_endpoints.saturating_add(1);
        }
        (InterfaceKind::AudioStreaming, ENDPOINT_TRANSFER_ISOCHRONOUS, true) => {
            capabilities.audio_iso_in_endpoints =
                capabilities.audio_iso_in_endpoints.saturating_add(1);
        }
        (InterfaceKind::AudioStreaming, ENDPOINT_TRANSFER_ISOCHRONOUS, false) => {
            capabilities.audio_iso_out_endpoints =
                capabilities.audio_iso_out_endpoints.saturating_add(1);
        }
        (InterfaceKind::Hid, ENDPOINT_TRANSFER_INTERRUPT, true) => {
            capabilities.hid_interrupt_in_endpoints =
                capabilities.hid_interrupt_in_endpoints.saturating_add(1);
        }
        (InterfaceKind::Hid, ENDPOINT_TRANSFER_INTERRUPT, false) => {
            capabilities.hid_interrupt_out_endpoints =
                capabilities.hid_interrupt_out_endpoints.saturating_add(1);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push<const N: usize>(out: &mut [u8; N], used: &mut usize, descriptor: &[u8]) {
        let end = *used + descriptor.len();
        out[*used..end].copy_from_slice(descriptor);
        *used = end;
    }

    fn composite_fixture() -> ([u8; 128], usize) {
        let mut bytes = [0u8; 128];
        let mut used = 9usize;
        push(&mut bytes, &mut used, &[9, 4, 0, 0, 0, 1, 1, 0, 0]);
        push(&mut bytes, &mut used, &[9, 4, 1, 0, 2, 1, 3, 0, 0]);
        push(&mut bytes, &mut used, &[7, 5, 0x01, 0x02, 64, 0, 0]);
        push(&mut bytes, &mut used, &[7, 5, 0x81, 0x02, 64, 0, 0]);
        push(&mut bytes, &mut used, &[9, 4, 2, 0, 0, 1, 2, 0, 0]);
        push(&mut bytes, &mut used, &[9, 4, 2, 1, 2, 1, 2, 0, 0]);
        push(&mut bytes, &mut used, &[7, 5, 0x02, 0x01, 192, 0, 1]);
        push(&mut bytes, &mut used, &[7, 5, 0x82, 0x01, 3, 0, 4]);
        push(&mut bytes, &mut used, &[9, 4, 3, 0, 1, 3, 0, 0, 0]);
        push(&mut bytes, &mut used, &[7, 5, 0x83, 0x03, 64, 0, 1]);
        bytes[..9].copy_from_slice(&[9, 2, used as u8, 0, 4, 1, 0, 0x80, 250]);
        (bytes, used)
    }

    #[test]
    fn classifies_composite_dj_controller() {
        let (bytes, used) = composite_fixture();
        let c = inspect_configuration(&bytes[..used]).unwrap();
        assert_eq!(c.interface_descriptors, 5);
        assert_eq!(c.audio_control_interfaces, 1);
        assert_eq!(c.midi_streaming_interfaces, 1);
        assert_eq!(c.midi_bulk_out_endpoints, 1);
        assert_eq!(c.midi_bulk_in_endpoints, 1);
        assert_eq!(c.audio_streaming_interfaces, 2);
        assert_eq!(c.audio_iso_out_endpoints, 1);
        assert_eq!(c.audio_iso_in_endpoints, 1);
        assert_eq!(c.hid_interfaces, 1);
        assert_eq!(c.hid_interrupt_in_endpoints, 1);
        assert!(c.has_bidirectional_midi());
        assert!(c.has_usb_audio());
        assert!(c.has_audio_output());
    }

    #[test]
    fn locates_midi_streaming_bulk_endpoints() {
        let (bytes, used) = composite_fixture();
        let midi = find_midi_streaming_interface(&bytes[..used])
            .unwrap()
            .unwrap();

        assert_eq!(midi.interface_number, 1);
        assert_eq!(midi.alternate_setting, 0);
        assert_eq!(
            midi.bulk_in,
            Some(UsbEndpointDescriptor {
                address: 0x81,
                max_packet_size: 64,
            })
        );
        assert_eq!(
            midi.bulk_out,
            Some(UsbEndpointDescriptor {
                address: 0x01,
                max_packet_size: 64,
            })
        );
        assert!(midi.is_bidirectional());
    }

    #[test]
    fn usb_midi_channel_voice_round_trips() {
        let note = UsbMidiEvent::decode([0x19, 0x91, 0x24, 0x7f]).unwrap();
        assert_eq!(note.cable, 1);
        assert_eq!(note.len, 3);
        assert_eq!(
            note.channel_voice(),
            Some(MidiChannelVoice {
                cable: 1,
                status: 0x91,
                data1: 0x24,
                data2: Some(0x7f),
            })
        );

        let cc = MidiChannelVoice::new(2, 0xb3, 0x10, Some(0x40)).unwrap();
        assert_eq!(cc.to_usb_event().as_usb_packet(), [0x2b, 0xb3, 0x10, 0x40]);

        let program = MidiChannelVoice::new(0, 0xc0, 0x05, None).unwrap();
        assert_eq!(
            program.to_usb_event().as_usb_packet(),
            [0x0c, 0xc0, 0x05, 0x00]
        );
    }

    #[test]
    fn usb_midi_rejects_reserved_and_mismatched_packets() {
        assert_eq!(UsbMidiEvent::decode([0x00, 0, 0, 0]), None);
        let mismatch = UsbMidiEvent::decode([0x09, 0xb0, 0x10, 0x7f]).unwrap();
        assert_eq!(mismatch.channel_voice(), None);
        assert_eq!(
            MidiChannelVoice::new(16, 0x90, 0x10, Some(0x7f)),
            Err(UsbMidiEncodeError::InvalidCable(16))
        );
        assert_eq!(
            MidiChannelVoice::new(0, 0x90, 0x10, None),
            Err(UsbMidiEncodeError::MissingData2)
        );
    }

    #[test]
    fn keeps_vendor_only_controller_visible_without_claiming_midi() {
        let mut bytes = [0u8; 18];
        bytes[..9].copy_from_slice(&[9, 2, 18, 0, 1, 1, 0, 0x80, 50]);
        bytes[9..].copy_from_slice(&[9, 4, 0, 0, 0, 0xff, 0, 0, 0]);
        let c = inspect_configuration(&bytes).unwrap();
        assert_eq!(c.vendor_interfaces, 1);
        assert!(!c.has_midi_streaming());
        assert!(!c.has_usb_audio());
    }

    #[test]
    fn malformed_descriptor_fails_closed() {
        let mut bytes = [0u8; 11];
        bytes[..9].copy_from_slice(&[9, 2, 11, 0, 0, 1, 0, 0x80, 50]);
        bytes[9] = 0;
        bytes[10] = 4;
        assert_eq!(
            inspect_configuration(&bytes),
            Err(ControllerUsbDescriptorError::DescriptorTooShort { offset: 9 })
        );
    }

    #[test]
    fn advertised_total_length_cannot_escape_buffer() {
        let bytes = [9, 2, 32, 0, 0, 1, 0, 0x80, 50];
        assert_eq!(
            inspect_configuration(&bytes),
            Err(ControllerUsbDescriptorError::InvalidConfigurationLength)
        );
    }
}
