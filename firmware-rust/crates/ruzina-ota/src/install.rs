//! Streaming install of a signed bundle into the inactive A/B slot (M2.5
//! `p4_ota` + push/pull handlers).
//!
//! The order is the safety argument. The signed header is read, parsed and
//! its signature verified, the bundle length bound to the signed image size,
//! a pull offer bound to the signed version, and the first image bytes
//! checked as an ESP32-P4 application, all before playback stops or the slot
//! is erased. Every later failure aborts the slot and resumes playback, and
//! the boot slot is switched only after the image hash, the channel hash
//! (pull) and the written app descriptor all match.

use crate::bundle::{
    self, Expected, HEADER_SIZE, IMAGE_HEADER_SIZE, Manifest, ManifestError, Version,
};
use crate::channel::Offer;
use crate::version::{ReleaseOrder, compare};
use sha2::{Digest, Sha256};

/// Consecutive read timeouts tolerated before a transfer counts as stalled
/// (M2.5 `HTTP_IDLE_RETRY_MAX`). Any progress resets the count.
pub const IDLE_RETRY_MAX: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// No bytes within one transport timeout; not end of stream.
    Timeout,
    Transport,
}

/// The bundle byte stream (HTTP request body or HTTPS download). `Ok(0)` is
/// end of stream. TLS CA/hostname verification is the source's obligation.
pub trait BundleSource {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ReadError>;
}

/// The inactive application slot and boot selection.
pub trait SlotFlash {
    type Error: Copy;
    /// Size of the slot the next update goes to, `None` without one.
    fn update_slot_size(&mut self) -> Option<u32>;
    /// Opens (erases) the update slot.
    fn begin(&mut self, image_size: u32) -> Result<(), Self::Error>;
    fn write(&mut self, data: &[u8]) -> Result<(), Self::Error>;
    /// Leaves the slot unbootable; a no-op once the slot is closed.
    fn abort(&mut self);
    /// Closes the slot; the platform validates the ESP image structure.
    fn end(&mut self) -> Result<(), Self::Error>;
    /// The written image's ESP app descriptor `project_name` and `version`.
    fn descriptor(&mut self) -> Result<([u8; 32], [u8; 32]), Self::Error>;
    /// Boots the written slot next (as a pending candidate).
    fn set_boot(&mut self) -> Result<(), Self::Error>;
}

/// Loads and audio stop before the slot is erased (M2.5
/// `audio_engine_suspend_loads_and_stop_all`).
pub trait Playback {
    fn stop_for_update(&mut self) -> bool;
    fn resume(&mut self);
}

#[derive(Clone, Copy, Debug)]
pub enum Policy<'a> {
    /// Signed local upload: any signed version installs, so a signed older
    /// build remains the service rollback path.
    Local,
    /// Pull: the bundle must be the offered one and newer than the running
    /// build, checked again over the signed version.
    Pull {
        offer: &'a Offer,
        running_version: &'a str,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Content-Length of the upload or download.
    pub declared_len: u32,
    pub expected: Expected<'a>,
    pub public_key: &'a [u8; 65],
    pub policy: Policy<'a>,
}

/// Where a transfer failed, for phase diagnostics without URLs or secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Length,
    ReadSignedHeader,
    ParseSignedHeader,
    VerifySignature,
    ReleaseBinding,
    ReadImageHeader,
    ImageHeader,
    Slot,
    StopAudio,
    BeginFlash,
    DownloadImage,
    WriteFlash,
    ImageHash,
    ChannelHash,
    FinalizeImage,
    Descriptor,
    SetBoot,
}

impl Stage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Length => "bundle length",
            Self::ReadSignedHeader => "read signed header",
            Self::ParseSignedHeader => "parse signed header",
            Self::VerifySignature => "verify signature",
            Self::ReleaseBinding => "release binding",
            Self::ReadImageHeader => "read image header",
            Self::ImageHeader => "image header",
            Self::Slot => "update slot",
            Self::StopAudio => "stop audio",
            Self::BeginFlash => "begin flash",
            Self::DownloadImage => "download image",
            Self::WriteFlash => "write flash",
            Self::ImageHash => "image hash",
            Self::ChannelHash => "channel hash",
            Self::FinalizeImage => "finalize image",
            Self::Descriptor => "app descriptor",
            Self::SetBoot => "set boot slot",
        }
    }

    /// Whether the update slot was open (and is now aborted).
    pub const fn touched_flash(self) -> bool {
        matches!(
            self,
            Self::DownloadImage
                | Self::WriteFlash
                | Self::ImageHash
                | Self::ChannelHash
                | Self::FinalizeImage
                | Self::Descriptor
                | Self::SetBoot
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    /// Bundle length differs from the signed size (or the pull offer).
    Length,
    Manifest(ManifestError),
    Stalled,
    Truncated,
    Transport,
    /// Signed version is not the offered release.
    ReleaseMismatch,
    NotNewer,
    NotP4Image,
    NoSlot,
    SlotTooSmall,
    Playback,
    Flash,
    ImageHash,
    ChannelHash,
    WrongProject,
    /// The written image's version is not the signed one.
    VersionMismatch,
}

impl Fault {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Length => "size does not match the manifest",
            Self::Manifest(error) => error.name(),
            Self::Stalled => "transfer stalled",
            Self::Truncated => "transfer truncated",
            Self::Transport => "transport error",
            Self::ReleaseMismatch => "signed bundle version does not match the offered release",
            Self::NotNewer => "signed bundle is not newer than the running build",
            Self::NotP4Image => "not an ESP32-P4 firmware image",
            Self::NoSlot => "no OTA target slot",
            Self::SlotTooSmall => "invalid image size",
            Self::Playback => "cannot stop playback",
            Self::Flash => "flash operation failed",
            Self::ImageHash => "firmware SHA-256 mismatch",
            Self::ChannelHash => "bundle hash does not match the channel",
            Self::WrongProject => "wrong firmware project",
            Self::VersionMismatch => "signed version does not match image",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Failure {
    pub stage: Stage,
    pub fault: Fault,
}

/// Proof that a verified image is set to boot next. The reboot runs from a
/// dedicated task pinned to [`RebootPermit::CORE`] after the service log is
/// flushed: the M2.2 push-OTA reboot panicked in the ESP32-P4 cache
/// write-back when an unpinned helper restarted from the other core.
#[derive(Debug, Eq, PartialEq)]
pub struct RebootPermit {
    version: Version,
}

impl RebootPermit {
    pub const CORE: u8 = 0;
    pub const DELAY_MS: u32 = 1000;

    pub fn version(&self) -> &str {
        self.version.as_str()
    }
}

struct Reader<'s, S> {
    source: &'s mut S,
    idle: u8,
}

impl<S: BundleSource> Reader<'_, S> {
    fn fill(&mut self, buf: &mut [u8], stage: Stage) -> Result<(), Failure> {
        let mut have = 0;
        while have < buf.len() {
            have += self.some(&mut buf[have..], stage)?;
        }
        Ok(())
    }

    fn some(&mut self, buf: &mut [u8], stage: Stage) -> Result<usize, Failure> {
        let fail = |fault| Failure { stage, fault };
        loop {
            match self.source.read(buf) {
                Ok(0) => return Err(fail(Fault::Truncated)),
                Ok(got) => {
                    self.idle = 0;
                    return Ok(got.min(buf.len()));
                }
                Err(ReadError::Timeout) => {
                    self.idle += 1;
                    if self.idle >= IDLE_RETRY_MAX {
                        return Err(fail(Fault::Stalled));
                    }
                }
                Err(ReadError::Transport) => return Err(fail(Fault::Transport)),
            }
        }
    }
}

/// M2.5 `p4_ota_policy_text_matches`: the field terminates inside its bounds.
pub fn descriptor_text_matches(field: &[u8; 32], expected: &str) -> bool {
    field
        .iter()
        .position(|byte| *byte == 0)
        .is_some_and(|len| !expected.is_empty() && &field[..len] == expected.as_bytes())
}

fn checks_before_flash<S: BundleSource>(
    reader: &mut Reader<'_, S>,
    request: &Request<'_>,
    header: &mut [u8; HEADER_SIZE],
    image_head: &mut [u8; IMAGE_HEADER_SIZE],
) -> Result<Manifest, Failure> {
    let fail = |stage, fault| Failure { stage, fault };
    if let Policy::Pull { offer, .. } = request.policy
        && offer.size != request.declared_len
    {
        return Err(fail(Stage::Length, Fault::Length));
    }
    if (request.declared_len as usize) < HEADER_SIZE + IMAGE_HEADER_SIZE {
        return Err(fail(Stage::Length, Fault::Length));
    }
    reader.fill(header, Stage::ReadSignedHeader)?;
    let manifest = bundle::parse_header(header, &request.expected)
        .map_err(|error| fail(Stage::ParseSignedHeader, Fault::Manifest(error)))?;
    bundle::verify_signature(header, request.public_key)
        .map_err(|error| fail(Stage::VerifySignature, Fault::Manifest(error)))?;
    if u64::from(request.declared_len) != HEADER_SIZE as u64 + u64::from(manifest.image_size) {
        return Err(fail(Stage::Length, Fault::Length));
    }
    if let Policy::Pull {
        offer,
        running_version,
    } = request.policy
    {
        if offer.release.as_str() != manifest.version.as_str() {
            return Err(fail(Stage::ReleaseBinding, Fault::ReleaseMismatch));
        }
        if compare(manifest.version.as_str(), running_version) != ReleaseOrder::Newer {
            return Err(fail(Stage::ReleaseBinding, Fault::NotNewer));
        }
    }
    reader.fill(image_head, Stage::ReadImageHeader)?;
    if !bundle::image_header_valid(image_head, request.expected.chip_id) {
        return Err(fail(Stage::ImageHeader, Fault::NotP4Image));
    }
    Ok(manifest)
}

/// Runs one install. `buf` bounds each read (M2.5 uses 4 KiB). On success
/// playback stays stopped for the reboot; on any failure after it stopped it
/// is resumed and an opened slot is aborted.
pub fn install<S, F, P>(
    source: &mut S,
    flash: &mut F,
    playback: &mut P,
    request: &Request<'_>,
    buf: &mut [u8],
) -> Result<RebootPermit, Failure>
where
    S: BundleSource,
    F: SlotFlash,
    P: Playback,
{
    let fail = |stage, fault| Failure { stage, fault };
    let mut reader = Reader { source, idle: 0 };
    let mut header = [0; HEADER_SIZE];
    let mut image_head = [0; IMAGE_HEADER_SIZE];
    let manifest = checks_before_flash(&mut reader, request, &mut header, &mut image_head)?;

    match flash.update_slot_size() {
        None => return Err(fail(Stage::Slot, Fault::NoSlot)),
        Some(size) if manifest.image_size > size => {
            return Err(fail(Stage::Slot, Fault::SlotTooSmall));
        }
        Some(_) => {}
    }
    if !playback.stop_for_update() {
        return Err(fail(Stage::StopAudio, Fault::Playback));
    }
    if flash.begin(manifest.image_size).is_err() {
        playback.resume();
        return Err(fail(Stage::BeginFlash, Fault::Flash));
    }
    let result = stream(
        &mut reader,
        flash,
        request,
        &manifest,
        &header,
        &image_head,
        buf,
    );
    if result.is_err() {
        flash.abort();
        playback.resume();
    }
    result
}

fn stream<S: BundleSource, F: SlotFlash>(
    reader: &mut Reader<'_, S>,
    flash: &mut F,
    request: &Request<'_>,
    manifest: &Manifest,
    header: &[u8; HEADER_SIZE],
    image_head: &[u8; IMAGE_HEADER_SIZE],
    buf: &mut [u8],
) -> Result<RebootPermit, Failure> {
    let fail = |stage, fault| Failure { stage, fault };
    let mut image_hash = Sha256::new();
    let mut bundle_hash = Sha256::new();
    bundle_hash.update(header);
    let mut chunk = |data: &[u8], image_hash: &mut Sha256, bundle_hash: &mut Sha256| {
        flash
            .write(data)
            .map_err(|_| fail(Stage::WriteFlash, Fault::Flash))?;
        image_hash.update(data);
        bundle_hash.update(data);
        Ok::<_, Failure>(())
    };
    chunk(image_head, &mut image_hash, &mut bundle_hash)?;
    let mut remaining = manifest.image_size as usize - IMAGE_HEADER_SIZE;
    while remaining > 0 {
        let want = remaining.min(buf.len());
        let got = reader.some(&mut buf[..want], Stage::DownloadImage)?;
        chunk(&buf[..got], &mut image_hash, &mut bundle_hash)?;
        remaining -= got;
    }
    if <[u8; 32]>::from(image_hash.finalize()) != manifest.image_sha256 {
        return Err(fail(Stage::ImageHash, Fault::ImageHash));
    }
    if let Policy::Pull { offer, .. } = request.policy
        && <[u8; 32]>::from(bundle_hash.finalize()) != offer.sha256
    {
        return Err(fail(Stage::ChannelHash, Fault::ChannelHash));
    }
    flash
        .end()
        .map_err(|_| fail(Stage::FinalizeImage, Fault::Flash))?;
    let (project, version) = flash
        .descriptor()
        .map_err(|_| fail(Stage::Descriptor, Fault::Flash))?;
    if !descriptor_text_matches(&project, request.expected.project) {
        return Err(fail(Stage::Descriptor, Fault::WrongProject));
    }
    if !descriptor_text_matches(&version, manifest.version.as_str()) {
        return Err(fail(Stage::Descriptor, Fault::VersionMismatch));
    }
    flash
        .set_boot()
        .map_err(|_| fail(Stage::SetBoot, Fault::Flash))?;
    Ok(RebootPermit {
        version: manifest.version,
    })
}
