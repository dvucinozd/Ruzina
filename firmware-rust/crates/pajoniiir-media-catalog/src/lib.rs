#![no_std]
#![forbid(unsafe_code)]

use core::str;

use pajoniiir_core::MediaTrackId;
use pajoniiir_media_session::MediaLease;

pub const MEDIA_CATALOG_MAX_TRACKS: usize = 1_024;
pub const CATALOG_TITLE_MAX: usize = 128;
pub const CATALOG_ARTIST_MAX: usize = 96;
pub const CATALOG_ALBUM_MAX: usize = 96;
pub const CATALOG_KEY_MAX: usize = 16;
pub const CATALOG_PATH_MAX: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogTextError {
    TooLong { len: usize, max: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogText<const N: usize> {
    bytes: [u8; N],
    len: u16,
}

impl<const N: usize> CatalogText<N> {
    pub const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };

    pub fn try_from_str(value: &str) -> Result<Self, CatalogTextError> {
        let max = N.min(u16::MAX as usize);
        if value.len() > max {
            return Err(CatalogTextError::TooLong {
                len: value.len(),
                max,
            });
        }

        let mut bytes = [0u8; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u16,
        })
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len()]
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(self.as_bytes()).expect("CatalogText is constructible only from valid UTF-8")
    }
}

impl<const N: usize> Default for CatalogText<N> {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogTrack {
    pub track_id: MediaTrackId,
    pub source_track_id: u32,
    pub bpm_x100: u32,
    pub duration_ms: u32,
    pub title: CatalogText<CATALOG_TITLE_MAX>,
    pub artist: CatalogText<CATALOG_ARTIST_MAX>,
    pub album: CatalogText<CATALOG_ALBUM_MAX>,
    pub key: CatalogText<CATALOG_KEY_MAX>,
    pub audio_path: CatalogText<CATALOG_PATH_MAX>,
    pub analysis_path: CatalogText<CATALOG_PATH_MAX>,
}

impl CatalogTrack {
    const EMPTY: Self = Self {
        track_id: MediaTrackId([0; 16]),
        source_track_id: 0,
        bpm_x100: 0,
        duration_ms: 0,
        title: CatalogText::EMPTY,
        artist: CatalogText::EMPTY,
        album: CatalogText::EMPTY,
        key: CatalogText::EMPTY,
        audio_path: CatalogText::EMPTY,
        analysis_path: CatalogText::EMPTY,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogBuildError {
    Full { capacity: usize },
    DuplicateTrackId(MediaTrackId),
    DuplicateSourceTrackId(u32),
}

pub struct CatalogBuilder<const N: usize> {
    lease: MediaLease,
    tracks: [CatalogTrack; N],
    len: usize,
}

impl<const N: usize> CatalogBuilder<N> {
    pub const fn new(lease: MediaLease) -> Self {
        Self {
            lease,
            tracks: [CatalogTrack::EMPTY; N],
            len: 0,
        }
    }

    pub const fn lease(&self) -> MediaLease {
        self.lease
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn is_full(&self) -> bool {
        self.len == N
    }

    pub fn tracks(&self) -> &[CatalogTrack] {
        &self.tracks[..self.len]
    }

    pub fn push(&mut self, track: CatalogTrack) -> Result<usize, CatalogBuildError> {
        if self.len >= N {
            return Err(CatalogBuildError::Full { capacity: N });
        }
        if self
            .tracks()
            .iter()
            .any(|existing| existing.track_id == track.track_id)
        {
            return Err(CatalogBuildError::DuplicateTrackId(track.track_id));
        }
        if self
            .tracks()
            .iter()
            .any(|existing| existing.source_track_id == track.source_track_id)
        {
            return Err(CatalogBuildError::DuplicateSourceTrackId(
                track.source_track_id,
            ));
        }

        let index = self.len;
        self.tracks[index] = track;
        self.len += 1;
        Ok(index)
    }

    pub fn finish(self, revision: u32) -> CatalogSnapshot<N> {
        CatalogSnapshot {
            lease: self.lease,
            revision,
            tracks: self.tracks,
            len: self.len,
        }
    }
}

#[derive(Clone)]
pub struct CatalogSnapshot<const N: usize> {
    lease: MediaLease,
    revision: u32,
    tracks: [CatalogTrack; N],
    len: usize,
}

impl<const N: usize> CatalogSnapshot<N> {
    pub const fn lease(&self) -> MediaLease {
        self.lease
    }

    pub const fn revision(&self) -> u32 {
        self.revision
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn tracks(&self) -> &[CatalogTrack] {
        &self.tracks[..self.len]
    }

    pub fn get(&self, index: usize) -> Option<&CatalogTrack> {
        self.tracks().get(index)
    }

    pub fn find_by_track_id(&self, track_id: MediaTrackId) -> Option<(usize, &CatalogTrack)> {
        self.tracks()
            .iter()
            .enumerate()
            .find(|(_, track)| track.track_id == track_id)
    }

    pub fn find_by_source_track_id(&self, source_track_id: u32) -> Option<(usize, &CatalogTrack)> {
        self.tracks()
            .iter()
            .enumerate()
            .find(|(_, track)| track.source_track_id == source_track_id)
    }
}

pub struct CatalogPublisher<const N: usize> {
    current: Option<CatalogSnapshot<N>>,
}

impl<const N: usize> CatalogPublisher<N> {
    pub const fn new() -> Self {
        Self { current: None }
    }

    pub const fn current(&self) -> Option<&CatalogSnapshot<N>> {
        self.current.as_ref()
    }

    pub fn publish(&mut self, next: CatalogSnapshot<N>) -> Option<CatalogSnapshot<N>> {
        self.current.replace(next)
    }

    pub fn invalidate(&mut self, lease: MediaLease) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|snapshot| snapshot.lease == lease)
        {
            self.current = None;
            true
        } else {
            false
        }
    }
}

impl<const N: usize> Default for CatalogPublisher<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_media_session::{MediaGeneration, MediaSourceId};

    fn lease(source: u32) -> MediaLease {
        MediaLease {
            generation: MediaGeneration::initial(),
            source: MediaSourceId::new(source).unwrap(),
        }
    }

    fn text<const N: usize>(value: &str) -> CatalogText<N> {
        CatalogText::try_from_str(value).unwrap()
    }

    fn track(id: u8, source_track_id: u32, title: &str) -> CatalogTrack {
        CatalogTrack {
            track_id: MediaTrackId([id; 16]),
            source_track_id,
            bpm_x100: 12_834,
            duration_ms: 321_000,
            title: text(title),
            artist: text("Artist"),
            album: text("Album"),
            key: text("8A"),
            audio_path: text("/Contents/test.wav"),
            analysis_path: text("/PIONEER/USBANLZ/P001/00000042/ANLZ0000.DAT"),
        }
    }

    #[test]
    fn bounded_text_rejects_overflow_without_truncation() {
        assert_eq!(
            CatalogText::<4>::try_from_str("hello"),
            Err(CatalogTextError::TooLong { len: 5, max: 4 })
        );
        assert_eq!(
            CatalogText::<4>::try_from_str("test").unwrap().as_str(),
            "test"
        );
    }

    #[test]
    fn builder_rejects_duplicate_identity_and_capacity_overflow() {
        let mut builder = CatalogBuilder::<2>::new(lease(1));
        builder.push(track(1, 10, "One")).unwrap();
        assert_eq!(
            builder.push(track(1, 11, "Duplicate stable")),
            Err(CatalogBuildError::DuplicateTrackId(MediaTrackId([1; 16])))
        );
        assert_eq!(
            builder.push(track(2, 10, "Duplicate source")),
            Err(CatalogBuildError::DuplicateSourceTrackId(10))
        );
        builder.push(track(2, 20, "Two")).unwrap();
        assert_eq!(
            builder.push(track(3, 30, "Three")),
            Err(CatalogBuildError::Full { capacity: 2 })
        );
    }

    #[test]
    fn snapshot_is_immutable_and_lookup_preserves_row_identity() {
        let mut builder = CatalogBuilder::<4>::new(lease(1));
        builder.push(track(1, 10, "One")).unwrap();
        builder.push(track(2, 20, "Two")).unwrap();

        let snapshot = builder.finish(7);
        assert_eq!(snapshot.lease(), lease(1));
        assert_eq!(snapshot.revision(), 7);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.get(1).unwrap().title.as_str(), "Two");
        assert_eq!(
            snapshot.find_by_track_id(MediaTrackId([1; 16])).unwrap().0,
            0
        );
        assert_eq!(snapshot.find_by_source_track_id(20).unwrap().0, 1);
    }

    #[test]
    fn publisher_keeps_old_snapshot_until_new_build_is_complete() {
        let mut publisher = CatalogPublisher::<4>::new();

        let mut first = CatalogBuilder::new(lease(1));
        first.push(track(1, 10, "Old")).unwrap();
        publisher.publish(first.finish(1));
        assert_eq!(
            publisher.current().unwrap().get(0).unwrap().title.as_str(),
            "Old"
        );

        let mut next = CatalogBuilder::new(lease(2));
        next.push(track(2, 20, "New")).unwrap();

        assert_eq!(
            publisher.current().unwrap().get(0).unwrap().title.as_str(),
            "Old"
        );
        let retired = publisher.publish(next.finish(2)).unwrap();
        assert_eq!(retired.revision(), 1);
        assert_eq!(
            publisher.current().unwrap().get(0).unwrap().title.as_str(),
            "New"
        );

        assert!(!publisher.invalidate(lease(1)));
        assert!(publisher.invalidate(lease(2)));
        assert!(publisher.current().is_none());
    }
}
