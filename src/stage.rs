use crate::envelope::ADSRConfig;
use crate::gen::oscillator::Oscillator;
use crate::instruments::{KickDrum, KickConfig, SnareDrum, SnareConfig, HiHat, HiHatConfig, TomDrum, TomConfig};
use crate::effects::limiter::BrickWallLimiter;

pub struct Stage {
    pub sample_rate: f32,
    pub instruments: Vec<Oscillator>, // Keep for backward compatibility
    pub limiter: BrickWallLimiter,
    pub sequencer: Sequencer,
    
    // Drum instruments for sequencer
    pub kick: KickDrum,
    pub snare: SnareDrum,
    pub hihat: HiHat,
    pub tom: TomDrum,
    
    // Harmonic distortion settings
    pub saturation: f32, // 0.0 to 1.0, where 0.0 is no distortion
    
    // Current time tracking
    current_time: f32,
}

/// A 16-step drum sequencer that manages pattern playback for multiple instruments
#[derive(Debug, Clone)]
pub struct Sequencer {
    /// 16-step patterns for each instrument (4 instruments, 16 steps each)
    patterns: [[bool; 16]; 4],
    /// Current step (0-15)
    current_step: usize,
    /// Whether the sequencer is playing
    is_playing: bool,
    /// BPM (beats per minute)
    bpm: f32,
    /// Time of the last step in seconds
    last_step_time: f32,
    /// Time interval between steps in seconds
    step_interval: f32,
}

impl Stage {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            instruments: Vec::new(),
            limiter: BrickWallLimiter::new(1.0), // Default threshold at 1.0 to prevent clipping
            sequencer: Sequencer::new(),
            
            // Initialize drum instruments with default presets
            kick: KickDrum::with_config(sample_rate, KickConfig::default()),
            snare: SnareDrum::with_config(sample_rate, SnareConfig::default()),
            hihat: HiHat::with_config(sample_rate, HiHatConfig::closed_default()),
            tom: TomDrum::with_config(sample_rate, TomConfig::default()),
            
            // Initialize harmonic distortion
            saturation: 0.0, // No distortion by default
            
            // Initialize current time
            current_time: 0.0,
        }
    }

    pub fn add(&mut self, mut instrument: Oscillator) {
        // Ensure the instrument uses the same sample rate as the stage
        instrument.sample_rate = self.sample_rate;
        self.instruments.push(instrument);
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Update internal time tracking
        self.current_time = current_time;
        
        // Update sequencer and trigger instruments if needed
        if self.sequencer.is_playing {
            self.sequencer.update(current_time);

            // Check if we should trigger instruments on the current step
            if self.sequencer.should_trigger_step(current_time) {
                let current_step = self.sequencer.current_step;

                // Trigger drum instruments based on patterns
                // Pattern 0: Kick
                if self.sequencer.patterns[0][current_step] {
                    self.kick.trigger(current_time);
                }
                // Pattern 1: Snare
                if self.sequencer.patterns[1][current_step] {
                    self.snare.trigger(current_time);
                }
                // Pattern 2: Hi-hat
                if self.sequencer.patterns[2][current_step] {
                    self.hihat.trigger(current_time);
                }
                // Pattern 3: Tom
                if self.sequencer.patterns[3][current_step] {
                    self.tom.trigger(current_time);
                }

                // Basic oscillators are NOT triggered by the sequencer
                // They should only be triggered manually via "Trigger all instruments" button

                // Mark that we've processed this step
                self.sequencer.last_step_time = current_time;
                self.sequencer.advance_step();
            }
        }

        let mut output = 0.0;
        
        // Add drum instrument outputs
        output += self.kick.tick(current_time);
        output += self.snare.tick(current_time);
        output += self.hihat.tick(current_time);
        output += self.tom.tick(current_time);
        
        // Add legacy instruments for backward compatibility
        for instrument in &mut self.instruments {
            output += instrument.tick(current_time);
        }
        
        // Apply harmonic distortion if enabled
        if self.saturation > 0.0 {
            output = self.apply_harmonic_distortion(output);
        }
        
        // Apply limiter to the combined output
        self.limiter.process(output)
    }

    pub fn trigger_all(&mut self) {
        for instrument in &mut self.instruments {
            instrument.trigger(self.current_time);
        }
    }

    pub fn trigger_instrument(&mut self, index: usize) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.trigger(self.current_time);
        }
    }

    pub fn set_instrument_volume(&mut self, index: usize, volume: f32) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.set_volume(volume);
        }
    }

    pub fn get_instrument_volume(&self, index: usize) -> f32 {
        if let Some(instrument) = self.instruments.get(index) {
            instrument.volume
        } else {
            0.0
        }
    }

    pub fn release_instrument(&mut self, index: usize) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.release(self.current_time);
        }
    }

    pub fn release_all(&mut self) {
        for instrument in &mut self.instruments {
            instrument.release(self.current_time);
        }
    }

    pub fn set_instrument_adsr(&mut self, index: usize, config: ADSRConfig) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.set_adsr(config);
        }
    }

    pub fn set_instrument_frequency(&mut self, index: usize, frequency_hz: f32) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.frequency_hz = frequency_hz;
        }
    }

    pub fn get_instrument_frequency(&self, index: usize) -> f32 {
        if let Some(instrument) = self.instruments.get(index) {
            instrument.frequency_hz
        } else {
            0.0
        }
    }

    pub fn set_instrument_waveform(&mut self, index: usize, waveform: crate::gen::waveform::Waveform) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.waveform = waveform;
        }
    }

    pub fn get_instrument_waveform(&self, index: usize) -> crate::gen::waveform::Waveform {
        if let Some(instrument) = self.instruments.get(index) {
            instrument.waveform
        } else {
            crate::gen::waveform::Waveform::Sine
        }
    }

    pub fn set_instrument_modulator_frequency(&mut self, index: usize, frequency_hz: f32) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.set_modulator_frequency(frequency_hz);
        }
    }

    pub fn get_instrument_modulator_frequency(&self, index: usize) -> f32 {
        if let Some(instrument) = self.instruments.get(index) {
            instrument.get_modulator_frequency()
        } else {
            0.0
        }
    }

    pub fn set_instrument_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(instrument) = self.instruments.get_mut(index) {
            instrument.set_enabled(enabled);
        }
    }

    pub fn is_instrument_enabled(&self, index: usize) -> bool {
        if let Some(instrument) = self.instruments.get(index) {
            instrument.is_enabled()
        } else {
            false
        }
    }

    /// Set the limiter threshold (typically 0.0 to 1.0)
    pub fn set_limiter_threshold(&mut self, threshold: f32) {
        self.limiter.threshold = threshold;
    }

    /// Get the current limiter threshold
    pub fn get_limiter_threshold(&self) -> f32 {
        self.limiter.threshold
    }

    // Sequencer control methods

    /// Start the sequencer
    pub fn sequencer_play(&mut self) {
        // Initialize with proper timing - use a small offset to ensure proper initialization
        self.sequencer.play_at_time(0.001);
    }
    
    /// Start the sequencer with a specific time
    pub fn sequencer_play_at_time(&mut self, time: f32) {
        self.sequencer.play_at_time(time);
    }

    /// Stop the sequencer
    pub fn sequencer_stop(&mut self) {
        self.sequencer.stop();
    }

    /// Reset the sequencer to step 0
    pub fn sequencer_reset(&mut self) {
        self.sequencer.reset();
    }

    /// Clear all patterns
    pub fn sequencer_clear_all(&mut self) {
        self.sequencer.clear_all();
    }

    /// Set a step for a specific instrument
    pub fn sequencer_set_step(&mut self, instrument: usize, step: usize, enabled: bool) {
        self.sequencer.set_step(instrument, step, enabled);
    }

    /// Get a step for a specific instrument
    pub fn sequencer_get_step(&self, instrument: usize, step: usize) -> bool {
        self.sequencer.get_step(instrument, step)
    }

    /// Set the BPM
    pub fn sequencer_set_bpm(&mut self, bpm: f32) {
        self.sequencer.set_bpm(bpm);
    }

    /// Get the current BPM
    pub fn sequencer_get_bpm(&self) -> f32 {
        self.sequencer.bpm
    }

    /// Get the current step (0-15)
    pub fn sequencer_get_current_step(&self) -> usize {
        self.sequencer.current_step
    }

    /// Check if the sequencer is playing
    pub fn sequencer_is_playing(&self) -> bool {
        self.sequencer.is_playing
    }
    
    /// Set up default test patterns for the drums
    pub fn sequencer_set_default_patterns(&mut self) {
        // Clear existing patterns
        self.sequencer.clear_all();
        
        // Kick: On beats 1, 5, 9, 13 (quarter notes)
        self.sequencer.set_step(0, 0, true);  // Beat 1
        self.sequencer.set_step(0, 4, true);  // Beat 5
        self.sequencer.set_step(0, 8, true);  // Beat 9
        self.sequencer.set_step(0, 12, true); // Beat 13
        
        // Snare: On beats 5, 13 (backbeat)
        self.sequencer.set_step(1, 4, true);  // Beat 5
        self.sequencer.set_step(1, 12, true); // Beat 13
        
        // Hi-hat: On off-beats (8th notes)
        for i in 0..16 {
            if i % 2 == 1 {
                self.sequencer.set_step(2, i, true);
            }
        }
        
        // Tom: Sparse pattern on beats 7, 15
        self.sequencer.set_step(3, 6, true);  // Beat 7
        self.sequencer.set_step(3, 14, true); // Beat 15
    }
    
    /// Get drum instrument configurations
    pub fn get_kick_config(&self) -> KickConfig {
        self.kick.config
    }
    
    pub fn get_snare_config(&self) -> SnareConfig {
        self.snare.config
    }
    
    pub fn get_hihat_config(&self) -> HiHatConfig {
        self.hihat.config
    }
    
    pub fn get_tom_config(&self) -> TomConfig {
        self.tom.config
    }
    
    /// Set drum instrument configurations
    pub fn set_kick_config(&mut self, config: KickConfig) {
        self.kick.set_config(config);
    }
    
    pub fn set_snare_config(&mut self, config: SnareConfig) {
        self.snare.set_config(config);
    }
    
    pub fn set_hihat_config(&mut self, config: HiHatConfig) {
        self.hihat.set_config(config);
    }
    
    pub fn set_tom_config(&mut self, config: TomConfig) {
        self.tom.set_config(config);
    }
    
    /// Apply harmonic distortion using soft clipping
    fn apply_harmonic_distortion(&self, input: f32) -> f32 {
        // Use saturation parameter to control distortion amount
        let drive = 1.0 + self.saturation * 9.0; // Scale from 1.0 to 10.0
        let gain = 1.0 / drive.sqrt(); // Compensate for increased volume
        
        // Apply soft clipping using hyperbolic tangent
        let driven = input * drive;
        let clipped = driven.tanh();
        
        // Apply makeup gain to maintain overall volume
        clipped * gain
    }
    
    /// Set the saturation level (0.0 to 1.0)
    pub fn set_saturation(&mut self, saturation: f32) {
        self.saturation = saturation.clamp(0.0, 1.0);
    }
    
    /// Get the current saturation level
    pub fn get_saturation(&self) -> f32 {
        self.saturation
    }
    
    /// Trigger the kick drum
    pub fn trigger_kick(&mut self) {
        self.kick.trigger(self.current_time);
    }
    
    /// Trigger the snare drum
    pub fn trigger_snare(&mut self) {
        self.snare.trigger(self.current_time);
    }
    
    /// Trigger the hi-hat
    pub fn trigger_hihat(&mut self) {
        self.hihat.trigger(self.current_time);
    }
    
    /// Trigger the tom drum
    pub fn trigger_tom(&mut self) {
        self.tom.trigger(self.current_time);
    }
}

impl Sequencer {
    pub fn new() -> Self {
        Self {
            patterns: [[false; 16]; 4],
            current_step: 0,
            is_playing: false,
            bpm: 120.0,
            last_step_time: 0.0,
            step_interval: 60.0 / (120.0 * 4.0), // 16th notes at 120 BPM
        }
    }

    pub fn play(&mut self) {
        self.is_playing = true;
    }
    
    pub fn play_at_time(&mut self, time: f32) {
        self.is_playing = true;
        self.last_step_time = time;
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
    }

    pub fn reset(&mut self) {
        self.current_step = 0;
        self.last_step_time = 0.0;
    }

    pub fn clear_all(&mut self) {
        self.patterns = [[false; 16]; 4];
    }

    pub fn set_step(&mut self, instrument: usize, step: usize, enabled: bool) {
        if instrument < 4 && step < 16 {
            self.patterns[instrument][step] = enabled;
        }
    }

    pub fn get_step(&self, instrument: usize, step: usize) -> bool {
        if instrument < 4 && step < 16 {
            self.patterns[instrument][step]
        } else {
            false
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        // Clamp BPM to reasonable range
        self.bpm = bpm.max(60.0).min(180.0);
        // Recalculate step interval (16th notes)
        self.step_interval = 60.0 / (self.bpm * 4.0);
    }

    pub fn update(&mut self, current_time: f32) {
        // This method is called from tick() to update internal state
        // The actual triggering logic is handled in tick()
    }

    pub fn should_trigger_step(&self, current_time: f32) -> bool {
        // Check if enough time has passed for the next step
        current_time - self.last_step_time >= self.step_interval
    }

    pub fn advance_step(&mut self) {
        self.current_step = (self.current_step + 1) % 16;
    }
}

