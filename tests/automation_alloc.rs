//! Allocator probe: macros and motions never allocate or free on the render thread.

use gooey::ffi::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn macros_and_motions_render_without_allocating() {
    unsafe {
        let engine = gooey_engine_new(48_000.0);
        let mut output = vec![0.0_f32; 256 * 2];
        let render = |output: &mut Vec<f32>| {
            TRACK.store(true, Ordering::SeqCst);
            gooey_engine_render(engine, output.as_mut_ptr(), 256);
            TRACK.store(false, Ordering::SeqCst);
        };
        // Warm up lazily initialized render state before measuring.
        render(&mut output);

        let targets = [
            (PARAM_TARGET_POLY, 0, POLY_PARAM_FILTER_CUTOFF, 0.9, 0.1),
            (
                PARAM_TARGET_DRUM,
                INSTRUMENT_HIHAT,
                HIHAT_PARAM_TONE,
                0.2,
                0.9,
            ),
            (PARAM_TARGET_DRUM, INSTRUMENT_TOM, TOM_PARAM_DECAY, 0.2, 0.9),
            (
                PARAM_TARGET_GLOBAL_EFFECT,
                EFFECT_LOWPASS_FILTER,
                FILTER_PARAM_CUTOFF,
                500.0,
                8000.0,
            ),
            (
                PARAM_TARGET_GLOBAL_EFFECT,
                EFFECT_PLATE_REVERB,
                PLATE_PARAM_SIZE,
                0.2,
                0.8,
            ),
        ];
        for (macro_index, &(kind, index, param, from, to)) in targets.iter().enumerate() {
            assert!(gooey_engine_macro_add_mapping(
                engine,
                macro_index as u32,
                kind,
                index,
                param,
                from,
                to
            ));
            let slot = macro_index as u32;
            assert!(gooey_engine_motion_configure(
                engine,
                slot,
                macro_index as u32,
                1.0
            ));
            assert!(gooey_engine_motion_set_duration(
                engine,
                slot,
                MOTION_DURATION_MS,
                20.0
            ));
            assert!(gooey_engine_motion_set_end_mode(
                engine,
                slot,
                slot % 3 // hold, return, snap back
            ));
        }
        assert!(gooey_engine_macro_set_value(engine, 4, 0.5));
        for slot in 0..targets.len() as u32 {
            assert!(gooey_engine_motion_trigger(engine, slot));
        }

        ALLOCATIONS.store(0, Ordering::Relaxed);
        DEALLOCATIONS.store(0, Ordering::Relaxed);
        // Apply commands, run, and complete every motion (20-40 ms).
        for _ in 0..16 {
            render(&mut output);
        }
        assert!(gooey_engine_motion_stop_all(engine));
        render(&mut output);

        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
        assert_eq!(DEALLOCATIONS.load(Ordering::Relaxed), 0);
        for slot in 0..targets.len() as u32 {
            assert_eq!(
                gooey_engine_motion_get_state(engine, slot),
                MOTION_STATE_IDLE
            );
        }
        // The hold motion finished at its target; return/snap-back ones came home.
        assert_eq!(gooey_engine_macro_get_value(engine, 0), 1.0);
        assert_eq!(gooey_engine_macro_get_value(engine, 1), 0.0);
        assert_eq!(gooey_engine_macro_get_value(engine, 2), 0.0);
        assert_eq!(gooey_engine_macro_get_value(engine, 4), 0.5);

        gooey_engine_free(engine);
    }
}
