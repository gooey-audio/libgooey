//! Host-defined mixer graph: named submix [`Track`]s fed by engine
//! [`SourceId`]s.
//!
//! Where the loop [`Mixer`](crate::mixer::Mixer) owns a fixed set of loop
//! channels, the `MixerGraph` sits one layer up: it owns an arbitrary,
//! host-defined set of named tracks and a routing table mapping each engine
//! source (the drum kit, bass, poly synth, granulator, loop mixer) to a track.
//! Each track is a mixer strip — gain, stereo balance, mute/solo, peak meter —
//! plus its own reorderable [`EffectChain`] rack. Tracks sum to the master bus,
//! which the host engine then scales by master gain and runs through the global
//! effects + limiter.
//!
//! Real-time contract: the audio thread never grows the graph. Track creation,
//! routing, and rack edits are config-time operations (host-serialized with the
//! render call, exactly like the loop-effect API). The per-sample path
//! ([`scatter`](MixerGraph::scatter) / [`mix_down`](MixerGraph::mix_down)) only
//! touches pre-sized storage and never allocates.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::frame::StereoFrame;
use crate::mixer::EffectChain;
use crate::utils::SmoothedParam;

/// Source: the drum kit (kick/snare/hihat/tom summed).
pub const SOURCE_DRUMKIT: u32 = 0;
/// Source: the bass voice.
pub const SOURCE_BASS: u32 = 1;
/// Source: the polyphonic synth.
pub const SOURCE_POLYSYNTH: u32 = 2;
/// Source: the granulator.
pub const SOURCE_GRANULATOR: u32 = 3;
/// Source: the loop mixer (all loop channels summed).
pub const SOURCE_LOOPMIXER: u32 = 4;
/// Number of routable engine sources.
pub const SOURCE_COUNT: usize = 5;
/// First dynamically registered sampler source. Legacy source IDs remain stable.
pub const SOURCE_SAMPLER_BASE: u32 = SOURCE_COUNT as u32;
/// Maximum sampler racks registered by the FFI engine.
pub const SAMPLER_SOURCE_COUNT: usize = 4;
const SOURCE_CAPACITY: usize = SOURCE_COUNT + SAMPLER_SOURCE_COUNT;

/// Maximum track gain (allows up to +6 dB of makeup on a submix).
const MAX_TRACK_GAIN: f32 = 2.0;

/// Apply a stereo balance to an already-stereo frame. `pan` is 0.0 (hard left)
/// .. 0.5 (center) .. 1.0 (hard right). Center is a true identity, so a
/// default-centered track sums bit-identically to an un-panned mix.
fn balanced(frame: StereoFrame, pan: f32) -> StereoFrame {
    let pan = pan.clamp(0.0, 1.0);
    let left_gain = (2.0 * (1.0 - pan)).min(1.0);
    let right_gain = (2.0 * pan).min(1.0);
    StereoFrame {
        l: frame.l * left_gain,
        r: frame.r * right_gain,
    }
}

/// A named submix bus: a mixer strip (gain / balance / mute-solo / peak) plus a
/// reorderable effect rack. Sources routed to this track are summed, run through
/// the strip and rack, then added to the master bus.
pub struct Track {
    name: CString,
    /// Fader, 0.0..=[`MAX_TRACK_GAIN`], 10 ms smoothing.
    gain: SmoothedParam,
    /// Stereo balance, 0.0..=1.0 (0.5 = center), 10 ms smoothing.
    pan: SmoothedParam,
    /// Click-free mute/solo multiplier (target 0.0 or 1.0).
    mute_gain: SmoothedParam,
    muted: AtomicBool,
    soloed: AtomicBool,
    /// Post-strip peak since last read (f32 bits, read-and-reset by UI).
    peak: AtomicU32,
    /// Track-level effect rack (reused from the loop-channel effect model).
    rack: EffectChain,
}

impl Track {
    fn new(name: CString, sample_rate: f32) -> Self {
        Self {
            name,
            gain: SmoothedParam::new(1.0, 0.0, MAX_TRACK_GAIN, sample_rate, 10.0),
            pan: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, 10.0),
            mute_gain: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0),
            muted: AtomicBool::new(false),
            soloed: AtomicBool::new(false),
            peak: AtomicU32::new(0.0_f32.to_bits()),
            rack: EffectChain::new(),
        }
    }

    fn record_peak(&self, level: f32) {
        let prev = f32::from_bits(self.peak.load(Ordering::Relaxed));
        if level > prev {
            self.peak.store(level.to_bits(), Ordering::Relaxed);
        }
    }
}

/// A host-defined set of tracks plus the source→track routing table.
pub struct MixerGraph {
    tracks: Vec<Track>,
    /// `routes[source_kind]` = target track index (or `None` = source is muted).
    routes: [Option<usize>; SOURCE_CAPACITY],
    active_sources: [bool; SOURCE_CAPACITY],
    /// Per-track accumulator, always `tracks.len()` long. Resized only when the
    /// layout changes (config time); cleared and summed each sample.
    scratch: Vec<StereoFrame>,
    sample_rate: f32,
    bpm: f32,
}

impl MixerGraph {
    /// Create an empty graph (no tracks, every source unrouted/silent).
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        Self {
            tracks: Vec::new(),
            routes: [None; SOURCE_CAPACITY],
            active_sources: std::array::from_fn(|index| index < SOURCE_COUNT),
            scratch: Vec::new(),
            sample_rate,
            bpm,
        }
    }

    /// Create the default 4-track layout used out of the box: "Drums", "Bass",
    /// "Synth", "Loops" at unity gain / centered / no rack, with DrumKit→0,
    /// Bass→1, PolySynth→2, and Granulator+LoopMixer→3. With this layout the
    /// summed output is bit-identical to a flat instrument+loop mix.
    pub fn with_default_layout(sample_rate: f32, bpm: f32) -> Self {
        let mut graph = Self::new(sample_rate, bpm);
        let drums = graph.add_track(CString::new("Drums").unwrap());
        let bass = graph.add_track(CString::new("Bass").unwrap());
        let synth = graph.add_track(CString::new("Synth").unwrap());
        let loops = graph.add_track(CString::new("Loops").unwrap());
        graph.route(SOURCE_DRUMKIT, drums);
        graph.route(SOURCE_BASS, bass);
        graph.route(SOURCE_POLYSYNTH, synth);
        graph.route(SOURCE_GRANULATOR, loops);
        graph.route(SOURCE_LOOPMIXER, loops);
        graph
    }

    /// Reset to an empty layout (no tracks, no routes).
    pub fn reset(&mut self) {
        self.tracks.clear();
        self.scratch.clear();
        self.routes = [None; SOURCE_CAPACITY];
    }

    /// Append a named track. Returns its index. Grows the render scratch so the
    /// audio thread never allocates.
    pub fn add_track(&mut self, name: CString) -> usize {
        self.tracks.push(Track::new(name, self.sample_rate));
        self.scratch.push(StereoFrame::default());
        self.tracks.len() - 1
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Borrow a track's name (valid until the track is renamed/removed or the
    /// graph is reset). `None` for an out-of-range index.
    pub fn track_name(&self, track: usize) -> Option<&CStr> {
        self.tracks.get(track).map(|t| t.name.as_c_str())
    }

    /// Rename a track. Returns `false` for an out-of-range index.
    pub fn set_track_name(&mut self, track: usize, name: CString) -> bool {
        match self.tracks.get_mut(track) {
            Some(t) => {
                t.name = name;
                true
            }
            None => false,
        }
    }

    /// First track index whose name equals `name`, if any.
    pub fn track_index_by_name(&self, name: &CStr) -> Option<usize> {
        self.tracks.iter().position(|t| t.name.as_c_str() == name)
    }

    // --- strip controls (all clamp; no-op on bad index) ---

    pub fn set_track_gain(&mut self, track: usize, gain: f32) {
        if let Some(t) = self.tracks.get_mut(track) {
            t.gain.set_target(gain.clamp(0.0, MAX_TRACK_GAIN));
        }
    }

    pub fn track_gain(&self, track: usize) -> f32 {
        self.tracks.get(track).map_or(1.0, |t| t.gain.target())
    }

    pub fn set_track_pan(&mut self, track: usize, pan: f32) {
        if let Some(t) = self.tracks.get_mut(track) {
            t.pan.set_target(pan.clamp(0.0, 1.0));
        }
    }

    pub fn track_pan(&self, track: usize) -> f32 {
        self.tracks.get(track).map_or(0.5, |t| t.pan.target())
    }

    pub fn set_track_mute(&self, track: usize, muted: bool) {
        if let Some(t) = self.tracks.get(track) {
            t.muted.store(muted, Ordering::Release);
        }
    }

    pub fn track_mute(&self, track: usize) -> bool {
        self.tracks
            .get(track)
            .is_some_and(|t| t.muted.load(Ordering::Acquire))
    }

    pub fn set_track_solo(&self, track: usize, soloed: bool) {
        if let Some(t) = self.tracks.get(track) {
            t.soloed.store(soloed, Ordering::Release);
        }
    }

    pub fn track_solo(&self, track: usize) -> bool {
        self.tracks
            .get(track)
            .is_some_and(|t| t.soloed.load(Ordering::Acquire))
    }

    /// Read-and-reset a track's post-strip peak. `None` for a bad index.
    pub fn track_peak_swap(&self, track: usize) -> Option<f32> {
        self.tracks
            .get(track)
            .map(|t| f32::from_bits(t.peak.swap(0.0_f32.to_bits(), Ordering::Relaxed)))
    }

    // --- routing ---

    /// Route a source to a track. Returns `false` for a bad source kind or
    /// track index.
    pub fn route(&mut self, source_kind: u32, track: usize) -> bool {
        if self.source_is_active(source_kind) && track < self.tracks.len() {
            self.routes[source_kind as usize] = Some(track);
            true
        } else {
            false
        }
    }

    /// Unroute a source (it becomes silent). Returns `true` if it was routed.
    pub fn unroute(&mut self, source_kind: u32) -> bool {
        if self.source_is_active(source_kind) {
            self.routes[source_kind as usize].take().is_some()
        } else {
            false
        }
    }

    /// Track a source currently routes to, if any.
    pub fn route_of(&self, source_kind: u32) -> Option<usize> {
        self.source_is_active(source_kind)
            .then(|| self.routes[source_kind as usize])
            .flatten()
    }

    /// Enable one bounded config-time source slot (used by sampler rack registration).
    pub fn register_source(&mut self, source_kind: u32) -> bool {
        let Some(active) = self.active_sources.get_mut(source_kind as usize) else {
            return false;
        };
        *active = true;
        true
    }

    fn source_is_active(&self, source_kind: u32) -> bool {
        self.active_sources
            .get(source_kind as usize)
            .copied()
            .unwrap_or(false)
    }

    // --- track effect rack (mirrors the loop-effect API) ---

    pub fn effect_add(&mut self, track: usize, effect_id: u32) -> Option<usize> {
        let (sr, bpm) = (self.sample_rate, self.bpm);
        self.tracks.get_mut(track)?.rack.add(effect_id, sr, bpm)
    }

    pub fn effect_remove(&mut self, track: usize, slot: usize) -> bool {
        self.tracks
            .get_mut(track)
            .is_some_and(|t| t.rack.remove(slot))
    }

    pub fn effect_move(&mut self, track: usize, slot: usize, new_position: usize) -> bool {
        self.tracks
            .get_mut(track)
            .is_some_and(|t| t.rack.move_effect(slot, new_position))
    }

    pub fn effect_clear(&mut self, track: usize) {
        if let Some(t) = self.tracks.get_mut(track) {
            t.rack.clear();
        }
    }

    pub fn effect_set_param(&self, track: usize, slot: usize, param: u32, value: f32) {
        if let Some(t) = self.tracks.get(track) {
            t.rack.set_param(slot, param, value);
        }
    }

    pub fn effect_count(&self, track: usize) -> usize {
        self.tracks.get(track).map_or(0, |t| t.rack.len())
    }

    pub fn effect_type_at(&self, track: usize, slot: usize) -> Option<u32> {
        self.tracks
            .get(track)
            .and_then(|t| t.rack.effect_type_at(slot))
    }

    /// Propagate a new tempo to every track's note-synced effects.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        for t in &self.tracks {
            t.rack.set_bpm(bpm);
        }
    }

    // --- render path (allocation-free) ---

    /// Zero every per-track accumulator. Call once at the top of each sample.
    pub fn clear_scratch(&mut self) {
        for s in &mut self.scratch {
            *s = StereoFrame::default();
        }
    }

    /// Add a source's stereo frame into its routed track's accumulator. No-op if
    /// the source is unrouted.
    pub fn scatter(&mut self, source_kind: u32, frame: StereoFrame) {
        if let Some(track) = self.route_of(source_kind) {
            if let Some(slot) = self.scratch.get_mut(track) {
                *slot += frame;
            }
        }
    }

    /// Recompute per-track mute/solo targets. Call once per buffer. Solo is
    /// scoped across tracks: any soloed track silences the un-soloed ones.
    pub fn update_mute_solo_targets(&mut self) {
        let any_soloed = self.tracks.iter().any(|t| t.soloed.load(Ordering::Relaxed));
        for t in &mut self.tracks {
            let muted = t.muted.load(Ordering::Relaxed);
            let soloed = t.soloed.load(Ordering::Relaxed);
            let target = if soloed {
                1.0
            } else if any_soloed || muted {
                0.0
            } else {
                1.0
            };
            t.mute_gain.set_target(target);
        }
    }

    /// Snap every track strip smoother to its current target. This is useful for
    /// offline renders that reset time and should honor just-applied host strip
    /// changes from sample zero instead of replaying a real-time fade.
    pub fn snap_strip_params(&mut self) {
        self.update_mute_solo_targets();
        for t in &mut self.tracks {
            t.gain.snap();
            t.pan.snap();
            t.mute_gain.snap();
        }
    }

    /// Apply each track's strip (gain × mute/solo, balance) and effect rack to
    /// its accumulated frame, capture its post-strip peak, and return the summed
    /// master frame. Allocation-free.
    pub fn mix_down(&mut self) -> StereoFrame {
        let MixerGraph {
            tracks, scratch, ..
        } = self;
        let mut master = StereoFrame::default();
        for (i, track) in tracks.iter_mut().enumerate() {
            let gain = track.gain.tick() * track.mute_gain.tick();
            let mut f = scratch[i].scaled(gain);
            f = balanced(f, track.pan.tick());
            f = track.rack.process(f);
            track.record_peak(f.l.abs().max(f.r.abs()));
            master += f;
        }
        master
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{EFFECT_DELAY, EFFECT_LOWPASS_FILTER, EFFECT_REVERB};

    const SR: f32 = 44_100.0;
    const BPM: f32 = 120.0;

    #[test]
    fn default_layout_has_expected_tracks_and_routes() {
        let graph = MixerGraph::with_default_layout(SR, BPM);

        assert_eq!(graph.track_count(), 4);
        assert_eq!(graph.track_name(0).unwrap().to_str().unwrap(), "Drums");
        assert_eq!(graph.track_name(1).unwrap().to_str().unwrap(), "Bass");
        assert_eq!(graph.track_name(2).unwrap().to_str().unwrap(), "Synth");
        assert_eq!(graph.track_name(3).unwrap().to_str().unwrap(), "Loops");
        assert_eq!(graph.route_of(SOURCE_DRUMKIT), Some(0));
        assert_eq!(graph.route_of(SOURCE_BASS), Some(1));
        assert_eq!(graph.route_of(SOURCE_POLYSYNTH), Some(2));
        assert_eq!(graph.route_of(SOURCE_GRANULATOR), Some(3));
        assert_eq!(graph.route_of(SOURCE_LOOPMIXER), Some(3));
    }

    #[test]
    fn routing_rejects_invalid_source_or_track() {
        let mut graph = MixerGraph::new(SR, BPM);
        let track = graph.add_track(CString::new("A").unwrap());

        assert!(graph.route(SOURCE_DRUMKIT, track));
        assert!(!graph.route(SOURCE_COUNT as u32, track));
        assert!(!graph.route(SOURCE_BASS, track + 1));
        assert_eq!(graph.route_of(SOURCE_COUNT as u32), None);
        assert!(graph.unroute(SOURCE_DRUMKIT));
        assert!(!graph.unroute(SOURCE_DRUMKIT));
        assert!(!graph.unroute(SOURCE_COUNT as u32));
    }

    #[test]
    fn strip_controls_clamp_and_default_on_bad_index() {
        let mut graph = MixerGraph::new(SR, BPM);
        let track = graph.add_track(CString::new("A").unwrap());

        graph.set_track_gain(track, 3.0);
        assert_eq!(graph.track_gain(track), 2.0);
        graph.set_track_gain(track, -1.0);
        assert_eq!(graph.track_gain(track), 0.0);
        assert_eq!(graph.track_gain(track + 1), 1.0);

        graph.set_track_pan(track, 2.0);
        assert_eq!(graph.track_pan(track), 1.0);
        graph.set_track_pan(track, -1.0);
        assert_eq!(graph.track_pan(track), 0.0);
        assert_eq!(graph.track_pan(track + 1), 0.5);

        graph.set_track_mute(track, true);
        graph.set_track_solo(track, true);
        assert!(graph.track_mute(track));
        assert!(graph.track_solo(track));
        assert!(!graph.track_mute(track + 1));
        assert!(!graph.track_solo(track + 1));
    }

    #[test]
    fn effect_rack_adds_moves_removes_and_clears() {
        let mut graph = MixerGraph::new(SR, BPM);
        let track = graph.add_track(CString::new("A").unwrap());

        assert_eq!(graph.effect_add(track, EFFECT_DELAY), Some(0));
        assert_eq!(graph.effect_add(track, EFFECT_LOWPASS_FILTER), Some(1));
        assert_eq!(graph.effect_add(track, EFFECT_REVERB), Some(2));
        assert_eq!(graph.effect_count(track), 3);

        assert!(graph.effect_move(track, 2, 0));
        assert_eq!(graph.effect_type_at(track, 0), Some(EFFECT_REVERB));
        assert_eq!(graph.effect_type_at(track, 1), Some(EFFECT_DELAY));
        assert_eq!(graph.effect_type_at(track, 2), Some(EFFECT_LOWPASS_FILTER));

        assert!(graph.effect_remove(track, 1));
        assert_eq!(graph.effect_count(track), 2);
        assert!(!graph.effect_remove(track, 99));

        graph.effect_clear(track);
        assert_eq!(graph.effect_count(track), 0);
        assert_eq!(graph.effect_add(track + 1, EFFECT_DELAY), None);
    }

    #[test]
    fn mix_down_records_and_resets_track_peak() {
        let mut graph = MixerGraph::new(SR, BPM);
        let track = graph.add_track(CString::new("A").unwrap());
        assert!(graph.route(SOURCE_DRUMKIT, track));

        graph.clear_scratch();
        graph.scatter(SOURCE_DRUMKIT, StereoFrame { l: 0.25, r: -0.5 });
        let out = graph.mix_down();

        assert_eq!(out, StereoFrame { l: 0.25, r: -0.5 });
        assert_eq!(graph.track_peak_swap(track), Some(0.5));
        assert_eq!(graph.track_peak_swap(track), Some(0.0));
        assert_eq!(graph.track_peak_swap(track + 1), None);
    }

    #[test]
    fn snap_strip_params_applies_current_targets_immediately() {
        let mut graph = MixerGraph::new(SR, BPM);
        let track = graph.add_track(CString::new("A").unwrap());
        assert!(graph.route(SOURCE_DRUMKIT, track));

        graph.set_track_gain(track, 0.5);
        graph.set_track_pan(track, 0.0);
        graph.snap_strip_params();

        graph.clear_scratch();
        graph.scatter(SOURCE_DRUMKIT, StereoFrame::mono(1.0));
        assert_eq!(graph.mix_down(), StereoFrame { l: 0.5, r: 0.0 });

        graph.set_track_mute(track, true);
        graph.snap_strip_params();
        graph.clear_scratch();
        graph.scatter(SOURCE_DRUMKIT, StereoFrame::mono(1.0));
        assert_eq!(graph.mix_down(), StereoFrame::default());

        let solo_track = graph.add_track(CString::new("B").unwrap());
        graph.set_track_mute(track, false);
        graph.set_track_solo(solo_track, true);
        graph.snap_strip_params();
        graph.clear_scratch();
        graph.scatter(SOURCE_DRUMKIT, StereoFrame::mono(1.0));
        assert_eq!(graph.mix_down(), StereoFrame::default());
    }
}
