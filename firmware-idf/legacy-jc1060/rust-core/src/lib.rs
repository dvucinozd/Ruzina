#![no_std]

#[cfg(test)]
extern crate std;

#[cfg(feature = "media-load")]
extern crate alloc;

#[cfg(feature = "media-bridge")]
pub mod media_bridge;

#[cfg(feature = "idf-media-bridge")]
mod idf_media_bridge;

#[cfg(feature = "exfat-fixture")]
pub mod exfat_fixture;

#[cfg(feature = "media-load")]
pub mod media_load;

#[cfg(feature = "idf-media-load")]
mod idf_alloc;

#[cfg(feature = "controller-midi")]
mod controller_midi;

#[cfg(feature = "controller-audio")]
mod controller_audio;

#[cfg(feature = "deck-play")]
mod deck_play;

#[cfg(feature = "deck-mix")]
mod deck_mix;

// This probe reuses the existing HAL-independent product crate. The native
// firmware entry point, esp-hal and esp-rtos are deliberately not linked.
#[unsafe(no_mangle)]
pub extern "C" fn ruzina_legacy_pitch_centipercent(raw: u16) -> i16 {
    ruzina_deck::tempo_centipercent_from_raw(raw, 10)
}

#[cfg(not(test))]
unsafe extern "C" {
    fn abort() -> !;
}

/// Hold live FP registers throughout a long, non-cooperative loop.
///
/// # Safety
/// `active` must point to a live aligned C atomic uint32_t, exclusively used as
/// this task's activity flag. Other tasks may read it atomically. No ownership
/// of the pointer crosses the call. The caller bounds count to 1..=4_000_000.
#[cfg(feature = "memory-task")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_fp_hold(seed: f32, count: u32, active: *mut u32) -> f32 {
    if active.is_null() || !(active as usize).is_multiple_of(4) || count == 0 || count > 4_000_000 {
        return f32::NAN;
    }
    let result: f32;
    // ft0 and fa1 stay live across timer preemption; no calls or voluntary yield.
    // Atomic flag publication follows FP initialization and clears after the loop.
    unsafe {
        core::arch::asm!(
            "fmv.s ft0, fa0",
            "li {tmp}, 1",
            "amoswap.w.rl zero, {tmp}, ({flag})",
            "2:",
            "fadd.s ft0, ft0, fa1",
            "addi {remaining}, {remaining}, -1",
            "bnez {remaining}, 2b",
            "amoswap.w.rl zero, zero, ({flag})",
            "fmv.s fa0, ft0",
            flag = in(reg) active,
            remaining = inout(reg) count => _,
            tmp = out(reg) _,
            inlateout("fa0") seed => result,
            in("fa1") 1.0f32,
            out("ft0") _,
            options(nostack),
        );
    }
    result
}

/// Process one bounded stereo EQ block. C retains allocation/free ownership.
///
/// # Safety
/// Non-null pointers must reference aligned, valid, disjoint arrays of
/// `frames * 2` i16 samples for this synchronous call. No concurrent access is
/// allowed. Pointers are never retained; invalid sizes/alignment/overlap reject.
#[cfg(feature = "memory-task")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ruzina_legacy_eq_block(
    input: *const i16,
    output: *mut i16,
    frames: usize,
) -> i32 {
    if input.is_null()
        || output.is_null()
        || frames == 0
        || frames > 256
        || !(input as usize).is_multiple_of(2)
        || !(output as usize).is_multiple_of(2)
    {
        return -1;
    }
    let bytes = frames * 4;
    let (Some(input_end), Some(output_end)) = (
        (input as usize).checked_add(bytes),
        (output as usize).checked_add(bytes),
    ) else {
        return -1;
    };
    if (input as usize) < output_end && (output as usize) < input_end {
        return -1;
    }
    let samples = unsafe { core::slice::from_raw_parts(input, frames * 2) };
    let result = unsafe { core::slice::from_raw_parts_mut(output, frames * 2) };
    let mut eq = ruzina_audio_dsp::EqState::new(44_100);
    eq.set_raw(6000, 9000, 11000);
    for (src, dst) in samples.chunks_exact(2).zip(result.chunks_exact_mut(2)) {
        let frame = eq.process_pcm_frame(ruzina_audio_dsp::PcmFrame {
            left: src[0],
            right: src[1],
        });
        dst[0] = frame.left;
        dst[1] = frame.right;
    }
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // No unwinding across C: IDF owns termination and reset policy.
    unsafe { abort() }
}
