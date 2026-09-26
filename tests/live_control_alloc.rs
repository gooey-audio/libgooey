//! Dedicated allocator probe for render-boundary live-control application.

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
fn applying_prepared_commands_allocates_and_deallocates_nothing_in_render() {
    unsafe {
        let engine = gooey_engine_new(48_000.0);
        let control = gooey_engine_live_control_new(engine);
        let effects = [
            GooeyEffectDescriptor {
                effect: EFFECT_LOWPASS_FILTER,
                params: std::ptr::null(),
                param_count: 0,
            },
            GooeyEffectDescriptor {
                effect: EFFECT_DELAY,
                params: std::ptr::null(),
                param_count: 0,
            },
            GooeyEffectDescriptor {
                effect: EFFECT_REVERB,
                params: std::ptr::null(),
                param_count: 0,
            },
        ];
        assert_ne!(
            gooey_live_control_replace_track_rack(
                control,
                0,
                effects.as_ptr(),
                effects.len() as u32,
            ),
            0
        );
        assert_ne!(gooey_live_control_set_track_gain(control, 0, 0.7), 0);
        assert_ne!(
            gooey_live_control_set_source_trim(control, SOURCE_DRUMKIT, 1.2),
            0
        );
        assert_ne!(
            gooey_live_control_submit_drum_pattern(control, &GooeyDrumPattern::default()),
            0
        );

        let mut output = [0.0_f32; 2];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        DEALLOCATIONS.store(0, Ordering::Relaxed);
        TRACK.store(true, Ordering::SeqCst);
        gooey_engine_render(engine, output.as_mut_ptr(), 1);
        TRACK.store(false, Ordering::SeqCst);

        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
        assert_eq!(DEALLOCATIONS.load(Ordering::Relaxed), 0);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}
