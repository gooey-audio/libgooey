#[cfg(feature = "visualization")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "visualization")]
use std::collections::VecDeque;

#[cfg(feature = "visualization")]
pub mod waveform_display;

#[cfg(feature = "visualization")]
pub mod spectrogram;

#[cfg(feature = "visualization")]
pub use waveform_display::{DisplayEvent, WaveformDisplay};

#[cfg(feature = "visualization")]
pub use spectrogram::SpectrogramAnalyzer;

/// Thread-safe circular buffer for storing audio samples
#[cfg(feature = "visualization")]
pub struct AudioBuffer {
    samples: Arc<Mutex<VecDeque<f32>>>,
    capacity: usize,
}

#[cfg(feature = "visualization")]
impl AudioBuffer {
    /// Create a new audio buffer with the given capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// Push a sample into the buffer (thread-safe)
    pub fn push(&self, sample: f32) {
        let mut samples = self.samples.lock().unwrap();
        if samples.len() >= self.capacity {
            samples.pop_front();
        }
        samples.push_back(sample);
    }

    /// Get a snapshot of current samples (thread-safe)
    pub fn get_samples(&self) -> Vec<f32> {
        let samples = self.samples.lock().unwrap();
        samples.iter().copied().collect()
    }

    /// Get the capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clone the Arc for sharing between threads
    pub fn clone_arc(&self) -> Arc<Mutex<VecDeque<f32>>> {
        self.samples.clone()
    }
}

#[cfg(feature = "visualization")]
impl Clone for AudioBuffer {
    fn clone(&self) -> Self {
        Self {
            samples: self.samples.clone(),
            capacity: self.capacity,
        }
    }
}
