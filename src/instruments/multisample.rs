//! Multi-sampled, velocity-layered keyboard instrument.
//!
//! Where [`crate::instruments::SamplerRack`] maps one buffer to one pad, a
//! multi-sample instrument maps *many* buffers across the keyboard: each
//! [`SampleZone`] covers a key range and a velocity range, and is resampled
//! from its own `root_key` to whatever note is played. That is what makes a
//! recorded acoustic instrument (a piano) sound like the instrument rather
//! than like one pitch-shifted note.
//!
//! This module is pure DSP and does no file I/O, matching the sampler rack's
//! contract: the host decodes audio and hands over PCM. Loading a pack from
//! disk lives in [`crate::instruments::multisample_pack`] behind the `bounce`
//! feature.
//!
//! Zone storage reuses [`StereoSampleBuffer`], which is already `Arc`-backed
//! (cheap to clone into a voice) and reads fractional positions with cubic
//! interpolation.

use std::sync::Arc;

use crate::engine::Instrument;
use crate::envelope::{ADSRConfig, Envelope};
use crate::frame::StereoFrame;
use crate::mixer::StereoSampleBuffer;
use crate::utils::SmoothedParam;

/// Simultaneous sounding samples. A pedalled piano chord progression layers
/// far more notes than a synth patch, so this is deliberately generous.
pub const MULTISAMPLE_VOICE_COUNT: usize = 32;

/// Extra voice slots that exist only so a *stolen* note can ramp to silence
/// instead of being cut mid-sample.
///
/// Stealing needs the victim's slot immediately, so fading it in place is not
/// possible — it is moved here (a cheap struct move; the PCM is behind an
/// `Arc`) and finishes its ~6 ms fade while the new note starts. These are not
/// playable polyphony and do not appear in
/// [`MultiSampleInstrument::active_voice_count`].
const FADE_SLOT_COUNT: usize = 4;

/// Upper bound on zones in one map, so a malformed pack cannot exhaust memory.
pub const MULTISAMPLE_MAX_ZONES: usize = 1024;

/// Velocity layers kept when thinning a large pack. Six is enough for a smooth
/// piano dynamic range without the footprint of a 16-layer library.
pub const DEFAULT_VELOCITY_LAYERS: usize = 6;

/// Frames of linear fade applied at the start and end of a zone's playable
/// region, so a trimmed-in start or a hard end never clicks.
const CLICK_GUARD_FRAMES: f64 = 32.0;

/// Seconds over which a voice is faded out when it is stolen, or when the same
/// key is struck again (piano "self-masking"). Short enough to be inaudible as
/// a duck, long enough to avoid a click.
const FAST_FADE_SECS: f32 = 0.006;

/// Largest release time a preset may ask for, in seconds.
const MAX_RELEASE_SECS: f32 = 8.0;

/// How a zone's buffer behaves when the cursor reaches the end of the sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
    /// Play once to the end of the region, then the voice ends.
    #[default]
    NoLoop,
    /// Play once, ignoring note-off (used for percussive one-shots).
    OneShot,
    /// Loop the `[loop_start, loop_end)` region for the life of the voice.
    LoopContinuous,
    /// Loop while the note is held; play out past `loop_end` once released.
    LoopSustain,
}

/// Which gesture starts a zone: striking the key, or letting it go. Release
/// zones carry damper/string noise and are triggered on note-off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoneTrigger {
    #[default]
    Attack,
    Release,
}

/// One mapped recording: a buffer plus the key range, velocity range, and
/// tuning metadata that decide when and how it is played.
#[derive(Clone, Debug)]
pub struct SampleZone {
    pub buffer: StereoSampleBuffer,
    /// Lowest MIDI note this zone answers to.
    pub lokey: u8,
    /// Highest MIDI note this zone answers to.
    pub hikey: u8,
    /// The note the recording was made at. Playback ratio is
    /// `2^((note - root_key)/12)`.
    pub root_key: u8,
    /// Lowest MIDI velocity (1–127) this zone answers to.
    pub lovel: u8,
    /// Highest MIDI velocity (1–127) this zone answers to.
    pub hivel: u8,
    /// Fine tuning in cents, folded into the playback ratio.
    pub tune_cents: f32,
    /// Per-zone trim in decibels, for level-matching layers.
    pub volume_db: f32,
    /// Stereo balance, 0.0 = hard left, 0.5 = center (identity), 1.0 = right.
    pub pan: f32,
    pub loop_mode: LoopMode,
    /// Loop region in source frames. Ignored unless `loop_mode` loops.
    pub loop_start: usize,
    pub loop_end: usize,
    /// First frame played (skips leading silence).
    pub offset: usize,
    /// Last frame played, or `None` for "to the end of the buffer".
    pub end: Option<usize>,
    /// Amplitude envelope. For a piano the recording carries the decay, so this
    /// is usually a near-instant attack, full sustain, and a short release.
    pub envelope: ADSRConfig,
    /// How strongly velocity scales amplitude, 0.0–1.0. The recorded layers
    /// already carry most of the dynamic change; this smooths *within* a layer.
    pub amp_veltrack: f32,
    pub trigger: ZoneTrigger,
}

impl SampleZone {
    /// A zone covering a single key at full velocity, with sensible defaults.
    /// Callers override the fields they care about.
    pub fn new(buffer: StereoSampleBuffer, root_key: u8) -> Self {
        Self {
            buffer,
            lokey: root_key,
            hikey: root_key,
            root_key,
            lovel: 1,
            hivel: 127,
            tune_cents: 0.0,
            volume_db: 0.0,
            pan: 0.5,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            offset: 0,
            end: None,
            envelope: ADSRConfig::new(0.001, 0.001, 1.0, 0.4),
            amp_veltrack: 0.6,
            trigger: ZoneTrigger::Attack,
        }
    }

    pub fn with_key_range(mut self, lokey: u8, hikey: u8) -> Self {
        self.lokey = lokey.min(hikey);
        self.hikey = hikey.max(lokey);
        self
    }

    pub fn with_velocity_range(mut self, lovel: u8, hivel: u8) -> Self {
        self.lovel = lovel.min(hivel).max(1);
        self.hivel = hivel.max(lovel).max(1);
        self
    }

    /// Validate the zone against its buffer. Returns the reason on failure.
    fn validate(&self) -> Result<(), String> {
        if self.lokey > self.hikey {
            return Err(format!(
                "zone key range is inverted: {}..{}",
                self.lokey, self.hikey
            ));
        }
        if self.lovel > self.hivel || self.hivel == 0 {
            return Err(format!(
                "zone velocity range is invalid: {}..{}",
                self.lovel, self.hivel
            ));
        }
        if !self.tune_cents.is_finite()
            || !self.volume_db.is_finite()
            || !self.pan.is_finite()
            || !self.amp_veltrack.is_finite()
        {
            return Err("zone has a non-finite parameter".to_string());
        }
        if self.offset >= self.buffer.len() {
            return Err(format!(
                "zone offset {} is past the end of a {}-frame buffer",
                self.offset,
                self.buffer.len()
            ));
        }
        if let Some(end) = self.end {
            if end <= self.offset {
                return Err(format!(
                    "zone end {end} must be greater than offset {}",
                    self.offset
                ));
            }
        }
        if self.loops() {
            let (start, end) = (self.loop_start, self.loop_end);
            if end <= start || end > self.buffer.len() {
                return Err(format!(
                    "zone loop region {start}..{end} does not fit a {}-frame buffer",
                    self.buffer.len()
                ));
            }
        }
        Ok(())
    }

    fn loops(&self) -> bool {
        matches!(
            self.loop_mode,
            LoopMode::LoopContinuous | LoopMode::LoopSustain
        )
    }

    /// Last frame this zone plays, honoring an explicit `end` trim.
    fn end_frame(&self) -> f64 {
        self.end
            .map(|e| e.min(self.buffer.len()))
            .unwrap_or_else(|| self.buffer.len()) as f64
    }
}

/// A keyboard-wide mapping: every zone, plus a per-key index so voice
/// allocation never scans the whole zone list on the audio thread.
#[derive(Debug, Default)]
pub struct SampleMap {
    zones: Vec<SampleZone>,
    /// For each MIDI note, the zone indices whose key range covers it.
    by_key: Vec<Vec<u16>>,
}

impl SampleMap {
    pub fn new() -> Self {
        Self {
            zones: Vec::new(),
            by_key: Vec::new(),
        }
    }

    /// Add a zone. Rejects malformed zones rather than letting them reach the
    /// audio thread.
    pub fn push_zone(&mut self, zone: SampleZone) -> Result<(), String> {
        if self.zones.len() >= MULTISAMPLE_MAX_ZONES {
            return Err(format!(
                "sample map is limited to {MULTISAMPLE_MAX_ZONES} zones"
            ));
        }
        zone.validate()?;
        self.zones.push(zone);
        Ok(())
    }

    /// Finalize the map: build the per-key index and hand back a shared handle.
    /// A map is immutable once built, so swapping one in is a pointer swap.
    pub fn build(mut self) -> Arc<SampleMap> {
        let mut by_key: Vec<Vec<u16>> = vec![Vec::new(); 128];
        for (index, zone) in self.zones.iter().enumerate() {
            for key in zone.lokey..=zone.hikey {
                by_key[key as usize].push(index as u16);
            }
        }
        self.by_key = by_key;
        Arc::new(self)
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    pub fn zone(&self, index: usize) -> Option<&SampleZone> {
        self.zones.get(index)
    }

    /// Lowest and highest playable MIDI note, or `None` for an empty map.
    pub fn key_range(&self) -> Option<(u8, u8)> {
        let lo = self.zones.iter().map(|z| z.lokey).min()?;
        let hi = self.zones.iter().map(|z| z.hikey).max()?;
        Some((lo, hi))
    }

    /// Number of distinct velocity layers, measured by distinct `hivel` values
    /// across attack zones.
    pub fn velocity_layers(&self) -> usize {
        let mut tops: Vec<u8> = self
            .zones
            .iter()
            .filter(|z| z.trigger == ZoneTrigger::Attack)
            .map(|z| z.hivel)
            .collect();
        tops.sort_unstable();
        tops.dedup();
        tops.len()
    }

    /// The zone that should sound for `note` at `velocity` (1–127), or `None`
    /// when the map does not cover that combination. Later zones win ties, so
    /// a pack that overlaps regions behaves like an SFZ player.
    pub fn select(&self, note: u8, velocity: u8, trigger: ZoneTrigger) -> Option<usize> {
        let candidates = self.by_key.get(note as usize)?;
        candidates
            .iter()
            .rev()
            .map(|&index| index as usize)
            .find(|&index| {
                let zone = &self.zones[index];
                zone.trigger == trigger && velocity >= zone.lovel && velocity <= zone.hivel
            })
    }
}

// ---------------------------------------------------------------------------
// Config / Params
// ---------------------------------------------------------------------------

mod ranges {
    /// Multiplier applied to each zone's authored release time (damper speed).
    /// Deliberately **centered**: 0.5 is exactly 1.0x, so the default preset
    /// honors a pack's `ampeg_release` rather than quietly rescaling it.
    /// 0.0 = 0.25x (tight damper), 1.0 = 4.0x (slow damper).
    pub fn release_multiplier(normalized: f32) -> f32 {
        0.25 * 16.0_f32.powf(normalized.clamp(0.0, 1.0))
    }

    /// 0.0 = mono (fully collapsed), 0.5 = as recorded, 1.0 = 2x wide.
    pub fn width(normalized: f32) -> f32 {
        normalized.clamp(0.0, 1.0) * 2.0
    }
}

/// Static preset for a multi-sample instrument. All values are normalized 0–1
/// per the repo convention; the instrument denormalizes internally.
#[derive(Clone, Copy, Debug)]
pub struct MultiSampleConfig {
    /// Output level.
    pub volume: f32,
    /// How strongly velocity scales amplitude on top of layer selection.
    pub velocity_track: f32,
    /// Damper speed, as a multiplier on each zone's authored release.
    /// **0.5 is neutral** — the pack plays exactly as authored.
    pub release: f32,
    /// Stereo width of the recorded image. 0.5 is the recorded image.
    pub stereo_width: f32,
}

impl MultiSampleConfig {
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Self {
        Self {
            volume: 0.8,
            velocity_track: 0.6,
            release: 0.5, // 1.0x — honor the pack's own damper
            stereo_width: 0.5,
        }
    }

    /// Slower damper and a gentler velocity response — ballad playing.
    pub fn soft() -> Self {
        Self {
            volume: 0.75,
            velocity_track: 0.75,
            release: 0.65, // ~1.7x
            stereo_width: 0.5,
        }
    }

    /// Tight damper and a wide image — comping and rhythm parts.
    pub fn bright() -> Self {
        Self {
            volume: 0.85,
            velocity_track: 0.45,
            release: 0.38, // ~0.7x
            stereo_width: 0.65,
        }
    }
}

pub struct MultiSampleParams {
    pub volume: SmoothedParam,
    pub velocity_track: SmoothedParam,
    pub release: SmoothedParam,
    pub stereo_width: SmoothedParam,
}

impl MultiSampleParams {
    pub fn from_config(config: &MultiSampleConfig, sample_rate: f32) -> Self {
        Self {
            volume: SmoothedParam::new_normalized(config.volume, sample_rate),
            velocity_track: SmoothedParam::new_normalized(config.velocity_track, sample_rate),
            release: SmoothedParam::new_normalized(config.release, sample_rate),
            stereo_width: SmoothedParam::new_normalized(config.stereo_width, sample_rate),
        }
    }

    pub fn tick(&mut self) {
        self.volume.tick();
        self.velocity_track.tick();
        self.release.tick();
        self.stereo_width.tick();
    }

    pub fn snap_all(&mut self) {
        self.volume.snap();
        self.velocity_track.snap();
        self.release.snap();
        self.stereo_width.snap();
    }
}

// ---------------------------------------------------------------------------
// Voices
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MsVoice {
    /// `Some` while sounding. Holding the buffer (not just a map index) keeps
    /// the PCM alive if the map is swapped mid-note.
    buffer: Option<StereoSampleBuffer>,
    note: u8,
    zone: usize,
    position: f64,
    increment: f64,
    start: f64,
    end: f64,
    loop_start: f64,
    loop_end: f64,
    loop_mode: LoopMode,
    gain: f32,
    pan: f32,
    envelope: Envelope,
    elapsed_secs: f64,
    dt: f64,
    trigger_order: u64,
    /// The key is still physically down.
    held: bool,
    /// The key was released while the sustain pedal was down, so the voice
    /// keeps ringing until the pedal lifts.
    sustained: bool,
    /// Fast-fade multiplier used for stealing and self-masking. 1.0 = normal.
    fade: f32,
    /// Per-sample decrement applied to `fade`. Zero means no fade in progress.
    fade_step: f32,
}

impl Default for MsVoice {
    fn default() -> Self {
        Self {
            buffer: None,
            note: 0,
            zone: 0,
            position: 0.0,
            increment: 1.0,
            start: 0.0,
            end: 0.0,
            loop_start: 0.0,
            loop_end: 0.0,
            loop_mode: LoopMode::NoLoop,
            gain: 0.0,
            pan: 0.5,
            envelope: Envelope::new(),
            elapsed_secs: 0.0,
            dt: 0.0,
            trigger_order: 0,
            held: false,
            sustained: false,
            fade: 1.0,
            fade_step: 0.0,
        }
    }
}

impl MsVoice {
    fn active(&self) -> bool {
        self.buffer.is_some()
    }

    /// Whether the voice has begun releasing (either by note-off or by a fade).
    fn releasing(&self) -> bool {
        self.envelope.release_time_start.is_some() || self.fade_step > 0.0
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        note: u8,
        zone_index: usize,
        zone: &SampleZone,
        engine_rate: f32,
        gain: f32,
        release_scale: f32,
        trigger_order: u64,
    ) {
        let frames = zone.buffer.len() as f64;
        self.note = note;
        self.zone = zone_index;
        self.start = (zone.offset as f64).min(frames - 1.0).max(0.0);
        self.end = zone.end_frame().max(self.start + 1.0);
        self.position = self.start;

        let semitones = (note as f64 - zone.root_key as f64) + zone.tune_cents as f64 / 100.0;
        self.increment =
            (zone.buffer.sample_rate() as f64 / engine_rate as f64) * (semitones / 12.0).exp2();

        self.loop_mode = zone.loop_mode;
        self.loop_start = zone.loop_start as f64;
        self.loop_end = zone.loop_end as f64;

        self.gain = gain;
        self.pan = zone.pan.clamp(0.0, 1.0);

        // The zone's own release is the damper character of the recording;
        // the instrument's `release` param scales it so a host can play the
        // whole map tighter or looser without re-authoring the pack.
        let mut config = zone.envelope;
        config.release_time = (config.release_time * release_scale).clamp(0.001, MAX_RELEASE_SECS);
        self.envelope.set_config(config);
        self.envelope.trigger(0.0);

        self.elapsed_secs = 0.0;
        self.dt = 1.0 / engine_rate as f64;
        self.trigger_order = trigger_order;
        self.held = true;
        self.sustained = false;
        self.fade = 1.0;
        self.fade_step = 0.0;
        self.buffer = Some(zone.buffer.clone());
    }

    /// Begin a fast fade to silence. Used for voice stealing and for the
    /// piano's note self-masking, both of which must not click.
    fn begin_fast_fade(&mut self) {
        if !self.active() || self.fade_step > 0.0 {
            return;
        }
        let samples = (FAST_FADE_SECS as f64 / self.dt.max(f64::MIN_POSITIVE)).max(1.0);
        self.fade_step = (1.0 / samples) as f32;
    }

    /// A one-shot plays to the end of its region no matter what the key or the
    /// pedal does — that is the whole meaning of the mode. Only [`Self::stop`]
    /// (transport stop, teardown) cuts it short.
    fn ignores_note_off(&self) -> bool {
        self.loop_mode == LoopMode::OneShot
    }

    fn release(&mut self) {
        if self.ignores_note_off() {
            return;
        }
        self.envelope.release(self.elapsed_secs);
    }

    fn stop(&mut self) {
        self.buffer = None;
        self.held = false;
        self.sustained = false;
    }

    fn tick(&mut self) -> StereoFrame {
        let Some(buffer) = self.buffer.as_ref() else {
            return StereoFrame::default();
        };

        let looping = matches!(
            self.loop_mode,
            LoopMode::LoopContinuous | LoopMode::LoopSustain
        ) && !(self.loop_mode == LoopMode::LoopSustain && self.releasing());

        let frame = if looping {
            buffer.read_wrapped(self.position)
        } else {
            buffer.read_interpolated(self.position)
        };

        // Fade in from the zone's own start frame and out toward its end, so a
        // trimmed region never begins or ends on a discontinuity. A looping
        // voice has no end edge to guard.
        let from_start = (self.position - self.start) / CLICK_GUARD_FRAMES;
        let click_guard = if looping {
            from_start.clamp(0.0, 1.0) as f32
        } else {
            let to_end = (self.end - self.position) / CLICK_GUARD_FRAMES;
            from_start.min(to_end).clamp(0.0, 1.0) as f32
        };

        let amplitude = self.envelope.get_amplitude(self.elapsed_secs);
        let gain = click_guard * amplitude * self.fade * self.gain;

        self.position += self.increment;
        self.elapsed_secs += self.dt;

        if self.fade_step > 0.0 {
            self.fade -= self.fade_step;
        }

        if looping && self.position >= self.loop_end {
            let span = self.loop_end - self.loop_start;
            if span > 0.0 {
                // Fold by the full overshoot, not one span. A short loop read at
                // a transposed increment larger than the span would otherwise
                // still sit past `loop_end` after a single subtraction, and go
                // on to either read outside the authored region or trip the
                // `position >= end` check below and stop the voice.
                self.position =
                    self.loop_start + (self.position - self.loop_start).rem_euclid(span);
            }
        }

        if self.position >= self.end || !self.envelope.is_active || self.fade <= 0.0 {
            self.stop();
        }

        frame.scaled(gain).balanced(self.pan)
    }
}

// ---------------------------------------------------------------------------
// Instrument
// ---------------------------------------------------------------------------

/// A polyphonic, velocity-layered sample player.
///
/// Construct it with an empty map and it renders silence; call
/// [`MultiSampleInstrument::set_map`] to load a pack. It is stereo-native via
/// [`MultiSampleInstrument::tick_frame`], and also implements [`Instrument`]
/// (whose `tick` downmixes) so it can be added to a native
/// [`crate::engine::Engine`] alongside the synth instruments.
pub struct MultiSampleInstrument {
    sample_rate: f32,
    map: Arc<SampleMap>,
    pub params: MultiSampleParams,
    voices: [MsVoice; MULTISAMPLE_VOICE_COUNT],
    /// Stolen voices ramping to silence. See [`FADE_SLOT_COUNT`].
    fading: [MsVoice; FADE_SLOT_COUNT],
    trigger_counter: u64,
    sustain_pedal: bool,
    /// Note staged by the sequencer via `Instrument::set_midi_note`.
    pending_note: Option<u8>,
}

impl MultiSampleInstrument {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, MultiSampleConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: MultiSampleConfig) -> Self {
        Self {
            sample_rate,
            map: Arc::new(SampleMap::new()),
            params: MultiSampleParams::from_config(&config, sample_rate),
            voices: std::array::from_fn(|_| MsVoice::default()),
            fading: std::array::from_fn(|_| MsVoice::default()),
            trigger_counter: 0,
            sustain_pedal: false,
            pending_note: None,
        }
    }

    pub fn with_map(sample_rate: f32, map: Arc<SampleMap>) -> Self {
        let mut instrument = Self::new(sample_rate);
        instrument.set_map(map);
        instrument
    }

    /// Swap in a new mapping. Sounding voices keep their cloned buffers and
    /// ring out normally, so loading a pack mid-performance never hard-cuts.
    pub fn set_map(&mut self, map: Arc<SampleMap>) {
        self.map = map;
    }

    pub fn map(&self) -> &Arc<SampleMap> {
        &self.map
    }

    pub fn set_config(&mut self, config: MultiSampleConfig) {
        self.params.volume.set_target(config.volume);
        self.params.velocity_track.set_target(config.velocity_track);
        self.params.release.set_target(config.release);
        self.params.stereo_width.set_target(config.stereo_width);
    }

    pub fn snap_params(&mut self) {
        self.params.snap_all();
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Strike a key. `velocity` is normalized 0–1 and is converted to the MIDI
    /// 1–127 range used for layer selection. Returns false when the map has no
    /// zone for that note and velocity.
    pub fn note_on(&mut self, note: u8, velocity: f32) -> bool {
        if note > 127 {
            return false;
        }
        let velocity = velocity.clamp(0.0, 1.0);
        let midi_velocity = ((velocity * 127.0).round() as u8).max(1);

        // Piano self-masking: re-striking a key damps whatever that key was
        // already ringing, instead of letting two copies stack up.
        for voice in &mut self.voices {
            if voice.active() && voice.note == note {
                voice.begin_fast_fade();
            }
        }

        self.start_zone_voice(note, midi_velocity, velocity, ZoneTrigger::Attack)
    }

    /// Release a key. With the sustain pedal down the voice keeps ringing and
    /// is only released when the pedal lifts. Voices from
    /// [`LoopMode::OneShot`] zones ignore this entirely and play to the end.
    pub fn note_off(&mut self, note: u8) {
        let mut damped_a_string = false;
        for voice in &mut self.voices {
            if !voice.active() || voice.note != note || !voice.held {
                continue;
            }
            // The key is up either way; only what happens to the *sound*
            // depends on the pedal and the zone's loop mode.
            voice.held = false;
            if voice.ignores_note_off() {
                continue;
            }
            if self.sustain_pedal {
                voice.sustained = true;
            } else {
                voice.release();
                damped_a_string = true;
            }
        }

        // Damper noise only makes sense once a string is actually stopped.
        if damped_a_string {
            self.start_zone_voice(note, 64, 0.5, ZoneTrigger::Release);
        }
    }

    /// Press or lift the sustain pedal (MIDI CC64).
    pub fn set_sustain_pedal(&mut self, down: bool) {
        if self.sustain_pedal == down {
            return;
        }
        self.sustain_pedal = down;
        if down {
            return;
        }
        for voice in &mut self.voices {
            if voice.active() && voice.sustained {
                voice.sustained = false;
                voice.release();
            }
        }
    }

    pub fn sustain_pedal(&self) -> bool {
        self.sustain_pedal
    }

    /// Release every sounding voice, ignoring the pedal. One-shot voices still
    /// play out; use [`Self::stop_all`] to cut everything.
    pub fn release_all(&mut self) {
        for voice in &mut self.voices {
            if voice.active() {
                voice.held = false;
                voice.sustained = false;
                voice.release();
            }
        }
    }

    /// Cut every voice immediately. Only for transport stops and teardown —
    /// this can click, unlike [`Self::release_all`].
    pub fn stop_all(&mut self) {
        for voice in self.voices.iter_mut().chain(self.fading.iter_mut()) {
            voice.stop();
        }
    }

    /// Notes currently sounding in the playable pool. Stolen voices finishing
    /// their fade are excluded: they are no longer notes, and counting them
    /// would make a UI read above the instrument's polyphony.
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// The zone a sounding voice is playing, for UI readouts. Returns
    /// `(note, zone_index)` pairs for active voices, newest last.
    pub fn sounding_zones(&self) -> Vec<(u8, usize)> {
        let mut sounding: Vec<_> = self
            .voices
            .iter()
            .filter(|v| v.active())
            .map(|v| (v.trigger_order, v.note, v.zone))
            .collect();
        sounding.sort_unstable_by_key(|(order, _, _)| *order);
        sounding
            .into_iter()
            .map(|(_, note, zone)| (note, zone))
            .collect()
    }

    fn start_zone_voice(
        &mut self,
        note: u8,
        midi_velocity: u8,
        velocity: f32,
        trigger: ZoneTrigger,
    ) -> bool {
        let Some(zone_index) = self.map.select(note, midi_velocity, trigger) else {
            return false;
        };
        // `map` is an Arc; clone the handle so the zone borrow does not keep
        // `self` borrowed while we take a mutable voice.
        let map = Arc::clone(&self.map);
        let Some(zone) = map.zone(zone_index) else {
            return false;
        };

        let veltrack = self.params.velocity_track.get() * zone.amp_veltrack;
        // Amplitude follows velocity within the layer, blended toward unity by
        // `veltrack`, so a pack with many layers can dial the extra scaling
        // down and one with few layers can lean on it.
        let velocity_gain = 1.0 - veltrack * (1.0 - velocity);
        let gain = velocity_gain * db_to_gain(zone.volume_db);
        let release_scale = ranges::release_multiplier(self.params.release.get());

        let voice_index = self.allocate_voice();
        self.trigger_counter = self.trigger_counter.wrapping_add(1);
        let order = self.trigger_counter;
        self.voices[voice_index].start(
            note,
            zone_index,
            zone,
            self.sample_rate,
            gain,
            release_scale,
            order,
        );
        true
    }

    /// Prefer a free voice; then the oldest voice that is already releasing;
    /// then the oldest voice overall. A stolen voice is moved to a fade slot so
    /// it ramps to silence rather than being cut off mid-sample.
    fn allocate_voice(&mut self) -> usize {
        if let Some(index) = self.voices.iter().position(|v| !v.active()) {
            return index;
        }

        let oldest_releasing = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.releasing())
            .min_by_key(|(_, v)| v.trigger_order)
            .map(|(i, _)| i);

        let index = oldest_releasing.unwrap_or_else(|| {
            self.voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.trigger_order)
                .map(|(i, _)| i)
                .unwrap_or(0)
        });

        // Vacate the slot before the caller overwrites it. `MsVoice` holds its
        // PCM behind an `Arc`, so this move is a handful of words, not a copy
        // of the sample.
        let victim = std::mem::take(&mut self.voices[index]);
        self.retire_to_fade_slot(victim);
        index
    }

    /// Park a stolen voice in a fade slot and start its ramp to silence.
    fn retire_to_fade_slot(&mut self, mut victim: MsVoice) {
        victim.begin_fast_fade();
        let slot = self
            .fading
            .iter()
            .position(|v| !v.active())
            .unwrap_or_else(|| {
                // Every fade slot is busy. Recycle the one furthest through its
                // ramp: it is the quietest, so cutting it is the least audible
                // choice available.
                self.fading
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.fade.total_cmp(&b.fade))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        self.fading[slot] = victim;
    }

    /// Generate one stereo sample. This is the instrument's native output;
    /// [`Instrument::tick_stereo`] forwards to it and [`Instrument::tick`]
    /// downmixes it. Named distinctly from the trait method so calling it on a
    /// concrete `MultiSampleInstrument` is never ambiguous.
    pub fn tick_frame(&mut self) -> StereoFrame {
        self.params.tick();

        let mut out = StereoFrame::default();
        for voice in &mut self.voices {
            out += voice.tick();
        }
        // Stolen voices finishing their ramp still make sound — that is the
        // point of them.
        for voice in &mut self.fading {
            out += voice.tick();
        }

        let out = apply_width(out, ranges::width(self.params.stereo_width.get()));

        // Fixed headroom sized for a full four-note pedalled chord. Scaling by
        // the live voice count would zipper on every chord change, the same
        // reason PolySynth uses a constant divisor.
        out.scaled(self.params.volume.get() * 0.25)
    }
}

/// Mid/side width control. `width == 1.0` is the recorded image, `0.0` is mono,
/// `2.0` doubles the side signal.
#[inline]
fn apply_width(frame: StereoFrame, width: f32) -> StereoFrame {
    if width == 1.0 {
        return frame;
    }
    let mid = 0.5 * (frame.l + frame.r);
    let side = 0.5 * (frame.l - frame.r) * width;
    StereoFrame {
        l: mid + side,
        r: mid - side,
    }
}

#[inline]
fn db_to_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

impl Instrument for MultiSampleInstrument {
    fn trigger_with_velocity(&mut self, _time: f64, velocity: f32) {
        let note = self.pending_note.take().unwrap_or(60);
        self.note_on(note, velocity);
    }

    fn tick(&mut self, _current_time: f64) -> f32 {
        self.tick_frame().downmix()
    }

    /// Hand the engine the recorded stereo image instead of a downmix, so a
    /// piano routed through [`crate::engine::Engine::tick_stereo`] keeps the
    /// width that makes a multi-sampled instrument sound three-dimensional.
    fn tick_stereo(&mut self, _current_time: f64) -> Option<StereoFrame> {
        Some(self.tick_frame())
    }

    /// True while anything is still producing sound, fade tails included — an
    /// engine must not treat the instrument as finished mid-ramp.
    fn is_active(&self) -> bool {
        self.voices
            .iter()
            .chain(self.fading.iter())
            .any(|v| v.active())
    }

    fn set_midi_note(&mut self, note: u8) {
        self.pending_note = Some(note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    /// A constant-amplitude buffer, so playback level is easy to assert on.
    fn flat_buffer(frames: usize, value: f32) -> StereoSampleBuffer {
        StereoSampleBuffer::from_channels(vec![value; frames], vec![value; frames], SR).unwrap()
    }

    /// A buffer whose amplitude ramps 0..1, so a read position maps to a value.
    fn ramp_buffer(frames: usize) -> StereoSampleBuffer {
        let ramp: Vec<f32> = (0..frames).map(|i| i as f32 / frames as f32).collect();
        StereoSampleBuffer::from_channels(ramp.clone(), ramp, SR).unwrap()
    }

    /// Two velocity layers over one octave, rooted at C4. The buffers are three
    /// seconds long so a test can distinguish "the note was damped" from "the
    /// recording simply ran out".
    fn two_layer_map() -> Arc<SampleMap> {
        const THREE_SECONDS: usize = 3 * SR as usize;
        let mut map = SampleMap::new();
        map.push_zone(
            SampleZone::new(flat_buffer(THREE_SECONDS, 0.5), 60)
                .with_key_range(55, 67)
                .with_velocity_range(1, 63),
        )
        .unwrap();
        map.push_zone(
            SampleZone::new(flat_buffer(THREE_SECONDS, 1.0), 60)
                .with_key_range(55, 67)
                .with_velocity_range(64, 127),
        )
        .unwrap();
        map.build()
    }

    fn peak(instrument: &mut MultiSampleInstrument, frames: usize) -> f32 {
        (0..frames).fold(0.0_f32, |acc, _| {
            let f = instrument.tick_frame();
            acc.max(f.l.abs()).max(f.r.abs())
        })
    }

    #[test]
    fn empty_map_is_silent_and_never_panics() {
        let mut piano = MultiSampleInstrument::new(SR);
        assert!(!piano.note_on(60, 1.0));
        assert!(!piano.is_active());
        assert_eq!(peak(&mut piano, 128), 0.0);
        piano.note_off(60);
        piano.release_all();
    }

    #[test]
    fn note_outside_the_map_does_not_allocate_a_voice() {
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        assert!(!piano.note_on(40, 1.0));
        assert_eq!(piano.active_voice_count(), 0);
        assert!(piano.note_on(60, 1.0));
        assert_eq!(piano.active_voice_count(), 1);
    }

    #[test]
    fn velocity_selects_the_matching_layer() {
        let map = two_layer_map();
        assert_eq!(map.select(60, 30, ZoneTrigger::Attack), Some(0));
        assert_eq!(map.select(60, 100, ZoneTrigger::Attack), Some(1));
        assert_eq!(map.select(60, 63, ZoneTrigger::Attack), Some(0));
        assert_eq!(map.select(60, 64, ZoneTrigger::Attack), Some(1));
        assert_eq!(map.velocity_layers(), 2);
        assert_eq!(map.key_range(), Some((55, 67)));
    }

    #[test]
    fn hard_hit_is_louder_than_soft_hit() {
        let map = two_layer_map();

        let mut soft = MultiSampleInstrument::with_map(SR, Arc::clone(&map));
        soft.snap_params();
        assert!(soft.note_on(60, 0.2));
        let soft_peak = peak(&mut soft, 512);

        let mut hard = MultiSampleInstrument::with_map(SR, map);
        hard.snap_params();
        assert!(hard.note_on(60, 1.0));
        let hard_peak = peak(&mut hard, 512);

        assert!(
            hard_peak > soft_peak * 1.5,
            "hard {hard_peak} vs soft {soft_peak}"
        );
    }

    #[test]
    fn pitch_ratio_follows_the_distance_from_the_root_key() {
        // A ramp buffer read twice as fast reaches twice the value.
        let mut map = SampleMap::new();
        map.push_zone(SampleZone::new(ramp_buffer(44_100), 60).with_key_range(48, 84))
            .unwrap();
        let map = map.build();

        let mut root = MultiSampleInstrument::with_map(SR, Arc::clone(&map));
        root.snap_params();
        root.note_on(60, 1.0);
        let root_value = peak(&mut root, 4410);

        let mut octave_up = MultiSampleInstrument::with_map(SR, map);
        octave_up.snap_params();
        octave_up.note_on(72, 1.0);
        let up_value = peak(&mut octave_up, 4410);

        let ratio = up_value / root_value;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "an octave up should advance twice as fast, ratio was {ratio}"
        );
    }

    #[test]
    fn sustain_pedal_holds_notes_until_it_lifts() {
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        piano.set_sustain_pedal(true);
        piano.note_on(60, 1.0);
        piano.note_off(60);

        // Under the pedal the voice keeps ringing well past its release time.
        for _ in 0..44_100 {
            piano.tick_frame();
        }
        assert!(piano.is_active(), "pedalled note should still be sounding");

        piano.set_sustain_pedal(false);
        for _ in 0..44_100 {
            piano.tick_frame();
        }
        assert!(!piano.is_active(), "lifting the pedal should damp the note");
    }

    #[test]
    fn note_off_without_the_pedal_releases_promptly() {
        // Control: a held note is still ringing after one second, because the
        // recording is three seconds long.
        let mut held = MultiSampleInstrument::with_map(SR, two_layer_map());
        held.snap_params();
        held.note_on(60, 1.0);
        for _ in 0..44_100 {
            held.tick_frame();
        }
        assert!(held.is_active(), "a held note should still be sounding");

        // Releasing the key damps it well inside that window.
        let mut released = MultiSampleInstrument::with_map(SR, two_layer_map());
        released.snap_params();
        released.note_on(60, 1.0);
        released.note_off(60);
        for _ in 0..44_100 {
            released.tick_frame();
        }
        assert!(!released.is_active());
    }

    #[test]
    fn restriking_a_key_does_not_stack_voices() {
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        for _ in 0..8 {
            piano.note_on(60, 1.0);
            // Long enough for the ~6 ms self-mask fade to complete.
            for _ in 0..1024 {
                piano.tick_frame();
            }
        }
        assert_eq!(
            piano.active_voice_count(),
            1,
            "self-masking should leave one voice per key"
        );
    }

    #[test]
    fn voice_pool_is_bounded_and_output_stays_finite() {
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        for i in 0..(MULTISAMPLE_VOICE_COUNT + 16) {
            // Spread across keys so self-masking does not hide the stealing.
            piano.note_on(55 + (i % 13) as u8, 1.0);
        }
        assert!(piano.active_voice_count() <= MULTISAMPLE_VOICE_COUNT);
        for _ in 0..2048 {
            let frame = piano.tick_frame();
            assert!(frame.l.is_finite() && frame.r.is_finite());
        }
    }

    #[test]
    fn map_swap_lets_sounding_voices_ring_out() {
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        piano.note_on(60, 1.0);
        for _ in 0..256 {
            piano.tick_frame();
        }
        piano.set_map(Arc::new(SampleMap::new()));
        assert!(piano.is_active(), "swapping the map must not cut a voice");
        assert!(peak(&mut piano, 256) > 0.0);
        // ...but the empty map cannot start anything new.
        assert!(!piano.note_on(60, 1.0));
    }

    #[test]
    fn zone_validation_rejects_malformed_input() {
        let mut map = SampleMap::new();
        let mut zone = SampleZone::new(flat_buffer(128, 1.0), 60);
        zone.offset = 500;
        assert!(map.push_zone(zone).is_err());

        let mut zone = SampleZone::new(flat_buffer(128, 1.0), 60);
        zone.loop_mode = LoopMode::LoopContinuous;
        zone.loop_start = 64;
        zone.loop_end = 4096;
        assert!(map.push_zone(zone).is_err());

        let mut zone = SampleZone::new(flat_buffer(128, 1.0), 60);
        zone.tune_cents = f32::NAN;
        assert!(map.push_zone(zone).is_err());

        assert!(map
            .push_zone(SampleZone::new(flat_buffer(128, 1.0), 60))
            .is_ok());
    }

    #[test]
    fn a_stolen_voice_fades_instead_of_cutting() {
        // Fill every slot, then steal. The victim must keep sounding for the
        // length of its fade rather than vanishing on the next sample.
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        for i in 0..MULTISAMPLE_VOICE_COUNT {
            piano.note_on(55 + (i % 13) as u8, 1.0);
            piano.tick_frame();
        }
        assert_eq!(piano.active_voice_count(), MULTISAMPLE_VOICE_COUNT);

        assert_eq!(
            piano.fading.iter().filter(|v| v.active()).count(),
            0,
            "nothing should be fading yet"
        );

        // Force a steal. The victim must still be rendering — in a fade slot,
        // ramping down — rather than having been overwritten in place.
        piano.note_on(67, 1.0);
        let fading: Vec<&MsVoice> = piano.fading.iter().filter(|v| v.active()).collect();
        assert_eq!(fading.len(), 1, "the stolen voice should be kept as a tail");
        assert!(fading[0].fade_step > 0.0, "the tail should be ramping down");

        // Its gain really does decrease sample over sample.
        let first = piano.fading.iter().find(|v| v.active()).unwrap().fade;
        for _ in 0..16 {
            piano.tick_frame();
        }
        let later = piano.fading.iter().find(|v| v.active()).unwrap().fade;
        assert!(later < first, "fade should progress: {first} -> {later}");

        // The playable pool never exceeds its advertised polyphony...
        assert_eq!(piano.active_voice_count(), MULTISAMPLE_VOICE_COUNT);
        // ...and the tail finishes on its own well inside the fade time.
        for _ in 0..(SR * FAST_FADE_SECS) as usize + 64 {
            piano.tick_frame();
        }
        assert_eq!(
            piano.fading.iter().filter(|v| v.active()).count(),
            0,
            "the tail should have finished"
        );
    }

    #[test]
    fn stealing_repeatedly_stays_bounded_and_finite() {
        // Far more steals than there are fade slots, so the recycle path runs.
        let mut piano = MultiSampleInstrument::with_map(SR, two_layer_map());
        piano.snap_params();
        for round in 0..8 {
            for i in 0..MULTISAMPLE_VOICE_COUNT {
                piano.note_on(55 + ((i + round) % 13) as u8, 1.0);
            }
            for _ in 0..64 {
                let f = piano.tick_frame();
                assert!(f.l.is_finite() && f.r.is_finite());
            }
        }
        assert!(piano.active_voice_count() <= MULTISAMPLE_VOICE_COUNT);
    }

    #[test]
    fn one_shot_zones_ignore_note_off_and_the_pedal() {
        let mut map = SampleMap::new();
        let mut zone = SampleZone::new(flat_buffer(SR as usize, 1.0), 60);
        zone.loop_mode = LoopMode::OneShot;
        // A release short enough that, if it were applied, the voice would be
        // long gone by the time we check.
        zone.envelope = ADSRConfig::new(0.001, 0.001, 1.0, 0.01);
        map.push_zone(zone).unwrap();
        let map = map.build();

        // Note-off must not truncate it.
        let mut piano = MultiSampleInstrument::with_map(SR, Arc::clone(&map));
        piano.snap_params();
        piano.note_on(60, 1.0);
        piano.note_off(60);
        for _ in 0..(SR as usize / 2) {
            piano.tick_frame();
        }
        assert!(piano.is_active(), "a one-shot should play past note-off");

        // Neither should release_all.
        piano.release_all();
        for _ in 0..1024 {
            piano.tick_frame();
        }
        assert!(piano.is_active(), "a one-shot should play past release_all");

        // It still ends when the sample does.
        for _ in 0..SR as usize {
            piano.tick_frame();
        }
        assert!(!piano.is_active(), "a one-shot ends with its recording");

        // ...and stop_all is the hard cut that does work.
        let mut piano = MultiSampleInstrument::with_map(SR, map);
        piano.snap_params();
        piano.note_on(60, 1.0);
        piano.stop_all();
        assert!(!piano.is_active());
    }

    #[test]
    fn a_loop_shorter_than_its_increment_stays_inside_its_region() {
        // A 2-frame loop read 25 semitones up advances ~4.24 frames per sample.
        // The step is deliberately not a multiple of the span, so a single
        // `position -= span` leaves the cursor past `loop_end` and the voice
        // reads outside the region the pack authored.
        let mut map = SampleMap::new();
        let mut zone = SampleZone::new(flat_buffer(4096, 1.0), 35).with_key_range(35, 84);
        zone.loop_mode = LoopMode::LoopContinuous;
        zone.loop_start = 100;
        zone.loop_end = 102;
        zone.envelope = ADSRConfig::new(0.001, 0.001, 1.0, 0.05);
        map.push_zone(zone).unwrap();

        let mut piano = MultiSampleInstrument::with_map(SR, map.build());
        piano.snap_params();
        assert!(piano.note_on(60, 1.0));

        let mut worst = 0.0_f64;
        for _ in 0..8192 {
            let f = piano.tick_frame();
            assert!(f.l.is_finite());
            // Once the cursor has entered the loop it must never sit past the
            // loop end again.
            if let Some(voice) = piano.voices.iter().find(|v| v.active()) {
                if voice.position >= voice.loop_start {
                    worst = worst.max(voice.position);
                }
            }
        }
        assert!(
            worst < 102.0,
            "cursor escaped the loop region, reaching {worst} (loop ends at 102)"
        );
        assert!(
            piano.is_active(),
            "a short loop must keep looping, not stop early"
        );
    }

    #[test]
    fn continuous_loop_outlives_the_buffer_length() {
        let mut map = SampleMap::new();
        let mut zone = SampleZone::new(flat_buffer(1024, 1.0), 60);
        zone.loop_mode = LoopMode::LoopContinuous;
        zone.loop_start = 0;
        zone.loop_end = 1024;
        zone.envelope = ADSRConfig::new(0.001, 0.001, 1.0, 0.05);
        map.push_zone(zone).unwrap();

        let mut piano = MultiSampleInstrument::with_map(SR, map.build());
        piano.snap_params();
        piano.note_on(60, 1.0);
        // Far past the 1024-frame buffer; a looping voice must still sound.
        for _ in 0..8192 {
            piano.tick_frame();
        }
        assert!(piano.is_active());
        assert!(peak(&mut piano, 256) > 0.0);
    }

    #[test]
    fn release_zones_fire_on_note_off() {
        let mut map = SampleMap::new();
        map.push_zone(SampleZone::new(flat_buffer(4096, 1.0), 60))
            .unwrap();
        let mut release = SampleZone::new(flat_buffer(4096, 0.25), 60);
        release.trigger = ZoneTrigger::Release;
        map.push_zone(release).unwrap();
        let map = map.build();

        assert_eq!(map.select(60, 64, ZoneTrigger::Release), Some(1));

        let mut piano = MultiSampleInstrument::with_map(SR, map);
        piano.snap_params();
        piano.note_on(60, 1.0);
        assert_eq!(piano.active_voice_count(), 1);
        piano.note_off(60);
        assert_eq!(
            piano.active_voice_count(),
            2,
            "note-off should add the damper-noise voice"
        );
    }

    /// Frames a voice keeps sounding after note-off, which is the audible
    /// length of the damper.
    fn release_frames(config: MultiSampleConfig) -> usize {
        let mut map = SampleMap::new();
        let mut zone = SampleZone::new(flat_buffer(3 * SR as usize, 1.0), 60);
        // A deliberately long authored release, so scaling is easy to measure.
        zone.envelope = ADSRConfig::new(0.001, 0.001, 1.0, 0.4);
        map.push_zone(zone).unwrap();

        let mut piano = MultiSampleInstrument::with_config(SR, config);
        piano.set_map(map.build());
        piano.snap_params();
        piano.note_on(60, 1.0);
        piano.tick_frame();
        piano.note_off(60);

        let mut frames = 0;
        while piano.is_active() && frames < 3 * SR as usize {
            piano.tick_frame();
            frames += 1;
        }
        frames
    }

    #[test]
    fn the_default_preset_honors_a_packs_authored_release() {
        // A pack authored with a 0.4 s damper must play with a 0.4 s damper at
        // the default setting — the release param scales, it does not override.
        let frames = release_frames(MultiSampleConfig::default());
        let secs = frames as f32 / SR;
        assert!(
            (secs - 0.4).abs() < 0.02,
            "default release should be 1.0x (0.40 s), measured {secs:.3} s"
        );
    }

    #[test]
    fn the_release_param_is_centered_on_neutral() {
        assert!((ranges::release_multiplier(0.5) - 1.0).abs() < 1e-5);
        assert!(ranges::release_multiplier(0.0) < 0.5, "0.0 tightens");
        assert!(ranges::release_multiplier(1.0) > 2.0, "1.0 lengthens");

        // And the presets land where their names claim.
        let tight = release_frames(MultiSampleConfig::bright());
        let neutral = release_frames(MultiSampleConfig::default());
        let slow = release_frames(MultiSampleConfig::soft());
        assert!(
            tight < neutral && neutral < slow,
            "bright {tight} < default {neutral} < soft {slow}"
        );
    }

    #[test]
    fn presets_construct_and_stereo_width_collapses_to_mono() {
        let _ = MultiSampleInstrument::with_config(SR, MultiSampleConfig::soft());
        let _ = MultiSampleInstrument::with_config(SR, MultiSampleConfig::bright());

        let wide = StereoFrame { l: 1.0, r: -1.0 };
        let mono = apply_width(wide, 0.0);
        assert!((mono.l - mono.r).abs() < 1e-6);
        assert_eq!(apply_width(wide, 1.0), wide);
    }
}
