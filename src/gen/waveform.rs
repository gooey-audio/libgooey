#[derive(Debug, Clone, Copy)]
pub enum Waveform {
    Sine,
    Square,
    Saw,
    Triangle,
    RingMod,
    Noise,
}