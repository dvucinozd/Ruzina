//! IDF heap backing for Rust allocations in the legacy staticlib.
//!
//! Memory policy (ADR-008): library and analysis data live in PSRAM; internal
//! SRAM stays for DMA bounce buffers, queues and task stacks. There is no
//! internal-RAM fallback, so a profile selecting this feature must initialize
//! PSRAM for capability allocation (`CONFIG_SPIRAM`, `CONFIG_SPIRAM_USE_CAPS_ALLOC`,
//! as the memory profile does), or every allocation fails. A failed allocation
//! returns null; Rust's allocation-error handler then panics, and the panic
//! handler aborts into IDF's reset policy. PSRAM placement claims no DMA or
//! cache coherency: these buffers are CPU-only.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

// esp_heap_caps.h at the pinned ESP-IDF v6.1 commit fff9895c.
const MALLOC_CAP_8BIT: u32 = 1 << 2;
const MALLOC_CAP_SPIRAM: u32 = 1 << 10;
const PSRAM: u32 = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;

unsafe extern "C" {
    fn heap_caps_aligned_alloc(alignment: usize, size: usize, caps: u32) -> *mut c_void;
    fn heap_caps_free(ptr: *mut c_void);
}

struct IdfPsram;

// SAFETY: IDF's heap is internally locked and safe to call from any task.
// `heap_caps_aligned_alloc` accepts any power-of-two alignment, which `Layout`
// guarantees, and returns null on failure; `heap_caps_free` releases aligned
// allocations at this IDF version.
unsafe impl GlobalAlloc for IdfPsram {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { heap_caps_aligned_alloc(layout.align(), layout.size(), PSRAM).cast() }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { heap_caps_free(ptr.cast()) }
    }
}

// Host tests keep the std allocator and exercise `IdfPsram` through shims.
#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: IdfPsram = IdfPsram;

#[cfg(test)]
mod tests;
