//! Shared audio engine logic for both native (CPAL) and WASM (web)

pub mod audio_state;
pub mod envelope;
pub mod filters;
pub mod stage;

// New organized modules
pub mod instruments;
pub mod effects;
pub mod gen;

// Platform abstraction layer
pub mod platform;

// WASM bindings (web)
#[cfg(feature = "web")]
pub mod web {
    use super::envelope::ADSRConfig;
    use super::instruments::{HiHat, HiHatConfig, KickConfig, KickDrum, SnareConfig, SnareDrum, TomConfig, TomDrum};
    use super::gen::oscillator::Oscillator;
    use super::stage::Stage;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct WasmOscillator {
        oscillator: Oscillator,
    }

    #[wasm_bindgen]
    impl WasmOscillator {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32, frequency_hz: f32) -> WasmOscillator {
            WasmOscillator {
                oscillator: Oscillator::new(sample_rate, frequency_hz),
            }
        }

        #[wasm_bindgen]
        pub fn trigger(&mut self, time: f32) {
            self.oscillator.trigger(time);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.oscillator.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn release(&mut self, time: f32) {
            self.oscillator.release(time);
        }

        #[wasm_bindgen]
        pub fn set_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) {
            let config = ADSRConfig::new(attack, decay, sustain, release);
            self.oscillator.set_adsr(config);
        }

        #[wasm_bindgen]
        pub fn set_modulator_frequency(&mut self, frequency_hz: f32) {
            self.oscillator.set_modulator_frequency(frequency_hz);
        }

        #[wasm_bindgen]
        pub fn get_modulator_frequency(&self) -> f32 {
            self.oscillator.get_modulator_frequency()
        }
    }

    #[wasm_bindgen]
    pub struct WasmStage {
        stage: Stage,
    }

    #[wasm_bindgen]
    impl WasmStage {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32) -> WasmStage {
            WasmStage {
                stage: Stage::new(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn add_oscillator(&mut self, sample_rate: f32, frequency_hz: f32) {
            let oscillator = Oscillator::new(sample_rate, frequency_hz);
            self.stage.add(oscillator);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.stage.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn trigger_all(&mut self) {
            self.stage.trigger_all();
        }

        #[wasm_bindgen]
        pub fn trigger_instrument(&mut self, index: usize) {
            self.stage.trigger_instrument(index);
        }

        #[wasm_bindgen]
        pub fn set_instrument_volume(&mut self, index: usize, volume: f32) {
            self.stage.set_instrument_volume(index, volume);
        }

        #[wasm_bindgen]
        pub fn get_instrument_volume(&self, index: usize) -> f32 {
            self.stage.get_instrument_volume(index)
        }

        #[wasm_bindgen]
        pub fn release_instrument(&mut self, index: usize) {
            self.stage.release_instrument(index);
        }

        #[wasm_bindgen]
        pub fn release_all(&mut self) {
            self.stage.release_all();
        }

        #[wasm_bindgen]
        pub fn set_instrument_adsr(
            &mut self,
            index: usize,
            attack: f32,
            decay: f32,
            sustain: f32,
            release: f32,
        ) {
            let config = ADSRConfig::new(attack, decay, sustain, release);
            self.stage.set_instrument_adsr(index, config);
        }

        #[wasm_bindgen]
        pub fn set_instrument_frequency(&mut self, index: usize, frequency_hz: f32) {
            self.stage.set_instrument_frequency(index, frequency_hz);
        }

        #[wasm_bindgen]
        pub fn get_instrument_frequency(&self, index: usize) -> f32 {
            self.stage.get_instrument_frequency(index)
        }

        #[wasm_bindgen]
        pub fn set_instrument_waveform(&mut self, index: usize, waveform_type: u32) {
            let waveform = match waveform_type {
                0 => crate::gen::waveform::Waveform::Sine,
                1 => crate::gen::waveform::Waveform::Square,
                2 => crate::gen::waveform::Waveform::Saw,
                3 => crate::gen::waveform::Waveform::Triangle,
                4 => crate::gen::waveform::Waveform::RingMod,
                5 => crate::gen::waveform::Waveform::Noise,
                _ => crate::gen::waveform::Waveform::Sine, // Default to sine for invalid values
            };
            self.stage.set_instrument_waveform(index, waveform);
        }

        #[wasm_bindgen]
        pub fn get_instrument_waveform(&self, index: usize) -> u32 {
            let waveform = self.stage.get_instrument_waveform(index);
            match waveform {
                crate::gen::waveform::Waveform::Sine => 0,
                crate::gen::waveform::Waveform::Square => 1,
                crate::gen::waveform::Waveform::Saw => 2,
                crate::gen::waveform::Waveform::Triangle => 3,
                crate::gen::waveform::Waveform::RingMod => 4,
                crate::gen::waveform::Waveform::Noise => 5,
            }
        }

        #[wasm_bindgen]
        pub fn set_instrument_modulator_frequency(&mut self, index: usize, frequency_hz: f32) {
            self.stage
                .set_instrument_modulator_frequency(index, frequency_hz);
        }

        #[wasm_bindgen]
        pub fn get_instrument_modulator_frequency(&self, index: usize) -> f32 {
            self.stage.get_instrument_modulator_frequency(index)
        }

        #[wasm_bindgen]
        pub fn set_instrument_enabled(&mut self, index: usize, enabled: bool) {
            self.stage.set_instrument_enabled(index, enabled);
        }

        #[wasm_bindgen]
        pub fn is_instrument_enabled(&self, index: usize) -> bool {
            self.stage.is_instrument_enabled(index)
        }

        // Sequencer methods
        #[wasm_bindgen]
        pub fn sequencer_play(&mut self) {
            self.stage.sequencer_play();
        }
        
        #[wasm_bindgen]
        pub fn sequencer_play_at_time(&mut self, time: f32) {
            self.stage.sequencer_play_at_time(time);
        }

        #[wasm_bindgen]
        pub fn sequencer_stop(&mut self) {
            self.stage.sequencer_stop();
        }

        #[wasm_bindgen]
        pub fn sequencer_reset(&mut self) {
            self.stage.sequencer_reset();
        }

        #[wasm_bindgen]
        pub fn sequencer_clear_all(&mut self) {
            self.stage.sequencer_clear_all();
        }

        #[wasm_bindgen]
        pub fn sequencer_set_step(&mut self, instrument: usize, step: usize, enabled: bool) {
            self.stage.sequencer_set_step(instrument, step, enabled);
        }

        #[wasm_bindgen]
        pub fn sequencer_get_step(&self, instrument: usize, step: usize) -> bool {
            self.stage.sequencer_get_step(instrument, step)
        }

        #[wasm_bindgen]
        pub fn sequencer_set_bpm(&mut self, bpm: f32) {
            self.stage.sequencer_set_bpm(bpm);
        }

        #[wasm_bindgen]
        pub fn sequencer_get_bpm(&self) -> f32 {
            self.stage.sequencer_get_bpm()
        }

        #[wasm_bindgen]
        pub fn sequencer_get_current_step(&self) -> usize {
            self.stage.sequencer_get_current_step()
        }

        #[wasm_bindgen]
        pub fn sequencer_is_playing(&self) -> bool {
            self.stage.sequencer_is_playing()
        }
        
        #[wasm_bindgen]
        pub fn sequencer_set_default_patterns(&mut self) {
            self.stage.sequencer_set_default_patterns();
        }
        
        // Drum configuration getters
        #[wasm_bindgen]
        pub fn get_kick_frequency(&self) -> f32 {
            self.stage.get_kick_config().kick_frequency
        }
        
        #[wasm_bindgen]
        pub fn get_snare_frequency(&self) -> f32 {
            self.stage.get_snare_config().snare_frequency
        }
        
        #[wasm_bindgen]
        pub fn get_hihat_frequency(&self) -> f32 {
            self.stage.get_hihat_config().base_frequency
        }
        
        #[wasm_bindgen]
        pub fn get_tom_frequency(&self) -> f32 {
            self.stage.get_tom_config().tom_frequency
        }
        
        // Drum configuration setters
        #[wasm_bindgen]
        pub fn set_kick_config(&mut self, frequency: f32, punch: f32, sub: f32, click: f32, decay: f32, pitch_drop: f32, volume: f32) {
            let config = KickConfig::new(frequency, punch, sub, click, decay, pitch_drop, volume);
            self.stage.set_kick_config(config);
        }
        
        #[wasm_bindgen]
        pub fn set_snare_config(&mut self, frequency: f32, tonal: f32, noise: f32, crack: f32, decay: f32, pitch_drop: f32, volume: f32) {
            let config = SnareConfig::new(frequency, tonal, noise, crack, decay, pitch_drop, volume);
            self.stage.set_snare_config(config);
        }
        
        #[wasm_bindgen]
        pub fn set_hihat_config(&mut self, frequency: f32, resonance: f32, brightness: f32, decay: f32, attack: f32, volume: f32, is_open: bool) {
            let config = HiHatConfig::new(frequency, resonance, brightness, decay, attack, volume, is_open);
            self.stage.set_hihat_config(config);
        }
        
        #[wasm_bindgen]
        pub fn set_tom_config(&mut self, frequency: f32, tonal: f32, punch: f32, decay: f32, pitch_drop: f32, volume: f32) {
            let config = TomConfig::new(frequency, tonal, punch, decay, pitch_drop, volume);
            self.stage.set_tom_config(config);
        }
        
        // Drum preset loaders
        #[wasm_bindgen]
        pub fn load_kick_preset(&mut self, preset_name: &str) {
            let config = match preset_name {
                "punchy" => KickConfig::punchy(),
                "deep" => KickConfig::deep(),
                "tight" => KickConfig::tight(),
                _ => KickConfig::default(),
            };
            self.stage.set_kick_config(config);
        }
        
        #[wasm_bindgen]
        pub fn load_snare_preset(&mut self, preset_name: &str) {
            let config = match preset_name {
                "crispy" => SnareConfig::crispy(),
                "deep" => SnareConfig::deep(),
                "tight" => SnareConfig::tight(),
                "fat" => SnareConfig::fat(),
                _ => SnareConfig::default(),
            };
            self.stage.set_snare_config(config);
        }
        
        #[wasm_bindgen]
        pub fn load_hihat_preset(&mut self, preset_name: &str) {
            let config = match preset_name {
                "closed_default" => HiHatConfig::closed_default(),
                "open_default" => HiHatConfig::open_default(),
                "closed_tight" => HiHatConfig::closed_tight(),
                "open_bright" => HiHatConfig::open_bright(),
                "closed_dark" => HiHatConfig::closed_dark(),
                "open_long" => HiHatConfig::open_long(),
                _ => HiHatConfig::closed_default(),
            };
            self.stage.set_hihat_config(config);
        }
        
        #[wasm_bindgen]
        pub fn load_tom_preset(&mut self, preset_name: &str) {
            let config = match preset_name {
                "high_tom" => TomConfig::high_tom(),
                "mid_tom" => TomConfig::mid_tom(),
                "low_tom" => TomConfig::low_tom(),
                "floor_tom" => TomConfig::floor_tom(),
                _ => TomConfig::default(),
            };
            self.stage.set_tom_config(config);
        }
        
        // Saturation control methods
        #[wasm_bindgen]
        pub fn set_saturation(&mut self, saturation: f32) {
            self.stage.set_saturation(saturation);
        }
        
        #[wasm_bindgen]
        pub fn get_saturation(&self) -> f32 {
            self.stage.get_saturation()
        }
        
        // Individual drum trigger methods
        #[wasm_bindgen]
        pub fn trigger_kick(&mut self) {
            self.stage.trigger_kick();
        }
        
        #[wasm_bindgen]
        pub fn trigger_snare(&mut self) {
            self.stage.trigger_snare();
        }
        
        #[wasm_bindgen]
        pub fn trigger_hihat(&mut self) {
            self.stage.trigger_hihat();
        }
        
        #[wasm_bindgen]
        pub fn trigger_tom(&mut self) {
            self.stage.trigger_tom();
        }
    }

    #[wasm_bindgen]
    pub struct WasmKickDrum {
        kick_drum: KickDrum,
    }

    #[wasm_bindgen]
    impl WasmKickDrum {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32) -> WasmKickDrum {
            WasmKickDrum {
                kick_drum: KickDrum::new(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_with_preset(sample_rate: f32, preset_name: &str) -> WasmKickDrum {
            let config = match preset_name {
                "punchy" => KickConfig::punchy(),
                "deep" => KickConfig::deep(),
                "tight" => KickConfig::tight(),
                _ => KickConfig::default(),
            };
            WasmKickDrum {
                kick_drum: KickDrum::with_config(sample_rate, config),
            }
        }

        #[wasm_bindgen]
        pub fn trigger(&mut self, time: f32) {
            self.kick_drum.trigger(time);
        }

        #[wasm_bindgen]
        pub fn release(&mut self, time: f32) {
            self.kick_drum.release(time);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.kick_drum.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn is_active(&self) -> bool {
            self.kick_drum.is_active()
        }

        #[wasm_bindgen]
        pub fn set_volume(&mut self, volume: f32) {
            self.kick_drum.set_volume(volume);
        }

        #[wasm_bindgen]
        pub fn set_frequency(&mut self, frequency: f32) {
            self.kick_drum.set_frequency(frequency);
        }

        #[wasm_bindgen]
        pub fn set_decay(&mut self, decay_time: f32) {
            self.kick_drum.set_decay(decay_time);
        }

        #[wasm_bindgen]
        pub fn set_punch(&mut self, punch_amount: f32) {
            self.kick_drum.set_punch(punch_amount);
        }

        #[wasm_bindgen]
        pub fn set_sub(&mut self, sub_amount: f32) {
            self.kick_drum.set_sub(sub_amount);
        }

        #[wasm_bindgen]
        pub fn set_click(&mut self, click_amount: f32) {
            self.kick_drum.set_click(click_amount);
        }

        #[wasm_bindgen]
        pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
            self.kick_drum.set_pitch_drop(pitch_drop);
        }

        #[wasm_bindgen]
        pub fn set_config(
            &mut self,
            kick_frequency: f32,
            punch_amount: f32,
            sub_amount: f32,
            click_amount: f32,
            decay_time: f32,
            pitch_drop: f32,
            volume: f32,
        ) {
            let config = KickConfig::new(
                kick_frequency,
                punch_amount,
                sub_amount,
                click_amount,
                decay_time,
                pitch_drop,
                volume,
            );
            self.kick_drum.set_config(config);
        }
    }

    #[wasm_bindgen]
    pub struct WasmHiHat {
        hihat: HiHat,
    }

    #[wasm_bindgen]
    impl WasmHiHat {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32) -> WasmHiHat {
            WasmHiHat {
                hihat: HiHat::new(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_closed(sample_rate: f32) -> WasmHiHat {
            WasmHiHat {
                hihat: HiHat::new_closed(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_open(sample_rate: f32) -> WasmHiHat {
            WasmHiHat {
                hihat: HiHat::new_open(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_with_preset(sample_rate: f32, preset_name: &str) -> WasmHiHat {
            let config = match preset_name {
                "closed_default" => HiHatConfig::closed_default(),
                "open_default" => HiHatConfig::open_default(),
                "closed_tight" => HiHatConfig::closed_tight(),
                "open_bright" => HiHatConfig::open_bright(),
                "closed_dark" => HiHatConfig::closed_dark(),
                "open_long" => HiHatConfig::open_long(),
                _ => HiHatConfig::closed_default(),
            };
            WasmHiHat {
                hihat: HiHat::with_config(sample_rate, config),
            }
        }

        #[wasm_bindgen]
        pub fn trigger(&mut self, time: f32) {
            self.hihat.trigger(time);
        }

        #[wasm_bindgen]
        pub fn release(&mut self, time: f32) {
            self.hihat.release(time);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.hihat.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn is_active(&self) -> bool {
            self.hihat.is_active()
        }

        #[wasm_bindgen]
        pub fn set_volume(&mut self, volume: f32) {
            self.hihat.set_volume(volume);
        }

        #[wasm_bindgen]
        pub fn set_frequency(&mut self, frequency: f32) {
            self.hihat.set_frequency(frequency);
        }

        #[wasm_bindgen]
        pub fn set_decay(&mut self, decay_time: f32) {
            self.hihat.set_decay(decay_time);
        }

        #[wasm_bindgen]
        pub fn set_brightness(&mut self, brightness: f32) {
            self.hihat.set_brightness(brightness);
        }

        #[wasm_bindgen]
        pub fn set_resonance(&mut self, resonance: f32) {
            self.hihat.set_resonance(resonance);
        }

        #[wasm_bindgen]
        pub fn set_attack(&mut self, attack_time: f32) {
            self.hihat.set_attack(attack_time);
        }

        #[wasm_bindgen]
        pub fn set_open(&mut self, is_open: bool) {
            self.hihat.set_open(is_open);
        }

        #[wasm_bindgen]
        pub fn set_config(
            &mut self,
            base_frequency: f32,
            resonance: f32,
            brightness: f32,
            decay_time: f32,
            attack_time: f32,
            volume: f32,
            is_open: bool,
        ) {
            let config = HiHatConfig::new(
                base_frequency,
                resonance,
                brightness,
                decay_time,
                attack_time,
                volume,
                is_open,
            );
            self.hihat.set_config(config);
        }
    }

    #[wasm_bindgen]
    pub struct WasmSnareDrum {
        snare_drum: SnareDrum,
    }

    #[wasm_bindgen]
    impl WasmSnareDrum {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32) -> WasmSnareDrum {
            WasmSnareDrum {
                snare_drum: SnareDrum::new(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_with_preset(sample_rate: f32, preset_name: &str) -> WasmSnareDrum {
            let config = match preset_name {
                "crispy" => SnareConfig::crispy(),
                "deep" => SnareConfig::deep(),
                "tight" => SnareConfig::tight(),
                "fat" => SnareConfig::fat(),
                _ => SnareConfig::default(),
            };
            WasmSnareDrum {
                snare_drum: SnareDrum::with_config(sample_rate, config),
            }
        }

        #[wasm_bindgen]
        pub fn trigger(&mut self, time: f32) {
            self.snare_drum.trigger(time);
        }

        #[wasm_bindgen]
        pub fn release(&mut self, time: f32) {
            self.snare_drum.release(time);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.snare_drum.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn is_active(&self) -> bool {
            self.snare_drum.is_active()
        }

        #[wasm_bindgen]
        pub fn set_volume(&mut self, volume: f32) {
            self.snare_drum.set_volume(volume);
        }

        #[wasm_bindgen]
        pub fn set_frequency(&mut self, frequency: f32) {
            self.snare_drum.set_frequency(frequency);
        }

        #[wasm_bindgen]
        pub fn set_decay(&mut self, decay_time: f32) {
            self.snare_drum.set_decay(decay_time);
        }

        #[wasm_bindgen]
        pub fn set_tonal(&mut self, tonal_amount: f32) {
            self.snare_drum.set_tonal(tonal_amount);
        }

        #[wasm_bindgen]
        pub fn set_noise(&mut self, noise_amount: f32) {
            self.snare_drum.set_noise(noise_amount);
        }

        #[wasm_bindgen]
        pub fn set_crack(&mut self, crack_amount: f32) {
            self.snare_drum.set_crack(crack_amount);
        }

        #[wasm_bindgen]
        pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
            self.snare_drum.set_pitch_drop(pitch_drop);
        }

        #[wasm_bindgen]
        pub fn set_config(
            &mut self,
            snare_frequency: f32,
            tonal_amount: f32,
            noise_amount: f32,
            crack_amount: f32,
            decay_time: f32,
            pitch_drop: f32,
            volume: f32,
        ) {
            let config = SnareConfig::new(
                snare_frequency,
                tonal_amount,
                noise_amount,
                crack_amount,
                decay_time,
                pitch_drop,
                volume,
            );
            self.snare_drum.set_config(config);
        }
    }

    #[wasm_bindgen]
    pub struct WasmTomDrum {
        tom_drum: TomDrum,
    }

    #[wasm_bindgen]
    impl WasmTomDrum {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f32) -> WasmTomDrum {
            WasmTomDrum {
                tom_drum: TomDrum::new(sample_rate),
            }
        }

        #[wasm_bindgen]
        pub fn new_with_preset(sample_rate: f32, preset_name: &str) -> WasmTomDrum {
            let config = match preset_name {
                "high_tom" => TomConfig::high_tom(),
                "mid_tom" => TomConfig::mid_tom(),
                "low_tom" => TomConfig::low_tom(),
                "floor_tom" => TomConfig::floor_tom(),
                _ => TomConfig::default(),
            };
            WasmTomDrum {
                tom_drum: TomDrum::with_config(sample_rate, config),
            }
        }

        #[wasm_bindgen]
        pub fn trigger(&mut self, time: f32) {
            self.tom_drum.trigger(time);
        }

        #[wasm_bindgen]
        pub fn release(&mut self, time: f32) {
            self.tom_drum.release(time);
        }

        #[wasm_bindgen]
        pub fn tick(&mut self, current_time: f32) -> f32 {
            self.tom_drum.tick(current_time)
        }

        #[wasm_bindgen]
        pub fn is_active(&self) -> bool {
            self.tom_drum.is_active()
        }

        #[wasm_bindgen]
        pub fn set_volume(&mut self, volume: f32) {
            self.tom_drum.set_volume(volume);
        }

        #[wasm_bindgen]
        pub fn set_frequency(&mut self, frequency: f32) {
            self.tom_drum.set_frequency(frequency);
        }

        #[wasm_bindgen]
        pub fn set_decay(&mut self, decay_time: f32) {
            self.tom_drum.set_decay(decay_time);
        }

        #[wasm_bindgen]
        pub fn set_tonal(&mut self, tonal_amount: f32) {
            self.tom_drum.set_tonal(tonal_amount);
        }

        #[wasm_bindgen]
        pub fn set_punch(&mut self, punch_amount: f32) {
            self.tom_drum.set_punch(punch_amount);
        }

        #[wasm_bindgen]
        pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
            self.tom_drum.set_pitch_drop(pitch_drop);
        }

        #[wasm_bindgen]
        pub fn set_config(
            &mut self,
            tom_frequency: f32,
            tonal_amount: f32,
            punch_amount: f32,
            decay_time: f32,
            pitch_drop: f32,
            volume: f32,
        ) {
            let config = TomConfig::new(
                tom_frequency,
                tonal_amount,
                punch_amount,
                decay_time,
                pitch_drop,
                volume,
            );
            self.tom_drum.set_config(config);
        }
    }
}
