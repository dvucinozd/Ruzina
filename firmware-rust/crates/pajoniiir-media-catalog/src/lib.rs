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
pub struct CatalogPublicationId {
    pub lease: MediaLease,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogLoadCandidate {
    pub publication: CatalogPublicationId,
    pub index: u16,
    pub track: CatalogTrack,
}

pub trait CatalogView {
    fn lease(&self) -> MediaLease;
    fn revision(&self) -> u32;
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&CatalogTrack>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogSelection {
    publication: Option<CatalogPublicationId>,
    selected: u16,
    count: u16,
}

impl CatalogSelection {
    pub const fn new() -> Self {
        Self {
            publication: None,
            selected: 0,
            count: 0,
        }
    }

    pub const fn publication(&self) -> Option<CatalogPublicationId> {
        self.publication
    }

    pub const fn selected_index(&self) -> Option<u16> {
        if self.count == 0 {
            None
        } else {
            Some(self.selected)
        }
    }

    pub const fn count(&self) -> u16 {
        self.count
    }

    pub fn clear(&mut self) {
        self.publication = None;
        self.selected = 0;
        self.count = 0;
    }

    pub fn publish<V: CatalogView>(&mut self, snapshot: &V) {
        let count = snapshot.len().min(u16::MAX as usize) as u16;
        let publication = CatalogPublicationId {
            lease: snapshot.lease(),
            revision: snapshot.revision(),
        };
        let same_publication = self.publication == Some(publication);
        self.publication = Some(publication);
        self.count = count;
        if count == 0 {
            self.selected = 0;
        } else if !same_publication {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    pub fn invalidate(&mut self, lease: MediaLease) {
        if self.publication.is_some_and(|current| current.lease == lease) {
            self.publication = None;
            self.selected = 0;
            self.count = 0;
        }
    }

    pub fn browse(&mut self, delta: i16, multiplier: u16) -> bool {
        if self.count == 0 || delta == 0 {
            return false;
        }

        let step = i32::from(delta).saturating_mul(i32::from(multiplier.max(1)));
        let last = i32::from(self.count - 1);
        let next = i32::from(self.selected).saturating_add(step).clamp(0, last) as u16;
        let changed = next != self.selected;
        self.selected = next;
        changed
    }

    pub fn resolve<V: CatalogView>(&self, snapshot: &V) -> Option<CatalogLoadCandidate> {
        let publication = self.publication?;
        if publication.lease != snapshot.lease()
            || publication.revision != snapshot.revision()
            || self.count as usize != snapshot.len()
        {
            return None;
        }
        let track = *snapshot.get(self.selected as usize)?;
        Some(CatalogLoadCandidate {
            publication,
            index: self.selected,
            track,
        })
    }
}

impl Default for CatalogSelection {
    fn default() -> Self {
        Self::new()
    }
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

impl<const N: usize> CatalogView for CatalogSnapshot<N> {
    fn lease(&self) -> MediaLease {
        self.lease()
    }

    fn revision(&self) -> u32 {
        self.revision()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> Option<&CatalogTrack> {
        self.get(index)
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

/// Storage contract for a bounded catalog bank.
///
/// Implementations may place the backing storage in internal RAM, PSRAM or a
/// host-owned fixture. `push_prechecked` is called only after the builder has
/// verified capacity and duplicate identities.
pub trait CatalogBank {
    fn capacity(&self) -> usize;
    fn tracks(&self) -> &[CatalogTrack];
    fn clear(&mut self);
    fn push_prechecked(&mut self, track: CatalogTrack) -> usize;
}

/// Storage-generic transactional builder used when the catalog backing cannot
/// live inline in the builder value (for example, a large PSRAM allocation).
pub struct CatalogBankBuilder<B> {
    lease: MediaLease,
    bank: B,
}

impl<B: CatalogBank> CatalogBankBuilder<B> {
    pub fn new(lease: MediaLease, mut bank: B) -> Self {
        bank.clear();
        Self { lease, bank }
    }

    pub const fn lease(&self) -> MediaLease {
        self.lease
    }

    pub fn len(&self) -> usize {
        self.bank.tracks().len()
    }

    pub fn capacity(&self) -> usize {
        self.bank.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    pub fn tracks(&self) -> &[CatalogTrack] {
        self.bank.tracks()
    }

    pub fn push(&mut self, track: CatalogTrack) -> Result<usize, CatalogBuildError> {
        let capacity = self.capacity();
        if self.len() >= capacity {
            return Err(CatalogBuildError::Full { capacity });
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

        let expected = self.len();
        let index = self.bank.push_prechecked(track);
        debug_assert_eq!(index, expected);
        debug_assert_eq!(self.len(), expected + 1);
        Ok(index)
    }

    pub fn finish(self, revision: u32) -> CatalogBankSnapshot<B> {
        CatalogBankSnapshot {
            lease: self.lease,
            revision,
            bank: self.bank,
        }
    }

    pub fn into_bank(self) -> B {
        self.bank
    }
}

#[derive(Clone)]
pub struct CatalogBankSnapshot<B> {
    lease: MediaLease,
    revision: u32,
    bank: B,
}

impl<B: CatalogBank> CatalogBankSnapshot<B> {
    pub const fn lease(&self) -> MediaLease {
        self.lease
    }

    pub const fn revision(&self) -> u32 {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.bank.tracks().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn tracks(&self) -> &[CatalogTrack] {
        self.bank.tracks()
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

    pub fn into_bank(self) -> B {
        self.bank
    }
}

impl<B: CatalogBank> CatalogView for CatalogBankSnapshot<B> {
    fn lease(&self) -> MediaLease {
        self.lease()
    }

    fn revision(&self) -> u32 {
        self.revision()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> Option<&CatalogTrack> {
        self.get(index)
    }
}

pub struct CatalogBankPublisher<B> {
    current: Option<CatalogBankSnapshot<B>>,
}

impl<B: CatalogBank> CatalogBankPublisher<B> {
    pub const fn new() -> Self {
        Self { current: None }
    }

    pub const fn current(&self) -> Option<&CatalogBankSnapshot<B>> {
        self.current.as_ref()
    }

    pub fn publish(&mut self, next: CatalogBankSnapshot<B>) -> Option<CatalogBankSnapshot<B>> {
        self.current.replace(next)
    }

    pub fn take_current(&mut self) -> Option<CatalogBankSnapshot<B>> {
        self.current.take()
    }

    pub fn take_if_lease(&mut self, lease: MediaLease) -> Option<CatalogBankSnapshot<B>> {
        if self
            .current
            .as_ref()
            .is_some_and(|snapshot| snapshot.lease == lease)
        {
            self.current.take()
        } else {
            None
        }
    }

    pub fn invalidate(&mut self, lease: MediaLease) -> bool {
        self.take_if_lease(lease).is_some()
    }
}

impl<B: CatalogBank> Default for CatalogBankPublisher<B> {
    fn default() -> Self {
        Self::new()
    }
}

pub const MEDIA_CATALOG_BANK_BUDGET_BYTES: usize = 1_024 * 1_024;
pub const MEDIA_CATALOG_TRANSACTION_BUDGET_BYTES: usize = 2 * 1_024 * 1_024;
pub const MEDIA_CATALOG_BUILDER_BYTES: usize =
    core::mem::size_of::<CatalogBuilder<MEDIA_CATALOG_MAX_TRACKS>>();
pub const MEDIA_CATALOG_PUBLISHER_BYTES: usize =
    core::mem::size_of::<CatalogPublisher<MEDIA_CATALOG_MAX_TRACKS>>();
pub const MEDIA_CATALOG_TRANSACTION_BYTES: usize =
    MEDIA_CATALOG_BUILDER_BYTES + MEDIA_CATALOG_PUBLISHER_BYTES;
pub const MEDIA_CATALOG_TRACK_STORAGE_BYTES: usize =
    core::mem::size_of::<CatalogTrack>() * MEDIA_CATALOG_MAX_TRACKS;

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
    fn released_catalog_layout_stays_inside_transactional_memory_budget() {
        const {
            assert!(MEDIA_CATALOG_BUILDER_BYTES <= MEDIA_CATALOG_BANK_BUDGET_BYTES);
            assert!(MEDIA_CATALOG_PUBLISHER_BYTES <= MEDIA_CATALOG_BANK_BUDGET_BYTES);
            assert!(MEDIA_CATALOG_TRANSACTION_BYTES <= MEDIA_CATALOG_TRANSACTION_BUDGET_BYTES);
            assert!(MEDIA_CATALOG_TRACK_STORAGE_BYTES <= MEDIA_CATALOG_BANK_BUDGET_BYTES);
        }
    }

    #[derive(Clone)]
    struct TestBank<const N: usize> {
        tracks: [CatalogTrack; N],
        len: usize,
    }

    impl<const N: usize> TestBank<N> {
        const fn new() -> Self {
            Self {
                tracks: [CatalogTrack::EMPTY; N],
                len: 0,
            }
        }
    }

    impl<const N: usize> CatalogBank for TestBank<N> {
        fn capacity(&self) -> usize {
            N
        }

        fn tracks(&self) -> &[CatalogTrack] {
            &self.tracks[..self.len]
        }

        fn clear(&mut self) {
            self.len = 0;
        }

        fn push_prechecked(&mut self, track: CatalogTrack) -> usize {
            assert!(self.len < N);
            let index = self.len;
            self.tracks[index] = track;
            self.len += 1;
            index
        }
    }

    #[test]
    fn storage_backed_catalog_banks_publish_and_recycle_transactionally() {
        let mut publisher = CatalogBankPublisher::<TestBank<2>>::new();

        let mut first = CatalogBankBuilder::new(lease(1), TestBank::new());
        first.push(track(1, 10, "Old")).unwrap();
        assert!(publisher.publish(first.finish(1)).is_none());

        let mut next = CatalogBankBuilder::new(lease(2), TestBank::new());
        next.push(track(2, 20, "New")).unwrap();
        assert_eq!(
            publisher.current().unwrap().get(0).unwrap().title.as_str(),
            "Old"
        );

        let retired = publisher.publish(next.finish(2)).unwrap();
        assert_eq!(retired.revision(), 1);
        let recycled = retired.into_bank();

        let rebuilt = CatalogBankBuilder::new(lease(3), recycled);
        assert!(rebuilt.is_empty());
        assert_eq!(rebuilt.capacity(), 2);
        assert_eq!(
            publisher.current().unwrap().get(0).unwrap().title.as_str(),
            "New"
        );
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
    #[test]
    fn selection_is_bound_to_lease_and_revision() {
        let mut builder = CatalogBuilder::<3>::new(lease(1));
        builder.push(track(1, 101, "/a.wav")).unwrap();
        builder.push(track(2, 102, "/b.wav")).unwrap();
        builder.push(track(3, 103, "/c.wav")).unwrap();
        let snapshot = builder.finish(7);

        let mut selection = CatalogSelection::new();
        selection.publish(&snapshot);
        assert_eq!(selection.selected_index(), Some(0));
        assert!(selection.browse(1, 1));
        assert_eq!(selection.selected_index(), Some(1));
        let candidate = selection.resolve(&snapshot).unwrap();
        assert_eq!(candidate.publication.revision, 7);
        assert_eq!(candidate.index, 1);
        assert_eq!(candidate.track.source_track_id, 102);

        let mut replacement = CatalogBuilder::<3>::new(lease(1));
        replacement.push(track(4, 201, "/d.wav")).unwrap();
        let replacement = replacement.finish(8);
        assert_eq!(selection.resolve(&replacement), None);

        selection.publish(&replacement);
        assert_eq!(selection.selected_index(), Some(0));
        assert_eq!(selection.resolve(&replacement).unwrap().track.source_track_id, 201);
    }

    #[test]
    fn selection_clamps_normal_and_accelerated_browse() {
        let mut builder = CatalogBuilder::<4>::new(lease(2));
        for (id, source, path) in [
            (1, 1, "/1.wav"),
            (2, 2, "/2.wav"),
            (3, 3, "/3.wav"),
            (4, 4, "/4.wav"),
        ] {
            builder.push(track(id, source, path)).unwrap();
        }
        let snapshot = builder.finish(1);
        let mut selection = CatalogSelection::new();
        selection.publish(&snapshot);

        assert!(selection.browse(1, 1));
        assert_eq!(selection.selected_index(), Some(1));
        assert!(selection.browse(1, 10));
        assert_eq!(selection.selected_index(), Some(3));
        assert!(selection.browse(-1, 10));
        assert_eq!(selection.selected_index(), Some(0));
        assert!(!selection.browse(-1, 1));
    }

    #[test]
    fn selection_invalidation_is_generation_scoped() {
        let mut builder = CatalogBuilder::<1>::new(lease(3));
        builder.push(track(1, 1, "/one.wav")).unwrap();
        let snapshot = builder.finish(1);
        let mut selection = CatalogSelection::new();
        selection.publish(&snapshot);

        selection.invalidate(lease(4));
        assert!(selection.resolve(&snapshot).is_some());

        selection.invalidate(lease(3));
        assert_eq!(selection.publication(), None);
        assert_eq!(selection.selected_index(), None);
        assert_eq!(selection.resolve(&snapshot), None);
    }

}
