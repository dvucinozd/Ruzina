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
//!
//! The deck screen's task may register itself (`serve_task`). Slint allocates and
//! frees thousands of small objects in each frame, and through IDF's locked heap
//! each call took about 2.5 µs (s122). The registered task's small allocations
//! therefore come from `pool`: free lists per size class, carved from one PSRAM
//! arena the task takes once. The allocator also counts the task's allocations
//! and frees and the CPU cycles they take, for the slow-frame log.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

// esp_heap_caps.h at the pinned ESP-IDF v6.1 commit fff9895c.
const MALLOC_CAP_8BIT: u32 = 1 << 2;
const MALLOC_CAP_SPIRAM: u32 = 1 << 10;
const PSRAM: u32 = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;

unsafe extern "C" {
    fn heap_caps_aligned_alloc(alignment: usize, size: usize, caps: u32) -> *mut c_void;
    fn heap_caps_free(ptr: *mut c_void);
    fn xTaskGetCurrentTaskHandle() -> *mut c_void;
}

mod pool;

/// The PSRAM arena the registered task's small allocations come from, in bytes.
const ARENA: usize = 1 << 20;

/// The registered task, 0 for none.
static COUNTED_TASK: AtomicUsize = AtomicUsize::new(0);
/// The counted task's allocations and frees.
static CALLS: AtomicU32 = AtomicU32::new(0);
/// CPU cycles in them, in units of 8 cycles (a frame's worth fits).
static CYCLES_8: AtomicU32 = AtomicU32::new(0);

/// Serves the calling task's small allocations from the pool, and counts its
/// allocations, from now on. Called once, by one task.
#[cfg_attr(any(test, not(feature = "idf-slint-ui")), allow(dead_code))]
pub(crate) fn serve_task() {
    // SAFETY: as for any PSRAM allocation; the arena is never freed.
    let arena: *mut u8 = unsafe { heap_caps_aligned_alloc(pool::ALIGN, ARENA, PSRAM).cast() };
    if !arena.is_null() {
        pool::give(arena, ARENA);
    }
    // SAFETY: FreeRTOS returns the running task's handle; only its value is kept.
    let task = unsafe { xTaskGetCurrentTaskHandle() } as usize;
    COUNTED_TASK.store(task, Ordering::Relaxed);
}

/// The counted task's allocations and frees so far, and the cycles they took in units of
/// 8 (both wrap; take differences).
#[cfg_attr(any(test, not(feature = "idf-slint-ui")), allow(dead_code))]
pub(crate) fn counted() -> (u32, u32) {
    (
        CALLS.load(Ordering::Relaxed),
        CYCLES_8.load(Ordering::Relaxed),
    )
}

fn counting() -> bool {
    let task = COUNTED_TASK.load(Ordering::Relaxed);
    // SAFETY: as in `count_task`.
    task != 0 && task == unsafe { xTaskGetCurrentTaskHandle() } as usize
}

fn cycles() -> u32 {
    #[cfg(target_arch = "riscv32")]
    {
        let cycles: u32;
        // SAFETY: reads the cycle counter CSR (IDF's `rv_utils_get_cycle_count` on the P4).
        unsafe { core::arch::asm!("rdcycle {0}", out(reg) cycles) };
        cycles
    }
    #[cfg(not(target_arch = "riscv32"))]
    0
}

fn note(started: u32) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    CYCLES_8.fetch_add((cycles().wrapping_sub(started) + 4) / 8, Ordering::Relaxed);
}

struct IdfPsram;

// SAFETY: IDF's heap is internally locked and safe to call from any task.
// `heap_caps_aligned_alloc` accepts any power-of-two alignment, which `Layout`
// guarantees, and returns null on failure; `heap_caps_free` releases aligned
// allocations at this IDF version.
//
// The pool's blocks are handed out only to the registered task, which alone takes
// them back directly; any other task returns one through the pool's lock-free
// list (`pool::free`).
unsafe impl GlobalAlloc for IdfPsram {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !counting() {
            return unsafe { heap_caps_aligned_alloc(layout.align(), layout.size(), PSRAM).cast() };
        }
        let started = cycles();
        // SAFETY: this is the registered task, the only one the pool serves.
        let mut ptr = unsafe { pool::alloc(layout) };
        if ptr.is_null() {
            ptr = unsafe { heap_caps_aligned_alloc(layout.align(), layout.size(), PSRAM).cast() };
        }
        note(started);
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let counted = counting();
        let started = cycles();
        if pool::owns(ptr) {
            // SAFETY: `ptr` came from the pool with this layout; `counted` says whether
            // this is the registered task.
            unsafe { pool::free(ptr, layout, counted) };
        } else {
            unsafe { heap_caps_free(ptr.cast()) };
        }
        if counted {
            note(started);
        }
    }
}

// Host tests keep the std allocator and exercise `IdfPsram` through shims.
#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: IdfPsram = IdfPsram;

#[cfg(test)]
mod tests;
