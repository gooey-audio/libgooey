//! Probe the adapter/scope seam, not the legacy Engine's allocating sample path.
#![cfg(feature = "gui")]
use gooey::gui::{BlockRenderer, InterleavedAdapter, Telemetry, BLOCK_SIZE};
use gooey::StereoFrame;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Probe;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Probe {
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
static GLOBAL: Probe = Probe;

#[test]
fn interleaved_adapter_and_bounded_publication_allocate_nothing_per_block() {
    let telemetry = Telemetry::default();
    let mut renderer = InterleavedAdapter::new(|out: &mut [f32], _: f32| out.fill(0.25));
    let mut frames = [StereoFrame::default(); BLOCK_SIZE];
    TRACK.store(true, Ordering::SeqCst);
    for _ in 0..10_000 {
        renderer.render(&mut frames, 48_000.0);
        telemetry.publish(&mut frames);
    }
    TRACK.store(false, Ordering::SeqCst);
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(DEALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(
        telemetry.health.frames.load(Ordering::Relaxed),
        (BLOCK_SIZE * 10_000) as u64
    );
}
