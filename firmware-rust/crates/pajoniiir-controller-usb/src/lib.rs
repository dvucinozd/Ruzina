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
