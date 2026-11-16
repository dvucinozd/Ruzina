//! Free lists per size class for one task's small allocations.
//!
//! The arena is carved in 16-byte steps up to `MAX_SIZE`. A freed block goes back to
//! the list of its class and is handed out again first, so a frame's objects reuse
//! the blocks the last frame freed. Blocks never go back to the IDF heap.
//!
//! Only the registered task takes blocks, bumps the arena and pushes to or pops from
//! its own lists, so those need no lock. Another task that frees a block pushes it on
//! its class's `remote` list with a compare-and-swap; the registered task takes that
//! whole list at once when its own list is empty.

use core::alloc::Layout;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// The class step and the alignment of every block.
pub(super) const ALIGN: usize = 16;
/// The largest allocation the pool serves.
pub(super) const MAX_SIZE: usize = 256;
const CLASSES: usize = MAX_SIZE / ALIGN;

/// The arena: `[START, END)`; `NEXT` is where the next new block is carved.
static START: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Each class's free blocks, linked through their first word: the registered task's
/// own, and those other tasks gave back.
static OWN: [AtomicPtr<u8>; CLASSES] = [const { AtomicPtr::new(null_mut()) }; CLASSES];
static REMOTE: [AtomicPtr<u8>; CLASSES] = [const { AtomicPtr::new(null_mut()) }; CLASSES];

/// Gives the pool its arena, `size` bytes at `arena`, aligned to `ALIGN`.
pub(super) fn give(arena: *mut u8, size: usize) {
    let start = arena as usize;
    NEXT.store(start, Ordering::Relaxed);
    END.store(start + size, Ordering::Relaxed);
    // Published last: `owns` reads `START` first.
    START.store(start, Ordering::Release);
}

/// Whether `ptr` is one of the pool's blocks.
pub(super) fn owns(ptr: *mut u8) -> bool {
    let start = START.load(Ordering::Acquire);
    start != 0 && (start..END.load(Ordering::Relaxed)).contains(&(ptr as usize))
}

fn class(layout: Layout) -> Option<usize> {
    (layout.size() <= MAX_SIZE && layout.align() <= ALIGN)
        .then(|| layout.size().max(1).div_ceil(ALIGN) - 1)
}

/// A block for `layout`, or null when the pool does not serve it or is used up.
///
/// # Safety
/// Only the registered task may call this.
pub(super) unsafe fn alloc(layout: Layout) -> *mut u8 {
    let Some(class) = class(layout) else {
        return null_mut();
    };
    let mut block = OWN[class].load(Ordering::Relaxed);
    if block.is_null() {
        block = REMOTE[class].swap(null_mut(), Ordering::Acquire);
    }
    if !block.is_null() {
        // SAFETY: a free block's first word holds the next free block.
        let next = unsafe { block.cast::<*mut u8>().read() };
        OWN[class].store(next, Ordering::Relaxed);
        return block;
    }
    let size = (class + 1) * ALIGN;
    let next = NEXT.load(Ordering::Relaxed);
    if next == 0 || next + size > END.load(Ordering::Relaxed) {
        return null_mut();
    }
    NEXT.store(next + size, Ordering::Relaxed);
    next as *mut u8
}

/// Takes back a block. `own` is true on the registered task.
///
/// # Safety
/// `block` came from `alloc` with `layout`'s class and is not used again.
pub(super) unsafe fn free(block: *mut u8, layout: Layout, own: bool) {
    // A block the pool handed out has a class.
    let Some(class) = class(layout) else { return };
    let link = block.cast::<*mut u8>();
    if own {
        // SAFETY: the block is free and at least one word long.
        unsafe { link.write(OWN[class].load(Ordering::Relaxed)) };
        OWN[class].store(block, Ordering::Relaxed);
        return;
    }
    let mut head = REMOTE[class].load(Ordering::Relaxed);
    loop {
        // SAFETY: as above; nothing else holds the block.
        unsafe { link.write(head) };
        match REMOTE[class].compare_exchange_weak(head, block, Ordering::Release, Ordering::Relaxed)
        {
            Ok(_) => return,
            Err(now) => head = now,
        }
    }
}

#[cfg(test)]
pub(super) fn reset() {
    START.store(0, Ordering::Relaxed);
    END.store(0, Ordering::Relaxed);
    NEXT.store(0, Ordering::Relaxed);
    for list in OWN.iter().chain(&REMOTE) {
        list.store(null_mut(), Ordering::Relaxed);
    }
}
