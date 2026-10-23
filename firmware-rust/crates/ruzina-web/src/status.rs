//! Coherent status generations and generation-checked Web loads.
//!
//! Every status response carries one stamp: the boot, a monotonic response
//! sequence, the catalog publication and both decks' load generations, all
//! read under one snapshot. A client applies a response only if its stamp
//! supersedes the last one applied, so a slow response cannot roll the page
//! back. A Web load names the catalog revision it was listed from and is
//! refused (409) once the catalog changed (M2.5 `/api/load?generation=`).

use crate::HttpStatus;
use ruzina_media_catalog::{CatalogLoadCandidate, CatalogPublicationId, CatalogView};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusStamp {
    /// Random per boot; a new boot always supersedes.
    pub boot: u32,
    pub sequence: u32,
    pub catalog: Option<CatalogPublicationId>,
    pub load_generations: [u32; 2],
}

impl StatusStamp {
    /// Client-side stale-response rejection.
    pub fn supersedes(&self, applied: &Self) -> bool {
        self.boot != applied.boot
            || (self.sequence != applied.sequence
                && self.sequence.wrapping_sub(applied.sequence) < 0x8000_0000)
    }
}

/// Server side: stamps snapshots in the order they were taken.
pub struct StatusSequencer {
    boot: u32,
    sequence: u32,
}

impl StatusSequencer {
    pub const fn new(boot: u32) -> Self {
        Self { boot, sequence: 0 }
    }

    pub fn stamp(
        &mut self,
        catalog: Option<CatalogPublicationId>,
        load_generations: [u32; 2],
    ) -> StatusStamp {
        self.sequence = self.sequence.wrapping_add(1);
        StatusStamp {
            boot: self.boot,
            sequence: self.sequence,
            catalog,
            load_generations,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadRefusal {
    NoCatalog,
    /// The page listed the track from another catalog revision.
    StaleRevision,
    UnknownTrack,
}

impl LoadRefusal {
    pub const fn status(self) -> HttpStatus {
        HttpStatus::Conflict
    }
}

/// Resolves a Web load against the current publication. The candidate then
/// enters the same RP-04 admission and RP-06 load path as a controller LOAD.
pub fn resolve_load<V: CatalogView>(
    catalog: Option<&V>,
    track: u32,
    revision: u32,
) -> Result<CatalogLoadCandidate, LoadRefusal> {
    let catalog = catalog.ok_or(LoadRefusal::NoCatalog)?;
    if catalog.revision() != revision {
        return Err(LoadRefusal::StaleRevision);
    }
    let (index, found) = (0..catalog.len())
        .filter_map(|index| catalog.get(index).map(|found| (index, found)))
        .find(|(_, found)| found.source_track_id == track)
        .ok_or(LoadRefusal::UnknownTrack)?;
    Ok(CatalogLoadCandidate {
        publication: CatalogPublicationId {
            lease: catalog.lease(),
            revision: catalog.revision(),
        },
        index: u16::try_from(index).map_err(|_| LoadRefusal::UnknownTrack)?,
        track: *found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzina_core::MediaTrackId;
    use ruzina_media_catalog::{CatalogBuilder, CatalogSnapshot, CatalogTrack};
    use ruzina_media_session::{MediaGeneration, MediaLease, MediaSourceId};

    fn catalog(revision: u32) -> CatalogSnapshot<4> {
        let lease = MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(1).unwrap(),
        };
        let mut builder = CatalogBuilder::<4>::new(lease);
        for (id, source) in [(1u8, 10u32), (2, 42)] {
            builder
                .push(CatalogTrack {
                    track_id: MediaTrackId([id; 16]),
                    source_track_id: source,
                    ..CatalogTrack::EMPTY
                })
                .unwrap();
        }
        builder.finish(revision)
    }

    #[test]
    fn responses_apply_only_in_snapshot_order_or_after_a_reboot() {
        let mut server = StatusSequencer::new(0xabcd);
        let first = server.stamp(None, [0, 0]);
        let second = server.stamp(None, [1, 0]);
        assert!(second.supersedes(&first));
        assert!(
            !first.supersedes(&second),
            "a slow older response is dropped"
        );
        assert!(!second.supersedes(&second));
        let rebooted = StatusSequencer::new(0x1234).stamp(None, [0, 0]);
        assert!(rebooted.supersedes(&second));
        let wrapped = StatusStamp {
            sequence: 0,
            ..StatusStamp {
                sequence: u32::MAX,
                ..first
            }
        };
        assert!(wrapped.supersedes(&StatusStamp {
            sequence: u32::MAX,
            ..first
        }));
    }

    #[test]
    fn loads_resolve_only_against_the_listed_revision() {
        let current = catalog(3);
        let candidate = resolve_load(Some(&current), 42, 3).unwrap();
        assert_eq!(candidate.index, 1);
        assert_eq!(candidate.track.track_id, MediaTrackId([2; 16]));
        assert_eq!(candidate.publication.revision, 3);
        assert_eq!(
            resolve_load(Some(&current), 42, 2),
            Err(LoadRefusal::StaleRevision)
        );
        assert_eq!(
            resolve_load(Some(&current), 77, 3),
            Err(LoadRefusal::UnknownTrack)
        );
        assert_eq!(
            resolve_load::<CatalogSnapshot<4>>(None, 42, 3),
            Err(LoadRefusal::NoCatalog)
        );
        assert_eq!(LoadRefusal::StaleRevision.status(), HttpStatus::Conflict);
    }
}
