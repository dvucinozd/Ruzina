use core::cell::RefCell;

use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, raw::CriticalSectionRawMutex},
    signal::Signal,
};
use pajoniiir_controller_profile_storage::ProfileRegistry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProfileCatalogMatch {
    pub(crate) index: Option<u8>,
    pub(crate) revision: u32,
}

struct ProfileCatalog {
    registry: ProfileRegistry,
    revision: u32,
}

impl ProfileCatalog {
    const fn new() -> Self {
        Self {
            registry: ProfileRegistry::new(),
            revision: 0,
        }
    }

    fn replace(&mut self, registry: ProfileRegistry) -> u32 {
        self.registry = registry;
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.revision = 1;
        }
        self.revision
    }

    fn match_profile(&self, vid: u16, pid: u16) -> ProfileCatalogMatch {
        ProfileCatalogMatch {
            index: self.registry.match_index(vid, pid),
            revision: self.revision,
        }
    }
}

static PROFILE_CATALOG: CriticalSectionMutex<RefCell<ProfileCatalog>> =
    CriticalSectionMutex::new(RefCell::new(ProfileCatalog::new()));
static PROFILE_CATALOG_CHANGED: Signal<CriticalSectionRawMutex, u32> = Signal::new();

pub(crate) fn publish_scanned_registry(registry: ProfileRegistry) {
    let revision = PROFILE_CATALOG.lock(|cell| cell.borrow_mut().replace(registry));
    PROFILE_CATALOG_CHANGED.signal(revision);
}

pub(crate) fn match_scanned_profile(vid: u16, pid: u16) -> ProfileCatalogMatch {
    PROFILE_CATALOG.lock(|cell| cell.borrow().match_profile(vid, pid))
}

pub(crate) async fn wait_for_profile_catalog_change() -> u32 {
    PROFILE_CATALOG_CHANGED.wait().await
}
