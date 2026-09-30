//! Source-compatible migration error for the retired manually pumped window.
//! External clients needing that window can opt into `legacy-visualization`.
use super::AudioBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEvent {
    SpacePressed,
    Quit,
}
pub struct WaveformDisplay;
impl WaveformDisplay {
    pub fn new(_: AudioBuffer, _: u32, _: u32, _: f32) -> Result<Self, String> {
        Err("The standalone waveform window is retired. Run `cargo run --example omni_gui --features gui` or mount a GuiPanel. External compatibility clients may explicitly enable legacy-visualization; see docs/omni-gui.md.".into())
    }
    pub fn should_close(&self) -> bool {
        true
    }
    pub fn update(&mut self) -> Vec<DisplayEvent> {
        vec![DisplayEvent::Quit]
    }
}
