//! Shrink a multi-sample pack for shipping on a phone or in an audio-unit
//! extension.
//!
//! A full piano library is sized for a desktop sampler. Salamander Grand Piano
//! is 641 regions and 1.1 GB of WAV; loaded whole at full precision it costs
//! roughly 870 MB of RAM, which no phone will tolerate. This drops velocity
//! layers, caps how long each sample rings, and writes 16-bit — producing an
//! ordinary SFZ pack that loads through the normal path.
//!
//! Run with:
//!   cargo run --release --example prepare_piano_pack --features bounce -- \
//!       <source.sfz> <output-dir> [options]
//!
//! Options:
//!   --layers N        velocity layers to keep (default 8, 0 = all)
//!   --seconds S       cap each sample at S seconds (default 6, 0 = no cap)
//!   --keys LO-HI      restrict to a MIDI key range, e.g. 33-96
//!   --mono            collapse to mono, halving memory again
//!   --releases        include release (damper noise) samples
//!   --preset mobile   8 layers / 6s / stereo   (~230 MB for Salamander)
//!   --preset compact  4 layers / 4s / mono     (for tight memory ceilings)
//!
//! Example:
//!   cargo run --release --example prepare_piano_pack --features bounce -- \
//!       assets/piano/SalamanderGrandPianoV3_44.1khz16bit/SalamanderGrandPianoV3.sfz \
//!       assets/piano-mobile --preset mobile

#[cfg(feature = "bounce")]
use gooey::instruments::multisample_pack::PackLoadOptions;
#[cfg(feature = "bounce")]
use gooey::instruments::multisample_prep::{prepare_pack, PrepareOptions};

#[cfg(feature = "bounce")]
fn parse_args() -> Result<(String, String, PrepareOptions), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args[0].starts_with("--") {
        return Err("usage: prepare_piano_pack <source.sfz> <output-dir> [options]".to_string());
    }
    let source = args[0].clone();
    let out_dir = args[1].clone();

    let mut layers = Some(8usize);
    let mut seconds = Some(6.0f32);
    let mut keys = None;
    let mut mono = false;
    let mut releases = false;

    let mut i = 2;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--preset" => {
                match value()?.as_str() {
                    "mobile" => {
                        layers = Some(8);
                        seconds = Some(6.0);
                        mono = false;
                    }
                    "compact" => {
                        layers = Some(4);
                        seconds = Some(4.0);
                        mono = true;
                    }
                    other => return Err(format!("unknown preset '{other}'")),
                }
                i += 2;
            }
            "--layers" => {
                let n: usize = value()?.parse().map_err(|_| "--layers needs a number")?;
                layers = (n > 0).then_some(n);
                i += 2;
            }
            "--seconds" => {
                let s: f32 = value()?.parse().map_err(|_| "--seconds needs a number")?;
                seconds = (s > 0.0).then_some(s);
                i += 2;
            }
            "--keys" => {
                let raw = value()?;
                let (lo, hi) = raw
                    .split_once('-')
                    .ok_or("--keys wants LO-HI, e.g. 33-96")?;
                keys = Some((
                    lo.trim().parse::<u8>().map_err(|_| "bad low key")?,
                    hi.trim().parse::<u8>().map_err(|_| "bad high key")?,
                ));
                i += 2;
            }
            "--mono" => {
                mono = true;
                i += 1;
            }
            "--releases" => {
                releases = true;
                i += 1;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }

    let load = PackLoadOptions::default()
        .with_velocity_layers(layers)
        .with_max_seconds(seconds)
        .with_key_range(keys)
        .with_release_zones(releases);

    Ok((
        source,
        out_dir,
        PrepareOptions {
            load,
            mono,
            ..PrepareOptions::default()
        },
    ))
}

#[cfg(feature = "bounce")]
fn main() -> Result<(), String> {
    let (source, out_dir, options) = parse_args().inspect_err(|_| {
        eprintln!(
            "{}",
            include_str!("prepare_piano_pack.rs")
                .lines()
                .skip(2)
                .take_while(|l| l.starts_with("//!"))
                .map(|l| l.trim_start_matches("//!").trim_start())
                .collect::<Vec<_>>()
                .join("\n")
        );
    })?;

    println!("Reading {source}");
    println!(
        "  {} velocity layers, cap {}, {}",
        options
            .load
            .velocity_layers
            .map_or("all".to_string(), |n| n.to_string()),
        options
            .load
            .max_seconds
            .map_or("none".to_string(), |s| format!("{s}s")),
        if options.mono { "mono" } else { "stereo" },
    );
    if cfg!(debug_assertions) {
        println!("  (debug build — this is ~15x slower; add --release)");
    }

    let report = prepare_pack(&source, &out_dir, &options)?;

    println!("\n{}", report.summary());
    for warning in report.warnings.iter().take(6) {
        println!("  note: {warning}");
    }
    println!("\nWrote {}", report.sfz_path.display());
    println!(
        "  cargo run --release --example piano --features native,crossterm,bounce -- '{}'",
        report.sfz_path.display()
    );
    Ok(())
}

#[cfg(not(feature = "bounce"))]
fn main() {
    eprintln!("This example requires --features bounce");
}
