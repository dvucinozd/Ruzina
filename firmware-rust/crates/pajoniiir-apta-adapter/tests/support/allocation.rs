// Test-only allocator instrumentation; production crates forbid unsafe code.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! { static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Counter;
// SAFETY: forwards the exact allocation contract to System; the thread-local
// scalar counter allocates nothing and never inspects allocation memory.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static COUNTER: Counter = Counter;
pub fn start() {
    ALLOCATIONS.with(|c| c.set(Some(0)));
}
pub fn finish() {
    assert_eq!(ALLOCATIONS.with(|c| c.replace(None)), Some(0));
}
