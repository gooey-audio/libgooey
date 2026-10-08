//! Lock-free min/max waveform scope of the post-limiter master output.
//!
//! The render thread owns an [`OutputScopeCapture`] that folds each output
//! frame into a running min/max and publishes one bin per
//! `round(sample_rate / OUTPUT_SCOPE_POINT_COUNT)` frames, so the ring holds
//! about one second of audio. Any thread may read the newest bins through
//! [`OutputScopeBuffer::read`] while the engine renders. Ported from ebb's
//! `waveform.rs`.

use std::array;
use std::sync::atomic::{fence, AtomicU64, Ordering};

use crate::ffi::OUTPUT_SCOPE_POINT_COUNT;

const POINT_COUNT: usize = OUTPUT_SCOPE_POINT_COUNT as usize;
const POINTS_PER_SECOND: f32 = POINT_COUNT as f32;
/// Copies attempted before a reader that keeps getting lapped by the writer
/// blanks the overwritten bins instead of retrying.
const READ_ATTEMPTS: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OutputScopePeak {
    pub min: f32,
    pub max: f32,
}

/// Fixed ring of packed `{min, max}` bins plus a monotonic write position.
pub(crate) struct OutputScopeBuffer {
    peaks: [AtomicU64; POINT_COUNT],
    /// Bins fully published. Readers copy up to this position.
    write_position: AtomicU64,
    /// Bins whose publish has begun (`write_position` or one ahead). Readers
    /// re-check it after copying to detect slots overwritten mid-read.
    publish_started: AtomicU64,
}

impl OutputScopeBuffer {
    pub fn new() -> Self {
        Self {
            peaks: array::from_fn(|_| AtomicU64::new(pack_peak(OutputScopePeak::default()))),
            write_position: AtomicU64::new(0),
            publish_started: AtomicU64::new(0),
        }
    }

    /// Single-writer publish: the slot store is ordered before the position
    /// store, so a reader that observes the new position sees the new bin.
    fn publish(&self, peak: OutputScopePeak) {
        let position = self.write_position.load(Ordering::Relaxed);
        let index = position as usize % POINT_COUNT;
        // Pairs with the fence in `read_once`: a reader whose slot load sees
        // this overwrite is guaranteed to then see `publish_started > position`
        // and discard the slot as belonging to a newer ring generation.
        self.publish_started
            .store(position.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.peaks[index].store(pack_peak(sanitize_peak(peak)), Ordering::Relaxed);
        self.write_position
            .store(position.wrapping_add(1), Ordering::Release);
    }

    /// Copy the newest `min(len, POINT_COUNT)` bins into `out_min` / `out_max`,
    /// oldest first, zero-padding the front when fewer bins have been
    /// published. Returns the total number of bins published so far.
    ///
    /// If the writer laps the reader mid-copy, the copy is retried; after
    /// `READ_ATTEMPTS` the oldest bins it overwrote are zeroed rather than
    /// shown out of order.
    pub fn read(&self, out_min: &mut [f32], out_max: &mut [f32]) -> u64 {
        let count = out_min.len().min(out_max.len()).min(POINT_COUNT);
        let (out_min, out_max) = (&mut out_min[..count], &mut out_max[..count]);
        let mut attempt = 0;
        loop {
            let (end, padding, overwritten) = self.read_once(out_min, out_max);
            attempt += 1;
            if overwritten == 0 {
                return end;
            }
            if attempt == READ_ATTEMPTS {
                out_min[padding..padding + overwritten].fill(0.0);
                out_max[padding..padding + overwritten].fill(0.0);
                return end;
            }
        }
    }

    /// One copy pass. Returns the write position, the padding length, and how
    /// many of the oldest copied bins the writer may have overwritten with a
    /// newer ring generation while they were being read.
    fn read_once(&self, out_min: &mut [f32], out_max: &mut [f32]) -> (u64, usize, usize) {
        let count = out_min.len();
        let end = self.write_position.load(Ordering::Acquire);
        let available = end.min(count as u64) as usize;
        let start = end - available as u64;
        let padding = count - available;

        out_min[..padding].fill(0.0);
        out_max[..padding].fill(0.0);
        for offset in 0..available {
            let index = (start + offset as u64) as usize % POINT_COUNT;
            let peak = unpack_peak(self.peaks[index].load(Ordering::Relaxed));
            out_min[padding + offset] = peak.min;
            out_max[padding + offset] = peak.max;
        }

        // Bin `p` is clobbered once the writer starts publishing
        // `p + POINT_COUNT`, so only bins from `started - POINT_COUNT` on are
        // known to belong to this snapshot.
        fence(Ordering::Acquire);
        let started = self.publish_started.load(Ordering::Relaxed);
        let oldest_intact = started.saturating_sub(POINT_COUNT as u64);
        let overwritten = oldest_intact.saturating_sub(start).min(available as u64) as usize;
        (end, padding, overwritten)
    }
}

/// Render-thread accumulator for the bin currently being filled.
pub(crate) struct OutputScopeCapture {
    samples_per_peak: usize,
    samples_in_peak: usize,
    min: f32,
    max: f32,
}

impl OutputScopeCapture {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            samples_per_peak: samples_per_peak(sample_rate),
            samples_in_peak: 0,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
        }
    }

    /// Rescale the bin width and drop the partial bin. The engine's sample
    /// rate is fixed at construction today; this is the hook for a future
    /// runtime sample-rate change.
    #[allow(dead_code)]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.samples_per_peak = samples_per_peak(sample_rate);
        self.reset();
    }

    #[inline]
    pub fn push_stereo(&mut self, buffer: &OutputScopeBuffer, left: f32, right: f32) {
        let mono = if left.is_finite() && right.is_finite() {
            ((left + right) * 0.5).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        self.min = self.min.min(mono);
        self.max = self.max.max(mono);
        self.samples_in_peak += 1;

        if self.samples_in_peak >= self.samples_per_peak {
            buffer.publish(OutputScopePeak {
                min: self.min,
                max: self.max,
            });
            self.reset();
        }
    }

    /// Drop the partially accumulated bin.
    pub fn reset(&mut self) {
        self.samples_in_peak = 0;
        self.min = f32::INFINITY;
        self.max = f32::NEG_INFINITY;
    }
}

fn samples_per_peak(sample_rate: f32) -> usize {
    let sample_rate = if sample_rate.is_finite() {
        sample_rate.max(1.0)
    } else {
        1.0
    };
    (sample_rate / POINTS_PER_SECOND).round().max(1.0) as usize
}

fn sanitize_peak(peak: OutputScopePeak) -> OutputScopePeak {
    let mut min = if peak.min.is_finite() { peak.min } else { 0.0 }.clamp(-1.0, 1.0);
    let mut max = if peak.max.is_finite() { peak.max } else { 0.0 }.clamp(-1.0, 1.0);
    if min > max {
        std::mem::swap(&mut min, &mut max);
    }
    OutputScopePeak { min, max }
}

fn pack_peak(peak: OutputScopePeak) -> u64 {
    ((peak.min.to_bits() as u64) << 32) | peak.max.to_bits() as u64
}

fn unpack_peak(packed: u64) -> OutputScopePeak {
    OutputScopePeak {
        min: f32::from_bits((packed >> 32) as u32),
        max: f32::from_bits(packed as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(buffer: &OutputScopeBuffer) -> Vec<OutputScopePeak> {
        let mut min = vec![f32::NAN; POINT_COUNT];
        let mut max = vec![f32::NAN; POINT_COUNT];
        buffer.read(&mut min, &mut max);
        min.into_iter()
            .zip(max)
            .map(|(min, max)| OutputScopePeak { min, max })
            .collect()
    }

    #[test]
    fn snapshot_is_zero_padded_and_chronological() {
        let buffer = OutputScopeBuffer::new();
        buffer.publish(OutputScopePeak {
            min: -0.25,
            max: 0.5,
        });
        buffer.publish(OutputScopePeak {
            min: -0.75,
            max: 0.8,
        });

        let peaks = snapshot(&buffer);
        assert_eq!(peaks.len(), POINT_COUNT);
        assert!(peaks[..POINT_COUNT - 2]
            .iter()
            .all(|peak| *peak == OutputScopePeak::default()));
        assert_eq!(peaks[POINT_COUNT - 2].min, -0.25);
        assert_eq!(peaks[POINT_COUNT - 1].max, 0.8);
    }

    #[test]
    fn snapshot_keeps_the_newest_points_after_wraparound() {
        let buffer = OutputScopeBuffer::new();
        for index in 0..POINT_COUNT + 7 {
            let value = index as f32 / (POINT_COUNT + 7) as f32;
            buffer.publish(OutputScopePeak {
                min: -value,
                max: value,
            });
        }

        let peaks = snapshot(&buffer);
        let expected_first = 7.0 / (POINT_COUNT + 7) as f32;
        let expected_last = (POINT_COUNT + 6) as f32 / (POINT_COUNT + 7) as f32;
        assert_eq!(peaks[0].max, expected_first);
        assert_eq!(peaks[POINT_COUNT - 1].max, expected_last);
    }

    #[test]
    fn short_read_returns_the_newest_bins_and_write_position() {
        let buffer = OutputScopeBuffer::new();
        for index in 0..5 {
            let value = index as f32 * 0.1;
            buffer.publish(OutputScopePeak {
                min: -value,
                max: value,
            });
        }

        let mut min = [f32::NAN; 3];
        let mut max = [f32::NAN; 3];
        assert_eq!(buffer.read(&mut min, &mut max), 5);
        assert_eq!(max, [0.2, 0.3, 0.4]);
        assert_eq!(min, [-0.2, -0.3, -0.4]);

        let mut min = [f32::NAN; 8];
        let mut max = [f32::NAN; 8];
        assert_eq!(buffer.read(&mut min, &mut max), 5);
        assert_eq!(max, [0.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn concurrent_reads_never_mix_ring_generations() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        // Bin `p` carries `(p + 1) / 2^24`, exact in f32, so a coherent
        // snapshot is a run of zero padding followed by consecutive steps.
        const STEP: f32 = 1.0 / (1 << 24) as f32;
        const PUBLISHES: u64 = 1 << 22;

        let buffer = Arc::new(OutputScopeBuffer::new());
        let done = Arc::new(AtomicBool::new(false));
        let writer = {
            let buffer = Arc::clone(&buffer);
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                for position in 0..PUBLISHES {
                    let value = (position + 1) as f32 * STEP;
                    buffer.publish(OutputScopePeak {
                        min: -value,
                        max: value,
                    });
                }
                done.store(true, Ordering::Release);
            })
        };

        let mut min = vec![0.0; POINT_COUNT];
        let mut max = vec![0.0; POINT_COUNT];
        let mut reads = 0;
        while !done.load(Ordering::Acquire) || reads == 0 {
            let end = buffer.read(&mut min, &mut max);
            reads += 1;
            let first = max.iter().position(|value| *value != 0.0);
            let Some(first) = first else {
                continue;
            };
            assert_eq!(max[POINT_COUNT - 1], end as f32 * STEP, "newest bin");
            for index in first + 1..POINT_COUNT {
                assert_eq!(
                    max[index] - max[index - 1],
                    STEP,
                    "bin {index} after {reads} reads"
                );
                assert_eq!(min[index], -max[index]);
            }
        }
        writer.join().unwrap();
    }

    #[test]
    fn capture_accumulates_min_and_max_for_each_time_bin() {
        let buffer = OutputScopeBuffer::new();
        let mut capture = OutputScopeCapture::new(4_096.0);
        for value in [-0.2, 0.6, -0.8, 0.3] {
            capture.push_stereo(&buffer, value, value);
        }

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 1],
            OutputScopePeak {
                min: -0.8,
                max: 0.6
            }
        );
    }

    #[test]
    fn silence_advances_the_scope() {
        let buffer = OutputScopeBuffer::new();
        buffer.publish(OutputScopePeak {
            min: -1.0,
            max: 1.0,
        });
        let mut capture = OutputScopeCapture::new(POINTS_PER_SECOND);
        capture.push_stereo(&buffer, 0.0, 0.0);

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 2],
            OutputScopePeak {
                min: -1.0,
                max: 1.0
            }
        );
        assert_eq!(peaks[POINT_COUNT - 1], OutputScopePeak::default());
    }

    #[test]
    fn non_finite_and_out_of_range_values_are_sanitized() {
        let buffer = OutputScopeBuffer::new();
        buffer.publish(OutputScopePeak {
            min: f32::NAN,
            max: 4.0,
        });
        buffer.publish(OutputScopePeak {
            min: 0.8,
            max: -0.6,
        });

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 2],
            OutputScopePeak { min: 0.0, max: 1.0 }
        );
        assert_eq!(
            peaks[POINT_COUNT - 1],
            OutputScopePeak {
                min: -0.6,
                max: 0.8
            }
        );
    }

    #[test]
    fn non_finite_frames_count_as_silence() {
        let buffer = OutputScopeBuffer::new();
        let mut capture = OutputScopeCapture::new(2.0 * POINTS_PER_SECOND);
        capture.push_stereo(&buffer, f32::NAN, 0.5);
        capture.push_stereo(&buffer, 0.5, 0.5);

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 1],
            OutputScopePeak { min: 0.0, max: 0.5 }
        );
    }

    #[test]
    fn sample_rate_changes_reset_partial_bins() {
        let buffer = OutputScopeBuffer::new();
        let mut capture = OutputScopeCapture::new(4_096.0);
        capture.push_stereo(&buffer, 1.0, 1.0);
        capture.set_sample_rate(POINTS_PER_SECOND);
        capture.push_stereo(&buffer, 0.25, 0.25);

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 1],
            OutputScopePeak {
                min: 0.25,
                max: 0.25
            }
        );
    }

    #[test]
    fn reset_drops_the_partial_bin() {
        let buffer = OutputScopeBuffer::new();
        let mut capture = OutputScopeCapture::new(2.0 * POINTS_PER_SECOND);
        capture.push_stereo(&buffer, -1.0, -1.0);
        capture.reset();
        capture.push_stereo(&buffer, 0.25, 0.25);
        capture.push_stereo(&buffer, 0.5, 0.5);

        let peaks = snapshot(&buffer);
        assert_eq!(
            peaks[POINT_COUNT - 1],
            OutputScopePeak {
                min: 0.25,
                max: 0.5
            }
        );
    }
}
