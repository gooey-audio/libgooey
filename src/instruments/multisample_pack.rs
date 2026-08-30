//! Load a multi-sample pack from disk into a [`SampleMap`].
//!
//! A "pack" here is a directory containing WAV files plus an **SFZ** file that
//! maps them. SFZ is a plain-text format: a sequence of `<header>` lines, each
//! followed by `opcode=value` pairs. The headers nest, outermost first —
//! `<control>`, `<global>`, `<master>`, `<group>`, `<region>` — and each level
//! inherits the opcodes set above it. Only `<region>` produces a playable zone.
//!
//! A tiny example:
//!
//! ```text
//! <control> default_path=samples/
//! <global> ampeg_release=0.6
//! <group> hivel=80
//! <region> lokey=21 hikey=22 pitch_keycenter=21 sample=A0vL.wav
//! ```
//!
//! This parser deliberately implements only the SFZ v1 opcodes listed in
//! [`SUPPORTED_OPCODES`]. The "ARIA" superset (`#define`, `set_hdcc`,
//! `label_ccN`, keyswitch macros) is skipped rather than treated as an error,
//! so a pack authored for a full-featured player still loads — it just loses
//! the features libgooey does not model. Anything skipped is reported in the
//! warnings list so a host can surface it.
//!
//! This module is gated on the `bounce` feature because that is what pulls in
//! `hound`, the crate's only audio decoder — the same gate used by
//! [`crate::mixer::StereoSampleBuffer::from_wav`] and
//! [`crate::instruments::SampleBuffer::from_wav_mono`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::envelope::ADSRConfig;
use crate::instruments::multisample::{
    LoopMode, SampleMap, SampleZone, ZoneTrigger, DEFAULT_VELOCITY_LAYERS, MULTISAMPLE_MAX_ZONES,
};
use crate::mixer::StereoSampleBuffer;

/// The SFZ opcodes this loader understands. Anything else is skipped with a
/// warning. Documented here so a pack author can tell at a glance what will and
/// will not survive the import.
pub const SUPPORTED_OPCODES: &[&str] = &[
    "default_path",
    "sample",
    "lokey",
    "hikey",
    "key",
    "pitch_keycenter",
    "lovel",
    "hivel",
    "tune",
    "transpose",
    "volume",
    "pan",
    "width",
    "ampeg_attack",
    "ampeg_decay",
    "ampeg_sustain",
    "ampeg_release",
    "amp_veltrack",
    "loop_mode",
    "loopmode",
    "loop_start",
    "loopstart",
    "loop_end",
    "loopend",
    "offset",
    "end",
    "trigger",
    "seq_length",
    "seq_position",
    "locc64",
    "hicc64",
];

/// Fade applied where `max_seconds` cuts into a sample's decay. Long enough to
/// hide the step, short enough not to audibly shorten the note further.
pub const DEFAULT_TRIM_FADE_MS: f32 = 40.0;

/// How much of a pack to import. Filtering happens *before* any WAV is opened,
/// so thinning a 16-layer library to six layers also skips reading the files
/// belonging to the discarded layers.
#[derive(Clone, Debug)]
pub struct PackLoadOptions {
    /// Keep at most this many velocity layers, chosen evenly across the
    /// dynamic range with the loudest layer always retained. `None` keeps all.
    pub velocity_layers: Option<usize>,
    /// Restrict the imported key range (inclusive MIDI notes).
    pub key_range: Option<(u8, u8)>,
    /// Import `trigger=release` zones (damper and string noise). These roughly
    /// double a piano pack's footprint for a subtle effect, so they are off by
    /// default.
    pub include_release_zones: bool,
    /// Cap every sample at this many seconds, discarding the rest of its decay.
    ///
    /// This is usually the largest single lever on memory: a piano's low
    /// strings ring for twenty seconds or more, and that tail is only ever
    /// heard on a held or pedalled note. Truncation happens during decoding, so
    /// the discarded audio is never resident. `None` keeps every sample whole.
    pub max_seconds: Option<f32>,
    /// Fade applied at a truncation point, in milliseconds.
    ///
    /// A trimmed sample ends on real signal rather than silence, so cutting it
    /// dead would click. Only applied to samples that `max_seconds` actually
    /// shortened.
    pub trim_fade_ms: f32,
    /// Hard ceiling on imported zones.
    pub max_zones: usize,
}

impl Default for PackLoadOptions {
    fn default() -> Self {
        Self {
            velocity_layers: Some(DEFAULT_VELOCITY_LAYERS),
            key_range: None,
            include_release_zones: false,
            max_seconds: None,
            trim_fade_ms: DEFAULT_TRIM_FADE_MS,
            max_zones: MULTISAMPLE_MAX_ZONES,
        }
    }
}

impl PackLoadOptions {
    /// Import every region the pack defines, unthinned.
    pub fn everything() -> Self {
        Self {
            velocity_layers: None,
            key_range: None,
            include_release_zones: true,
            max_seconds: None,
            trim_fade_ms: DEFAULT_TRIM_FADE_MS,
            max_zones: MULTISAMPLE_MAX_ZONES,
        }
    }

    pub fn with_velocity_layers(mut self, layers: Option<usize>) -> Self {
        self.velocity_layers = layers;
        self
    }

    pub fn with_key_range(mut self, range: Option<(u8, u8)>) -> Self {
        self.key_range = range;
        self
    }

    pub fn with_release_zones(mut self, include: bool) -> Self {
        self.include_release_zones = include;
        self
    }

    /// Cap every sample's length. See [`Self::max_seconds`].
    pub fn with_max_seconds(mut self, seconds: Option<f32>) -> Self {
        self.max_seconds = seconds.filter(|s| s.is_finite() && *s > 0.0);
        self
    }
}

/// What a load produced: the map itself plus everything the loader had to skip.
#[derive(Debug)]
pub struct LoadedPack {
    pub map: SampleMap,
    /// Human-readable notes about skipped opcodes, missing files, and thinning
    /// decisions. Empty on a clean import.
    pub warnings: Vec<String>,
    /// Zones actually imported.
    pub zones_loaded: usize,
    /// Regions the SFZ declared, before filtering.
    pub regions_declared: usize,
}

/// Read an SFZ file and every WAV it references, producing a playable map.
///
/// `path` is the `.sfz` file. Sample paths inside it are resolved relative to
/// the SFZ's own directory, joined with `default_path` when present.
pub fn load_sfz(path: impl AsRef<Path>, options: &PackLoadOptions) -> Result<LoadedPack, String> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read SFZ {}: {e}", path.display()))?;
    let base = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    load_sfz_str(&text, &base, options)
}

/// Parse SFZ text that has already been read, resolving samples against `base`.
/// Split out from [`load_sfz`] so the parser can be tested without a filesystem.
pub fn load_sfz_str(
    text: &str,
    base: &Path,
    options: &PackLoadOptions,
) -> Result<LoadedPack, String> {
    let mut warnings = Vec::new();
    let regions = parse_regions(text, &mut warnings);
    let regions_declared = regions.len();
    if regions.is_empty() {
        return Err("SFZ declares no <region> blocks".to_string());
    }

    let regions = filter_regions(regions, options, &mut warnings);

    let mut map = SampleMap::new();
    let mut cache: HashMap<PathBuf, StereoSampleBuffer> = HashMap::new();
    let mut zones_loaded = 0;
    let mut trimmed_count = 0;

    for region in regions {
        if zones_loaded >= options.max_zones {
            warnings.push(format!(
                "stopped at the {} zone limit; later regions were skipped",
                options.max_zones
            ));
            break;
        }

        let Some(sample) = region.sample.as_ref() else {
            warnings.push("skipped a <region> with no sample= opcode".to_string());
            continue;
        };
        let full = base.join(&region.default_path).join(sample_to_path(sample));

        let buffer = match cache.get(&full) {
            Some(buffer) => buffer.clone(),
            None => match StereoSampleBuffer::from_wav_trimmed(&full, options.max_seconds) {
                Ok(buffer) => {
                    cache.insert(full.clone(), buffer.clone());
                    buffer
                }
                Err(error) => {
                    warnings.push(format!("skipped {}: {error}", full.display()));
                    continue;
                }
            },
        };

        // A sample that reached the cap was almost certainly cut mid-decay, so
        // it needs a real fade rather than the short de-click ramp. Samples
        // shorter than the cap ended naturally and are left alone.
        let trimmed = options
            .max_seconds
            .is_some_and(|cap| buffer.len() as f32 >= cap * buffer.sample_rate() - 1.0);
        let fade_frames = if trimmed {
            (options.trim_fade_ms.max(0.0) / 1000.0 * buffer.sample_rate()) as usize
        } else {
            0
        };
        if trimmed {
            trimmed_count += 1;
        }

        match region.into_zone(buffer, fade_frames) {
            Ok(zone) => match map.push_zone(zone) {
                Ok(()) => zones_loaded += 1,
                Err(error) => warnings.push(format!("skipped {}: {error}", full.display())),
            },
            Err(error) => warnings.push(format!("skipped {}: {error}", full.display())),
        }
    }

    if zones_loaded == 0 {
        return Err(format!(
            "SFZ declared {regions_declared} regions but none could be loaded; first problem: {}",
            warnings.first().map_or("unknown", String::as_str)
        ));
    }

    if let Some(cap) = options.max_seconds {
        if trimmed_count > 0 {
            warnings.push(format!(
                "trimmed {trimmed_count} of {zones_loaded} samples to {cap:.1}s"
            ));
        }
    }

    Ok(LoadedPack {
        map,
        warnings,
        zones_loaded,
        regions_declared,
    })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// One `<region>` with all inherited opcodes already flattened onto it.
#[derive(Clone, Debug, Default)]
struct Region {
    default_path: String,
    sample: Option<String>,
    lokey: Option<u8>,
    hikey: Option<u8>,
    pitch_keycenter: Option<u8>,
    lovel: u8,
    hivel: u8,
    tune_cents: f32,
    transpose: i32,
    volume_db: f32,
    pan: f32,
    ampeg_attack: f32,
    ampeg_decay: f32,
    ampeg_sustain: f32,
    ampeg_release: f32,
    amp_veltrack: f32,
    loop_mode: Option<LoopMode>,
    loop_start: usize,
    loop_end: usize,
    offset: usize,
    end: Option<usize>,
    trigger: ZoneTrigger,
    /// Round-robin position. Only position 1 (or unset) is imported.
    seq_position: u32,
}

impl Region {
    fn new() -> Self {
        Self {
            lovel: 1,
            hivel: 127,
            pan: 0.0,
            // SFZ defaults: essentially instant attack/decay, full sustain,
            // 1 ms release.
            ampeg_attack: 0.0,
            ampeg_decay: 0.0,
            ampeg_sustain: 100.0,
            ampeg_release: 0.001,
            amp_veltrack: 100.0,
            seq_position: 1,
            ..Default::default()
        }
    }

    fn into_zone(
        self,
        buffer: StereoSampleBuffer,
        fade_out_frames: usize,
    ) -> Result<SampleZone, String> {
        let root = self
            .pitch_keycenter
            .or(self.lokey)
            .ok_or("region has neither pitch_keycenter nor lokey")?;
        let lokey = self.lokey.unwrap_or(root);
        let hikey = self.hikey.unwrap_or(root);

        // SFZ `transpose` shifts which key sounds the root pitch, which is the
        // same thing as moving the root key the other way.
        let root = (root as i32 - self.transpose).clamp(0, 127) as u8;

        let mut zone = SampleZone::new(buffer, root)
            .with_key_range(lokey, hikey)
            .with_velocity_range(self.lovel, self.hivel);
        zone.tune_cents = self.tune_cents;
        zone.volume_db = self.volume_db;
        // SFZ pan is -100..100 with 0 = center; libgooey uses 0..1.
        zone.pan = (self.pan.clamp(-100.0, 100.0) + 100.0) / 200.0;
        zone.loop_mode = self.loop_mode.unwrap_or(LoopMode::NoLoop);
        zone.loop_start = self.loop_start;
        zone.loop_end = self.loop_end;
        zone.offset = self.offset;
        zone.end = self.end;
        zone.fade_out_frames = fade_out_frames;
        zone.envelope = ADSRConfig::new(
            self.ampeg_attack.max(0.0),
            self.ampeg_decay.max(0.0),
            (self.ampeg_sustain / 100.0).clamp(0.0, 1.0),
            self.ampeg_release.max(0.0),
        );
        zone.amp_veltrack = (self.amp_veltrack / 100.0).clamp(0.0, 1.0);
        zone.trigger = self.trigger;

        // A pack that declares a loop mode but no region is asking to loop the
        // whole sample.
        if zone.loop_mode != LoopMode::NoLoop
            && zone.loop_mode != LoopMode::OneShot
            && zone.loop_end == 0
        {
            zone.loop_start = 0;
            zone.loop_end = zone.buffer.len();
        }

        Ok(zone)
    }
}

/// Which header block is currently open. An opcode applies to that level and
/// to every level nested inside it that has already been derived.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Control,
    Global,
    Master,
    Group,
    Region,
}

/// Walk the SFZ text, applying header inheritance, and return one flattened
/// `Region` per `<region>` block.
///
/// Inheritance is modelled by cascading clones: opening `<group>` copies the
/// current `<master>` state, opening `<region>` copies the current `<group>`
/// state, and so on. Because opcodes appear *after* their header, a write at
/// one level is also applied to the inner levels already cloned from it.
fn parse_regions(text: &str, warnings: &mut Vec<String>) -> Vec<Region> {
    let mut control = Region::new();
    let mut global = Region::new();
    let mut master = Region::new();
    let mut group = Region::new();
    let mut current: Option<Region> = None;
    let mut level = Level::Control;
    let mut regions = Vec::new();
    let mut unknown: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            // ARIA preprocessor directive (`#define`, `#include`).
            warnings.push(format!("ignored preprocessor directive: {line}"));
            continue;
        }

        for token in split_tokens(line) {
            match token {
                Token::Header(name) => {
                    // Close whatever region was open before switching levels.
                    if let Some(region) = current.take() {
                        regions.push(region);
                    }
                    match name.as_str() {
                        "control" => level = Level::Control,
                        "global" => {
                            global = control.clone();
                            master = global.clone();
                            group = global.clone();
                            level = Level::Global;
                        }
                        "master" => {
                            master = global.clone();
                            group = master.clone();
                            level = Level::Master;
                        }
                        "group" => {
                            group = master.clone();
                            level = Level::Group;
                        }
                        "region" => {
                            current = Some(group.clone());
                            level = Level::Region;
                        }
                        "curve" | "effect" | "midi" => {
                            warnings.push(format!("ignored unsupported <{name}> block"));
                        }
                        other => warnings.push(format!("ignored unknown <{other}> block")),
                    }
                }
                Token::Opcode(key, value) => {
                    let mut supported = false;
                    // Write to the open level and to every inner level already
                    // cloned from it, so `<global> ampeg_release=0.6` still
                    // reaches a `<region>` opened later.
                    let mut targets: Vec<&mut Region> = match level {
                        Level::Control => vec![&mut control, &mut global, &mut master, &mut group],
                        Level::Global => vec![&mut global, &mut master, &mut group],
                        Level::Master => vec![&mut master, &mut group],
                        Level::Group => vec![&mut group],
                        Level::Region => Vec::new(),
                    };
                    if let Some(region) = current.as_mut() {
                        targets.push(region);
                    }
                    for target in targets {
                        supported |= apply_opcode(target, &key, &value);
                    }
                    if !supported && !unknown.contains(&key) {
                        unknown.push(key);
                    }
                }
            }
        }
    }

    if let Some(region) = current.take() {
        regions.push(region);
    }
    if !unknown.is_empty() {
        unknown.sort();
        warnings.push(format!(
            "ignored unsupported opcodes: {}",
            unknown.join(", ")
        ));
    }
    regions
}

enum Token {
    Header(String),
    Opcode(String, String),
}

/// Split one line into headers and `key=value` pairs. A value may contain
/// spaces (file names do), so a value runs until the next `key=` or `<header>`.
fn split_tokens(line: &str) -> Vec<Token> {
    let bytes: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i].is_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == '<' {
            let start = i + 1;
            let Some(close) = bytes[start..].iter().position(|&c| c == '>') else {
                break;
            };
            let name: String = bytes[start..start + close].iter().collect();
            tokens.push(Token::Header(name.trim().to_ascii_lowercase()));
            i = start + close + 1;
            continue;
        }

        // Read a key up to '='.
        let key_start = i;
        while i < bytes.len() && bytes[i] != '=' && !bytes[i].is_whitespace() && bytes[i] != '<' {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != '=' {
            // Stray token with no '='; skip it.
            continue;
        }
        let key: String = bytes[key_start..i].iter().collect();
        i += 1; // consume '='

        // Read the value up to the next `something=` or `<`.
        let value_start = i;
        let mut value_end = bytes.len();
        let mut probe = i;
        while probe < bytes.len() {
            if bytes[probe] == '<' {
                value_end = probe;
                break;
            }
            if bytes[probe] == '=' {
                // Back up over the key that owns this '=', and over the
                // whitespace separating it from our value.
                let mut back = probe;
                while back > value_start && !bytes[back - 1].is_whitespace() {
                    back -= 1;
                }
                while back > value_start && bytes[back - 1].is_whitespace() {
                    back -= 1;
                }
                value_end = back;
                break;
            }
            probe += 1;
        }

        let value: String = bytes[value_start..value_end.max(value_start)]
            .iter()
            .collect();
        tokens.push(Token::Opcode(
            key.trim().to_ascii_lowercase(),
            value.trim().to_string(),
        ));
        i = value_end;
    }

    tokens
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}

/// Apply one opcode to a region. Returns false if the opcode is not supported,
/// so the caller can collect it for the warnings list.
fn apply_opcode(region: &mut Region, key: &str, value: &str) -> bool {
    match key {
        "default_path" => region.default_path = value.replace('\\', "/"),
        "sample" => region.sample = Some(value.to_string()),
        "lokey" => region.lokey = parse_key(value),
        "hikey" => region.hikey = parse_key(value),
        "key" => {
            let key = parse_key(value);
            region.lokey = key;
            region.hikey = key;
            region.pitch_keycenter = key;
        }
        "pitch_keycenter" => region.pitch_keycenter = parse_key(value),
        "lovel" => region.lovel = value.parse().unwrap_or(region.lovel),
        "hivel" => region.hivel = value.parse().unwrap_or(region.hivel),
        "tune" | "pitch" => region.tune_cents = value.parse().unwrap_or(region.tune_cents),
        "transpose" => region.transpose = value.parse().unwrap_or(region.transpose),
        "volume" => region.volume_db = value.parse().unwrap_or(region.volume_db),
        "pan" => region.pan = value.parse().unwrap_or(region.pan),
        // `width` collapses or spreads the source image. libgooey applies width
        // instrument-wide, so accept and ignore it rather than warn on a very
        // common opcode.
        "width" => {}
        "ampeg_attack" => region.ampeg_attack = value.parse().unwrap_or(region.ampeg_attack),
        "ampeg_decay" => region.ampeg_decay = value.parse().unwrap_or(region.ampeg_decay),
        "ampeg_sustain" => region.ampeg_sustain = value.parse().unwrap_or(region.ampeg_sustain),
        "ampeg_release" => region.ampeg_release = value.parse().unwrap_or(region.ampeg_release),
        "amp_veltrack" => region.amp_veltrack = value.parse().unwrap_or(region.amp_veltrack),
        "loop_mode" | "loopmode" => region.loop_mode = parse_loop_mode(value),
        "loop_start" | "loopstart" => {
            region.loop_start = value.parse().unwrap_or(region.loop_start)
        }
        "loop_end" | "loopend" => region.loop_end = value.parse().unwrap_or(region.loop_end),
        "offset" => region.offset = value.parse().unwrap_or(region.offset),
        "end" => region.end = value.parse().ok(),
        "trigger" => {
            region.trigger = match value.to_ascii_lowercase().as_str() {
                "release" | "release_key" => ZoneTrigger::Release,
                _ => ZoneTrigger::Attack,
            }
        }
        "seq_position" => region.seq_position = value.parse().unwrap_or(region.seq_position),
        // Parsed for completeness; libgooey imports the first round robin only,
        // and pedal-conditional regions collapse to the pedal-up set.
        "seq_length" | "locc64" | "hicc64" => {}
        _ => return false,
    }
    true
}

fn parse_loop_mode(value: &str) -> Option<LoopMode> {
    match value.to_ascii_lowercase().as_str() {
        "no_loop" => Some(LoopMode::NoLoop),
        "one_shot" => Some(LoopMode::OneShot),
        "loop_continuous" => Some(LoopMode::LoopContinuous),
        "loop_sustain" => Some(LoopMode::LoopSustain),
        _ => None,
    }
}

/// SFZ key opcodes accept either a MIDI number (`60`) or a note name (`c4`,
/// `a#0`, `Db3`). Note names use C4 = 60, matching the SFZ default octave
/// offset.
fn parse_key(value: &str) -> Option<u8> {
    if let Ok(number) = value.parse::<i32>() {
        return Some(number.clamp(0, 127) as u8);
    }

    let value = value.trim().to_ascii_lowercase();
    let mut chars = value.chars();
    let letter = chars.next()?;
    let semitone = match letter {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };

    let rest: String = chars.collect();
    let (accidental, octave_text) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('s') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        _ => (0, rest.as_str()),
    };

    let octave: i32 = octave_text.trim().parse().ok()?;
    let note = (octave + 1) * 12 + semitone + accidental;
    (0..=127).contains(&note).then_some(note as u8)
}

/// SFZ paths use Windows separators; normalize so they resolve on unix.
fn sample_to_path(sample: &str) -> PathBuf {
    PathBuf::from(sample.replace('\\', "/"))
}

// ---------------------------------------------------------------------------
// Thinning
// ---------------------------------------------------------------------------

/// Drop regions the options exclude, and thin the velocity layers.
fn filter_regions(
    regions: Vec<Region>,
    options: &PackLoadOptions,
    warnings: &mut Vec<String>,
) -> Vec<Region> {
    let before = regions.len();
    let mut regions: Vec<Region> = regions
        .into_iter()
        .filter(|region| region.seq_position <= 1)
        .filter(|region| options.include_release_zones || region.trigger == ZoneTrigger::Attack)
        .filter(|region| match options.key_range {
            None => true,
            Some((lo, hi)) => {
                let region_lo = region.lokey.or(region.pitch_keycenter).unwrap_or(0);
                let region_hi = region.hikey.or(region.pitch_keycenter).unwrap_or(127);
                region_hi >= lo && region_lo <= hi
            }
        })
        .collect();

    if let Some(target) = options.velocity_layers {
        let kept = thin_velocity_layers(&mut regions, target);
        if let Some((from, to)) = kept {
            warnings.push(format!("thinned {from} velocity layers down to {to}"));
        }
    }

    if regions.len() != before {
        warnings.push(format!(
            "imported {} of {before} declared regions",
            regions.len()
        ));
    }
    regions
}

/// Keep at most `target` velocity layers, spread evenly across the dynamic
/// range with the loudest always retained, then stretch the survivors' `lovel`
/// so the layers still tile 1..=127 without a dead band.
///
/// Returns `Some((before, after))` when thinning actually happened.
fn thin_velocity_layers(regions: &mut Vec<Region>, target: usize) -> Option<(usize, usize)> {
    if target == 0 {
        return None;
    }

    let mut layers: Vec<u8> = regions
        .iter()
        .filter(|r| r.trigger == ZoneTrigger::Attack)
        .map(|r| r.hivel)
        .collect();
    layers.sort_unstable();
    layers.dedup();
    if layers.len() <= target {
        return None;
    }

    let before = layers.len();
    // Pick `target` indices evenly across the sorted layer tops, always taking
    // the last one so full-velocity playing still reaches the loudest samples.
    let keep: Vec<u8> = (0..target)
        .map(|i| {
            let index = ((i * (before - 1)) as f64 / (target - 1).max(1) as f64).round() as usize;
            layers[index.min(before - 1)]
        })
        .collect();

    regions.retain(|r| r.trigger != ZoneTrigger::Attack || keep.contains(&r.hivel));

    // Re-tile: each surviving layer starts one above the previous layer's top.
    for region in regions.iter_mut() {
        if region.trigger != ZoneTrigger::Attack {
            continue;
        }
        let below = keep
            .iter()
            .filter(|&&top| top < region.hivel)
            .max()
            .copied();
        region.lovel = below.map_or(1, |top| top.saturating_add(1));
    }

    Some((before, keep.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn parse(text: &str) -> (Vec<Region>, Vec<String>) {
        let mut warnings = Vec::new();
        let regions = parse_regions(text, &mut warnings);
        (regions, warnings)
    }

    #[test]
    fn header_levels_inherit_outward_in() {
        let (regions, _) = parse(
            "<control> default_path=samples/\n\
             <global> ampeg_release=0.6\n\
             <group> hivel=80 volume=-3\n\
             <region> lokey=21 hikey=22 pitch_keycenter=21 sample=A0vL.wav\n\
             <region> lokey=23 hikey=24 pitch_keycenter=23 sample=B0vL.wav\n\
             <group> lovel=81 hivel=127\n\
             <region> lokey=21 hikey=22 pitch_keycenter=21 sample=A0vH.wav\n",
        );

        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].default_path, "samples/");
        assert_eq!(regions[0].sample.as_deref(), Some("A0vL.wav"));
        assert_eq!(regions[0].hivel, 80);
        assert_eq!(regions[0].volume_db, -3.0);
        assert!((regions[0].ampeg_release - 0.6).abs() < 1e-6);

        // The second group replaces the first group's opcodes but keeps global.
        assert_eq!(regions[2].lovel, 81);
        assert_eq!(regions[2].hivel, 127);
        assert_eq!(regions[2].volume_db, 0.0);
        assert!((regions[2].ampeg_release - 0.6).abs() < 1e-6);
        assert_eq!(regions[2].default_path, "samples/");
    }

    #[test]
    fn opcodes_split_correctly_when_values_contain_spaces() {
        let (regions, _) =
            parse("<region> lokey=60 sample=Grand Piano/C4 v3.wav hikey=62 pitch_keycenter=60\n");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].sample.as_deref(), Some("Grand Piano/C4 v3.wav"));
        assert_eq!(regions[0].lokey, Some(60));
        assert_eq!(regions[0].hikey, Some(62));
    }

    #[test]
    fn comments_and_multiline_regions_are_handled() {
        let (regions, _) = parse(
            "// a leading comment\n\
             <region>\n\
             lokey=60 hikey=60   // trailing comment\n\
             pitch_keycenter=60\n\
             sample=C4.wav\n",
        );
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].hikey, Some(60));
        assert_eq!(regions[0].sample.as_deref(), Some("C4.wav"));
    }

    #[test]
    fn note_names_and_numbers_both_parse() {
        assert_eq!(parse_key("60"), Some(60));
        assert_eq!(parse_key("c4"), Some(60));
        assert_eq!(parse_key("C4"), Some(60));
        assert_eq!(parse_key("a0"), Some(21));
        assert_eq!(parse_key("a#0"), Some(22));
        assert_eq!(parse_key("bb0"), Some(22));
        assert_eq!(parse_key("c8"), Some(108));
        assert_eq!(parse_key("h4"), None);
        assert_eq!(parse_key("200"), Some(127));
    }

    #[test]
    fn aria_directives_are_skipped_with_a_warning() {
        let (regions, warnings) = parse(
            "#define $VELTRACK 100\n\
             <control> set_hdcc1=0.5 default_path=samples/\n\
             <region> key=60 sample=C4.wav label_cc1=Tone\n",
        );
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].default_path, "samples/");
        assert!(warnings
            .iter()
            .any(|w| w.contains("preprocessor directive")));
        assert!(warnings.iter().any(|w| w.contains("unsupported opcodes")));
    }

    #[test]
    fn loop_and_trigger_opcodes_map_through() {
        let (regions, _) = parse(
            "<region> key=21 sample=A0.wav loop_mode=loop_continuous \
             loop_start=1000 loop_end=2000 trigger=release offset=64 end=3000\n",
        );
        let region = &regions[0];
        assert_eq!(region.loop_mode, Some(LoopMode::LoopContinuous));
        assert_eq!(region.loop_start, 1000);
        assert_eq!(region.loop_end, 2000);
        assert_eq!(region.trigger, ZoneTrigger::Release);
        assert_eq!(region.offset, 64);
        assert_eq!(region.end, Some(3000));
    }

    /// Build `layers` velocity layers across two keys, as a big pack would.
    fn layered_regions(layers: usize) -> Vec<Region> {
        let step = 127 / layers;
        (0..layers)
            .flat_map(|i| {
                let lovel = (i * step + 1) as u8;
                let hivel = if i == layers - 1 {
                    127
                } else {
                    ((i + 1) * step) as u8
                };
                [60_u8, 63].into_iter().map(move |key| {
                    let mut region = Region::new();
                    region.lokey = Some(key);
                    region.hikey = Some(key + 2);
                    region.pitch_keycenter = Some(key);
                    region.lovel = lovel;
                    region.hivel = hivel;
                    region.sample = Some(format!("k{key}v{i}.wav"));
                    region
                })
            })
            .collect()
    }

    #[test]
    fn sixteen_layers_thin_to_six_and_still_tile_the_velocity_range() {
        let mut regions = layered_regions(16);
        let result = thin_velocity_layers(&mut regions, 6);
        assert_eq!(result, Some((16, 6)));

        let mut tops: Vec<u8> = regions.iter().map(|r| r.hivel).collect();
        tops.sort_unstable();
        tops.dedup();
        assert_eq!(tops.len(), 6);
        assert_eq!(*tops.last().unwrap(), 127, "loudest layer must survive");

        // Every velocity 1..=127 must land in exactly one surviving layer.
        for velocity in 1..=127u8 {
            let hits = regions
                .iter()
                .filter(|r| r.lokey == Some(60) && velocity >= r.lovel && velocity <= r.hivel)
                .count();
            assert_eq!(hits, 1, "velocity {velocity} matched {hits} layers");
        }
    }

    #[test]
    fn thinning_is_a_no_op_when_the_pack_is_already_small() {
        let mut regions = layered_regions(2);
        assert_eq!(thin_velocity_layers(&mut regions, 6), None);
        assert_eq!(regions.len(), 4);
    }

    // --- End-to-end load over real WAV files --------------------------------

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gooey_multisample_pack_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path, frames: usize, value: f32) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        let sample = (value * i16::MAX as f32) as i16;
        for _ in 0..frames {
            writer.write_sample(sample).unwrap();
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn loads_a_pack_end_to_end_and_thins_it() {
        let dir = temp_dir();
        let samples = dir.join("samples");
        std::fs::create_dir_all(&samples).unwrap();

        // Four velocity layers tiling 1..=127 on one key; ask for two.
        const LAYER_TOPS: [u8; 4] = [32, 64, 96, 127];
        let mut sfz = String::from("<control> default_path=samples/\n<global> ampeg_release=0.5\n");
        let mut lovel = 1;
        for (layer, hivel) in LAYER_TOPS.into_iter().enumerate() {
            let name = format!("c4v{layer}.wav");
            write_wav(&samples.join(&name), 512, 0.2 * (layer + 1) as f32);
            sfz.push_str(&format!(
                "<region> lokey=59 hikey=61 pitch_keycenter=60 lovel={lovel} hivel={hivel} sample={name}\n",
            ));
            lovel = hivel + 1;
        }
        // A region pointing at a file that is not there must not fail the load.
        sfz.push_str("<region> key=62 sample=missing.wav\n");

        let sfz_path = dir.join("piano.sfz");
        std::fs::write(&sfz_path, sfz).unwrap();

        let options = PackLoadOptions::default().with_velocity_layers(Some(2));
        let pack = load_sfz(&sfz_path, &options).unwrap();

        assert_eq!(pack.regions_declared, 5);
        assert!(pack.warnings.iter().any(|w| w.contains("missing.wav")));
        assert!(pack.warnings.iter().any(|w| w.contains("thinned")));

        let map = pack.map.build();
        assert_eq!(map.velocity_layers(), 2, "thinned to two layers");
        assert_eq!(map.key_range(), Some((59, 61)));
        // The whole velocity range still resolves to a zone.
        for velocity in [1, 5, 64, 100, 127] {
            assert!(
                map.select(60, velocity, ZoneTrigger::Attack).is_some(),
                "velocity {velocity} found no zone"
            );
        }
        // `default_path` resolved, so the WAVs really were read.
        assert!(!map.zone(0).unwrap().buffer.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn max_seconds_shortens_buffers_and_cuts_memory() {
        let dir = temp_dir();
        let samples = dir.join("samples");
        std::fs::create_dir_all(&samples).unwrap();
        // Four seconds of audio per sample.
        write_wav(&samples.join("a.wav"), 44_100 * 4, 0.5);
        std::fs::write(
            dir.join("p.sfz"),
            "<control> default_path=samples/\n<region> key=60 sample=a.wav\n",
        )
        .unwrap();
        let sfz = dir.join("p.sfz");

        let whole = load_sfz(&sfz, &PackLoadOptions::default()).unwrap();
        let whole_map = whole.map.build();
        assert_eq!(whole_map.zone(0).unwrap().buffer.len(), 44_100 * 4);

        // Cap at one second: a quarter of the frames, a quarter of the memory.
        let opts = PackLoadOptions::default().with_max_seconds(Some(1.0));
        let cut = load_sfz(&sfz, &opts).unwrap();
        assert!(cut.warnings.iter().any(|w| w.contains("trimmed 1")));
        let cut_map = cut.map.build();
        let zone = cut_map.zone(0).unwrap();
        assert_eq!(zone.buffer.len(), 44_100);
        assert!(
            cut_map.memory_bytes() * 4 <= whole_map.memory_bytes() + 64,
            "memory should scale with the trim: {} vs {}",
            cut_map.memory_bytes(),
            whole_map.memory_bytes()
        );
        // A trimmed sample gets a real fade; an untrimmed one does not.
        assert!(zone.fade_out_frames > 1000, "{}", zone.fade_out_frames);
        assert_eq!(whole_map.zone(0).unwrap().fade_out_frames, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_sample_shorter_than_the_cap_is_left_untrimmed() {
        let dir = temp_dir();
        let samples = dir.join("samples");
        std::fs::create_dir_all(&samples).unwrap();
        write_wav(&samples.join("short.wav"), 4410, 0.5); // 0.1s
        std::fs::write(
            dir.join("p.sfz"),
            "<control> default_path=samples/\n<region> key=60 sample=short.wav\n",
        )
        .unwrap();

        let opts = PackLoadOptions::default().with_max_seconds(Some(5.0));
        let pack = load_sfz(dir.join("p.sfz"), &opts).unwrap();
        assert!(!pack.warnings.iter().any(|w| w.contains("trimmed")));
        let map = pack.map.build();
        assert_eq!(map.zone(0).unwrap().buffer.len(), 4410);
        assert_eq!(
            map.zone(0).unwrap().fade_out_frames,
            0,
            "a sample that ended naturally needs no trim fade"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sixteen_bit_packs_load_at_half_the_memory_of_float_ones() {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join("samples")).unwrap();
        write_wav(&dir.join("samples/a.wav"), 1000, 0.5);

        // Same audio, written as 32-bit float.
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(dir.join("samples/b.wav"), spec).unwrap();
        for _ in 0..1000 {
            w.write_sample(0.5f32).unwrap();
            w.write_sample(0.5f32).unwrap();
        }
        w.finalize().unwrap();

        std::fs::write(
            dir.join("p.sfz"),
            "<control> default_path=samples/\n\
             <region> key=60 sample=a.wav\n\
             <region> key=64 sample=b.wav\n",
        )
        .unwrap();

        let pack = load_sfz(dir.join("p.sfz"), &PackLoadOptions::everything()).unwrap();
        let map = pack.map.build();
        let compact = map.zone(0).unwrap();
        let wide = map.zone(1).unwrap();
        assert!(
            compact.buffer.is_compact(),
            "16-bit source should stay 16-bit"
        );
        assert!(!wide.buffer.is_compact(), "float source should stay float");
        assert_eq!(
            compact.buffer.memory_bytes() * 2,
            wide.buffer.memory_bytes()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pack_whose_samples_are_all_missing_is_an_error() {
        let dir = temp_dir();
        let sfz_path = dir.join("broken.sfz");
        std::fs::write(&sfz_path, "<region> key=60 sample=nope.wav\n").unwrap();
        let error = load_sfz(&sfz_path, &PackLoadOptions::default()).unwrap_err();
        assert!(error.contains("none could be loaded"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_sfz_reports_the_path() {
        let error = load_sfz("/definitely/not/here.sfz", &PackLoadOptions::default()).unwrap_err();
        assert!(error.contains("here.sfz"), "{error}");
    }
}
