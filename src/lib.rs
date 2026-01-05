//! Shared audio engine logic for both native (CPAL) and WASM (web)

pub mod envelope;
pub mod filters;

// New organized modules
pub mod instruments;
pub mod effects;
pub mod gen;
pub mod sequencer;
pub mod engine;

// Visualization module (optional)
#[cfg(feature = "visualization")]
pub mod visualization;

// WASM bindings (web)
#[cfg(feature = "web")]
pub mod web {
    use super::envelope::ADSRConfig;
    use super::instruments::{HiHat, HiHatConfig, KickConfig, KickDrum, SnareConfig, SnareDrum, TomConfig, TomDrum};
    use super::gen::oscillator::Oscillator;
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

    // Note: WasmStage is temporarily commented out as it needs to be updated
    // to work with the new Engine API. The Stage struct no longer exists.
    // TODO: Refactor this to use Engine or create a compatibility layer

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
