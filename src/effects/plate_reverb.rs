//! Plate reverb effect using Jon Dattorro's figure-eight tank topology
//!
//! Implements the plate-class reverberator from Jon Dattorro's "Effect Design
//! Part 1" (JAES 1997): a predelay and input-bandwidth lowpass condition the
//! signal, four series allpasses diffuse it, and a "figure-eight" tank of two
//! cross-coupled branches (each a modulated allpass -> delay -> damping
//! lowpass -> decay -> allpass -> delay -> decay -> cross-feed) recirculates
//! it. Stereo output is drawn from seven taps per channel spread across BOTH
//! branches, which is what gives a plate its characteristically wide, dense
//! image from a mono-summed input.
//!
//! Unlike the dual-mono spring reverb, the tank is a single shared structure:
//! `process_stereo` mono-sums the input into the tank once per frame and the
//! left/right wet signals come from the cross-branch output taps. The dry
//! signal stays stereo.
//!
//! The size parameter rescales every tank delay and output tap through
//! fractional (linearly interpolated) reads over max-size buffers, so it can
//! move at runtime; sweeping it produces an intentional tape-style pitch bend.

use crate::effects::Effect;
use crate::frame::StereoFrame;
use crate::utils::smoother::SmoothedParam;

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Sample rate all of Dattorro's published delay lengths are specified at.
/// Lengths are rescaled by `sample_rate / DATTORRO_SR` at construction.
const DATTORRO_SR: f32 = 29_761.0;

/// Input diffusion allpass delays (samples at 29761 Hz) and gains.
const INPUT_AP_DELAYS: [f32; 4] = [142.0, 107.0, 379.0, 277.0];
const INPUT_AP_GAINS: [f32; 4] = [0.750, 0.750, 0.625, 0.625];

/// Tank delay lengths (samples at 29761 Hz). Branch A cross-feeds branch B
/// and vice versa, forming the figure-eight loop.
const TANK_AP1_A: f32 = 672.0;
const TANK_DELAY1_A: f32 = 4453.0;
const TANK_AP2_A: f32 = 1800.0;
const TANK_DELAY2_A: f32 = 3720.0;
const TANK_AP1_B: f32 = 908.0;
const TANK_DELAY1_B: f32 = 4217.0;
const TANK_AP2_B: f32 = 2656.0;
const TANK_DELAY2_B: f32 = 3163.0;

/// Gain of the modulated "decay diffusion 1" allpasses at the tank inputs.
const DECAY_DIFFUSION_1: f32 = 0.70;

/// Maximum LFO excursion of the modulated allpasses (samples at 29761 Hz).
const EXCURSION: f32 = 16.0;

/// Free-running LFO rates in Hz, one per branch. Deliberately non-harmonic so
/// the two branch modulations never phase-lock.
const LFO_RATE_A: f32 = 0.50;
const LFO_RATE_B: f32 = 0.71;

/// Input bandwidth one-pole coefficient (nearly open; the damping parameter is
/// the tone control, this just tames the very top before the tank).
const INPUT_BANDWIDTH: f32 = 0.9995;

/// Maximum tank feedback gain. The signal passes two decay stages per branch
/// traversal, so 0.95 leaves headroom for the (slightly non-unity) modulated
/// allpasses while still allowing a 20+ second tail at decay = 1.0.
const MAX_DECAY: f32 = 0.95;

/// Predelay range in milliseconds (the 0-1 knob maps linearly onto this).
const MAX_PREDELAY_MS: f32 = 200.0;

/// Dattorro's output tap weight.
const OUTPUT_SCALE: f32 = 0.6;

/// Largest tank scale the size knob can reach; buffers are allocated for this.
const MAX_SIZE_SCALE: f32 = 2.0;

/// Map the 0-1 size knob to a tank scale: 0.0 -> 0.25x, 0.5 -> 1.0x (the
/// published Dattorro plate), 1.0 -> 2.0x. Exponential in each half so equal
/// knob moves feel like equal size ratios.
fn size_to_scale(size: f32) -> f32 {
    if size <= 0.5 {
        4.0_f32.powf(2.0 * size - 1.0)
    } else {
        2.0_f32.powf(2.0 * size - 1.0)
    }
}

#[inline]
fn flush_denormal(x: &mut f32) {
    if x.abs() < DENORMAL_THRESHOLD {
        *x = 0.0;
    }
}

/// Circular buffer with fractional (linearly interpolated) reads. Serves as
/// both a plain delay line and, via [`DelayLine::allpass`], a Schroeder
/// allpass whose delay length may be fractional and modulated.
struct DelayLine {
    buf: Vec<f32>,
    idx: usize,
}

impl DelayLine {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity.max(4)],
            idx: 0,
        }
    }

    fn write(&mut self, x: f32) {
        self.buf[self.idx] = x;
        self.idx = (self.idx + 1) % self.buf.len();
    }

    /// Read the value written `offset` samples ago, BEFORE this sample's
    /// write (offset 1 = previous sample). Used to realize a delay of N
    /// samples as `read_frac(N)` followed by `write(x)`.
    fn read_frac(&self, offset: f32) -> f32 {
        let len = self.buf.len();
        let offset = offset.clamp(1.0, (len - 2) as f32);
        let whole = offset as usize;
        let frac = offset - whole as f32;
        let a = self.buf[(self.idx + len - whole) % len];
        let b = self.buf[(self.idx + len - whole - 1) % len];
        a + frac * (b - a)
    }

    /// Read the value written `offset` samples ago, AFTER this sample's write
    /// (offset 0 = the sample just written). Used by the output tap matrix,
    /// which reads the tank state once all of this frame's writes are done.
    fn tap_frac(&self, offset: f32) -> f32 {
        let len = self.buf.len();
        let offset = offset.clamp(0.0, (len - 2) as f32);
        let whole = offset as usize;
        let frac = offset - whole as f32;
        let a = self.buf[(self.idx + len - 1 - whole) % len];
        let b = self.buf[(self.idx + len - 2 - whole) % len];
        a + frac * (b - a)
    }

    /// Schroeder allpass H(z) = (g + z^-N) / (1 + g·z^-N) with a fractional,
    /// possibly modulated delay N. A time-varying N makes the filter slightly
    /// non-unity-gain; the tank's decay margin absorbs that.
    fn allpass(&mut self, input: f32, gain: f32, delay: f32) -> f32 {
        let delayed = self.read_frac(delay);
        let v = input - gain * delayed;
        self.write(v);
        gain * v + delayed
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.idx = 0;
    }
}

struct PlateState {
    predelay: DelayLine,
    bandwidth_state: f32,
    input_aps: [DelayLine; 4],
    /// Fixed input-diffusion delay lengths at the engine rate (not size-scaled).
    input_ap_delays: [f32; 4],

    // Branch A
    mod_ap_a: DelayLine,
    delay1_a: DelayLine,
    damp_state_a: f32,
    ap2_a: DelayLine,
    delay2_a: DelayLine,

    // Branch B
    mod_ap_b: DelayLine,
    delay1_b: DelayLine,
    damp_state_b: f32,
    ap2_b: DelayLine,
    delay2_b: DelayLine,

    /// Previous-sample cross-feed values (branch output * decay). Read before
    /// either branch updates, giving the figure-eight its one-sample latency.
    fb_a: f32,
    fb_b: f32,

    lfo_phase_a: f32,
    lfo_phase_b: f32,
    lfo_inc_a: f32,
    lfo_inc_b: f32,

    /// Tank delay lengths at the engine rate and size 1.0 (multiplied by the
    /// smoothed size scale at read time).
    len_ap1_a: f32,
    len_d1_a: f32,
    len_ap2_a: f32,
    len_d2_a: f32,
    len_ap1_b: f32,
    len_d1_b: f32,
    len_ap2_b: f32,
    len_d2_b: f32,
    /// LFO excursion in engine-rate samples (not size-scaled).
    excursion: f32,
    /// `sample_rate / DATTORRO_SR`, applied to the output tap offsets.
    sr_scale: f32,
    sample_rate: f32,

    decay_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,
    damping_smoothed: SmoothedParam,
    predelay_smoothed: SmoothedParam,
    width_smoothed: SmoothedParam,
    size_smoothed: SmoothedParam,
}

pub struct PlateReverbEffect {
    // Single shared tank (NOT per-channel [State; 2]): the figure-eight
    // topology mono-sums the input and derives stereo from cross-branch
    // output taps, so both channels read the same tank.
    state: UnsafeCell<PlateState>,
    decay_target: AtomicU32,
    mix_target: AtomicU32,
    damping_target: AtomicU32,
    predelay_target: AtomicU32,
    width_target: AtomicU32,
    size_target: AtomicU32,
}

// SAFETY: UnsafeCell state is only accessed from the audio thread via process()
unsafe impl Send for PlateReverbEffect {}
unsafe impl Sync for PlateReverbEffect {}

impl PlateReverbEffect {
    /// Create a plate reverb. `decay`, `mix`, and `damping` are 0-1 knobs;
    /// predelay defaults to 0, width to 1 (full stereo), size to 0.5 (the
    /// published Dattorro plate dimensions).
    pub fn new(sample_rate: f32, decay: f32, mix: f32, damping: f32) -> Self {
        let decay = decay.clamp(0.0, 1.0);
        let mix = mix.clamp(0.0, 1.0);
        let damping = damping.clamp(0.0, 1.0);
        let predelay = 0.0;
        let width = 1.0;
        let size = 0.5;

        let sr_scale = sample_rate / DATTORRO_SR;
        let excursion = EXCURSION * sr_scale;

        // Fixed-length lines are sized for their delay alone; size-scaled tank
        // lines get MAX_SIZE_SCALE headroom (+ excursion for the modulated
        // allpasses) so the fractional reads never wrap past the write head.
        let fixed_line = |base: f32| DelayLine::new((base * sr_scale).ceil() as usize + 4);
        let sized_line = |base: f32, headroom: f32| {
            DelayLine::new((base * MAX_SIZE_SCALE * sr_scale + headroom).ceil() as usize + 4)
        };

        let state = PlateState {
            predelay: DelayLine::new((MAX_PREDELAY_MS * 0.001 * sample_rate).ceil() as usize + 8),
            bandwidth_state: 0.0,
            input_aps: INPUT_AP_DELAYS.map(fixed_line),
            input_ap_delays: INPUT_AP_DELAYS.map(|d| (d * sr_scale).max(1.0)),

            mod_ap_a: sized_line(TANK_AP1_A, excursion),
            delay1_a: sized_line(TANK_DELAY1_A, 0.0),
            damp_state_a: 0.0,
            ap2_a: sized_line(TANK_AP2_A, 0.0),
            delay2_a: sized_line(TANK_DELAY2_A, 0.0),

            mod_ap_b: sized_line(TANK_AP1_B, excursion),
            delay1_b: sized_line(TANK_DELAY1_B, 0.0),
            damp_state_b: 0.0,
            ap2_b: sized_line(TANK_AP2_B, 0.0),
            delay2_b: sized_line(TANK_DELAY2_B, 0.0),

            fb_a: 0.0,
            fb_b: 0.0,

            lfo_phase_a: 0.0,
            lfo_phase_b: 0.0,
            lfo_inc_a: LFO_RATE_A / sample_rate,
            lfo_inc_b: LFO_RATE_B / sample_rate,

            len_ap1_a: TANK_AP1_A * sr_scale,
            len_d1_a: TANK_DELAY1_A * sr_scale,
            len_ap2_a: TANK_AP2_A * sr_scale,
            len_d2_a: TANK_DELAY2_A * sr_scale,
            len_ap1_b: TANK_AP1_B * sr_scale,
            len_d1_b: TANK_DELAY1_B * sr_scale,
            len_ap2_b: TANK_AP2_B * sr_scale,
            len_d2_b: TANK_DELAY2_B * sr_scale,
            excursion,
            sr_scale,
            sample_rate,

            decay_smoothed: SmoothedParam::new_normalized(decay, sample_rate),
            mix_smoothed: SmoothedParam::new_normalized(mix, sample_rate),
            damping_smoothed: SmoothedParam::new_normalized(damping, sample_rate),
            predelay_smoothed: SmoothedParam::new_normalized(predelay, sample_rate),
            width_smoothed: SmoothedParam::new_normalized(width, sample_rate),
            size_smoothed: SmoothedParam::new_normalized(size, sample_rate),
        };

        debug_assert!(
            state.mod_ap_a.buf.len() as f32 > TANK_AP1_A * MAX_SIZE_SCALE * sr_scale + excursion,
            "modulated allpass buffer must cover max size + excursion"
        );
        debug_assert!(
            state.delay1_a.buf.len() as f32 > TANK_DELAY1_A * MAX_SIZE_SCALE * sr_scale,
            "tank delay buffer must cover max size"
        );

        Self {
            state: UnsafeCell::new(state),
            decay_target: AtomicU32::new(decay.to_bits()),
            mix_target: AtomicU32::new(mix.to_bits()),
            damping_target: AtomicU32::new(damping.to_bits()),
            predelay_target: AtomicU32::new(predelay.to_bits()),
            width_target: AtomicU32::new(width.to_bits()),
            size_target: AtomicU32::new(size.to_bits()),
        }
    }

    pub fn set_decay(&self, value: f32) {
        self.decay_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_decay(&self) -> f32 {
        f32::from_bits(self.decay_target.load(Ordering::Relaxed))
    }

    pub fn set_mix(&self, value: f32) {
        self.mix_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    pub fn set_damping(&self, value: f32) {
        self.damping_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_damping(&self) -> f32 {
        f32::from_bits(self.damping_target.load(Ordering::Relaxed))
    }

    /// Predelay knob (0-1, maps linearly to 0-200 ms).
    pub fn set_predelay(&self, value: f32) {
        self.predelay_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_predelay(&self) -> f32 {
        f32::from_bits(self.predelay_target.load(Ordering::Relaxed))
    }

    /// Stereo width of the wet signal (0 = mono, 1 = full Dattorro taps).
    pub fn set_width(&self, value: f32) {
        self.width_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_width(&self) -> f32 {
        f32::from_bits(self.width_target.load(Ordering::Relaxed))
    }

    /// Size knob (0-1): rescales every tank delay from 0.25x to 2.0x, with
    /// 0.5 = the published plate. Sweeping it pitch-bends the tail (intended).
    pub fn set_size(&self, value: f32) {
        self.size_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_size(&self) -> f32 {
        f32::from_bits(self.size_target.load(Ordering::Relaxed))
    }

    /// Reset reverb state (clear all delay lines, filters, and the tank loop)
    pub fn reset(&self) {
        // SAFETY: Called from main thread when reverb is not processing
        let state = unsafe { &mut *self.state.get() };
        state.predelay.clear();
        state.bandwidth_state = 0.0;
        for ap in state.input_aps.iter_mut() {
            ap.clear();
        }
        state.mod_ap_a.clear();
        state.delay1_a.clear();
        state.damp_state_a = 0.0;
        state.ap2_a.clear();
        state.delay2_a.clear();
        state.mod_ap_b.clear();
        state.delay1_b.clear();
        state.damp_state_b = 0.0;
        state.ap2_b.clear();
        state.delay2_b.clear();
        state.fb_a = 0.0;
        state.fb_b = 0.0;
        state.lfo_phase_a = 0.0;
        state.lfo_phase_b = 0.0;
    }

    /// Advance the tank by one sample and return `(wet_l, wet_r, mix)`.
    /// Called exactly once per frame by both the mono and stereo paths.
    fn tick_tank(&self, state: &mut PlateState, input: f32) -> (f32, f32, f32) {
        let input = if input.is_finite() { input } else { 0.0 };

        // Update smoothed parameters from atomic targets
        state
            .decay_smoothed
            .set_target(f32::from_bits(self.decay_target.load(Ordering::Relaxed)));
        state
            .mix_smoothed
            .set_target(f32::from_bits(self.mix_target.load(Ordering::Relaxed)));
        state
            .damping_smoothed
            .set_target(f32::from_bits(self.damping_target.load(Ordering::Relaxed)));
        state
            .predelay_smoothed
            .set_target(f32::from_bits(self.predelay_target.load(Ordering::Relaxed)));
        state
            .width_smoothed
            .set_target(f32::from_bits(self.width_target.load(Ordering::Relaxed)));
        state
            .size_smoothed
            .set_target(f32::from_bits(self.size_target.load(Ordering::Relaxed)));

        let decay_knob = state.decay_smoothed.tick();
        let mix = state.mix_smoothed.tick();
        let damping = state.damping_smoothed.tick();
        let predelay_knob = state.predelay_smoothed.tick();
        let width = state.width_smoothed.tick();
        let size = size_to_scale(state.size_smoothed.tick());

        let decay_gain = decay_knob * MAX_DECAY;
        // Dattorro's rule for the second decay-diffusion stage: track decay,
        // bounded so short tails stay diffuse and long ones don't over-ring.
        let dd2 = (decay_gain + 0.15).clamp(0.25, 0.50);
        // Cap the one-pole coefficient below 1.0 so it can never latch DC.
        let damp = damping * 0.95;

        // Predelay (fractional read so the smoothed knob moves clicklessly)
        state.predelay.write(input);
        let predelay_samples = predelay_knob * MAX_PREDELAY_MS * 0.001 * state.sample_rate;
        let delayed_input = state.predelay.tap_frac(predelay_samples);

        // Input bandwidth lowpass
        state.bandwidth_state += INPUT_BANDWIDTH * (delayed_input - state.bandwidth_state);
        flush_denormal(&mut state.bandwidth_state);
        let mut sig = state.bandwidth_state;

        // Four series input-diffusion allpasses
        for (ap, (&delay, &gain)) in state
            .input_aps
            .iter_mut()
            .zip(state.input_ap_delays.iter().zip(INPUT_AP_GAINS.iter()))
        {
            sig = ap.allpass(sig, gain, delay);
        }

        // Free-running LFOs for the tank's modulated allpasses
        state.lfo_phase_a = (state.lfo_phase_a + state.lfo_inc_a).fract();
        state.lfo_phase_b = (state.lfo_phase_b + state.lfo_inc_b).fract();
        let lfo_a = (std::f32::consts::TAU * state.lfo_phase_a).sin();
        let lfo_b = (std::f32::consts::TAU * state.lfo_phase_b).sin();

        // Figure-eight tank. Capture BOTH cross-feeds before updating either
        // branch: the one-sample latency is what keeps the loop causal.
        let in_a = sig + state.fb_b;
        let in_b = sig + state.fb_a;

        let a1 = state.mod_ap_a.allpass(
            in_a,
            DECAY_DIFFUSION_1,
            state.len_ap1_a * size + lfo_a * state.excursion,
        );
        let d1a = state.delay1_a.read_frac(state.len_d1_a * size);
        state.delay1_a.write(a1);
        state.damp_state_a = d1a * (1.0 - damp) + state.damp_state_a * damp;
        flush_denormal(&mut state.damp_state_a);
        let a2 = state
            .ap2_a
            .allpass(state.damp_state_a * decay_gain, dd2, state.len_ap2_a * size);
        let d2a = state.delay2_a.read_frac(state.len_d2_a * size);
        state.delay2_a.write(a2);

        let b1 = state.mod_ap_b.allpass(
            in_b,
            DECAY_DIFFUSION_1,
            state.len_ap1_b * size + lfo_b * state.excursion,
        );
        let d1b = state.delay1_b.read_frac(state.len_d1_b * size);
        state.delay1_b.write(b1);
        state.damp_state_b = d1b * (1.0 - damp) + state.damp_state_b * damp;
        flush_denormal(&mut state.damp_state_b);
        let b2 = state
            .ap2_b
            .allpass(state.damp_state_b * decay_gain, dd2, state.len_ap2_b * size);
        let d2b = state.delay2_b.read_frac(state.len_d2_b * size);
        state.delay2_b.write(b2);

        state.fb_a = d2a * decay_gain;
        flush_denormal(&mut state.fb_a);
        state.fb_b = d2b * decay_gain;
        flush_denormal(&mut state.fb_b);

        // Dattorro's 7-tap output matrix (offsets in samples at 29761 Hz,
        // rescaled by sample rate and the live size scale). Left reads mostly
        // from branch B and right from branch A — the cross-branch taps are
        // what make the plate image wide yet coherent.
        let tap_scale = state.sr_scale * size;
        let yl = OUTPUT_SCALE
            * (state.delay1_b.tap_frac(266.0 * tap_scale)
                + state.delay1_b.tap_frac(2974.0 * tap_scale)
                - state.ap2_b.tap_frac(1913.0 * tap_scale)
                + state.delay2_b.tap_frac(1996.0 * tap_scale)
                - state.delay1_a.tap_frac(1990.0 * tap_scale)
                - state.ap2_a.tap_frac(187.0 * tap_scale)
                - state.delay2_a.tap_frac(1066.0 * tap_scale));
        let yr = OUTPUT_SCALE
            * (state.delay1_a.tap_frac(353.0 * tap_scale)
                + state.delay1_a.tap_frac(3627.0 * tap_scale)
                - state.ap2_a.tap_frac(1228.0 * tap_scale)
                + state.delay2_a.tap_frac(2673.0 * tap_scale)
                - state.delay1_b.tap_frac(2111.0 * tap_scale)
                - state.ap2_b.tap_frac(335.0 * tap_scale)
                - state.delay2_b.tap_frac(121.0 * tap_scale));

        // Wet-only width control (mid/side)
        let mid = 0.5 * (yl + yr);
        let side = 0.5 * (yl - yr) * width;
        (mid + side, mid - side, mix)
    }
}

impl Effect for PlateReverbEffect {
    fn process(&self, input: f32) -> f32 {
        // SAFETY: process() is only called from the audio thread
        let state = unsafe { &mut *self.state.get() };
        let input = if input.is_finite() { input } else { 0.0 };
        let (wet_l, wet_r, mix) = self.tick_tank(state, input);
        let result = input * (1.0 - mix) + 0.5 * (wet_l + wet_r) * mix;
        if result.is_finite() {
            result
        } else {
            input
        }
    }

    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        // SAFETY: see process(); the tank is shared and ticked once per frame.
        let state = unsafe { &mut *self.state.get() };
        let l = if input.l.is_finite() { input.l } else { 0.0 };
        let r = if input.r.is_finite() { input.r } else { 0.0 };
        let (wet_l, wet_r, mix) = self.tick_tank(state, 0.5 * (l + r));
        let out_l = l * (1.0 - mix) + wet_l * mix;
        let out_r = r * (1.0 - mix) + wet_r * mix;
        StereoFrame {
            l: if out_l.is_finite() { out_l } else { l },
            r: if out_r.is_finite() { out_r } else { r },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn impulse_response(reverb: &PlateReverbEffect, frames: usize) -> Vec<StereoFrame> {
        (0..frames)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                reverb.process_stereo(StereoFrame { l: x, r: x })
            })
            .collect()
    }

    #[test]
    fn stable_at_max_decay() {
        let reverb = PlateReverbEffect::new(SR, 1.0, 1.0, 0.0);
        let frames = (SR * 5.0) as usize;
        let out = impulse_response(&reverb, frames);

        let peak = |slice: &[StereoFrame]| {
            slice
                .iter()
                .fold(0.0_f32, |acc, f| acc.max(f.l.abs()).max(f.r.abs()))
        };
        let one_sec = SR as usize;
        let first = peak(&out[..one_sec]);
        let last = peak(&out[frames - one_sec..]);

        assert!(
            out.iter().all(|f| f.l.is_finite() && f.r.is_finite()),
            "output must stay finite at max decay"
        );
        assert!(
            out.iter().all(|f| f.l.abs() < 4.0 && f.r.abs() < 4.0),
            "output must stay bounded at max decay"
        );
        // Modulated allpasses ripple slightly, so allow slack, but the tail
        // must not grow: net loop gain stays below unity.
        assert!(
            last <= first * 1.5,
            "tail must not grow: first-second peak {first}, last-second peak {last}"
        );
    }

    #[test]
    fn decay_time_is_sane() {
        // decay = 0.5, no damping, full wet: the -60 dB point of the impulse
        // response should land well under a handful of seconds but not be
        // instant. Coarse windowed-RMS check, deliberately loose.
        let reverb = PlateReverbEffect::new(SR, 0.5, 1.0, 0.0);
        let frames = (SR * 5.0) as usize;
        let out = impulse_response(&reverb, frames);

        let window = (SR * 0.1) as usize;
        let rms: Vec<f32> = out
            .chunks(window)
            .map(|c| {
                let sum: f32 = c.iter().map(|f| f.l * f.l + f.r * f.r).sum();
                (sum / (2.0 * c.len() as f32)).sqrt()
            })
            .collect();

        let peak_rms = rms.iter().cloned().fold(0.0_f32, f32::max);
        let threshold = peak_rms * 0.001; // -60 dB
        let t60_window = rms.iter().position(|&r| r < threshold);

        let t60_secs = t60_window.map(|w| w as f32 * 0.1);
        assert!(
            matches!(t60_secs, Some(t) if (0.3..=4.0).contains(&t)),
            "expected -60 dB point between 0.3s and 4s, got {t60_secs:?}"
        );

        // Decay should be monotonic-ish once the tail is established.
        for pair in rms[2..].windows(2) {
            assert!(
                pair[1] <= pair[0] * 1.2,
                "tail should decay steadily: {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn decorrelates_left_and_right() {
        let reverb = PlateReverbEffect::new(SR, 0.7, 1.0, 0.3);
        let out = impulse_response(&reverb, SR as usize);
        let max_diff = out
            .iter()
            .fold(0.0_f32, |acc, f| acc.max((f.l - f.r).abs()));
        assert!(
            max_diff > 1e-3,
            "plate should produce decorrelated stereo, max |L-R| was {max_diff}"
        );
    }

    #[test]
    fn zero_width_collapses_to_mono() {
        let reverb = PlateReverbEffect::new(SR, 0.7, 1.0, 0.3);
        reverb.set_width(0.0);
        // Let the width smoother fully settle (it snaps to target only after
        // ~140 ms with the 15 ms time constant), then verify the wet output
        // is mono.
        for i in 0..13_230 {
            let x = if i % 1_000 == 0 { 1.0 } else { 0.0 };
            reverb.process_stereo(StereoFrame { l: x, r: x });
        }
        for i in 0..22_050 {
            let x = if i % 1_000 == 0 { 1.0 } else { 0.0 };
            let out = reverb.process_stereo(StereoFrame { l: x, r: x });
            assert!(
                (out.l - out.r).abs() < 1e-6,
                "width 0 must collapse the wet signal to mono, |L-R| = {}",
                (out.l - out.r).abs()
            );
        }
    }

    #[test]
    fn nan_input_produces_finite_output() {
        let reverb = PlateReverbEffect::new(SR, 0.8, 1.0, 0.2);
        for _ in 0..1_000 {
            let out = reverb.process_stereo(StereoFrame {
                l: f32::NAN,
                r: f32::INFINITY,
            });
            assert!(out.l.is_finite() && out.r.is_finite());
        }
        let out = reverb.process_stereo(StereoFrame { l: 0.5, r: 0.5 });
        assert!(out.l.is_finite() && out.r.is_finite());
    }

    #[test]
    fn reset_clears_state() {
        let reverb = PlateReverbEffect::new(SR, 0.9, 1.0, 0.0);
        impulse_response(&reverb, 8_192);
        reverb.reset();
        for _ in 0..8_192 {
            let out = reverb.process_stereo(StereoFrame { l: 0.0, r: 0.0 });
            assert_eq!(out.l, 0.0, "silence in must be silence out after reset");
            assert_eq!(out.r, 0.0, "silence in must be silence out after reset");
        }
    }

    #[test]
    fn constructor_and_setters_clamp() {
        let reverb = PlateReverbEffect::new(SR, 5.0, -1.0, 2.0);
        assert_eq!(reverb.get_decay(), 1.0);
        assert_eq!(reverb.get_mix(), 0.0);
        assert_eq!(reverb.get_damping(), 1.0);
        assert_eq!(reverb.get_predelay(), 0.0);
        assert_eq!(reverb.get_width(), 1.0);
        assert_eq!(reverb.get_size(), 0.5);

        reverb.set_predelay(7.0);
        reverb.set_width(-3.0);
        reverb.set_size(2.5);
        assert_eq!(reverb.get_predelay(), 1.0);
        assert_eq!(reverb.get_width(), 0.0);
        assert_eq!(reverb.get_size(), 1.0);
    }

    #[test]
    fn constructs_and_runs_at_common_sample_rates() {
        for sr in [22_050.0, 44_100.0, 48_000.0, 96_000.0] {
            let reverb = PlateReverbEffect::new(sr, 1.0, 1.0, 0.0);
            reverb.set_size(1.0); // max tank scale exercises buffer headroom
            let out = impulse_response(&reverb, 4_096);
            assert!(
                out.iter().all(|f| f.l.is_finite() && f.r.is_finite()),
                "sample rate {sr}: output must stay finite"
            );
        }
    }

    #[test]
    fn size_changes_the_tail() {
        let small = PlateReverbEffect::new(SR, 0.7, 1.0, 0.3);
        small.set_size(0.0);
        let large = PlateReverbEffect::new(SR, 0.7, 1.0, 0.3);
        large.set_size(1.0);
        // Settle smoothers before the impulse.
        for _ in 0..4_410 {
            small.process_stereo(StereoFrame { l: 0.0, r: 0.0 });
            large.process_stereo(StereoFrame { l: 0.0, r: 0.0 });
        }
        let out_small = impulse_response(&small, 22_050);
        let out_large = impulse_response(&large, 22_050);
        let max_diff = out_small
            .iter()
            .zip(out_large.iter())
            .fold(0.0_f32, |acc, (a, b)| acc.max((a.l - b.l).abs()));
        assert!(
            max_diff > 1e-3,
            "size must audibly change the response, max diff was {max_diff}"
        );
    }
}
