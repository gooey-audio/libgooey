//! The new channel voices must not allocate or free during rendering.

use gooey::ffi::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn both_channel_voices_render_with_locks_lfos_and_steps_without_allocation() {
    unsafe {
        let engine = gooey_engine_new(44_100.0);
        let mut output = [0.0_f32; 8192];
        for (kind, param) in [
            (INSTRUMENT_RESONATOR, RESONATOR_PARAM_PITCH),
            (INSTRUMENT_TWIN_CORE, TWIN_CORE_PARAM_TUNE),
        ] {
            gooey_engine_set_channel_instrument_type(engine, 1, kind);
            gooey_engine_set_channel_param_lock(engine, 1, param, 0.4);
            gooey_engine_sequencer_set_instrument_step_with_velocity(engine, 1, 0, true, 0.8);
            gooey_engine_sequencer_set_instrument_step_blend(engine, 1, 0, 0.9, 0.1);
            gooey_engine_sequencer_start(engine);
            gooey_engine_clear_lfo_routes(engine, 0);
            gooey_engine_add_lfo_route(engine, 0, 1, param, 0.5);
            gooey_engine_set_lfo_enabled(engine, 0, true);
            gooey_engine_render(engine, output.as_mut_ptr(), 4096); // warm up
            gooey_engine_trigger_channel(engine, 1);

            ALLOCS.store(0, Ordering::Relaxed);
            FREES.store(0, Ordering::Relaxed);
            TRACK.store(true, Ordering::SeqCst);
            gooey_engine_render(engine, output.as_mut_ptr(), 4096);
            TRACK.store(false, Ordering::SeqCst);
            assert_eq!(ALLOCS.load(Ordering::Relaxed), 0, "type {kind} allocated");
            assert_eq!(FREES.load(Ordering::Relaxed), 0, "type {kind} freed");
        }
        gooey_engine_free(engine);
    }
}
