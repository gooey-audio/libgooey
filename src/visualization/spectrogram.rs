use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::VecDeque;

/// Spectrogram analyzer using FFT
pub struct SpectrogramAnalyzer {
    fft_size: usize,
    sample_rate: f32,
    planner: FftPlanner<f32>,
    // Circular buffer for spectrogram history (each entry is a spectrum)
    history: VecDeque<Vec<f32>>,
    max_history: usize,
}

impl SpectrogramAnalyzer {
    pub fn new(fft_size: usize, sample_rate: f32, max_history: usize) -> Self {
        Self {
            fft_size,
            sample_rate,
            planner: FftPlanner::new(),
            history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    /// Analyze a chunk of samples and add to history
    pub fn analyze(&mut self, samples: &[f32]) {
        if samples.len() < self.fft_size {
            return;
        }

        // Take the last fft_size samples
        let start_idx = samples.len() - self.fft_size;
        let input_samples = &samples[start_idx..];

        // Apply Hanning window to reduce spectral leakage
        let windowed: Vec<Complex<f32>> = input_samples
            .iter()
            .enumerate()
            .map(|(i, &sample)| {
                let window = 0.5
                    * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / self.fft_size as f32).cos());
                Complex::new(sample * window, 0.0)
            })
            .collect();

        // Perform FFT
        let fft = self.planner.plan_fft_forward(self.fft_size);
        let mut buffer = windowed;
        fft.process(&mut buffer);

        // Convert to magnitudes (only first half, since second half is mirror)
        let num_bins = self.fft_size / 2;
        let magnitudes: Vec<f32> = buffer[..num_bins]
            .iter()
            .map(|c| {
                let mag = (c.re * c.re + c.im * c.im).sqrt();
                // Convert to dB scale for better visualization
                20.0 * (mag + 1e-10).log10()
            })
            .collect();

        // Add to history
        if self.history.len() >= self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(magnitudes);
    }

    /// Get the spectrogram history as a 2D array [time][frequency]
    pub fn get_history(&self) -> &VecDeque<Vec<f32>> {
        &self.history
    }

    /// Get the frequency for a given bin index
    pub fn bin_to_frequency(&self, bin: usize) -> f32 {
        bin as f32 * self.sample_rate / self.fft_size as f32
    }

    /// Get the number of frequency bins
    pub fn num_bins(&self) -> usize {
        self.fft_size / 2
    }
}
