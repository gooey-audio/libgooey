//! Safe, serializable four-track loop studio built on the existing engine.
//!
//! A `Studio` has exclusive engine ownership. Hosts serialize render/control
//! calls (for example with a mutex); no engine pointer escapes this module.
//!
//! ```no_run
//! use gooey::studio::{Control, Session, Studio};
//! # fn main() -> anyhow::Result<()> {
//! let mut studio = Studio::new(Session::demo(), 48_000)?;
//! studio.set_control(Control::Gain(1), 0.5)?;
//! studio.play(true);
//! let mut interleaved_stereo = [0.0; 1024];
//! studio.render(&mut interleaved_stereo)?;
//! let song = studio.snapshot_session();
//! song.save("song.json")?;
//! song.export_wav("mix.wav", 4, 2.0)?;
//! # Ok(()) }
//! ```
use crate::ffi::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    io::Write,
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

pub const LOOP_TICKS: u32 = 384;
const MAX_LOOP_FRAMES: u64 = 48_000 * 120;
pub const TRACK_NAMES: [&str; 4] = ["Drums", "Bass", "Nebula chords", "Audio loop"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Strip {
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub cutoff: f32,
    pub delay: f32,
    pub reverb: f32,
}
impl Default for Strip {
    fn default() -> Self {
        Self {
            gain: 0.7,
            pan: 0.5,
            mute: false,
            solo: false,
            cutoff: 18000.0,
            delay: 0.0,
            reverb: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Control {
    Gain(usize),
    Pan(usize),
    Mute(usize),
    Solo(usize),
    Cutoff(usize),
    Delay(usize),
    Reverb(usize),
    MasterGain,
    MasterDelay,
    MasterReverb,
    BassTone,
    ChordTone,
}
impl Control {
    pub fn label(self) -> String {
        if !self.valid() {
            return "Invalid track control".into();
        }
        match self {
            Self::Gain(t) => format!("{} · gain", TRACK_NAMES[t]),
            Self::Pan(t) => format!("{} · pan", TRACK_NAMES[t]),
            Self::Mute(t) => format!("{} · mute", TRACK_NAMES[t]),
            Self::Solo(t) => format!("{} · solo", TRACK_NAMES[t]),
            Self::Cutoff(t) => format!("{} · filter Hz", TRACK_NAMES[t]),
            Self::Delay(t) => format!("{} · delay", TRACK_NAMES[t]),
            Self::Reverb(t) => format!("{} · reverb", TRACK_NAMES[t]),
            Self::MasterGain => "Master · gain".into(),
            Self::MasterDelay => "Master · delay".into(),
            Self::MasterReverb => "Master · reverb".into(),
            Self::BassTone => "Bass · tone".into(),
            Self::ChordTone => "Nebula · tone".into(),
        }
    }
    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Self::Gain(_) | Self::MasterGain => 0.0..=1.5,
            Self::Cutoff(_) => 100.0..=18000.0,
            _ => 0.0..=1.0,
        }
    }
    fn valid(self) -> bool {
        match self {
            Self::Gain(t)
            | Self::Pan(t)
            | Self::Mute(t)
            | Self::Solo(t)
            | Self::Cutoff(t)
            | Self::Delay(t)
            | Self::Reverb(t) => t < 4,
            _ => true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub tick: u32,
    pub value: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Lane {
    pub control: Control,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChordEvent {
    pub tick: u32,
    pub duration: u32,
    pub degree: u32,
    pub root: u32,
    pub minor: bool,
    pub octave: i32,
    pub preset: u32,
    pub velocity: f32,
}
impl ChordEvent {
    fn ffi(&self) -> GooeyChordLoopEvent {
        GooeyChordLoopEvent {
            start_tick: self.tick,
            duration_ticks: self.duration,
            chord: GooeyChordEvent {
                target: GOOEY_CHORD_TARGET_POLY,
                target_id: 0,
                chord_set: CHORD_SET_SEVENTHS,
                root: self.root,
                scale_type: u32::from(self.minor),
                degree: self.degree,
                voicing: 0,
                preset: self.preset,
                octave: self.octave,
                velocity: self.velocity,
                gate: GOOEY_CHORD_GATE_HELD,
            },
        }
    }
}

/// Recorder events retain take/insertion order. Resolve overlaps in that order,
/// with the newest finalized take owning each tick. Older uncovered fragments
/// remain audible; the new take is never discarded merely to satisfy validation.
fn canonical_recorded_chords(events: &[ChordEvent]) -> Vec<ChordEvent> {
    let mut owner = [None; LOOP_TICKS as usize];
    for (index, event) in events.iter().enumerate() {
        for offset in 0..event.duration.min(LOOP_TICKS) {
            owner[((event.tick + offset) % LOOP_TICKS) as usize] = Some(index);
        }
    }
    let mut segments: Vec<(usize, u32, u32)> = Vec::new();
    let mut tick = 0;
    while tick < LOOP_TICKS {
        let start = tick;
        let index = owner[tick as usize];
        while tick < LOOP_TICKS && owner[tick as usize] == index {
            tick += 1;
        }
        if let Some(index) = index {
            segments.push((index, start, tick - start));
        }
    }
    // Join the two sides of a held gate crossing the loop seam. Keep its actual
    // note-on position rather than inventing a retrigger at tick zero.
    if segments.len() > 1 {
        let first = segments[0];
        let last = *segments.last().unwrap();
        if first.0 == last.0 && first.1 == 0 && last.1 + last.2 == LOOP_TICKS {
            segments.remove(0);
            let tail = segments.last_mut().unwrap();
            tail.2 += first.2;
        }
    }
    segments
        .into_iter()
        .map(|(index, tick, duration)| {
            let mut event = events[index].clone();
            event.tick = tick;
            event.duration = duration;
            event
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub tick: u32,
    pub instrument: u32,
    pub note: u8,
    pub velocity: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioLoop {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub source_bpm: f32,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub version: u32,
    pub title: String,
    pub bpm: f32,
    pub strips: [Strip; 4],
    pub master: f32,
    pub master_delay: f32,
    pub master_reverb: f32,
    pub bass_tone: f32,
    pub chord_tone: f32,
    /// Kick, snare, hat, tom, bass; a value of zero disables a step.
    pub steps: [[f32; 16]; 5],
    pub bass_notes: [u8; 16],
    pub chords: Vec<ChordEvent>,
    pub hits: Vec<Hit>,
    pub automation: Vec<Lane>,
    pub audio_loop: Option<AudioLoop>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: 1,
            title: "Untitled loop".into(),
            bpm: 116.0,
            strips: std::array::from_fn(|_| Strip::default()),
            master: 0.65,
            master_delay: 0.0,
            master_reverb: 0.0,
            bass_tone: 0.38,
            chord_tone: 0.55,
            steps: [[0.0; 16]; 5],
            bass_notes: [36; 16],
            chords: vec![],
            hits: vec![],
            automation: vec![],
            audio_loop: None,
        }
    }
}
impl Session {
    pub fn demo() -> Self {
        let mut s = Self {
            title: "Midnight orbit".into(),
            ..Self::default()
        };
        for (row, indices) in [
            (0, vec![0, 4, 8, 12]),
            (1, vec![4, 12]),
            (2, vec![0, 2, 4, 6, 8, 10, 12, 14]),
            (3, vec![14]),
            (4, vec![0, 3, 6, 8, 10, 13]),
        ] {
            for i in indices {
                s.steps[row][i] = if row == 2 { 0.35 } else { 0.8 };
            }
        }
        s.bass_notes = [
            36, 36, 36, 36, 36, 36, 43, 43, 34, 34, 36, 36, 36, 31, 31, 31,
        ];
        s.strips[2].gain = 0.45;
        s.strips[2].reverb = 0.22;
        s.strips[2].delay = 0.12;
        s.chords = [0, 5, 3, 4]
            .into_iter()
            .enumerate()
            .map(|(i, degree)| ChordEvent {
                tick: i as u32 * 96,
                duration: 84,
                degree,
                root: 0,
                minor: true,
                octave: 4,
                preset: POLY_PRESET_PAD,
                velocity: 0.65,
            })
            .collect();
        // A self-contained PCM shaker clip demonstrates the existing audio
        // loop player without bundling or downloading third-party recordings.
        let frames = (240.0 / s.bpm as f64 * 48000.0).round() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        let mut seed = 0x47a9_u32;
        for frame in 0..frames {
            let step_position = frame as f32 / frames as f32 * 16.0;
            let phase = step_position.fract();
            let envelope = (phase * 30.0).min(1.0) * (-phase * 12.0).exp();
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed >> 8) as f32 / 8388608.0 - 1.0;
            let value = noise * envelope * 0.12;
            let balance = if (step_position as usize).is_multiple_of(2) {
                0.75
            } else {
                0.25
            };
            samples.extend([value * balance, value * (1.0 - balance)]);
        }
        s.audio_loop = Some(AudioLoop {
            samples,
            sample_rate: 48000,
            source_bpm: s.bpm,
            name: "Built-in orbit shaker (generated PCM)".into(),
        });
        s
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported session version");
        ensure!(self.title.len() <= 256, "title too long");
        ensure!(
            self.bpm.is_finite() && (40.0..=240.0).contains(&self.bpm),
            "tempo must be 40–240 BPM"
        );
        for t in 0..4 {
            for c in [
                Control::Gain(t),
                Control::Pan(t),
                Control::Cutoff(t),
                Control::Delay(t),
                Control::Reverb(t),
            ] {
                self.validate_value(c, self.value(c))?;
            }
        }
        for c in [
            Control::MasterGain,
            Control::MasterDelay,
            Control::MasterReverb,
            Control::BassTone,
            Control::ChordTone,
        ] {
            self.validate_value(c, self.value(c))?;
        }
        ensure!(
            self.steps
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid step velocity"
        );
        ensure!(
            self.bass_notes.iter().all(|n| *n <= 127),
            "invalid MIDI note"
        );
        ensure!(
            self.chords.len() <= 512 && self.hits.len() <= 4096 && self.automation.len() <= 64,
            "session event limit exceeded"
        );
        for e in &self.chords {
            ensure!(
                e.tick < LOOP_TICKS
                    && (1..=LOOP_TICKS).contains(&e.duration)
                    && e.degree < 7
                    && e.root < 12
                    && (0..=8).contains(&e.octave)
                    && e.preset < POLY_PRESET_COUNT
                    && e.velocity.is_finite()
                    && (0.0..=1.0).contains(&e.velocity),
                "invalid chord event"
            );
        }
        for (i, a) in self.chords.iter().enumerate() {
            for b in &self.chords[..i] {
                let a_to_b = (b.tick + LOOP_TICKS - a.tick) % LOOP_TICKS;
                let b_to_a = (a.tick + LOOP_TICKS - b.tick) % LOOP_TICKS;
                ensure!(
                    a_to_b >= a.duration && b_to_a >= b.duration,
                    "overlapping chord gates"
                );
            }
        }
        for h in &self.hits {
            ensure!(
                h.tick < LOOP_TICKS
                    && h.instrument <= INSTRUMENT_BASS
                    && h.note <= 127
                    && h.velocity.is_finite()
                    && (0.0..=1.0).contains(&h.velocity),
                "invalid performance hit"
            );
        }
        for (i, lane) in self.automation.iter().enumerate() {
            ensure!(
                lane.control.valid()
                    && !lane.points.is_empty()
                    && lane.points.len() <= LOOP_TICKS as usize,
                "invalid automation lane"
            );
            ensure!(
                !self.automation[..i]
                    .iter()
                    .any(|l| l.control == lane.control),
                "duplicate automation lane"
            );
            let mut previous = None;
            for p in &lane.points {
                ensure!(
                    p.tick < LOOP_TICKS && previous.is_none_or(|t| t < p.tick),
                    "automation points must be ordered and unique"
                );
                self.validate_value(lane.control, p.value)?;
                previous = Some(p.tick);
            }
        }
        if let Some(l) = &self.audio_loop {
            ensure!(
                l.samples.len() >= 2
                    && l.samples.len() % 2 == 0
                    && l.samples.len() <= 48_000 * 2 * 120
                    && l.samples.iter().all(|s| s.is_finite())
                    && (8000..=192000).contains(&l.sample_rate)
                    && l.source_bpm.is_finite()
                    && (40.0..=240.0).contains(&l.source_bpm),
                "invalid audio loop (maximum 120 seconds at 48 kHz)"
            );
            ensure!(
                l.samples.len() as u64 / 2
                    <= u64::from(l.sample_rate)
                        .checked_mul(120)
                        .context("audio duration overflow")?,
                "audio loop exceeds 120 seconds at its sample rate"
            );
        }
        Ok(())
    }
    fn validate_value(&self, c: Control, v: f32) -> Result<()> {
        ensure!(
            c.valid() && v.is_finite() && c.range().contains(&v),
            "invalid control value"
        );
        Ok(())
    }
    pub fn value(&self, c: Control) -> f32 {
        if !c.valid() {
            return 0.0;
        }
        match c {
            Control::Gain(t) => self.strips[t].gain,
            Control::Pan(t) => self.strips[t].pan,
            Control::Mute(t) => {
                if self.strips[t].mute {
                    1.0
                } else {
                    0.0
                }
            }
            Control::Solo(t) => {
                if self.strips[t].solo {
                    1.0
                } else {
                    0.0
                }
            }
            Control::Cutoff(t) => self.strips[t].cutoff,
            Control::Delay(t) => self.strips[t].delay,
            Control::Reverb(t) => self.strips[t].reverb,
            Control::MasterGain => self.master,
            Control::MasterDelay => self.master_delay,
            Control::MasterReverb => self.master_reverb,
            Control::BassTone => self.bass_tone,
            Control::ChordTone => self.chord_tone,
        }
    }
    fn write_value(&mut self, c: Control, v: f32) {
        match c {
            Control::Gain(t) => self.strips[t].gain = v,
            Control::Pan(t) => self.strips[t].pan = v,
            Control::Mute(t) => self.strips[t].mute = v >= 0.5,
            Control::Solo(t) => self.strips[t].solo = v >= 0.5,
            Control::Cutoff(t) => self.strips[t].cutoff = v,
            Control::Delay(t) => self.strips[t].delay = v,
            Control::Reverb(t) => self.strips[t].reverb = v,
            Control::MasterGain => self.master = v,
            Control::MasterDelay => self.master_delay = v,
            Control::MasterReverb => self.master_reverb = v,
            Control::BassTone => self.bass_tone = v,
            Control::ChordTone => self.chord_tone = v,
        }
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        ensure!(
            bytes.len() <= 128 * 1024 * 1024,
            "serialized session exceeds 128 MiB; shorten the embedded audio loop"
        );
        let path = path.as_ref();
        let name = path.file_name().context("session path needs a filename")?;
        static SAVE_ID: AtomicU64 = AtomicU64::new(0);
        let pending = path.with_file_name(format!(
            ".{}.{}-{}.pending",
            name.to_string_lossy(),
            std::process::id(),
            SAVE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&pending, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&pending);
        }
        result
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        ensure!(
            std::fs::metadata(path.as_ref())?.len() <= 128 * 1024 * 1024,
            "session file exceeds 128 MiB"
        );
        let s: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        s.validate()?;
        Ok(s)
    }
    pub fn import_wav(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let mut r = hound::WavReader::open(path.as_ref())?;
        let spec = r.spec();
        // Validate hostile headers before arithmetic or collecting sample data.
        ensure!(
            (8000..=192000).contains(&spec.sample_rate),
            "WAV sample rate must be 8000–192000 Hz"
        );
        ensure!(
            spec.channels == 1 || spec.channels == 2,
            "WAV must be mono or stereo"
        );
        ensure!(
            u64::from(r.duration())
                <= u64::from(spec.sample_rate)
                    .checked_mul(120)
                    .context("WAV duration overflow")?,
            "WAV exceeds 120 seconds"
        );
        ensure!(
            u64::from(r.duration()) <= MAX_LOOP_FRAMES,
            "WAV exceeds stereo frame limit"
        );
        let sample_count = u64::from(r.duration())
            .checked_mul(u64::from(spec.channels))
            .context("WAV sample count overflow")?;
        ensure!(
            sample_count == u64::from(r.len()) && sample_count <= MAX_LOOP_FRAMES * 2,
            "invalid WAV sample count"
        );
        ensure!(
            match spec.sample_format {
                hound::SampleFormat::Float => spec.bits_per_sample == 32,
                hound::SampleFormat::Int => matches!(spec.bits_per_sample, 8 | 16 | 24 | 32),
            },
            "unsupported WAV sample format"
        );
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                r.samples::<f32>().collect::<std::result::Result<_, _>>()?
            }
            hound::SampleFormat::Int => r
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / (2_f32.powi(spec.bits_per_sample as i32 - 1))))
                .collect::<std::result::Result<_, _>>()?,
        };
        let samples = if spec.channels == 1 {
            raw.into_iter().flat_map(|v| [v, v]).collect()
        } else {
            raw
        };
        let mut replacement = self.clone();
        replacement.audio_loop = Some(AudioLoop {
            samples,
            sample_rate: spec.sample_rate,
            source_bpm: self.bpm,
            name: path.as_ref().display().to_string(),
        });
        replacement.validate()?;
        *self = replacement;
        Ok(())
    }
    /// Fresh-engine mixdown: exactly `bars` repeating 4/4 bars, followed by tails.
    pub fn export_wav(
        &self,
        path: impl AsRef<Path>,
        bars: u32,
        tail_seconds: f32,
    ) -> Result<RenderReport> {
        ensure!(
            (1..=1024).contains(&bars)
                && tail_seconds.is_finite()
                && (0.0..=30.0).contains(&tail_seconds),
            "invalid export duration"
        );
        let mut studio = Studio::new(self.clone(), 48000)?;
        studio.play(true);
        let musical_frames = (bars as f64 * 240.0 / self.bpm as f64 * 48000.0).round() as usize;
        let tail_frames = (tail_seconds * 48000.0).round() as usize;
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        let mut report = RenderReport {
            frames: 0,
            peak: 0.0,
            rms: 0.0,
        };
        let mut energy = 0_f64;
        let mut buffer = [0.0; 1024];
        for (frames, tail) in [(musical_frames, false), (tail_frames, true)] {
            if tail {
                studio.play(false);
            }
            let mut remaining = frames;
            while remaining > 0 {
                let n = remaining.min(512);
                studio.render(&mut buffer[..n * 2])?;
                for &s in &buffer[..n * 2] {
                    ensure!(s.is_finite(), "non-finite mixdown");
                    report.peak = report.peak.max(s.abs());
                    energy += f64::from(s).powi(2);
                    writer.write_sample(s)?;
                }
                remaining -= n;
                report.frames += n as u64;
            }
        }
        writer.finalize()?;
        report.rms = (energy / (report.frames * 2) as f64).sqrt() as f32;
        Ok(report)
    }
}
#[derive(Debug)]
pub struct RenderReport {
    pub frames: u64,
    pub peak: f32,
    pub rms: f32,
}

/// Unique engine owner; all unsafe calls are confined to this safe adapter.
pub struct Studio {
    engine: NonNull<GooeyEngine>,
    session: Session,
    sample_rate: u32,
    playing: bool,
    recording: bool,
    beat: f64,
    rendered_frames: u64,
    last_tick: Option<u64>,
}
// SAFETY: the engine has no thread affinity and contains Send DSP state. The
// unique pointer never escapes; methods require exclusive access, and Drop
// runs only after its owner (including a host mutex) has ceased rendering.
unsafe impl Send for Studio {}
impl Drop for Studio {
    fn drop(&mut self) {
        unsafe {
            gooey_engine_free(self.engine.as_ptr());
        }
    }
}
impl Studio {
    pub fn new(session: Session, sample_rate: u32) -> Result<Self> {
        session.validate()?;
        ensure!(
            (8000..=192000).contains(&sample_rate),
            "invalid sample rate"
        );
        let engine = NonNull::new(gooey_engine_new(sample_rate as f32))
            .context("engine allocation failed")?;
        let mut s = Self {
            engine,
            session,
            sample_rate,
            playing: false,
            recording: false,
            beat: 0.0,
            rendered_frames: 0,
            last_tick: None,
        };
        s.configure()?;
        // Install queued layout/rack/clip state while stopped, before hosts can
        // arm recording. Installing a clip resets the recorder's armed state.
        s.render(&mut [0.0; 2])?;
        Ok(s)
    }
    pub fn session(&self) -> &Session {
        &self.session
    }
    pub fn snapshot_session(&mut self) -> Session {
        self.capture_chords();
        self.session.clone()
    }
    /// Publish finalized recorded chords without copying embedded audio samples.
    pub fn refresh_performance(&mut self) {
        self.capture_chords();
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn playing(&self) -> bool {
        self.playing
    }
    pub fn recording(&self) -> bool {
        self.recording
    }
    pub fn tick(&self) -> u32 {
        ((self.beat * 96.0).floor() as u64 % LOOP_TICKS as u64) as u32
    }
    /// Musical cursor since the last Play, frozen while stopped.
    pub fn position_beats(&self) -> f64 {
        self.beat
    }
    fn configure(&mut self) -> Result<()> {
        let p = self.engine.as_ptr();
        unsafe {
            gooey_engine_set_bpm(p, self.session.bpm);
            gooey_engine_mixer_clear_layout(p);
            for name in TRACK_NAMES {
                let name = CString::new(name)?;
                gooey_engine_mixer_add_track(p, name.as_ptr());
            }
            for (source, t) in [
                (SOURCE_DRUMKIT, 0),
                (SOURCE_BASS, 1),
                (SOURCE_POLYSYNTH, 2),
                (SOURCE_LOOPMIXER, 3),
            ] {
                ensure!(
                    gooey_engine_mixer_route_source(p, source, t),
                    "routing rejected"
                );
            }
            gooey_engine_load_bass_preset(p, BASS_PRESET_SUB);
            gooey_engine_poly_set_preset(p, POLY_PRESET_PAD);
            for t in 0..4 {
                for effect in [EFFECT_LOWPASS_FILTER, EFFECT_DELAY, EFFECT_REVERB] {
                    ensure!(
                        gooey_engine_track_effect_add(p, t, effect) >= 0,
                        "rack rejected"
                    );
                }
            }
            gooey_engine_set_global_effect_enabled(p, EFFECT_DELAY, true);
            gooey_engine_set_global_effect_enabled(p, EFFECT_REVERB, true);
            gooey_engine_set_global_effect_enabled(p, EFFECT_LIMITER, true);
            gooey_engine_perf_set_record_mode(p, PERF_RECORD_MODE_OVERDUB);
        }
        for t in 0..4 {
            for c in [
                Control::Gain(t),
                Control::Pan(t),
                Control::Cutoff(t),
                Control::Delay(t),
                Control::Reverb(t),
            ] {
                self.apply(c, self.session.value(c));
            }
            self.set_mute(t, self.session.strips[t].mute)?;
            self.set_solo(t, self.session.strips[t].solo)?;
        }
        for c in [
            Control::MasterGain,
            Control::MasterDelay,
            Control::MasterReverb,
            Control::BassTone,
            Control::ChordTone,
        ] {
            self.apply(c, self.session.value(c));
        }
        self.sync_patterns();
        self.install_chords()?;
        if let Some(l) = &self.session.audio_loop {
            unsafe {
                ensure!(
                    gooey_engine_loop_load(
                        p,
                        0,
                        l.samples.as_ptr(),
                        (l.samples.len() / 2) as u32,
                        2,
                        l.sample_rate as f32
                    ),
                    "loop load rejected"
                );
                gooey_engine_loop_set_source_bpm(p, 0, l.source_bpm);
                gooey_engine_loop_set_pitch_mode(
                    p,
                    0,
                    if l.source_bpm == self.session.bpm {
                        PITCH_MODE_OFF
                    } else {
                        PITCH_MODE_PRESERVE_PITCH
                    },
                );
            }
        }
        Ok(())
    }
    pub fn replace(&mut self, session: Session) -> Result<()> {
        let replacement = Self::new(session, self.sample_rate)?;
        *self = replacement;
        Ok(())
    }
    fn sync_patterns(&mut self) {
        unsafe {
            for (row, id) in [
                INSTRUMENT_KICK,
                INSTRUMENT_SNARE,
                INSTRUMENT_HIHAT,
                INSTRUMENT_TOM,
                INSTRUMENT_BASS,
            ]
            .into_iter()
            .enumerate()
            {
                for step in 0..16 {
                    gooey_engine_sequencer_set_instrument_step_with_velocity(
                        self.engine.as_ptr(),
                        id,
                        step as u32,
                        self.session.steps[row][step] > 0.0,
                        self.session.steps[row][step],
                    );
                    if row == 4 {
                        gooey_engine_sequencer_set_instrument_step_note(
                            self.engine.as_ptr(),
                            id,
                            step as u32,
                            self.session.bass_notes[step],
                        );
                    }
                }
            }
        }
    }
    pub fn set_step(&mut self, row: usize, step: usize, velocity: f32, note: u8) -> Result<()> {
        ensure!(
            row < 5
                && step < 16
                && velocity.is_finite()
                && (0.0..=1.0).contains(&velocity)
                && note <= 127,
            "invalid step"
        );
        self.session.steps[row][step] = velocity;
        if row == 4 {
            self.session.bass_notes[step] = note;
        }
        self.sync_patterns();
        Ok(())
    }
    fn install_chords(&mut self) -> Result<()> {
        let events: Vec<_> = self.session.chords.iter().map(ChordEvent::ffi).collect();
        unsafe {
            ensure!(
                gooey_engine_chord_loop_replace(
                    self.engine.as_ptr(),
                    events.as_ptr(),
                    events.len() as u32,
                    LOOP_TICKS
                ) > 0,
                "chord clip rejected"
            );
        }
        Ok(())
    }
    pub fn clear_performance(&mut self) {
        self.session.chords.clear();
        self.session.hits.clear();
        unsafe {
            gooey_engine_perf_clear_clip(self.engine.as_ptr());
        }
    }
    pub fn clear_automation(&mut self) {
        self.session.automation.clear();
    }
    pub fn set_tempo(&mut self, bpm: f32) -> Result<()> {
        ensure!(
            bpm.is_finite() && (40.0..=240.0).contains(&bpm),
            "invalid tempo"
        );
        if self.session.bpm == bpm {
            return Ok(());
        }
        // The legacy sequencer's next trigger retains old-tempo sample timing.
        // Re-cue all sources rather than let the automation/engine clocks split.
        let restart = self.playing;
        if restart {
            self.play(false);
        }
        self.session.bpm = bpm;
        unsafe {
            gooey_engine_set_bpm(self.engine.as_ptr(), bpm);
            if let Some(audio) = &self.session.audio_loop {
                gooey_engine_loop_set_pitch_mode(
                    self.engine.as_ptr(),
                    0,
                    if audio.source_bpm == bpm {
                        PITCH_MODE_OFF
                    } else {
                        PITCH_MODE_PRESERVE_PITCH
                    },
                );
            }
        }
        if restart {
            self.play(true);
        }
        Ok(())
    }
    /// Correct an imported loop's authored tempo without changing song tempo.
    pub fn set_loop_source_tempo(&mut self, bpm: f32) -> Result<()> {
        ensure!(
            bpm.is_finite() && (40.0..=240.0).contains(&bpm),
            "invalid loop tempo"
        );
        let audio = self
            .session
            .audio_loop
            .as_mut()
            .context("no audio loop loaded")?;
        audio.source_bpm = bpm;
        unsafe {
            gooey_engine_loop_set_source_bpm(self.engine.as_ptr(), 0, bpm);
            gooey_engine_loop_set_pitch_mode(
                self.engine.as_ptr(),
                0,
                if bpm == self.session.bpm {
                    PITCH_MODE_OFF
                } else {
                    PITCH_MODE_PRESERVE_PITCH
                },
            );
        }
        Ok(())
    }
    pub fn set_mute(&mut self, t: usize, v: bool) -> Result<()> {
        self.set_control(Control::Mute(t), if v { 1.0 } else { 0.0 })
    }
    pub fn set_solo(&mut self, t: usize, v: bool) -> Result<()> {
        self.set_control(Control::Solo(t), if v { 1.0 } else { 0.0 })
    }
    pub fn set_control(&mut self, c: Control, v: f32) -> Result<()> {
        self.session.validate_value(c, v)?;
        if self.recording && self.playing {
            let tick = self.tick();
            let baseline = self.session.value(c);
            let index = if let Some(i) = self.session.automation.iter().position(|l| l.control == c)
            {
                i
            } else {
                self.session.automation.push(Lane {
                    control: c,
                    points: vec![Point {
                        tick: 0,
                        value: baseline,
                    }],
                });
                self.session.automation.len() - 1
            };
            let points = &mut self.session.automation[index].points;
            match points.binary_search_by_key(&tick, |p| p.tick) {
                Ok(i) => points[i].value = v,
                Err(i) => points.insert(i, Point { tick, value: v }),
            }
        }
        self.session.write_value(c, v);
        self.apply(c, v);
        Ok(())
    }
    fn apply(&mut self, c: Control, v: f32) {
        let p = self.engine.as_ptr();
        unsafe {
            match c {
                Control::Gain(t) => gooey_engine_mixer_set_track_gain(p, t as u32, v),
                Control::Pan(t) => gooey_engine_mixer_set_track_pan(p, t as u32, v),
                Control::Mute(t) => gooey_engine_mixer_set_track_mute(p, t as u32, v >= 0.5),
                Control::Solo(t) => gooey_engine_mixer_set_track_solo(p, t as u32, v >= 0.5),
                Control::Cutoff(t) => {
                    gooey_engine_track_effect_set_param(p, t as u32, 0, FILTER_PARAM_CUTOFF, v)
                }
                Control::Delay(t) => {
                    gooey_engine_track_effect_set_param(p, t as u32, 1, DELAY_PARAM_MIX, v)
                }
                Control::Reverb(t) => {
                    gooey_engine_track_effect_set_param(p, t as u32, 2, REVERB_PARAM_MIX, v)
                }
                Control::MasterGain => gooey_engine_set_master_gain(p, v),
                Control::MasterDelay => {
                    gooey_engine_set_global_effect_param(p, EFFECT_DELAY, DELAY_PARAM_MIX, v)
                }
                Control::MasterReverb => {
                    gooey_engine_set_global_effect_param(p, EFFECT_REVERB, REVERB_PARAM_MIX, v)
                }
                Control::BassTone => gooey_engine_set_bass_param(p, BASS_PARAM_FILTER_CUTOFF, v),
                Control::ChordTone => {
                    // Chord playback recalls a preset on each note-on. Update the
                    // producer's preset copies too, so the next chord retains the tone.
                    for preset in 0..POLY_PRESET_COUNT {
                        gooey_engine_poly_set_preset_param(p, preset, POLY_PARAM_FILTER_CUTOFF, v);
                    }
                    gooey_engine_poly_set_param(p, POLY_PARAM_FILTER_CUTOFF, v);
                }
            }
        }
    }
    /// Idempotent transport. Play after Stop re-cues every source at bar zero;
    /// Stop finalizes recording but leaves the displayed cursor and DSP tails.
    /// This deliberately is not a pause/resume API for the legacy sequencer.
    pub fn play(&mut self, on: bool) {
        if self.playing == on {
            return;
        }
        if !on && self.recording {
            self.record(false);
        }
        self.playing = on;
        unsafe {
            if on {
                self.beat = 0.0;
                self.rendered_frames = 0;
                self.last_tick = None;
                gooey_engine_sequencer_reset(self.engine.as_ptr());
                gooey_engine_loop_restart(self.engine.as_ptr(), 0);
                gooey_engine_sequencer_start(self.engine.as_ptr());
            } else {
                gooey_engine_sequencer_stop(self.engine.as_ptr());
                gooey_engine_poly_release(self.engine.as_ptr());
            }
            gooey_engine_loop_set_playing(
                self.engine.as_ptr(),
                0,
                on && self.session.audio_loop.is_some(),
            );
        }
    }
    pub fn rewind(&mut self) -> Result<()> {
        let playing = self.playing;
        let session = self.snapshot_session();
        self.replace(session)?;
        self.play(playing);
        Ok(())
    }
    pub fn record(&mut self, on: bool) {
        self.recording = on;
        unsafe {
            gooey_engine_perf_set_record_armed(self.engine.as_ptr(), on);
        }
        if !on {
            self.capture_chords();
        }
    }
    pub fn chord_recording(&self) -> bool {
        unsafe { gooey_engine_perf_is_recording(self.engine.as_ptr()) }
    }
    pub fn chord_on(
        &mut self,
        degree: u32,
        root: u32,
        minor: bool,
        octave: i32,
        preset: u32,
    ) -> Result<()> {
        ensure!(
            degree < 7 && root < 12 && (0..=8).contains(&octave) && preset < POLY_PRESET_COUNT,
            "invalid chord"
        );
        unsafe {
            gooey_engine_poly_trigger_chord(
                self.engine.as_ptr(),
                root,
                u32::from(minor),
                degree,
                0,
                preset,
                octave,
                0.85,
            );
        }
        Ok(())
    }
    pub fn chord_off(&mut self) {
        unsafe {
            gooey_engine_poly_release(self.engine.as_ptr());
        }
        self.capture_chords();
    }
    fn capture_chords(&mut self) {
        // Only replace the persisted clip after the staged initial snapshot has
        // actually been installed; otherwise saving a stopped new song loses it.
        unsafe {
            if gooey_engine_chord_loop_get_applied_generation(self.engine.as_ptr()) == 0 {
                return;
            }
            let mut events = Vec::new();
            for i in 0..gooey_engine_perf_get_event_count(self.engine.as_ptr()) {
                let mut e = ChordEvent {
                    tick: 0,
                    duration: 1,
                    degree: 0,
                    root: 0,
                    minor: false,
                    octave: 4,
                    preset: 1,
                    velocity: 0.8,
                };
                let mut scale = 0;
                gooey_engine_perf_get_event(
                    self.engine.as_ptr(),
                    i,
                    &mut e.tick,
                    &mut e.duration,
                    &mut e.root,
                    &mut scale,
                    &mut e.degree,
                    std::ptr::null_mut(),
                    &mut e.preset,
                    &mut e.octave,
                    &mut e.velocity,
                );
                e.minor = scale == 1;
                events.push(e);
            }
            let canonical = canonical_recorded_chords(&events);
            let changed = canonical != events;
            self.session.chords = canonical;
            if changed && !self.recording {
                // Live replay must use the same last-take clip as save/export.
                // Recording stays untouched until punch-out/Stop so staging a
                // snapshot cannot unexpectedly disarm a still-active overdub.
                let installed = self.install_chords();
                debug_assert!(installed.is_ok(), "canonical chord installation failed");
            }
        }
    }
    pub fn hit(&mut self, instrument: u32, note: u8, velocity: f32) -> Result<()> {
        let h = Hit {
            tick: self.tick(),
            instrument,
            note,
            velocity,
        };
        ensure!(
            instrument <= INSTRUMENT_BASS
                && note <= 127
                && velocity.is_finite()
                && (0.0..=1.0).contains(&velocity),
            "invalid hit"
        );
        self.play_hit(&h);
        if self.recording && self.playing {
            ensure!(self.session.hits.len() < 4096, "hit recording full");
            self.session.hits.push(h);
        }
        Ok(())
    }
    fn play_hit(&mut self, h: &Hit) {
        unsafe {
            if h.instrument == INSTRUMENT_BASS {
                let hz = crate::music::midi_to_freq(h.note) as f32;
                gooey_engine_set_bass_param(
                    self.engine.as_ptr(),
                    BASS_PARAM_FREQUENCY,
                    ((hz - 30.0) / 170.0).clamp(0.0, 1.0),
                );
            }
            gooey_engine_trigger_instrument_with_velocity(
                self.engine.as_ptr(),
                h.instrument,
                h.velocity,
            );
        }
    }
    pub fn peaks(&self) -> [f32; 4] {
        std::array::from_fn(|t| unsafe {
            gooey_engine_mixer_get_track_peak(self.engine.as_ptr(), t as u32)
        })
    }
    /// Render interleaved stereo. Parameter playback is sample-aligned to a
    /// 96-PPQ clock and independent of host callback size (step-held curves).
    pub fn render(&mut self, out: &mut [f32]) -> Result<()> {
        ensure!(
            out.len().is_multiple_of(2),
            "stereo buffer must contain complete frames"
        );
        ensure!(
            out.len() / 2 <= u32::MAX as usize,
            "render buffer too large"
        );
        let mut offset = 0;
        while offset < out.len() {
            let absolute = (self.beat * 96.0 + 1e-8).floor() as u64;
            if self.playing && self.last_tick != Some(absolute) {
                self.last_tick = Some(absolute);
                let tick = (absolute % LOOP_TICKS as u64) as u32;
                if tick == 0 && absolute > 0 {
                    // Keep legacy f32 trigger counters local to one bar. Use
                    // absolute musical position for the shared grid clock. The
                    // legacy PCM channel is not a grid slot: its continuous
                    // playhead is unaffected, including multibar phrases.
                    unsafe {
                        gooey_engine_sequencer_set_beat_position(
                            self.engine.as_ptr(),
                            absolute as f64 / 96.0,
                        );
                        gooey_engine_sequencer_start(self.engine.as_ptr());
                    }
                }
                if !self.recording {
                    // Fixed stack scratch avoids allocation in the audio loop.
                    let mut values = [None; 64];
                    for (i, lane) in self.session.automation.iter().enumerate() {
                        let point = lane
                            .points
                            .iter()
                            .rev()
                            .find(|p| p.tick <= tick)
                            .or_else(|| lane.points.last());
                        values[i] = point.map(|p| (lane.control, p.value));
                    }
                    for (c, v) in values.into_iter().flatten() {
                        self.session.write_value(c, v);
                        self.apply(c, v);
                    }
                }
                for i in 0..self.session.hits.len() {
                    if self.session.hits[i].tick == tick {
                        let h = self.session.hits[i].clone();
                        self.play_hit(&h);
                    }
                }
            }
            let frames = if self.playing {
                let until = ((absolute + 1) as f64 / 96.0 - self.beat) * 60.0
                    / self.session.bpm as f64
                    * self.sample_rate as f64;
                (until - 1e-7).ceil().max(1.0) as usize
            } else {
                (out.len() - offset) / 2
            };
            let n = frames.min((out.len() - offset) / 2);
            unsafe {
                gooey_engine_render(self.engine.as_ptr(), out[offset..].as_mut_ptr(), n as u32);
            }
            if self.playing {
                self.rendered_frames += n as u64;
                self.beat = self.rendered_frames as f64 * self.session.bpm as f64
                    / (60.0 * self.sample_rate as f64);
            }
            offset += n * 2;
        }
        Ok(())
    }
}
