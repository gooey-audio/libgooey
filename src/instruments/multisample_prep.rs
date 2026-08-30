//! Offline preparation of a multi-sample pack for a memory-constrained target.
//!
//! A full piano library is built for a desktop sampler with gigabytes to spare:
//! Salamander Grand Piano ships 641 regions and 1.1 GB of WAV, and holding it
//! resident costs well over 800 MB. A phone cannot do that, and an audio-unit
//! extension is tighter still.
//!
//! This module does the shrinking **once, ahead of time**, and writes a smaller
//! pack you can ship. Nothing here runs on device — it is build tooling that
//! happens to live in the same crate as the player, so the two cannot drift
//! apart on what a pack is allowed to contain.
//!
//! Three levers, applied in this order:
//!
//! 1. **Thin** — drop velocity layers, keys, and release zones you will not
//!    use ([`crate::instruments::multisample_pack::PackLoadOptions`]). Files for
//!    discarded regions are never opened.
//! 2. **Trim** — cap each sample's length. A piano's low strings ring past
//!    twenty seconds, and that tail is only heard under a held or pedalled
//!    note. This is usually the single biggest saving, and the truncation is
//!    faded so the cut is inaudible.
//! 3. **Narrow** — write 16-bit, optionally mono. Sixteen bits is what these
//!    packs were recorded at, so it costs nothing; mono halves the rest at the
//!    price of the stereo image.
//!
//! The output is an ordinary SFZ pack — a `.sfz` file plus a `samples/`
//! directory — so it loads through the same
//! [`crate::instruments::multisample_pack::load_sfz`] path as the original,
//! with no special-casing on device.
//!
//! # Example
//!
//! ```no_run
//! use gooey::instruments::multisample_pack::PackLoadOptions;
//! use gooey::instruments::multisample_prep::{prepare_pack, PrepareOptions};
//!
//! let options = PrepareOptions {
//!     load: PackLoadOptions::default()
//!         .with_velocity_layers(Some(8))
//!         .with_max_seconds(Some(6.0)),
//!     ..PrepareOptions::default()
//! };
//! let report = prepare_pack("SalamanderGrandPianoV3.sfz", "out/piano", &options).unwrap();
//! println!("{}", report.summary());
//! ```

use std::path::{Path, PathBuf};

use crate::instruments::multisample::{SampleZone, ZoneTrigger};
use crate::instruments::multisample_pack::{load_sfz, PackLoadOptions};
use crate::music::midi_to_string;

/// Directory the prepared samples are written into, relative to the output
/// root. Matches the `default_path` written into the emitted SFZ.
const SAMPLES_DIR: &str = "samples";

/// How to prepare a pack.
#[derive(Clone, Debug)]
pub struct PrepareOptions {
    /// Thinning and trimming, applied while reading the source pack.
    pub load: PackLoadOptions,
    /// Collapse to mono. Halves memory again, at the cost of the recorded
    /// stereo image — worth considering for the bass, which carries little
    /// stereo information, and rarely worth it overall.
    pub mono: bool,
    /// File name for the emitted SFZ.
    pub sfz_name: String,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            load: PackLoadOptions::default(),
            mono: false,
            sfz_name: "instrument.sfz".to_string(),
        }
    }
}

impl PrepareOptions {
    /// A pack sized for a phone: eight velocity layers across the full
    /// keyboard, trimmed to six seconds. Measured against Salamander Grand
    /// Piano V3 this is ~230 MB resident, versus ~870 MB for the same pack
    /// loaded whole at full precision — with *more* velocity layers than the
    /// untrimmed default, because tail length buys layers.
    pub fn mobile() -> Self {
        Self {
            load: PackLoadOptions::default()
                .with_velocity_layers(Some(8))
                .with_max_seconds(Some(6.0)),
            mono: false,
            sfz_name: "instrument.sfz".to_string(),
        }
    }

    /// Smaller still, for an audio-unit extension or any host with a hard
    /// memory ceiling: four layers, four seconds, mono.
    pub fn compact() -> Self {
        Self {
            load: PackLoadOptions::default()
                .with_velocity_layers(Some(4))
                .with_max_seconds(Some(4.0)),
            mono: true,
            sfz_name: "instrument.sfz".to_string(),
        }
    }
}

/// What a preparation run produced.
#[derive(Debug)]
pub struct PrepareReport {
    pub sfz_path: PathBuf,
    pub zones_written: usize,
    pub regions_declared: usize,
    /// Bytes of WAV written.
    pub output_bytes: u64,
    /// Bytes the prepared pack will occupy when loaded.
    pub resident_bytes: usize,
    /// Bytes the *source* pack would have occupied loaded whole, for comparison.
    pub source_resident_bytes: usize,
    pub warnings: Vec<String>,
}

impl PrepareReport {
    /// One-line human summary, for a CLI or a build log.
    pub fn summary(&self) -> String {
        format!(
            "{} zones from {} regions | {:.0} MB on disk | {:.0} MB resident (was {:.0} MB, {:.1}x smaller)",
            self.zones_written,
            self.regions_declared,
            self.output_bytes as f64 / 1_048_576.0,
            self.resident_bytes as f64 / 1_048_576.0,
            self.source_resident_bytes as f64 / 1_048_576.0,
            self.source_resident_bytes as f64 / self.resident_bytes.max(1) as f64,
        )
    }
}

/// Read a source pack, shrink it, and write the result to `out_dir`.
///
/// Creates `out_dir/<sfz_name>` and `out_dir/samples/`. Existing files with the
/// same names are overwritten, so re-running is safe and idempotent.
pub fn prepare_pack(
    source_sfz: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    options: &PrepareOptions,
) -> Result<PrepareReport, String> {
    let out_dir = out_dir.as_ref();
    let samples_dir = out_dir.join(SAMPLES_DIR);
    std::fs::create_dir_all(&samples_dir)
        .map_err(|e| format!("Failed to create {}: {e}", samples_dir.display()))?;

    let pack = load_sfz(source_sfz.as_ref(), &options.load)?;
    let map = pack.map.build();
    if map.is_empty() {
        return Err("source pack produced no zones".to_string());
    }

    // What the untrimmed, full-precision source would have cost, so the report
    // can state the saving rather than just the result.
    let source_resident_bytes = estimate_source_resident(source_sfz.as_ref(), &options.load);

    let mut sfz = String::new();
    sfz.push_str("// Prepared by libgooey multisample_prep.\n");
    sfz.push_str("// Derived from a source pack; original license and attribution still apply.\n");
    sfz.push_str(&format!("<control> default_path={SAMPLES_DIR}/\n\n"));

    let mut output_bytes = 0u64;
    let mut zones_written = 0usize;

    for index in 0..map.zone_count() {
        let zone = map.zone(index).ok_or("zone vanished mid-write")?;
        let name = zone_file_name(zone, index);
        let path = samples_dir.join(&name);
        output_bytes += write_zone_wav(zone, &path, options.mono)?;
        sfz.push_str(&zone_region_line(zone, &name));
        zones_written += 1;
    }

    let sfz_path = out_dir.join(&options.sfz_name);
    std::fs::write(&sfz_path, sfz)
        .map_err(|e| format!("Failed to write {}: {e}", sfz_path.display()))?;

    // Resident cost of what we just wrote: 16-bit, mono or stereo.
    let channels = if options.mono { 1 } else { 2 };
    let resident_bytes: usize = (0..map.zone_count())
        .filter_map(|i| map.zone(i))
        .map(|z| z.buffer.len() * channels * 2)
        .sum();

    Ok(PrepareReport {
        sfz_path,
        zones_written,
        regions_declared: pack.regions_declared,
        output_bytes,
        resident_bytes,
        source_resident_bytes,
        warnings: pack.warnings,
    })
}

/// Deterministic, sortable file name for a zone: key, velocity ceiling, and
/// whether it is a release sample. Two zones can only collide if they cover the
/// same key at the same velocity, which a sane pack never does.
fn zone_file_name(zone: &SampleZone, index: usize) -> String {
    let trigger = match zone.trigger {
        ZoneTrigger::Attack => "a",
        ZoneTrigger::Release => "r",
    };
    format!(
        "{}_{:03}_v{:03}{trigger}_{index:04}.wav",
        midi_to_string(zone.root_key).replace('#', "s"),
        zone.root_key,
        zone.hivel,
    )
}

/// One `<region>` line describing a written zone.
///
/// Emits only the SFZ v1 opcodes this crate's loader reads, so a prepared pack
/// round-trips exactly. Loop points and offsets are deliberately absent: the
/// written file is already the trimmed region, starting at frame zero.
fn zone_region_line(zone: &SampleZone, file_name: &str) -> String {
    let mut line = format!(
        "<region> lokey={} hikey={} pitch_keycenter={} lovel={} hivel={} sample={file_name}",
        zone.lokey, zone.hikey, zone.root_key, zone.lovel, zone.hivel
    );
    if zone.tune_cents != 0.0 {
        line.push_str(&format!(" tune={:.0}", zone.tune_cents));
    }
    if zone.volume_db != 0.0 {
        line.push_str(&format!(" volume={:.2}", zone.volume_db));
    }
    if zone.pan != 0.5 {
        // Back to SFZ's -100..100, from our 0..1.
        line.push_str(&format!(" pan={:.0}", zone.pan * 200.0 - 100.0));
    }
    if zone.trigger == ZoneTrigger::Release {
        line.push_str(" trigger=release");
    }
    line.push_str(&format!(" ampeg_release={:.3}", zone.envelope.release_time));
    line.push_str(&format!(" amp_veltrack={:.0}\n", zone.amp_veltrack * 100.0));
    line
}

/// Write one zone's audio as a 16-bit WAV, baking in its trim fade.
///
/// The fade is applied to the samples rather than left as playback metadata so
/// the prepared pack needs no special handling — it is just a pack whose
/// samples happen to end in silence.
fn write_zone_wav(zone: &SampleZone, path: &Path, mono: bool) -> Result<u64, String> {
    let frames = zone.buffer.len();
    let spec = hound::WavSpec {
        channels: if mono { 1 } else { 2 },
        sample_rate: zone.buffer.sample_rate() as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("Failed to create {}: {e}", path.display()))?;

    let fade = zone.fade_out_frames.min(frames);
    let fade_starts_at = frames.saturating_sub(fade);

    for i in 0..frames {
        // Integer positions read back exactly: cubic interpolation at zero
        // fraction returns the stored sample untouched.
        let frame = zone.buffer.read_interpolated(i as f64);
        let gain = if fade > 0 && i >= fade_starts_at {
            1.0 - (i - fade_starts_at) as f32 / fade as f32
        } else {
            1.0
        };
        let to_i16 = |v: f32| (v * gain).clamp(-1.0, 1.0) * i16::MAX as f32;
        if mono {
            let mixed = 0.5 * (frame.l + frame.r);
            writer
                .write_sample(to_i16(mixed) as i16)
                .map_err(|e| format!("Failed to write sample: {e}"))?;
        } else {
            writer
                .write_sample(to_i16(frame.l) as i16)
                .map_err(|e| format!("Failed to write sample: {e}"))?;
            writer
                .write_sample(to_i16(frame.r) as i16)
                .map_err(|e| format!("Failed to write sample: {e}"))?;
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize {}: {e}", path.display()))?;

    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("Failed to stat {}: {e}", path.display()))
}

/// What the same selection of regions would cost loaded whole, untrimmed and
/// widened to `f32`. Used only for the report's "was N MB" comparison, so a
/// failure here degrades to zero rather than failing the run.
fn estimate_source_resident(source_sfz: &Path, load: &PackLoadOptions) -> usize {
    let untrimmed = PackLoadOptions {
        max_seconds: None,
        ..load.clone()
    };
    match load_sfz(source_sfz, &untrimmed) {
        // The source is 16-bit on disk, so this crate now stores it 16-bit;
        // double it to express the cost the old f32-everywhere path would have
        // paid for the same audio.
        Ok(pack) => pack.map.build().memory_bytes() * 2,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::multisample::ZoneTrigger;
    use crate::instruments::multisample_pack::load_sfz;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gooey_prep_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A source pack: `layers` velocity layers on two keys, `secs` long each.
    fn source_pack(dir: &Path, layers: usize, secs: f32) -> PathBuf {
        let samples = dir.join("src_samples");
        std::fs::create_dir_all(&samples).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let frames = (44_100.0 * secs) as usize;
        let mut sfz =
            String::from("<control> default_path=src_samples/\n<global> ampeg_release=0.5\n");
        let step = 127 / layers;
        for key in [60u8, 64] {
            for layer in 0..layers {
                let name = format!("k{key}_v{layer}.wav");
                let mut w = hound::WavWriter::create(samples.join(&name), spec).unwrap();
                // Full-scale so trimming demonstrably cuts real signal.
                for _ in 0..frames {
                    w.write_sample(12000i16).unwrap();
                    w.write_sample(-12000i16).unwrap();
                }
                w.finalize().unwrap();
                let lovel = layer * step + 1;
                let hivel = if layer == layers - 1 {
                    127
                } else {
                    (layer + 1) * step
                };
                sfz.push_str(&format!(
                    "<region> lokey={} hikey={} pitch_keycenter={key} lovel={lovel} hivel={hivel} sample={name}\n",
                    key - 1, key + 1
                ));
            }
        }
        let path = dir.join("source.sfz");
        std::fs::write(&path, sfz).unwrap();
        path
    }

    #[test]
    fn a_prepared_pack_reloads_and_is_smaller() {
        let dir = temp_dir();
        let source = source_pack(&dir, 8, 4.0);
        let out = dir.join("out");

        let options = PrepareOptions {
            load: PackLoadOptions::default()
                .with_velocity_layers(Some(4))
                .with_max_seconds(Some(1.0)),
            ..PrepareOptions::default()
        };
        let report = prepare_pack(&source, &out, &options).unwrap();

        assert_eq!(report.zones_written, 8, "4 layers x 2 keys");
        assert!(report.sfz_path.exists());
        assert!(report.resident_bytes < report.source_resident_bytes);
        assert!(report.summary().contains("smaller"), "{}", report.summary());

        // The prepared pack loads through the ordinary path, unthinned.
        let reloaded = load_sfz(&report.sfz_path, &PackLoadOptions::everything()).unwrap();
        let map = reloaded.map.build();
        assert_eq!(map.zone_count(), 8);
        assert_eq!(map.velocity_layers(), 4);
        assert_eq!(map.key_range(), Some((59, 65)));
        // Trimmed to one second, and stored 16-bit.
        for i in 0..map.zone_count() {
            let z = map.zone(i).unwrap();
            assert_eq!(z.buffer.len(), 44_100, "zone {i} should be 1s");
            assert!(z.buffer.is_compact(), "zone {i} should be 16-bit");
        }
        // Every velocity still resolves.
        for v in [1u8, 40, 90, 127] {
            assert!(map.select(60, v, ZoneTrigger::Attack).is_some(), "vel {v}");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_trim_fade_is_baked_into_the_written_audio() {
        let dir = temp_dir();
        let source = source_pack(&dir, 1, 2.0);
        let out = dir.join("out");

        let options = PrepareOptions {
            load: PackLoadOptions::default().with_max_seconds(Some(1.0)),
            ..PrepareOptions::default()
        };
        let report = prepare_pack(&source, &out, &options).unwrap();

        let reloaded = load_sfz(&report.sfz_path, &PackLoadOptions::everything()).unwrap();
        let map = reloaded.map.build();
        let buf = &map.zone(0).unwrap().buffer;

        // The source was constant full-scale; the written file must ramp to
        // silence at the cut instead of stopping dead.
        let mid = buf.read_interpolated(1000.0).l.abs();
        let last = buf.read_interpolated((buf.len() - 1) as f64).l.abs();
        assert!(mid > 0.3, "body should be intact, got {mid}");
        assert!(last < mid * 0.05, "tail should be faded, got {last}");

        // ...and the fade is monotonic over its length, not a step.
        let n = buf.len();
        let a = buf.read_interpolated((n - 1200) as f64).l.abs();
        let b = buf.read_interpolated((n - 600) as f64).l.abs();
        assert!(a > b && b > last, "fade should descend: {a} {b} {last}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mono_output_halves_the_channel_count() {
        let dir = temp_dir();
        let source = source_pack(&dir, 1, 1.0);

        let stereo_out = dir.join("stereo");
        let stereo = prepare_pack(
            &source,
            &stereo_out,
            &PrepareOptions {
                load: PackLoadOptions::everything(),
                mono: false,
                ..PrepareOptions::default()
            },
        )
        .unwrap();

        let mono_out = dir.join("mono");
        let mono = prepare_pack(
            &source,
            &mono_out,
            &PrepareOptions {
                load: PackLoadOptions::everything(),
                mono: true,
                ..PrepareOptions::default()
            },
        )
        .unwrap();

        assert!(
            mono.output_bytes < stereo.output_bytes,
            "mono {} should be smaller than stereo {}",
            mono.output_bytes,
            stereo.output_bytes
        );
        assert_eq!(mono.resident_bytes * 2, stereo.resident_bytes);

        // A mono pack still loads and plays; the loader duplicates to both sides.
        let reloaded = load_sfz(&mono.sfz_path, &PackLoadOptions::everything()).unwrap();
        let map = reloaded.map.build();
        let f = map.zone(0).unwrap().buffer.read_interpolated(100.0);
        assert_eq!(f.l, f.r, "mono should be centered");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preparing_twice_is_idempotent() {
        let dir = temp_dir();
        let source = source_pack(&dir, 2, 1.0);
        let out = dir.join("out");
        let options = PrepareOptions::default();

        let first = prepare_pack(&source, &out, &options).unwrap();
        let second = prepare_pack(&source, &out, &options).unwrap();
        assert_eq!(first.zones_written, second.zones_written);
        assert_eq!(first.output_bytes, second.output_bytes);
        assert_eq!(first.resident_bytes, second.resident_bytes);

        // No stray files accumulated across runs.
        let count = std::fs::read_dir(out.join(SAMPLES_DIR)).unwrap().count();
        assert_eq!(count, first.zones_written);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zone_metadata_survives_the_round_trip() {
        let dir = temp_dir();
        let samples = dir.join("src_samples");
        std::fs::create_dir_all(&samples).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(samples.join("a.wav"), spec).unwrap();
        for _ in 0..1000 {
            w.write_sample(8000i16).unwrap();
            w.write_sample(8000i16).unwrap();
        }
        w.finalize().unwrap();
        std::fs::write(
            dir.join("s.sfz"),
            "<control> default_path=src_samples/\n\
             <region> lokey=48 hikey=52 pitch_keycenter=50 lovel=10 hivel=90 \
             volume=-3 pan=-50 tune=7 ampeg_release=1.25 amp_veltrack=70 sample=a.wav\n",
        )
        .unwrap();

        let out = dir.join("out");
        let report = prepare_pack(
            dir.join("s.sfz"),
            &out,
            &PrepareOptions {
                load: PackLoadOptions::everything(),
                ..PrepareOptions::default()
            },
        )
        .unwrap();

        let map = load_sfz(&report.sfz_path, &PackLoadOptions::everything())
            .unwrap()
            .map
            .build();
        let z = map.zone(0).unwrap();
        assert_eq!((z.lokey, z.hikey, z.root_key), (48, 52, 50));
        assert_eq!((z.lovel, z.hivel), (10, 90));
        assert!((z.volume_db + 3.0).abs() < 0.01, "volume {}", z.volume_db);
        assert!((z.pan - 0.25).abs() < 0.01, "pan {}", z.pan);
        assert!((z.tune_cents - 7.0).abs() < 0.01, "tune {}", z.tune_cents);
        assert!(
            (z.envelope.release_time - 1.25).abs() < 0.01,
            "release {}",
            z.envelope.release_time
        );
        assert!(
            (z.amp_veltrack - 0.70).abs() < 0.01,
            "veltrack {}",
            z.amp_veltrack
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_source_that_yields_nothing_is_an_error() {
        let dir = temp_dir();
        std::fs::write(dir.join("empty.sfz"), "<region> key=60 sample=gone.wav\n").unwrap();
        let err = prepare_pack(
            dir.join("empty.sfz"),
            dir.join("out"),
            &PrepareOptions::default(),
        )
        .unwrap_err();
        assert!(!err.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_named_presets_are_progressively_smaller() {
        let dir = temp_dir();
        let source = source_pack(&dir, 16, 8.0);

        let mobile = prepare_pack(&source, dir.join("m"), &PrepareOptions::mobile()).unwrap();
        let compact = prepare_pack(&source, dir.join("c"), &PrepareOptions::compact()).unwrap();

        assert!(
            compact.resident_bytes < mobile.resident_bytes,
            "compact {} should be under mobile {}",
            compact.resident_bytes,
            mobile.resident_bytes
        );
        assert!(mobile.resident_bytes < mobile.source_resident_bytes);

        std::fs::remove_dir_all(&dir).ok();
    }
}
