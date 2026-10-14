//! Vactrol low-pass gate after Parker and D'Angelo, "A Digital Model of the
//! Buchla Lowpass-Gate" (DAFx-13).
//!
//! The audio path is the 292's "Both" mode: a passive two-pole network whose
//! cutoff and gain both fall as the vactrol resistance `Rf` rises. It is
//! discretized with trapezoidal integrators in topology-preserving form, so the
//! resistance can move every sample. The vactrol itself follows the paper's
//! heuristic: a one-pole lag on LED current that rises in about 12 ms, falls in
//! about 250 ms, and responds faster when already bright.

const C1: f64 = 1.0e-9;
const C2: f64 = 220.0e-12;
const R_ALPHA: f64 = 5.0e6;
/// Datasheet fit for the VTL5C3/2: `Rf = A / If^1.4 + B`.
const RF_A: f64 = 3.464;
const RF_B: f64 = 1_136.212;
const IF_MIN: f64 = 10.0e-6;
const IF_MAX: f64 = 40.0e-3;
const RISE_SECONDS: f64 = 0.012;
/// VTL5C3/2 datasheet release; callers may shorten it.
const FALL_SECONDS: f64 = 0.25;
const MIN_FALL_SECONDS: f64 = 0.005;
/// At full drive the vactrol responds this much faster than when dark.
const BRIGHT_SPEEDUP: f64 = 0.7;

pub struct VactrolLpg {
    half_period: f64,
    sample_rate: f64,
    fall_seconds: f64,
    /// LED current normalized to `IF_MAX`, after the vactrol's memory lag.
    vactrol: f64,
    s_x: f64,
    s_out: f64,
}

impl VactrolLpg {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0) as f64;
        Self {
            half_period: 0.5 / sample_rate,
            sample_rate,
            fall_seconds: FALL_SECONDS,
            vactrol: 0.0,
            s_x: 0.0,
            s_out: 0.0,
        }
    }

    /// Set the vactrol release time constant (defaults to the datasheet 250 ms).
    pub fn set_fall_seconds(&mut self, seconds: f32) {
        if seconds.is_finite() {
            self.fall_seconds = (seconds as f64).clamp(MIN_FALL_SECONDS, 10.0);
        }
    }

    /// Process one sample. `control` in 0..=1 drives the vactrol LED linearly.
    #[inline]
    pub fn process(&mut self, input: f32, control: f32) -> f32 {
        let control = if control.is_finite() {
            control.clamp(0.0, 1.0) as f64
        } else {
            0.0
        };
        let base = if control > self.vactrol {
            RISE_SECONDS
        } else {
            self.fall_seconds
        };
        let tau = base * (1.0 - BRIGHT_SPEEDUP * self.vactrol);
        let coeff = (1.0 / (tau * self.sample_rate)).min(1.0);
        self.vactrol += (control - self.vactrol) * coeff;

        let led = (self.vactrol * IF_MAX).max(IF_MIN);
        let rf = RF_A / led.powf(1.4) + RF_B;

        // States [Vx, Vout]; solve (I - hA) y = s + hBu for the trapezoidal step.
        let h = self.half_period;
        let x_rate = h / (C2 * rf);
        let out_rate = h / (C1 * rf);
        let m11 = 1.0 + 2.0 * x_rate;
        let m12 = -x_rate;
        let m21 = -out_rate;
        let m22 = 1.0 + out_rate + h / (C1 * R_ALPHA);
        let input = if input.is_finite() { input as f64 } else { 0.0 };
        let r1 = self.s_x + x_rate * input;
        let r2 = self.s_out;
        let det = m11 * m22 - m12 * m21;
        let vx = (r1 * m22 - m12 * r2) / det;
        let vout = (m11 * r2 - m21 * r1) / det;
        self.s_x = 2.0 * vx - self.s_x;
        self.s_out = 2.0 * vout - self.s_out;

        if vout.is_finite() {
            vout as f32
        } else {
            self.reset();
            0.0
        }
    }

    /// Current vactrol brightness in 0..=1, for metering and tests.
    pub fn vactrol(&self) -> f32 {
        self.vactrol as f32
    }

    pub fn reset(&mut self) {
        self.vactrol = 0.0;
        self.s_x = 0.0;
        self.s_out = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn tone_rms(lpg: &mut VactrolLpg, hz: f32, control: f32, seconds: f32) -> f32 {
        let frames = (seconds * SAMPLE_RATE) as usize;
        let energy: f32 = (0..frames)
            .map(|n| {
                let x = (std::f32::consts::TAU * hz * n as f32 / SAMPLE_RATE).sin();
                let y = lpg.process(x, control);
                y * y
            })
            .sum();
        (energy / frames as f32).sqrt()
    }

    #[test]
    fn open_gate_passes_audio_and_closed_gate_silences_it() {
        let mut lpg = VactrolLpg::new(SAMPLE_RATE);
        tone_rms(&mut lpg, 440.0, 1.0, 0.2);
        let open = tone_rms(&mut lpg, 440.0, 1.0, 0.1);
        assert!(open > 0.6, "open={open}");

        let mut lpg = VactrolLpg::new(SAMPLE_RATE);
        let closed = tone_rms(&mut lpg, 440.0, 0.0, 0.1);
        assert!(closed < 0.05, "closed={closed}");
    }

    #[test]
    fn release_is_slower_than_attack() {
        let mut lpg = VactrolLpg::new(SAMPLE_RATE);
        let attack_frames = (0..)
            .take_while(|_| lpg.process(0.0, 1.0) >= 0.0 && lpg.vactrol() < 0.5)
            .count();
        let release_frames = (0..)
            .take_while(|_| lpg.process(0.0, 0.0) >= 0.0 && lpg.vactrol() > 0.5 * 0.5)
            .count();
        assert!(
            release_frames > attack_frames * 5,
            "attack={attack_frames} release={release_frames}"
        );
    }

    #[test]
    fn falling_control_darkens_before_it_silences() {
        let ratios: Vec<f32> = [1.0, 0.1, 0.03, 0.01, 0.003, 0.001]
            .iter()
            .map(|&level| {
                let measure = |hz| {
                    let mut lpg = VactrolLpg::new(SAMPLE_RATE);
                    lpg.vactrol = level as f64;
                    tone_rms(&mut lpg, hz, level, 0.05)
                };
                measure(6_000.0) / measure(200.0).max(1.0e-9)
            })
            .collect();
        assert!(
            ratios.windows(2).all(|pair| pair[1] < pair[0]),
            "{ratios:?}"
        );
        assert!(ratios[5] < ratios[0] * 0.25, "{ratios:?}");
    }
}
