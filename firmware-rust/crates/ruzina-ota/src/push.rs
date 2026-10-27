//! Incremental local upload. Storage is constant (signed header + image head),
//! independent of image size. Call cancel on every disconnect/deadline/error.
use crate::{
    bundle::{self, Expected, HEADER_SIZE, IMAGE_HEADER_SIZE, Manifest},
    install::{Playback, SlotFlash, descriptor_text_matches},
};
use sha2::{Digest, Sha256};

pub struct Push {
    prefix: [u8; HEADER_SIZE + IMAGE_HEADER_SIZE],
    prefix_len: usize,
    total: usize,
    received: usize,
    manifest: Option<Manifest>,
    hash: Sha256,
    stopped: bool,
    finished: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushError {
    Length,
    Invalid,
    Flash,
    Busy,
    Inactive,
}
impl Push {
    pub fn new(total: usize) -> Result<Self, PushError> {
        if !(HEADER_SIZE + IMAGE_HEADER_SIZE..=HEADER_SIZE + bundle::MAX_IMAGE_SIZE as usize)
            .contains(&total)
        {
            return Err(PushError::Length);
        }
        Ok(Self {
            prefix: [0; HEADER_SIZE + IMAGE_HEADER_SIZE],
            prefix_len: 0,
            total,
            received: 0,
            manifest: None,
            hash: Sha256::new(),
            stopped: false,
            finished: false,
        })
    }
    pub fn feed<F: SlotFlash, P: Playback>(
        &mut self,
        mut bytes: &[u8],
        expected: &Expected<'_>,
        key: &[u8; 65],
        flash: &mut F,
        playback: &mut P,
    ) -> Result<(), PushError> {
        let result = self.feed_inner(&mut bytes, expected, key, flash, playback);
        if result.is_err() {
            self.cancel(flash, playback);
        }
        result
    }
    fn feed_inner<F: SlotFlash, P: Playback>(
        &mut self,
        bytes: &mut &[u8],
        expected: &Expected<'_>,
        key: &[u8; 65],
        flash: &mut F,
        playback: &mut P,
    ) -> Result<(), PushError> {
        if self.finished {
            return Err(PushError::Inactive);
        }
        if bytes.len() > 4096 || bytes.len() > self.total - self.received {
            return Err(PushError::Length);
        }
        self.received += bytes.len();
        if self.prefix_len < self.prefix.len() {
            let take = bytes.len().min(self.prefix.len() - self.prefix_len);
            self.prefix[self.prefix_len..self.prefix_len + take].copy_from_slice(&bytes[..take]);
            self.prefix_len += take;
            *bytes = &bytes[take..];
            if self.prefix_len < self.prefix.len() {
                return Ok(());
            }
            let header = &self.prefix[..HEADER_SIZE];
            let manifest =
                bundle::parse_header(header, expected).map_err(|_| PushError::Invalid)?;
            bundle::verify_signature(header, key).map_err(|_| PushError::Invalid)?;
            if self.total != HEADER_SIZE + manifest.image_size as usize {
                return Err(PushError::Length);
            }
            let image = &self.prefix[HEADER_SIZE..];
            if !bundle::image_header_valid(image, expected.chip_id) {
                return Err(PushError::Invalid);
            }
            if flash
                .update_slot_size()
                .is_none_or(|size| size < manifest.image_size)
            {
                return Err(PushError::Flash);
            }
            if !playback.stop_for_update() {
                return Err(PushError::Busy);
            }
            self.stopped = true;
            flash
                .begin(manifest.image_size)
                .map_err(|_| PushError::Flash)?;
            flash.write(image).map_err(|_| PushError::Flash)?;
            self.hash.update(image);
            self.manifest = Some(manifest);
        }
        if !bytes.is_empty() {
            flash.write(bytes).map_err(|_| PushError::Flash)?;
            self.hash.update(bytes);
        }
        Ok(())
    }
    pub fn finish<F: SlotFlash, P: Playback>(
        &mut self,
        expected: &Expected<'_>,
        flash: &mut F,
        playback: &mut P,
    ) -> Result<crate::install::RebootPermit, PushError> {
        let result = (|| {
            if self.finished {
                return Err(PushError::Inactive);
            }
            if self.received != self.total {
                return Err(PushError::Length);
            }
            let manifest = self.manifest.ok_or(PushError::Invalid)?;
            if <[u8; 32]>::from(self.hash.clone().finalize()) != manifest.image_sha256 {
                return Err(PushError::Invalid);
            }
            flash.end().map_err(|_| PushError::Flash)?;
            let (project, version) = flash.descriptor().map_err(|_| PushError::Flash)?;
            if !descriptor_text_matches(&project, expected.project)
                || !descriptor_text_matches(&version, manifest.version.as_str())
            {
                return Err(PushError::Invalid);
            }
            flash.set_boot().map_err(|_| PushError::Flash)?;
            self.finished = true;
            // Deliberately keep playback stopped until the candidate reboot.
            self.stopped = false;
            Ok(crate::install::RebootPermit::verified(manifest.version))
        })();
        if result.is_err() {
            self.cancel(flash, playback);
        }
        result
    }
    pub fn cancel<F: SlotFlash, P: Playback>(&mut self, flash: &mut F, playback: &mut P) {
        if self.stopped {
            flash.abort();
            playback.resume();
            self.stopped = false;
        }
        self.finished = true;
    }
}
