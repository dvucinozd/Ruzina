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
/// The tests share the shim and the registered task, so they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn shim() {
    *SHIM.lock().unwrap() = Some(Shim {
        live: HashMap::new(),
        calls: Vec::new(),
        freed: Vec::new(),
        fail_above: 1 << 20,
    });
}

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

std::thread_local! {
    static TASK: u8 = const { 0 };
}

#[unsafe(no_mangle)]
extern "C" fn xTaskGetCurrentTaskHandle() -> *mut c_void {
    // Each test thread is its own task.
    TASK.with(|task| core::ptr::from_ref(task).cast_mut().cast())
}

#[test]
fn allocations_request_psram_with_the_layout_alignment_and_fail_closed() {
    let _serial = SERIAL.lock().unwrap();
    shim();
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

#[test]
fn the_registered_task_reuses_pool_blocks_and_others_give_them_back() {
    let _serial = SERIAL.lock().unwrap();
    shim();
    let allocator = IdfPsram;
    serve_task();
    let arena = SHIM.lock().unwrap().as_ref().unwrap().calls[0];
    assert_eq!(arena, (pool::ALIGN, ARENA, PSRAM));

    // Small allocations come from the pool, aligned, and a freed block comes back first.
    let small = Layout::from_size_align(24, 8).unwrap();
    let a = unsafe { allocator.alloc(small) };
    let b = unsafe { allocator.alloc(small) };
    assert!(pool::owns(a) && pool::owns(b) && a != b);
    assert!((a as usize).is_multiple_of(pool::ALIGN));
    unsafe { allocator.dealloc(a, small) };
    assert_eq!(unsafe { allocator.alloc(small) }, a);

    // Too large, or aligned past a block, goes to the heap.
    let large = Layout::from_size_align(pool::MAX_SIZE + 1, 4).unwrap();
    let aligned = Layout::from_size_align(16, 32).unwrap();
    let c = unsafe { allocator.alloc(large) };
    let d = unsafe { allocator.alloc(aligned) };
    assert!(!pool::owns(c) && !pool::owns(d));

    // Another task's small allocation goes to the heap; a pool block it frees comes
    // back to the registered task.
    let given = b as usize;
    let other = std::thread::spawn(move || {
        let mine = unsafe { IdfPsram.alloc(small) };
        unsafe { IdfPsram.dealloc(given as *mut u8, small) };
        mine as usize
    })
    .join()
    .unwrap() as *mut u8;
    assert!(!pool::owns(other));
    assert_eq!(unsafe { allocator.alloc(small) }, b);

    // A used-up arena falls back to the heap.
    let biggest = Layout::from_size_align(pool::MAX_SIZE, 4).unwrap();
    let mut taken = 0;
    while pool::owns(unsafe { allocator.alloc(biggest) }) {
        taken += 1;
    }
    assert!(taken > ARENA / pool::MAX_SIZE - 2 && taken < ARENA / pool::MAX_SIZE);

    for (ptr, layout) in [
        (a, small),
        (b, small),
        (c, large),
        (d, aligned),
        (other, small),
    ] {
        unsafe { allocator.dealloc(ptr, layout) };
    }
    COUNTED_TASK.store(0, Ordering::Relaxed);
    pool::reset();
    let shim = SHIM.lock().unwrap().take().unwrap();
    // The arena, `c`, `d`, `other` and the fallback block are all that reached the heap.
    assert_eq!(shim.calls.len(), 5);
    assert_eq!(shim.freed.len(), 3);
}
