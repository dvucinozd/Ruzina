//! Product load preflight over the IDF SD owner's single exFAT volume.
//!
//! The shared `ruzina-media-load` preflight (audio stat, ANLZ DAT/EXT, source
//! cues) runs through `FsLoadFiles`, which checks the live lease around every
//! filesystem operation, cache hits included. The caller keeps the catalog,
//! the deck's load gate and the final commit. Analysis publication allocates;
//! the `idf-media-load` target form takes those allocations from the IDF PSRAM
//! heap (`idf_alloc`); the `sd-rust-load` profile selects it.
#![forbid(unsafe_code)]

use crate::exfat_fixture::Volume;
use alloc::{boxed::Box, vec};
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId};
use ruzina_media_fs::{AsyncFileSystem, LeaseBound};
use ruzina_media_identity::{VolumeIdentity, derive_exfat_volume_identity};
use ruzina_media_io::OwnedBlockDevice;
use ruzina_media_load::pdb::{
    FsCatalogError, FsCatalogFiles, ImportError, PdbNameStage, PdbProbe, VecCatalogStage,
    VecNameMap, probe_export_pdb,
};
use ruzina_media_load::{
    ANLZ_READ_BYTES, FsLoadFiles, LoadFiles, SourceAnalysis, load_source_analysis,
    validate_audio_source,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The lease or catalog publication ended; discard everything read.
    Stale,
    /// The audio path is not a non-empty regular file on this volume.
    Source,
}

pub type Prepared = SourceAnalysis<[u8; ANLZ_READ_BYTES]>;

/// Validate the candidate's audio file and import its analysis under one live
/// lease. Only a lease and publication still current after the last read
/// return `Ok`; the caller must recheck both again at its own commit. A
/// canceled preflight leaves the client's pending ticket for drain, as a
/// canceled mount does; discard the volume and never resume its cursors.
pub async fn preflight<D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    volume: &mut Volume<D>,
    candidate: CatalogLoadCandidate,
    generation: u32,
    is_current: &impl Fn() -> bool,
    publication_current: &impl Fn(CatalogPublicationId) -> bool,
) -> Result<Prepared, Error> {
    let mut files: FsLoadFiles<'_, _, _, _, [u8; ANLZ_READ_BYTES]> =
        FsLoadFiles::new(volume, is_current, publication_current);
    if !files.publication_current(candidate.publication) {
        return Err(Error::Stale);
    }
    if !validate_audio_source(&mut files, &candidate).await {
        if !files.publication_current(candidate.publication) {
            return Err(Error::Stale);
        }
        return Err(Error::Source);
    }
    let prepared =
        load_source_analysis(&mut files, candidate, [0; ANLZ_READ_BYTES], generation).await;
    if !files.publication_current(candidate.publication) {
        return Err(Error::Stale);
    }
    Ok(prepared)
}

/// The mounted volume's identity, derived as the native USB0 owner derives it
/// (`ruzina.volume.exfat.v1`), so track ids match across platforms.
pub fn volume_identity<D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    volume: &Volume<D>,
) -> VolumeIdentity {
    let boot = volume.boot_info();
    derive_exfat_volume_identity(
        boot.volume_serial,
        boot.bytes_per_sector,
        boot.sectors_per_cluster,
        boot.volume_length,
        volume.device().range().first_block(),
    )
}

pub type CatalogImportError<D> = ImportError<FsCatalogError<<Volume<D> as AsyncFileSystem>::Error>>;

/// Import `export.pdb` from the volume into `stage` under the volume's lease,
/// with the lease checked around every read. A failed or stale import
/// publishes nothing; `stage` keeps no partial catalog. The caller prepares
/// the stage for the lease and clears `names` first, and invalidates the
/// stage when the lease ends.
pub async fn import_catalog<D: OwnedBlockDevice<Buffer = [u8; 512]>>(
    volume: &mut Volume<D>,
    volume_identity: VolumeIdentity,
    is_current: &impl Fn() -> bool,
    names: &mut PdbNameStage<VecNameMap>,
    stage: &mut VecCatalogStage,
) -> Result<PdbProbe, CatalogImportError<D>> {
    let lease = volume.lease();
    let mut files = FsCatalogFiles::new(volume, is_current);
    // The page buffer moves through every nested read by value; boxed, each
    // move is a pointer instead of 4 KiB of stack temporaries per await.
    let buffer: Box<[u8]> = vec![0u8; 4096].into_boxed_slice();
    let (_, result) =
        probe_export_pdb(&mut files, lease, volume_identity, buffer, names, stage).await;
    result
}

#[cfg(test)]
mod tests;
