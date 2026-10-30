//! Host shims record what the allocator asks IDF for. They do not substitute
//! for PSRAM initialization, capacity or heap-integrity evidence on the device.
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::vec::Vec;

struct Shim {
    live: HashMap<usize, Layout>,
    calls: Vec<(usize, usize, u32)>,
    freed: Vec<usize>,
    fail_above: usize,
}

static SHIM: Mutex<Option<Shim>> = Mutex::new(None);

#[unsafe(no_mangle)]
extern "C" fn heap_caps_aligned_alloc(alignment: usize, size: usize, caps: u32) -> *mut c_void {
    let mut guard = SHIM.lock().unwrap();
    let shim = guard.as_mut().unwrap();
    shim.calls.push((alignment, size, caps));
    if size > shim.fail_above {
        return core::ptr::null_mut();
    }
    let layout = Layout::from_size_align(size, alignment).unwrap();
    let ptr = unsafe { std::alloc::alloc(layout) };
    shim.live.insert(ptr as usize, layout);
    ptr.cast()
}

#[unsafe(no_mangle)]
extern "C" fn heap_caps_free(ptr: *mut c_void) {
    let mut guard = SHIM.lock().unwrap();
    let shim = guard.as_mut().unwrap();
    shim.freed.push(ptr as usize);
    let layout = shim
        .live
        .remove(&(ptr as usize))
        .expect("freed a live block");
    unsafe { std::alloc::dealloc(ptr.cast(), layout) };
}

#[test]
fn allocations_request_psram_with_the_layout_alignment_and_fail_closed() {
    *SHIM.lock().unwrap() = Some(Shim {
        live: HashMap::new(),
        calls: Vec::new(),
        freed: Vec::new(),
        fail_above: 1 << 20,
    });
    let allocator = IdfPsram;

    let layout = Layout::from_size_align(33, 64).unwrap();
    let ptr = unsafe { allocator.alloc(layout) };
    assert!(!ptr.is_null());
    assert!((ptr as usize).is_multiple_of(64));
    unsafe { ptr.write_bytes(0xa5, 33) };
    unsafe { allocator.dealloc(ptr, layout) };

    let small = Layout::new::<u64>();
    let word = unsafe { allocator.alloc(small) };
    assert!((word as usize).is_multiple_of(core::mem::align_of::<u64>()));
    unsafe { allocator.dealloc(word, small) };

    let huge = Layout::from_size_align(2 << 20, 4).unwrap();
    assert!(unsafe { allocator.alloc(huge) }.is_null());

    let shim = SHIM.lock().unwrap().take().unwrap();
    let psram = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;
    assert_eq!(
        shim.calls,
        [
            (64, 33, psram),
            (core::mem::align_of::<u64>(), 8, psram),
            (4, 2 << 20, psram),
        ]
    );
    assert_eq!(shim.freed, [ptr as usize, word as usize]);
    assert!(shim.live.is_empty());
}
